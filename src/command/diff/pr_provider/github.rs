//! GitHub provider: everything is driven through the `gh` CLI (GraphQL for PR
//! metadata and viewed-file state, the contents API for file blobs).

use std::collections::HashSet;
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use serde::Deserialize;
use spinoff::{spinners, Color, Spinner};

use crate::command::diff::git::build_file_diff;
use crate::command::diff::types::FileDiff;
use crate::command::diff::PrInfo;

use super::{
    decoded_path_segments, parse_http_url, strip_http_userinfo, HttpUrl, PrError, PrProvider,
};

/// Max concurrent `gh api` requests when fetching PR file contents.
/// GitHub's documented secondary rate limit caps concurrent requests at 100
/// (shared across REST+GraphQL); 8 keeps us comfortably under that while
/// still giving a large speedup over serial fetching.
const PR_FETCH_CONCURRENCY: usize = 8;

const VIEWED_FILES_QUERY: &str = r#"
query($owner: String!, $name: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      files(first: 100, after: $after) {
        nodes { path viewerViewedState }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
}
"#;

#[derive(Clone, Debug)]
pub(super) struct GitHubRepository {
    owner: String,
    repo: String,
}

impl GitHubRepository {
    pub(super) fn with_number(&self, number: u64) -> GitHubPrReference {
        GitHubPrReference {
            repository: self.clone(),
            number,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct GitHubPrReference {
    repository: GitHubRepository,
    number: u64,
}

pub(super) fn parse_pr_url(url: HttpUrl<'_>) -> Option<GitHubPrReference> {
    if !url.host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    let parts = decoded_path_segments(url.path)?;
    if parts.len() < 4 || parts.len() > 5 || parts[2] != "pull" {
        return None;
    }
    if parts.len() == 5 && parts[4] != "files" && parts[4] != "commits" {
        return None;
    }
    Some(GitHubPrReference {
        repository: GitHubRepository {
            owner: parts[0].clone(),
            repo: parts[1].clone(),
        },
        number: parts[3].parse().ok()?,
    })
}

pub(super) fn parse_repository(input: &str) -> Option<GitHubRepository> {
    let input = input.trim().trim_end_matches('/');
    let normalized = strip_http_userinfo(input);
    if let Some(url) = parse_http_url(normalized.as_ref()) {
        if !url.host.eq_ignore_ascii_case("github.com") {
            return None;
        }
        return repository_from_path(url.path);
    }

    if let Some(path) = input.strip_prefix("git@github.com:") {
        return repository_from_path(&format!("/{}", path));
    }
    if let Some(path) = input.strip_prefix("ssh://git@github.com/") {
        return repository_from_path(&format!("/{}", path));
    }

    repository_from_path(&format!("/{}", input))
}

fn repository_from_path(path: &str) -> Option<GitHubRepository> {
    let mut parts = decoded_path_segments(path)?;
    if parts.len() != 2 {
        return None;
    }
    let repo = parts.pop()?.trim_end_matches(".git").to_string();
    let owner = parts.pop()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(GitHubRepository { owner, repo })
}

/// The `gh api graphql` response envelope: `{ "data": { ... } }`.
#[derive(Deserialize)]
struct GraphQl<T> {
    data: Option<T>,
}

#[derive(Deserialize)]
struct RepoNode<T> {
    repository: Option<PullRequestNode<T>>,
}

#[derive(Deserialize)]
struct PullRequestNode<T> {
    #[serde(rename = "pullRequest")]
    pull_request: Option<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrNode {
    id: String,
    base_ref_name: Option<String>,
    head_ref_name: Option<String>,
    base_repository: Option<RepoOwner>,
    head_repository: Option<RepoOwner>,
}

#[derive(Deserialize)]
struct RepoOwner {
    owner: Owner,
}

#[derive(Deserialize)]
struct Owner {
    login: String,
}

pub(super) fn fetch_pr_info(reference: &GitHubPrReference) -> Result<PrInfo, PrError> {
    let number = reference.number;
    let repo_owner = reference.repository.owner.clone();
    let repo_name = reference.repository.repo.clone();

    // Use GraphQL to get the PR node ID, branch refs, and repo owners
    let query = format!(
        r#"query {{ repository(owner: "{}", name: "{}") {{ pullRequest(number: {}) {{ id url baseRefName headRefName baseRepository {{ owner {{ login }} }} headRepository {{ owner {{ login }} }} }} }} }}"#,
        repo_owner, repo_name, number
    );

    let output = Command::new("gh")
        .args(["api", "graphql", "-f", &format!("query={}", query)])
        .output()
        .map_err(|e| format!("Failed to run gh api graphql: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PrError::Other(format!(
            "gh api graphql failed: {}",
            stderr.trim()
        )));
    }

    let resp: GraphQl<RepoNode<PrNode>> = serde_json::from_slice(&output.stdout)
        .map_err(|e| PrError::Other(format!("could not parse gh graphql response: {}", e)))?;
    let pr = resp
        .data
        .and_then(|d| d.repository)
        .and_then(|r| r.pull_request)
        .ok_or_else(|| PrError::NotFound(format!("PR #{} not found", number)))?;

    Ok(PrInfo {
        provider: PrProvider::GitHub { node_id: pr.id },
        number,
        repo_owner: repo_owner.clone(),
        repo_name,
        base_ref: pr.base_ref_name.unwrap_or_else(|| "base".to_string()),
        head_ref: pr.head_ref_name.unwrap_or_else(|| "head".to_string()),
        base_repo_owner: pr
            .base_repository
            .map(|r| r.owner.login)
            .unwrap_or(repo_owner),
        head_repo_owner: pr.head_repository.map(|r| r.owner.login),
    })
}

pub(super) fn detect_current_branch_pr(
    repository: &GitHubRepository,
    branch: &str,
) -> Result<String, PrError> {
    let repo = format!("{}/{}", repository.owner, repository.repo);
    let output = Command::new("gh")
        .args([
            "pr", "view", branch, "--repo", &repo, "--json", "number", "-q", ".number",
        ])
        .output()
        .map_err(|e| format!("Failed to run gh: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr.trim();
        if msg.is_empty() {
            return Err(PrError::NotFound(
                "No PR found for the current branch".to_string(),
            ));
        }
        return Err(PrError::Other(msg.to_string()));
    }
    let number = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if number.is_empty() {
        return Err(PrError::NotFound(
            "No PR found for the current branch".to_string(),
        ));
    }
    Ok(number)
}

pub(super) fn file_web_url(pr: &PrInfo, filename: &str) -> String {
    format!(
        "https://github.com/{}/{}/pull/{}/files#diff-{}",
        pr.repo_owner,
        pr.repo_name,
        pr.number,
        file_anchor(filename)
    )
}

/// SHA-256 file anchor used by GitHub's PR "Files changed" deep links
/// (`#diff-<sha256(path)>`).
fn file_anchor(filename: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(filename.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[derive(Deserialize)]
struct PrFiles {
    files: FileConnection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileConnection {
    nodes: Vec<FileNode>,
    page_info: PageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileNode {
    path: String,
    viewer_viewed_state: String,
}

/// Fetch the list of files that are marked as viewed on GitHub
pub(super) fn fetch_viewed_files(pr_info: &PrInfo) -> Result<HashSet<String>, PrError> {
    fetch_all_viewed_files(|after| fetch_viewed_files_page(pr_info, after))
}

fn fetch_viewed_files_page(pr_info: &PrInfo, after: Option<&str>) -> Result<Vec<u8>, PrError> {
    let mut command = Command::new("gh");
    command
        .args(["api", "graphql"])
        .arg("-f")
        .arg(format!("query={VIEWED_FILES_QUERY}"))
        .arg("-f")
        .arg(format!("owner={}", pr_info.repo_owner))
        .arg("-f")
        .arg(format!("name={}", pr_info.repo_name))
        .arg("-F")
        .arg(format!("number={}", pr_info.number));
    if let Some(cursor) = after {
        command.arg("-f").arg(format!("after={cursor}"));
    }

    let output = command
        .output()
        .map_err(|e| format!("Failed to run gh api graphql: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PrError::Other(format!(
            "gh api graphql failed: {}",
            stderr.trim()
        )));
    }

    Ok(output.stdout)
}

fn fetch_all_viewed_files<F>(mut fetch_page: F) -> Result<HashSet<String>, PrError>
where
    F: FnMut(Option<&str>) -> Result<Vec<u8>, PrError>,
{
    let mut viewed_paths = HashSet::new();
    let mut cursor = None;

    loop {
        let body = fetch_page(cursor.as_deref())?;
        let next_cursor = accumulate_viewed_files_page(&body, &mut viewed_paths)?;
        if next_cursor.is_none() {
            return Ok(viewed_paths);
        }
        if next_cursor == cursor {
            return Err(PrError::Other(
                "github graphql returned a repeated file-page cursor".to_string(),
            ));
        }
        cursor = next_cursor;
    }
}

fn accumulate_viewed_files_page(
    body: &[u8],
    viewed_paths: &mut HashSet<String>,
) -> Result<Option<String>, PrError> {
    let resp: GraphQl<RepoNode<PrFiles>> = serde_json::from_slice(body)
        .map_err(|e| PrError::Other(format!("could not parse gh graphql response: {}", e)))?;
    let files = resp
        .data
        .and_then(|d| d.repository)
        .and_then(|r| r.pull_request)
        .map(|p| p.files)
        .ok_or_else(|| {
            PrError::NotFound(
                "github graphql response did not include pull request files".to_string(),
            )
        })?;

    viewed_paths.extend(
        files
            .nodes
            .into_iter()
            .filter(|node| node.viewer_viewed_state == "VIEWED")
            .map(|node| node.path),
    );

    if files.page_info.has_next_page {
        files.page_info.end_cursor.map(Some).ok_or_else(|| {
            PrError::Other("github graphql file page is missing its end cursor".to_string())
        })
    } else {
        Ok(None)
    }
}

/// Mark a file as viewed on GitHub PR (blocking)
fn mark_file_as_viewed_sync(node_id: &str, file_path: &str) -> Result<(), PrError> {
    let mutation = format!(
        r#"mutation {{ markFileAsViewed(input: {{ pullRequestId: "{}", path: "{}" }}) {{ clientMutationId }} }}"#,
        node_id, file_path
    );

    let output = Command::new("gh")
        .args(["api", "graphql", "-f", &format!("query={}", mutation)])
        .output()
        .map_err(|e| format!("Failed to run gh api graphql: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PrError::Other(stderr.trim().to_string()));
    }

    Ok(())
}

/// Unmark a file as viewed on GitHub PR (blocking)
fn unmark_file_as_viewed_sync(node_id: &str, file_path: &str) -> Result<(), PrError> {
    let mutation = format!(
        r#"mutation {{ unmarkFileAsViewed(input: {{ pullRequestId: "{}", path: "{}" }}) {{ clientMutationId }} }}"#,
        node_id, file_path
    );

    let output = Command::new("gh")
        .args(["api", "graphql", "-f", &format!("query={}", mutation)])
        .output()
        .map_err(|e| format!("Failed to run gh api graphql: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PrError::Other(stderr.trim().to_string()));
    }

    Ok(())
}

pub(super) fn set_file_viewed(node_id: &str, file_path: &str, viewed: bool) -> Result<(), PrError> {
    if viewed {
        mark_file_as_viewed_sync(node_id, file_path)
    } else {
        unmark_file_as_viewed_sync(node_id, file_path)
    }
}

pub(super) fn load_pr_file_diffs(pr_info: &PrInfo) -> Result<Vec<FileDiff>, PrError> {
    let repo_arg = format!("{}/{}", pr_info.repo_owner, pr_info.repo_name);

    let mut spinner = Spinner::new(
        spinners::Dots,
        format!(
            "Fetching file list for {}/{}#{}",
            pr_info.repo_owner, pr_info.repo_name, pr_info.number
        ),
        Color::Cyan,
    );

    let output = Command::new("gh")
        .args([
            "pr",
            "diff",
            &pr_info.number.to_string(),
            "--repo",
            &repo_arg,
        ])
        .output();

    let output = match output {
        Ok(o) => o,
        Err(e) => {
            let msg = format!("Failed to run gh pr diff: {}", e);
            spinner.fail(&msg);
            return Err(PrError::Other(msg));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = format!("gh pr diff failed: {}", stderr.trim());
        spinner.fail(&msg);
        return Err(PrError::Other(msg));
    }

    let diff_output = String::from_utf8_lossy(&output.stdout);
    let changed_files = parse_changed_files_from_diff(&diff_output);
    let n = changed_files.len();

    if n == 0 {
        spinner.success("PR has no changed files");
        return Ok(Vec::new());
    }

    let base_repo = format!("{}/{}", pr_info.base_repo_owner, pr_info.repo_name);
    let head_repo = pr_info
        .head_repo_owner
        .as_ref()
        .map(|owner| format!("{}/{}", owner, pr_info.repo_name))
        .unwrap_or_else(|| base_repo.clone());

    let contents = fetch_pr_file_contents_parallel(
        &changed_files,
        &base_repo,
        &pr_info.base_ref,
        &head_repo,
        &pr_info.head_ref,
        &mut spinner,
    );

    let file_diffs: Vec<FileDiff> = changed_files
        .into_iter()
        .zip(contents)
        .map(|(filename, (old_content, new_content))| {
            build_file_diff(filename, old_content, new_content)
        })
        .collect();

    spinner.success(&format!("Fetched {} files", n));
    Ok(file_diffs)
}

#[derive(Clone, Copy)]
enum Side {
    Old,
    New,
}

struct FetchTask {
    idx: usize,
    filename: String,
    repo: String,
    git_ref: String,
    side: Side,
}

enum FetchEvent {
    Started(String),
    Finished {
        idx: usize,
        side: Side,
        filename: String,
        content: String,
    },
}

/// Fetch (old, new) contents for every changed file using a bounded worker
/// pool, updating `spinner` with live progress.
fn fetch_pr_file_contents_parallel(
    files: &[String],
    base_repo: &str,
    base_ref: &str,
    head_repo: &str,
    head_ref: &str,
    spinner: &mut Spinner,
) -> Vec<(String, String)> {
    let n = files.len();
    let mut tasks: Vec<FetchTask> = Vec::with_capacity(2 * n);
    for (idx, filename) in files.iter().enumerate() {
        tasks.push(FetchTask {
            idx,
            filename: filename.clone(),
            repo: base_repo.to_string(),
            git_ref: base_ref.to_string(),
            side: Side::Old,
        });
        tasks.push(FetchTask {
            idx,
            filename: filename.clone(),
            repo: head_repo.to_string(),
            git_ref: head_ref.to_string(),
            side: Side::New,
        });
    }
    // Pop from the back, so process files in listed order.
    tasks.reverse();

    let total = tasks.len();
    let queue = Arc::new(Mutex::new(tasks));
    let (tx, rx) = mpsc::channel::<FetchEvent>();

    let worker_count = PR_FETCH_CONCURRENCY.min(total);
    let mut handles = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let queue = Arc::clone(&queue);
        let tx = tx.clone();
        handles.push(thread::spawn(move || loop {
            let task = { queue.lock().unwrap().pop() };
            let Some(task) = task else { break };
            let _ = tx.send(FetchEvent::Started(task.filename.clone()));
            let content = fetch_file_content_from_github(&task.repo, &task.git_ref, &task.filename);
            let _ = tx.send(FetchEvent::Finished {
                idx: task.idx,
                side: task.side,
                filename: task.filename,
                content,
            });
        }));
    }
    drop(tx);

    let mut contents: Vec<(String, String)> = vec![(String::new(), String::new()); n];
    let mut done = 0usize;
    let mut in_flight: Vec<String> = Vec::new();
    let mut last_finished: Option<String> = None;

    while let Ok(ev) = rx.recv() {
        match ev {
            FetchEvent::Started(name) => {
                in_flight.push(name);
            }
            FetchEvent::Finished {
                idx,
                side,
                filename,
                content,
            } => {
                if let Some(pos) = in_flight.iter().position(|f| f == &filename) {
                    in_flight.swap_remove(pos);
                }
                match side {
                    Side::Old => contents[idx].0 = content,
                    Side::New => contents[idx].1 = content,
                }
                done += 1;
                last_finished = Some(filename);
            }
        }
        spinner.update_text(format_fetch_progress(
            done,
            total,
            &in_flight,
            last_finished.as_deref(),
        ));
    }

    for h in handles {
        let _ = h.join();
    }

    contents
}

fn format_fetch_progress(
    done: usize,
    total: usize,
    in_flight: &[String],
    last_finished: Option<&str>,
) -> String {
    let current = if let Some(name) = in_flight.last() {
        name.as_str()
    } else {
        last_finished.unwrap_or_default()
    };
    if current.is_empty() {
        format!("Fetching files [{}/{}]", done, total)
    } else {
        format!("Fetching files [{}/{}] · {}", done, total, current)
    }
}

fn fetch_file_content_from_github(repo: &str, git_ref: &str, path: &str) -> String {
    let api_path = format!("repos/{}/contents/{}?ref={}", repo, path, git_ref);
    let output = Command::new("gh")
        .args([
            "api",
            &api_path,
            "-H",
            "Accept: application/vnd.github.raw+json",
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => String::new(),
    }
}

fn parse_changed_files_from_diff(diff: &str) -> Vec<String> {
    let mut files = Vec::new();

    for line in diff.lines() {
        if line.starts_with("diff --git") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let b_path = parts[3];
                if let Some(filename) = b_path.strip_prefix("b/") {
                    files.push(filename.to_string());
                } else {
                    files.push(b_path.to_string());
                }
            }
        }
    }

    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_exact_github_pull_urls() {
        let reference = parse_http_url("https://github.com/owner/repo/pull/123")
            .and_then(parse_pr_url)
            .expect("should parse");
        assert_eq!(reference.repository.owner, "owner");
        assert_eq!(reference.repository.repo, "repo");
        assert_eq!(reference.number, 123);
        assert!(parse_http_url("https://notgithub.com/owner/repo/pull/123")
            .and_then(parse_pr_url)
            .is_none());
        assert!(parse_http_url("https://github.com/owner/repo/issues/123")
            .and_then(parse_pr_url)
            .is_none());
        assert!(
            parse_http_url("https://github.com/owner/repo/pull/123/unknown")
                .and_then(parse_pr_url)
                .is_none()
        );
    }

    #[test]
    fn parses_github_pull_subpage_urls() {
        for url in [
            "https://github.com/owner/repo/pull/123/files",
            "https://github.com/owner/repo/pull/123/commits",
        ] {
            let reference = parse_http_url(url)
                .and_then(parse_pr_url)
                .expect("supported PR subpage");
            assert_eq!(reference.number, 123);
        }
    }

    #[test]
    fn parses_github_https_and_ssh_repositories() {
        let https = parse_repository("https://github.com/owner/repo.git").expect("https");
        let credentialed =
            parse_repository("https://user:TOKEN@github.com/owner/repo.git").expect("credentials");
        let ssh = parse_repository("git@github.com:owner/repo.git").expect("ssh");
        assert_eq!(https.owner, "owner");
        assert_eq!(https.repo, "repo");
        assert_eq!(credentialed.owner, "owner");
        assert_eq!(credentialed.repo, "repo");
        assert_eq!(ssh.owner, "owner");
        assert_eq!(ssh.repo, "repo");
    }

    #[test]
    fn parses_pr_info_graphql_with_deleted_head_fork() {
        let body = serde_json::json!({
            "data": { "repository": { "pullRequest": {
                "id": "PR_node1", "url": "https://github.com/o/r/pull/1",
                "baseRefName": "main", "headRefName": "feature",
                "baseRepository": { "owner": { "login": "base-owner" } },
                "headRepository": null
            }}}
        });
        let resp: GraphQl<RepoNode<PrNode>> = serde_json::from_value(body).unwrap();
        let pr = resp.data.unwrap().repository.unwrap().pull_request.unwrap();
        assert_eq!(pr.id, "PR_node1");
        assert_eq!(pr.base_ref_name.as_deref(), Some("main"));
        assert_eq!(pr.base_repository.unwrap().owner.login, "base-owner");
        assert!(pr.head_repository.is_none());
    }

    #[test]
    fn parses_viewed_state_graphql() {
        let body = serde_json::json!({
            "data": { "repository": { "pullRequest": { "files": { "nodes": [
                { "path": "a.rs", "viewerViewedState": "VIEWED" },
                { "path": "b.rs", "viewerViewedState": "UNVIEWED" }
            ], "pageInfo": { "hasNextPage": false, "endCursor": null }}}}}
        });
        let resp: GraphQl<RepoNode<PrFiles>> = serde_json::from_value(body).unwrap();
        let nodes = resp
            .data
            .unwrap()
            .repository
            .unwrap()
            .pull_request
            .unwrap()
            .files
            .nodes;
        assert_eq!(nodes[0].viewer_viewed_state, "VIEWED");
    }

    #[test]
    fn accumulates_viewed_files_across_graphql_pages() {
        let pages = [
            serde_json::json!({
                "data": { "repository": { "pullRequest": { "files": {
                    "nodes": [
                        { "path": "a.rs", "viewerViewedState": "VIEWED" },
                        { "path": "b.rs", "viewerViewedState": "UNVIEWED" }
                    ],
                    "pageInfo": { "hasNextPage": true, "endCursor": "cursor-1" }
                }}}}
            })
            .to_string()
            .into_bytes(),
            serde_json::json!({
                "data": { "repository": { "pullRequest": { "files": {
                    "nodes": [
                        { "path": "c.rs", "viewerViewedState": "VIEWED" }
                    ],
                    "pageInfo": { "hasNextPage": false, "endCursor": "cursor-2" }
                }}}}
            })
            .to_string()
            .into_bytes(),
        ];
        let mut pages = pages.into_iter();
        let mut cursors = Vec::new();

        let viewed = fetch_all_viewed_files(|cursor| {
            cursors.push(cursor.map(str::to_owned));
            Ok(pages.next().expect("requested page"))
        })
        .expect("all pages");

        assert_eq!(cursors, vec![None, Some("cursor-1".to_string())]);
        assert_eq!(
            viewed,
            HashSet::from(["a.rs".to_string(), "c.rs".to_string()])
        );
    }

    #[test]
    fn rejects_partial_graphql_page_without_next_cursor() {
        let page = serde_json::json!({
            "data": { "repository": { "pullRequest": { "files": {
                "nodes": [{ "path": "a.rs", "viewerViewedState": "VIEWED" }],
                "pageInfo": { "hasNextPage": true, "endCursor": null }
            }}}}
        })
        .to_string()
        .into_bytes();

        let error = fetch_all_viewed_files(|_| Ok(page.clone())).expect_err("missing cursor");

        assert!(error.to_string().contains("missing its end cursor"));
    }
}
