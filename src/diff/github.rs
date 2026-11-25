use std::collections::HashSet;
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use serde::Deserialize;
use spinoff::{spinners, Color, Spinner};

use super::FileDiff;

/// Bounded concurrency for `gh api` content requests, comfortably below GitHub's
/// 100-request secondary rate limit.
const PR_FETCH_CONCURRENCY: usize = 8;

#[derive(Clone)]
pub struct PrInfo {
    pub number: u64,
    pub node_id: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub base_ref: String,
    pub head_ref: String,
    pub base_repo_owner: String,
    pub head_repo_owner: Option<String>,
}

pub fn parse_pr_input(input: &str) -> Option<(Option<String>, Option<String>, u64)> {
    if input.starts_with("http://") || input.starts_with("https://") {
        let parts: Vec<&str> = input.trim_end_matches('/').split('/').collect();
        let pos = parts.iter().position(|&p| p == "pull")?;
        let num = parts.get(pos + 1)?.parse::<u64>().ok()?;
        if pos >= 2 {
            Some((
                Some(parts[pos - 2].to_string()),
                Some(parts[pos - 1].to_string()),
                num,
            ))
        } else {
            Some((None, None, num))
        }
    } else {
        input.parse::<u64>().ok().map(|num| (None, None, num))
    }
}

pub fn resolve_origin_repo() -> Result<String, String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map_err(|e| format!("Failed to run git: {e}"))?;
    if !output.status.success() {
        return Err(
            "Could not determine repository. Set origin remote or use --origin owner/repo"
                .to_string(),
        );
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let url = url.strip_suffix(".git").unwrap_or(&url);
    let path = url
        .split("github.com")
        .nth(1)
        .ok_or_else(|| format!("Origin URL is not a GitHub URL: {url}"))?;
    let path = path.trim_start_matches(':').trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 2 {
        Ok(format!("{}/{}", parts[0], parts[1]))
    } else {
        Err(format!("Could not parse owner/repo from origin URL: {url}"))
    }
}

#[derive(Deserialize)]
struct GraphQlEnvelope<T> {
    data: Option<T>,
}

