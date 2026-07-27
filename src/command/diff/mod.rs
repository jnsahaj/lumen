mod annotation;
mod app;
mod context;
mod coordinates;
mod diff_algo;
pub mod git;
mod global_search;
pub mod highlight;
mod render;
mod search;
mod state;
mod sticky_lines;
mod text_edit;
pub mod theme;
mod types;
mod watcher;

use std::collections::HashSet;
use std::io;
use std::process::{self, Command};
use std::sync::Arc;
use std::thread;

use spinoff::{spinners, Color, Spinner};

use crate::commit_reference::CommitReference;
use crate::provider::LumenProvider;
use crate::vcs::VcsBackend;

pub struct DiffOptions {
    pub reference: Option<CommitReference>,
    pub pr: Option<String>,
    pub detect_pr: bool,
    pub file: Option<Vec<String>>,
    pub watch: bool,
    pub theme: Option<String>,
    pub stacked: bool,
    pub focus: Option<String>,
    pub origin: Option<String>,
    pub wrap: bool,
    pub guide: bool,
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
        Err(format!(
            "Could not parse owner/repo from origin URL: {}",
            url
        ))
    }
}

fn fetch_pr_info(pr_input: &str, repo_override: Option<&str>) -> Result<PrInfo, String> {
    let (owner, repo, number) = parse_pr_input(pr_input).ok_or_else(|| {
        format!(
            "Invalid PR reference: {}. Use a PR number or URL.",
            pr_input
        )
    })?;

    let repo_full = match (&owner, &repo, repo_override) {
        (Some(o), Some(r), _) => format!("{}/{}", o, r),
        (_, _, Some(r)) => r.to_string(),
        _ => resolve_origin_repo()?,
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

/// Concatenate the non-binary files in `file_diffs` into a single unified
/// diff, as if `git diff` had produced them all in one invocation. Used to
/// feed the whole working-tree diff to the model for grouped summarization.
pub fn combined_unified_diff(file_diffs: &[types::FileDiff]) -> String {
    let mut combined = String::new();

    for file_diff in file_diffs {
        if file_diff.is_binary {
            continue;
        }

        combined.push_str(&format!(
            "diff --git a/{path} b/{path}\n",
            path = file_diff.filename
        ));

        let a_path = format!("a/{}", file_diff.filename);
        let b_path = format!("b/{}", file_diff.filename);
        let body = similar::TextDiff::from_lines(&file_diff.old_content, &file_diff.new_content)
            .unified_diff()
            .header(&a_path, &b_path)
            .to_string();
        combined.push_str(&body);
    }

    combined
}

/// A single grouped-summary request: the diff identity it's keyed against
/// (see `current_diff_identity` in `app.rs`), the combined diff text to send
/// to the model, and the ground-truth file list used to reconcile the
/// model's grouping against what actually changed.
pub struct GenerateRequest {
    pub identity: String,
    pub combined_diff: String,
    pub ground_truth: Vec<String>,
}

/// Result of a `GenerateRequest`, tagged with the identity it was requested
/// for so the receiver can route it back to the right cache entry even if
/// the user has since navigated to a different diff.
pub type GroupResult = (
    String,
    Result<crate::grouped_summary::GroupedSummary, String>,
);

/// Spawn the single background grouping worker for the lifetime of the TUI
/// session. Owns `provider` and a Tokio `handle` captured on a runtime
/// thread, loops on `req_rx` until the sender side is dropped (TUI exit),
/// and reports `(identity, result)` back over `res_tx` for each request in
/// turn. Requests are processed serially — the previous per-keypress thread
/// spawning is replaced by this persistent worker so overlapping requests
/// can't race each other.
///
/// `handle` must be captured on a thread that is already inside a Tokio
/// runtime context (e.g. via `tokio::runtime::Handle::current()` from the
/// synchronous TUI loop, which itself runs on a Tokio worker thread). A
/// plain `std::thread::spawn` closure has no ambient runtime, so calling
/// `Handle::current()` *inside* the spawned thread panics with "there is no
/// reactor running" — the handle must be captured outside and moved in.
pub fn spawn_group_worker(
    provider: Arc<LumenProvider>,
    handle: tokio::runtime::Handle,
    req_rx: std::sync::mpsc::Receiver<GenerateRequest>,
    res_tx: std::sync::mpsc::Sender<GroupResult>,
) {
    thread::spawn(move || {
        for req in req_rx {
            let GenerateRequest {
                identity,
                combined_diff,
                ground_truth,
            } = req;
            let result = handle.block_on(async {
                let cmd = crate::command::explain::ExplainCommand {
                    git_entity: crate::git_entity::GitEntity::Diff(
                        crate::git_entity::diff::Diff::WorkingTree {
                            staged: false,
                            diff: combined_diff,
                        },
                    ),
                    query: None,
                    grouped: true,
                };
                provider.explain_grouped(&cmd).await
            });

            let parsed = result
                .map_err(|e| e.to_string())
                .and_then(|raw| {
                    crate::grouped_summary::parse_grouped_summary(&raw).map_err(|e| e.to_string())
                })
                .map(|mut summary| {
                    crate::grouped_summary::reconcile_groups(&mut summary, &ground_truth);
                    summary
                });

            let _ = res_tx.send((identity, parsed));
        }
    });
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

fn detect_current_branch_pr() -> Result<String, String> {
    let output = Command::new("gh")
        .args(["pr", "view", "--json", "number", "-q", ".number"])
        .output()
        .map_err(|e| format!("Failed to run gh: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr.trim();
        if msg.is_empty() {
            return Err("No PR found for the current branch".to_string());
        }
        return Err(msg.to_string());
    }
    let number = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if number.is_empty() {
        return Err("No PR found for the current branch".to_string());
    }
    Ok(number)
}

pub fn run_diff_ui(
    mut options: DiffOptions,
    backend: &dyn VcsBackend,
    provider: Arc<LumenProvider>,
) -> io::Result<()> {
    // Spawn the single grouping worker up front and hand every dispatch
    // branch below its own request-sender clone plus the (single) result
    // receiver. `provider` is moved into the worker here — it's no longer
    // threaded down into `app::run_app*`.
    let handle = tokio::runtime::Handle::current();
    let (req_tx, req_rx) = std::sync::mpsc::channel::<GenerateRequest>();
    let (res_tx, res_rx) = std::sync::mpsc::channel::<GroupResult>();
    spawn_group_worker(provider, handle, req_rx, res_tx);

    // Resolve --detect-pr into options.pr
    if options.detect_pr && options.pr.is_none() {
        let mut spinner = Spinner::new(
            spinners::Dots,
            "Detecting PR for current branch",
            Color::Cyan,
        );
        match detect_current_branch_pr() {
            Ok(number) => {
                spinner.success(&format!("Detected PR #{}", number));
                options.pr = Some(number);
            }
            Err(e) => {
                spinner.fail(&e);
                process::exit(1);
            }
        }
    }

    // Handle PR mode
    if let Some(ref pr_input) = options.pr {
        let spinner_msg = match parse_pr_input(pr_input) {
            Some((Some(owner), Some(repo), number)) => {
                format!("Fetching PR {}/{}#{}", owner, repo, number)
            }
            Some((_, _, number)) => {
                format!("Fetching PR #{}", number)
            }
            None => "Fetching PR".to_string(),
        };
        let mut spinner = Spinner::new(spinners::Dots, spinner_msg, Color::Cyan);
        match fetch_pr_info(pr_input, options.origin.as_deref()) {
            Ok(pr_info) => {
                spinner.success("Fetched PR metadata");
                return app::run_app_with_pr(options, pr_info, backend, req_tx.clone(), res_rx);
            }
            Err(e) => {
                spinner.fail(&e);
                process::exit(1);
            }
        }
    }

    // Also check if the reference looks like a PR (number or URL)
    if let Some(CommitReference::Single(ref input)) = options.reference {
        if input.contains("/pull/") || input.parse::<u64>().is_ok() {
            let spinner_msg = match parse_pr_input(input) {
                Some((Some(owner), Some(repo), number)) => {
                    format!("Fetching PR {}/{}#{}", owner, repo, number)
                }
                Some((_, _, number)) => {
                    format!("Fetching PR #{}", number)
                }
                None => "Fetching PR".to_string(),
            };
            let mut spinner = Spinner::new(spinners::Dots, spinner_msg, Color::Cyan);
            match fetch_pr_info(input, options.origin.as_deref()) {
                Ok(pr_info) => {
                    spinner.success("Fetched PR metadata");
                    return app::run_app_with_pr(options, pr_info, backend, req_tx.clone(), res_rx);
                }
                Err(e) => {
                    spinner.fail(&e);
                    process::exit(1);
                }
            }
        }
    }

    // Handle stacked mode for range references
    if options.stacked {
        if let Some(ref reference) = options.reference {
            let (from, to) = match reference {
                CommitReference::Range { from, to } => (from.clone(), to.clone()),
                CommitReference::TripleDots { from, to } => {
                    // Get merge-base for triple dots
                    let merge_base = backend
                        .get_merge_base(from, to)
                        .unwrap_or_else(|_| from.clone());
                    (merge_base, to.clone())
                }
                CommitReference::Single(_) | CommitReference::RangeToWorkingTree { .. } => {
                    eprintln!(
                        "\x1b[91merror:\x1b[0m --stacked requires a range (e.g., main..feature)"
                    );
                    process::exit(1);
                }
            };

            let commits = match backend.get_commits_in_range(&from, &to) {
                Ok(c) if c.is_empty() => {
                    eprintln!(
                        "\x1b[91merror:\x1b[0m No commits found in range {}..{}",
                        from, to
                    );
                    process::exit(1);
                }
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\x1b[91merror:\x1b[0m {}", e);
                    process::exit(1);
                }
            };

            return app::run_app_stacked(options, commits, backend, req_tx.clone(), res_rx);
        } else {
            eprintln!("\x1b[91merror:\x1b[0m --stacked requires a range (e.g., main..feature)");
            process::exit(1);
        }
    }

    app::run_app(options, None, backend, req_tx, res_rx)
}

#[cfg(test)]
mod tests {
    use super::types::{FileDiff, FileStatus};
    use super::*;

    #[test]
    fn combined_unified_diff_emits_a_diff_git_header_per_file() {
        let file_diffs = vec![FileDiff {
            filename: "src/a.rs".to_string(),
            old_content: "old\n".to_string(),
            new_content: "new\n".to_string(),
            status: FileStatus::Modified,
            is_binary: false,
        }];

        let combined = combined_unified_diff(&file_diffs);

        assert!(combined.contains("diff --git a/src/a.rs b/src/a.rs\n"));
        assert!(combined.contains("-old\n"));
        assert!(combined.contains("+new\n"));
    }

    #[test]
    fn combined_unified_diff_concatenates_headers_for_multiple_files_in_order() {
        let file_diffs = vec![
            FileDiff {
                filename: "src/a.rs".to_string(),
                old_content: "old a\n".to_string(),
                new_content: "new a\n".to_string(),
                status: FileStatus::Modified,
                is_binary: false,
            },
            FileDiff {
                filename: "src/b.rs".to_string(),
                old_content: "old b\n".to_string(),
                new_content: "new b\n".to_string(),
                status: FileStatus::Modified,
                is_binary: false,
            },
        ];

        let combined = combined_unified_diff(&file_diffs);

        let a_pos = combined
            .find("diff --git a/src/a.rs b/src/a.rs\n")
            .expect("missing header for a.rs");
        let b_pos = combined
            .find("diff --git a/src/b.rs b/src/b.rs\n")
            .expect("missing header for b.rs");
        assert!(a_pos < b_pos);
    }

    #[test]
    fn combined_unified_diff_skips_binary_files() {
        let file_diffs = vec![
            FileDiff {
                filename: "image.png".to_string(),
                old_content: String::new(),
                new_content: String::new(),
                status: FileStatus::Added,
                is_binary: true,
            },
            FileDiff {
                filename: "src/a.rs".to_string(),
                old_content: "old\n".to_string(),
                new_content: "new\n".to_string(),
                status: FileStatus::Modified,
                is_binary: false,
            },
        ];

        let combined = combined_unified_diff(&file_diffs);

        assert!(!combined.contains("image.png"));
        assert!(combined.contains("diff --git a/src/a.rs b/src/a.rs\n"));
    }
}
