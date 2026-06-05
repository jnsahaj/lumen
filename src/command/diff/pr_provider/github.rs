//! GitHub provider: everything is driven through the `gh` CLI (GraphQL for PR
//! metadata and viewed-file state, the contents API for file blobs).

use std::collections::HashSet;
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use spinoff::{spinners, Color, Spinner};

use crate::command::diff::git::build_file_diff;
use crate::command::diff::types::FileDiff;
use crate::command::diff::PrInfo;

use super::{PrError, PrProvider, ProviderData};

/// Max concurrent `gh api` requests when fetching PR file contents.
/// GitHub's documented secondary rate limit caps concurrent requests at 100
/// (shared across REST+GraphQL); 8 keeps us comfortably under that while
/// still giving a large speedup over serial fetching.
const PR_FETCH_CONCURRENCY: usize = 8;

pub struct GitHubProvider;

impl PrProvider for GitHubProvider {
    fn matches_url(&self, input: &str) -> bool {
        input.starts_with("http") && input.contains("/pull/")
    }

    fn matches_origin(&self, origin: &str) -> bool {
        origin.contains("github.com")
    }

    fn fetch_pr_info(&self, input: &str, repo_override: Option<&str>) -> Result<PrInfo, PrError> {
        fetch_pr_info(input, repo_override)
    }

    fn detect_current_branch_pr(&self, _repo_override: Option<&str>) -> Result<String, PrError> {
        detect_current_branch_pr()
    }

    fn load_pr_file_diffs(&self, pr: &PrInfo) -> Result<Vec<FileDiff>, PrError> {
        load_pr_file_diffs(pr)
    }

    fn supports_viewed_sync(&self) -> bool {
        true
    }

    fn fetch_viewed_files(&self, pr: &PrInfo) -> Result<HashSet<String>, PrError> {
        fetch_viewed_files(pr)
    }

    fn set_file_viewed(&self, pr: &PrInfo, path: &str, viewed: bool) -> Result<(), PrError> {
        let ProviderData::GitHub { node_id } = &pr.data else {
            return Ok(()); // not a GitHub PR; nothing to sync
        };
        if viewed {
            mark_file_as_viewed_sync(node_id, path)
        } else {
            unmark_file_as_viewed_sync(node_id, path)
        }
    }

    fn file_web_url(&self, pr: &PrInfo, filename: &str) -> Option<String> {
        Some(format!(
            "https://github.com/{}/{}/pull/{}/files#diff-{}",
            pr.repo_owner,
            pr.repo_name,
            pr.number,
            file_anchor(filename)
        ))
    }
}

// ---------------------------------------------------------------------------
// PR metadata
// ---------------------------------------------------------------------------

fn parse_pr_input(input: &str) -> Option<(Option<String>, Option<String>, u64)> {
    // Try to parse as a URL first
    if input.starts_with("http://") || input.starts_with("https://") {
        // Extract PR number and repo info from URL
        // Format: https://github.com/owner/repo/pull/123
        let parts: Vec<&str> = input.trim_end_matches('/').split('/').collect();
        if parts.len() >= 2 {
            if let Some(pos) = parts.iter().position(|&p| p == "pull") {
                if pos + 1 < parts.len() {
                    if let Ok(num) = parts[pos + 1].parse::<u64>() {
                        // Extract owner and repo
                        if pos >= 2 {
                            let owner = parts[pos - 2].to_string();
                            let repo = parts[pos - 1].to_string();
                            return Some((Some(owner), Some(repo), num));
                        }
                        return Some((None, None, num));
                    }
                }
            }
        }
        None
    } else {
        // Try to parse as a PR number
        input.parse::<u64>().ok().map(|num| (None, None, num))
    }
}

fn resolve_origin_repo() -> Result<String, String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map_err(|e| format!("Failed to run git: {}", e))?;
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
        .ok_or_else(|| format!("Origin URL is not a GitHub URL: {}", url))?;
    let path = path.trim_start_matches(':').trim_start_matches('/');
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 2 {
        Ok(format!("{}/{}", parts[0], parts[1]))
    } else {
        Err(format!("Could not parse owner/repo from origin URL: {}", url))
    }
}

fn fetch_pr_info(pr_input: &str, repo_override: Option<&str>) -> Result<PrInfo, PrError> {
    let (owner, repo, number) = parse_pr_input(pr_input).ok_or_else(|| {
        PrError::InvalidRef(format!(
            "Invalid PR reference: {}. Use a PR number or URL.",
            pr_input
        ))
    })?;

    let repo_full = match (&owner, &repo, repo_override) {
        (Some(o), Some(r), _) => format!("{}/{}", o, r),
        (_, _, Some(r)) => r.to_string(),
        _ => resolve_origin_repo()?,
    };

    let (repo_owner, repo_name) = {
        let parts: Vec<&str> = repo_full.split('/').collect();
        if parts.len() != 2 {
            return Err(PrError::InvalidRef(format!("Invalid repo format: {}", repo_full)));
        }
        (
            owner.unwrap_or_else(|| parts[0].to_string()),
            repo.unwrap_or_else(|| parts[1].to_string()),
        )
    };

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
        return Err(PrError::Other(format!("gh api graphql failed: {}", stderr.trim())));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);

    // Parse the GraphQL response
    let node_id = extract_json_string(&json_str, "id")
        .ok_or_else(|| "Could not parse PR node ID from GraphQL response".to_string())?;
    let base_ref =
        extract_json_string(&json_str, "baseRefName").unwrap_or_else(|| "base".to_string());
    let head_ref =
        extract_json_string(&json_str, "headRefName").unwrap_or_else(|| "head".to_string());

    // Extract repo owners from nested structure
    let base_repo_owner =
        extract_nested_login(&json_str, "baseRepository").unwrap_or_else(|| repo_owner.clone());
    let head_repo_owner = extract_nested_login(&json_str, "headRepository");

    Ok(PrInfo {
        provider: &GitHubProvider,
        number,
        repo_owner,
        repo_name,
        base_ref,
        head_ref,
        base_repo_owner,
        head_repo_owner,
        data: ProviderData::GitHub { node_id },
    })
}

fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\":\"", key);
    if let Some(start) = json.find(&pattern) {
        let value_start = start + pattern.len();
        if let Some(end) = json[value_start..].find('"') {
            return Some(json[value_start..value_start + end].to_string());
        }
    }
    None
}

fn extract_nested_login(json: &str, parent_key: &str) -> Option<String> {
    // Look for pattern like "baseRepository":{"owner":{"login":"username"}}
    // or handle null case like "headRepository":null
    let pattern = format!("\"{}\":", parent_key);
    if let Some(start) = json.find(&pattern) {
        let after_key = &json[start + pattern.len()..];
        // Check if it's null
        if after_key.trim_start().starts_with("null") {
            return None;
        }
        // Look for login within this section
        if let Some(login_start) = after_key.find("\"login\":\"") {
            let value_start = login_start + 9;
            let after_login = &after_key[value_start..];
            if let Some(end) = after_login.find('"') {
                return Some(after_login[..end].to_string());
            }
        }
    }
    None
}

fn detect_current_branch_pr() -> Result<String, PrError> {
    let output = Command::new("gh")
        .args(["pr", "view", "--json", "number", "-q", ".number"])
        .output()
        .map_err(|e| format!("Failed to run gh: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr.trim();
        if msg.is_empty() {
            return Err(PrError::NotFound("No PR found for the current branch".to_string()));
        }
        return Err(PrError::Other(msg.to_string()));
    }
    let number = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if number.is_empty() {
        return Err(PrError::NotFound("No PR found for the current branch".to_string()));
    }
    Ok(number)
}

/// SHA-256 file anchor used by GitHub's PR "Files changed" deep links
/// (`#diff-<sha256(path)>`).
fn file_anchor(filename: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(filename.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// Viewed-file state
// ---------------------------------------------------------------------------

/// Fetch the list of files that are marked as viewed on GitHub
fn fetch_viewed_files(pr_info: &PrInfo) -> Result<HashSet<String>, PrError> {
    let query = format!(
        r#"query {{ repository(owner: "{}", name: "{}") {{ pullRequest(number: {}) {{ files(first: 100) {{ nodes {{ path viewerViewedState }} }} }} }} }}"#,
        pr_info.repo_owner, pr_info.repo_name, pr_info.number
    );

    let output = Command::new("gh")
        .args(["api", "graphql", "-f", &format!("query={}", query)])
        .output()
        .map_err(|e| format!("Failed to run gh api graphql: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PrError::Other(format!("gh api graphql failed: {}", stderr.trim())));
    }

    let json_str = String::from_utf8_lossy(&output.stdout);

    // Parse the response to find viewed files
    // Look for patterns like: "path":"filename","viewerViewedState":"VIEWED"
    let mut viewed_files = HashSet::new();

    // Simple parsing: find all path/viewerViewedState pairs
    let mut remaining = json_str.as_ref();
    while let Some(path_start) = remaining.find("\"path\":\"") {
        let path_value_start = path_start + 8;
        let after_path = &remaining[path_value_start..];
        if let Some(path_end) = after_path.find('"') {
            let path = &after_path[..path_end];

            // Look for viewerViewedState after this path
            let after_path_str = &after_path[path_end..];
            if let Some(state_start) = after_path_str.find("\"viewerViewedState\":\"") {
                let state_value_start = state_start + 21;
                let after_state = &after_path_str[state_value_start..];
                if let Some(state_end) = after_state.find('"') {
                    let state = &after_state[..state_end];
                    if state == "VIEWED" {
                        viewed_files.insert(path.to_string());
                    }
                }
            }

            remaining = &remaining[path_value_start + path_end..];
        } else {
            break;
        }
    }

    Ok(viewed_files)
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

// ---------------------------------------------------------------------------
// File diffs (gh pr diff + parallel contents fetch)
// ---------------------------------------------------------------------------

fn load_pr_file_diffs(pr_info: &PrInfo) -> Result<Vec<FileDiff>, PrError> {
    let repo_arg = format!("{}/{}", pr_info.repo_owner, pr_info.repo_name);

    let mut spinner = Spinner::new(
        spinners::Dots,
        format!(
            "Fetching file list for {}/{}#{}",
            pr_info.repo_owner, pr_info.repo_name, pr_info.number
        ),
        Color::Cyan,
    );

    // Get PR diff to find changed files
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
        .zip(contents.into_iter())
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
        spinner.update_text(format_fetch_progress(done, total, &in_flight, last_finished.as_deref()));
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
    } else if let Some(name) = last_finished {
        name
    } else {
        ""
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
    fn github_matches_pull_urls() {
        assert!(GitHubProvider.matches_url("https://github.com/owner/repo/pull/123"));
        assert!(!GitHubProvider.matches_url("https://dev.azure.com/o/p/_git/r/pullrequest/1"));
        assert!(GitHubProvider.matches_origin("git@github.com:owner/repo.git"));
    }
}