#[derive(Deserialize)]
struct RepoPullRequestData<T> {
    repository: Option<PullRequestHolder<T>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestHolder<T> {
    pull_request: Option<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrMetadataNode {
    id: String,
    base_ref_name: Option<String>,
    head_ref_name: Option<String>,
    base_repository: Option<RepoOwnerHolder>,
    head_repository: Option<RepoOwnerHolder>,
}

#[derive(Deserialize)]
struct RepoOwnerHolder {
    owner: OwnerLogin,
}

#[derive(Deserialize)]
struct OwnerLogin {
    login: String,
}

#[derive(Deserialize)]
struct PrFilesNode {
    files: Option<PrFileNodes>,
}

#[derive(Deserialize)]
struct PrFileNodes {
    nodes: Option<Vec<Option<PrFileEntry>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrFileEntry {
    path: String,
    viewer_viewed_state: String,
}

fn parse_pr_info_response(
    json_bytes: &[u8],
    repo_owner: String,
    repo_name: String,
    number: u64,
) -> Result<PrInfo, String> {
    let envelope: GraphQlEnvelope<RepoPullRequestData<PrMetadataNode>> =
        serde_json::from_slice(json_bytes)
            .map_err(|e| format!("Could not parse PR GraphQL response: {e}"))?;
    let pr = envelope
        .data
        .and_then(|d| d.repository)
        .and_then(|r| r.pull_request)
        .ok_or_else(|| "Pull request not found in GraphQL response".to_string())?;

    let base_repo_owner = pr
        .base_repository
        .map(|r| r.owner.login)
        .unwrap_or_else(|| repo_owner.clone());
    let head_repo_owner = pr.head_repository.map(|r| r.owner.login);

    Ok(PrInfo {
        number,
        node_id: pr.id,
        repo_owner,
        repo_name,
        base_ref: pr.base_ref_name.unwrap_or_else(|| "base".to_string()),
        head_ref: pr.head_ref_name.unwrap_or_else(|| "head".to_string()),
        base_repo_owner,
        head_repo_owner,
    })
}

fn parse_viewed_files_response(json_bytes: &[u8]) -> Result<HashSet<String>, String> {
    let envelope: GraphQlEnvelope<RepoPullRequestData<PrFilesNode>> =
        serde_json::from_slice(json_bytes)
            .map_err(|e| format!("Could not parse viewed files GraphQL response: {e}"))?;
    let nodes = envelope
        .data
        .and_then(|d| d.repository)
        .and_then(|r| r.pull_request)
        .and_then(|pr| pr.files)
        .and_then(|f| f.nodes)
        .unwrap_or_default();

    Ok(nodes
        .into_iter()
        .flatten()
        .filter(|entry| entry.viewer_viewed_state == "VIEWED")
        .map(|entry| entry.path)
        .collect())
}

pub fn fetch_pr_info(pr_input: &str, repo_override: Option<&str>) -> Result<PrInfo, String> {
    let (owner, repo, number) = parse_pr_input(pr_input)
        .ok_or_else(|| format!("Invalid PR reference: {pr_input}. Use a PR number or URL."))?;

    let repo_full = match (&owner, &repo, repo_override) {
        (Some(o), Some(r), _) => format!("{o}/{r}"),
        (_, _, Some(r)) => r.to_string(),
        _ => resolve_origin_repo()?,
    };

    let (repo_owner, repo_name) = {
        let parts: Vec<&str> = repo_full.split('/').collect();
        if parts.len() != 2 {
            return Err(format!("Invalid repo format: {repo_full}"));
        }
        (
            owner.unwrap_or_else(|| parts[0].to_string()),
            repo.unwrap_or_else(|| parts[1].to_string()),
        )
    };

    const QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { \
        repository(owner: $owner, name: $name) { \
            pullRequest(number: $number) { \
                id baseRefName headRefName \
                baseRepository { owner { login } } \
                headRepository { owner { login } } \
            } \
        } \
    }";

    let output = Command::new("gh")
        .args([
            "api",
            "graphql",
            "-f",
            &format!("query={QUERY}"),
            "-f",
            &format!("owner={repo_owner}"),
            "-f",
            &format!("name={repo_name}"),
            "-F",
            &format!("number={number}"),
        ])
        .output()
        .map_err(|e| format!("Failed to run gh api graphql: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api graphql failed: {}", stderr.trim()));
    }

    parse_pr_info_response(&output.stdout, repo_owner, repo_name, number)
}

pub fn fetch_viewed_files(pr_info: &PrInfo) -> Result<HashSet<String>, String> {
    const QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { \
        repository(owner: $owner, name: $name) { \
            pullRequest(number: $number) { \
                files(first: 100) { \
                    nodes { path viewerViewedState } \
                } \
            } \
        } \
    }";

    let output = Command::new("gh")
        .args([
            "api",
            "graphql",
            "-f",
            &format!("query={QUERY}"),
            "-f",
            &format!("owner={}", pr_info.repo_owner),
            "-f",
            &format!("name={}", pr_info.repo_name),
            "-F",
            &format!("number={}", pr_info.number),
        ])
        .output()
        .map_err(|e| format!("Failed to run gh api graphql: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api graphql failed: {}", stderr.trim()));
    }

    parse_viewed_files_response(&output.stdout)
}

pub fn mark_file_as_viewed_async(pr_info: &PrInfo, file_path: &str) {
    set_file_viewed_async(&pr_info.node_id, file_path, true);
}

pub fn unmark_file_as_viewed_async(pr_info: &PrInfo, file_path: &str) {
    set_file_viewed_async(&pr_info.node_id, file_path, false);
}

fn set_file_viewed_async(node_id: &str, file_path: &str, viewed: bool) {
    let node_id = node_id.to_string();
    let path = file_path.to_string();
    thread::spawn(move || {
        let _ = set_file_viewed_sync(&node_id, &path, viewed);
    });
}

fn set_file_viewed_sync(node_id: &str, file_path: &str, viewed: bool) -> Result<(), String> {
    let mutation = if viewed {
        "mutation($pullRequestId: ID!, $path: String!) { \
            markFileAsViewed(input: { pullRequestId: $pullRequestId, path: $path }) { \
                clientMutationId \
            } \
        }"
    } else {
        "mutation($pullRequestId: ID!, $path: String!) { \
            unmarkFileAsViewed(input: { pullRequestId: $pullRequestId, path: $path }) { \
                clientMutationId \
            } \
        }"
    };

    let output = Command::new("gh")
        .args([
            "api",
            "graphql",
            "-f",
            &format!("query={mutation}"),
            "-f",
            &format!("pullRequestId={node_id}"),
            "-f",
            &format!("path={file_path}"),
        ])
        .output()
        .map_err(|e| format!("Failed to run gh api graphql: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

pub fn detect_current_branch_pr() -> Result<String, String> {
    let output = Command::new("gh")
        .args(["pr", "view", "--json", "number", "-q", ".number"])
        .output()
        .map_err(|e| format!("Failed to run gh: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr.trim();
        return Err(if msg.is_empty() {
            "No PR found for the current branch".to_string()
        } else {
            msg.to_string()
        });
    }
    let number = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if number.is_empty() {
        return Err("No PR found for the current branch".to_string());
    }
    Ok(number)
}

pub fn load_pr_file_diffs(pr_info: &PrInfo) -> Result<Vec<FileDiff>, String> {
    let mut spinner = Spinner::new(
        spinners::Dots,
        format!(
            "Fetching file list for {}/{}#{}",
            pr_info.repo_owner, pr_info.repo_name, pr_info.number
        ),
        Color::Cyan,
    );
    load_pr_file_diffs_inner(pr_info, Some(&mut spinner))
}

/// Reloads PR diffs without writing terminal spinner frames (safe while Ratatui owns the screen).
pub fn load_pr_file_diffs_silent(pr_info: &PrInfo) -> Result<Vec<FileDiff>, String> {
    load_pr_file_diffs_inner(pr_info, None)
}

fn load_pr_file_diffs_inner(
    pr_info: &PrInfo,
    mut spinner: Option<&mut Spinner>,
) -> Result<Vec<FileDiff>, String> {
    let repo_arg = format!("{}/{}", pr_info.repo_owner, pr_info.repo_name);

    let output = match Command::new("gh")
        .args([
            "pr",
            "diff",
            &pr_info.number.to_string(),
            "--repo",
            &repo_arg,
        ])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            let msg = format!("Failed to run gh pr diff: {e}");
            if let Some(s) = spinner {
                s.clear();
            }
            return Err(msg);
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = format!("gh pr diff failed: {}", stderr.trim());
        if let Some(s) = spinner {
            s.clear();
        }
        return Err(msg);
    }

    let diff_output = String::from_utf8_lossy(&output.stdout);
    let changed_files = parse_changed_files_from_diff(&diff_output);
    let n = changed_files.len();

    if n == 0 {
        if let Some(s) = spinner {
            s.success("PR has no changed files");
        }
        return Ok(Vec::new());
    }

    let base_repo = format!("{}/{}", pr_info.base_repo_owner, pr_info.repo_name);
    let head_repo = pr_info
        .head_repo_owner
        .as_ref()
        .map(|owner| format!("{owner}/{}", pr_info.repo_name))
        .unwrap_or_else(|| base_repo.clone());

    let contents = fetch_pr_file_contents_parallel(
        &changed_files,
        &base_repo,
        &pr_info.base_ref,
        &head_repo,
        &pr_info.head_ref,
        spinner.as_deref_mut(),
    );

    let file_diffs = changed_files
        .into_iter()
        .zip(contents)
        .map(|(filename, (old_content, new_content))| {
            FileDiff::from_contents(filename, old_content, new_content)
        })
        .collect();

    if let Some(s) = spinner {
        s.success(&format!("Fetched {n} files"));
    }
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

fn fetch_pr_file_contents_parallel(
    files: &[String],
    base_repo: &str,
    base_ref: &str,
    head_repo: &str,
    head_ref: &str,
    mut spinner: Option<&mut Spinner>,
) -> Vec<(String, String)> {
    let n = files.len();
    let mut tasks = Vec::with_capacity(2 * n);
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
            let task = queue.lock().unwrap().pop();
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
            FetchEvent::Started(name) => in_flight.push(name),
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
        if let Some(s) = spinner.as_deref_mut() {
            s.update_text(format_fetch_progress(
                done,
                total,
                &in_flight,
                last_finished.as_deref(),
            ));
        }
    }

    for handle in handles {
        let _ = handle.join();
    }

    contents
}

fn format_fetch_progress(
    done: usize,
    total: usize,
    in_flight: &[String],
    last_finished: Option<&str>,
) -> String {
    let current = in_flight
        .last()
        .map(String::as_str)
        .or(last_finished)
        .unwrap_or_default();
    if current.is_empty() {
        format!("Fetching files [{done}/{total}]")
    } else {
        format!("Fetching files [{done}/{total}] · {current}")
    }
}

fn fetch_file_content_from_github(repo: &str, git_ref: &str, path: &str) -> String {
    let api_path = format!("repos/{repo}/contents/{path}?ref={git_ref}");
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
    diff.lines()
        .filter(|line| line.starts_with("diff --git"))
        .filter_map(|line| {
            let b_path = line.split_whitespace().nth(3)?;
            Some(b_path.strip_prefix("b/").unwrap_or(b_path).to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pr_input_number_and_url() {
        assert_eq!(parse_pr_input("123"), Some((None, None, 123)));
        assert_eq!(
            parse_pr_input("https://github.com/octocat/hello/pull/42/"),
            Some((Some("octocat".to_string()), Some("hello".to_string()), 42))
        );
        assert_eq!(parse_pr_input("not-a-pr"), None);
    }

    #[test]
    fn test_parse_pr_info_response_handles_forks_and_deleted_head() {
        let json = br#"{
            "data": {
                "repository": {
                    "pullRequest": {
                        "id": "PR_kwDO123",
                        "baseRefName": "main",
                        "headRefName": "feat/auth",
                        "baseRepository": { "owner": { "login": "upstream-org" } },
                        "headRepository": null
                    }
                }
            }
        }"#;

        let info =
            parse_pr_info_response(json, "upstream-org".into(), "repo".into(), 99).expect("parsed");
        assert_eq!(info.node_id, "PR_kwDO123");
        assert_eq!(info.base_ref, "main");
        assert_eq!(info.head_ref, "feat/auth");
        assert_eq!(info.base_repo_owner, "upstream-org");
        assert_eq!(info.head_repo_owner, None);
    }

    #[test]
    fn test_parse_viewed_files_response_filters_only_viewed() {
        let json = br#"{
            "data": {
                "repository": {
                    "pullRequest": {
                        "files": {
                            "nodes": [
                                { "path": "src/a.rs", "viewerViewedState": "VIEWED" },
                                { "path": "src/b.rs", "viewerViewedState": "UNVIEWED" },
                                { "path": "docs/\"quoted\".md", "viewerViewedState": "VIEWED" }
                            ]
                        }
                    }
                }
            }
        }"#;

        let viewed = parse_viewed_files_response(json).expect("parsed");
        assert_eq!(
            viewed,
            HashSet::from(["src/a.rs".to_string(), "docs/\"quoted\".md".to_string()])
        );
    }
}
