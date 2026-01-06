mod app;
mod context;
mod diff_algo;
mod git;
pub mod highlight;
mod render;
mod search;
mod state;
mod sticky_lines;
pub mod theme;
mod types;
mod watcher;

use std::collections::HashSet;
use std::io;
use std::process::{self, Command};
use std::thread;

use crate::commit_reference::CommitReference;

pub struct DiffOptions {
    pub reference: Option<CommitReference>,
    pub pr: Option<String>,
    pub file: Option<Vec<String>>,
    pub watch: bool,
    pub theme: Option<String>,
}

#[derive(Clone)]
pub struct PrInfo {
    pub number: u64,
    pub node_id: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub base_ref: String,
    pub head_ref: String,
    pub base_repo_owner: String,
    pub head_repo_owner: Option<String>, // None if head repo was deleted (fork deleted)
}

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

fn fetch_pr_info(pr_input: &str) -> Result<PrInfo, String> {
    let (owner, repo, number) = parse_pr_input(pr_input).ok_or_else(|| {
        format!(
            "Invalid PR reference: {}. Use a PR number or URL.",
            pr_input
        )
    })?;

    // Build gh command with repo if available
    let repo_arg = match (&owner, &repo) {
        (Some(o), Some(r)) => Some(format!("{}/{}", o, r)),
        _ => None,
    };

    // Get PR URL, repo info, and node ID via GraphQL (more reliable for node_id)
    let repo_full = match &repo_arg {
        Some(r) => r.clone(),
        None => {
            // Get repo from current directory
            let output = Command::new("gh")
                .args([
                    "repo",
                    "view",
                    "--json",
                    "nameWithOwner",
                    "-q",
                    ".nameWithOwner",
                ])
                .output()
                .map_err(|e| format!("Failed to get current repo: {}", e))?;
            if !output.status.success() {
                return Err(
                    "Could not determine repository. Run from a git repo or specify --repo"
                        .to_string(),
                );
            }
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
    };

    let (repo_owner, repo_name) = {
        let parts: Vec<&str> = repo_full.split('/').collect();
        if parts.len() != 2 {
            return Err(format!("Invalid repo format: {}", repo_full));
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
        return Err(format!("gh api graphql failed: {}", stderr.trim()));
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
        number,
        node_id,
        repo_owner,
        repo_name,
        base_ref,
        head_ref,
        base_repo_owner,
        head_repo_owner,
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

/// Fetch the list of files that are marked as viewed on GitHub
pub fn fetch_viewed_files(pr_info: &PrInfo) -> Result<HashSet<String>, String> {
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
        return Err(format!("gh api graphql failed: {}", stderr.trim()));
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

/// Mark a file as viewed on GitHub PR (non-blocking, spawns a thread)
pub fn mark_file_as_viewed_async(pr_info: &PrInfo, file_path: &str) {
    let node_id = pr_info.node_id.clone();
    let path = file_path.to_string();

    thread::spawn(move || {
        let _ = mark_file_as_viewed_sync(&node_id, &path);
    });
}

/// Unmark a file as viewed on GitHub PR (non-blocking, spawns a thread)
pub fn unmark_file_as_viewed_async(pr_info: &PrInfo, file_path: &str) {
    let node_id = pr_info.node_id.clone();
    let path = file_path.to_string();

    thread::spawn(move || {
        let _ = unmark_file_as_viewed_sync(&node_id, &path);
    });
}

/// Mark a file as viewed on GitHub PR (blocking)
fn mark_file_as_viewed_sync(node_id: &str, file_path: &str) -> Result<(), String> {
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
        return Err(stderr.trim().to_string());
    }

    Ok(())
}

/// Unmark a file as viewed on GitHub PR (blocking)
fn unmark_file_as_viewed_sync(node_id: &str, file_path: &str) -> Result<(), String> {
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
        return Err(stderr.trim().to_string());
    }

    Ok(())
}

pub fn run_diff_ui(options: DiffOptions) -> io::Result<()> {
    // Handle PR mode
    if let Some(ref pr_input) = options.pr {
        match fetch_pr_info(pr_input) {
            Ok(pr_info) => {
                return app::run_app_with_pr(options, pr_info);
            }
            Err(e) => {
                eprintln!("\x1b[91merror:\x1b[0m {}", e);
                process::exit(1);
            }
        }
    }

    // Also check if the reference looks like a PR (number or URL)
    if let Some(CommitReference::Single(ref input)) = options.reference {
        if input.contains("/pull/") || input.parse::<u64>().is_ok() {
            match fetch_pr_info(input) {
                Ok(pr_info) => {
                    return app::run_app_with_pr(options, pr_info);
                }
                Err(_) => {
                    // Fall through to normal diff handling if it's not a valid PR
                }
            }
        }
    }

    app::run_app(options, None)
}
