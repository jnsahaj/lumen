use std::fs;
use std::process::Command;

use super::types::{FileDiff, FileStatus};
use super::{DiffOptions, PrInfo};
use crate::commit_reference::CommitReference;
use crate::vcs::VcsBackend;

/// Information about a single commit for stacked diff navigation
#[derive(Clone)]
pub struct CommitInfo {
    pub sha: String,
    pub short_sha: String,
    pub message: String,
}

pub fn get_current_branch(backend: &dyn VcsBackend) -> String {
    backend
        .get_current_branch()
        .ok()
        .flatten()
        .unwrap_or_else(|| "unknown".to_string())
}

/// Resolved references for diff comparison
pub enum DiffRefs {
    /// Uncommitted changes (working tree vs HEAD)
    WorkingTree,
    /// Single commit (SHA vs SHA^)
    Single(String),
    /// Range between two refs
    Range { from: String, to: String },
}

impl DiffRefs {
    pub fn from_options(options: &DiffOptions, backend: &dyn VcsBackend) -> Self {
        match &options.reference {
            None => DiffRefs::WorkingTree,
            Some(CommitReference::Single(sha)) => DiffRefs::Single(sha.clone()),
            Some(CommitReference::Range { from, to }) => DiffRefs::Range {
                from: from.clone(),
                to: to.clone(),
            },
            Some(CommitReference::TripleDots { from, to }) => {
                // Get merge-base for triple dots
                let merge_base = backend
                    .get_merge_base(from, to)
                    .unwrap_or_else(|_| from.clone());
                DiffRefs::Range {
                    from: merge_base,
                    to: to.clone(),
                }
            }
        }
    }
}

/// Get the list of files changed
pub fn get_changed_files(options: &DiffOptions, backend: &dyn VcsBackend) -> Vec<String> {
    let refs = DiffRefs::from_options(options, backend);

    let files: Vec<String> = match refs {
        DiffRefs::Single(sha) => backend.get_changed_files(&sha).unwrap_or_default(),
        DiffRefs::Range { from, to } => backend
            .get_range_changed_files(&from, &to)
            .unwrap_or_default(),
        DiffRefs::WorkingTree => backend.get_working_tree_changed_files().unwrap_or_default(),
    };

    if let Some(ref filter) = options.file {
        files.into_iter().filter(|f| filter.contains(f)).collect()
    } else {
        files
    }
}

/// Get content of a file at the "old" side of the diff
pub fn get_old_content(filename: &str, refs: &DiffRefs, backend: &dyn VcsBackend) -> String {
    use std::path::Path;

    let ref_str = match refs {
        DiffRefs::Single(sha) => format!("{}^", sha), // Parent of commit
        DiffRefs::Range { from, .. } => from.clone(),
        DiffRefs::WorkingTree => backend.working_copy_parent_ref().to_string(),
    };

    backend
        .get_file_content_at_ref(&ref_str, Path::new(filename))
        .unwrap_or_default()
}

/// Get content of a file at the "new" side of the diff
pub fn get_new_content(filename: &str, refs: &DiffRefs, backend: &dyn VcsBackend) -> String {
    use std::path::Path;

    match refs {
        DiffRefs::Single(sha) => backend
            .get_file_content_at_ref(sha, Path::new(filename))
            .unwrap_or_default(),
        DiffRefs::Range { to, .. } => backend
            .get_file_content_at_ref(to, Path::new(filename))
            .unwrap_or_default(),
        DiffRefs::WorkingTree => {
            // Read from working tree (actual filesystem)
            fs::read_to_string(filename).unwrap_or_default()
        }
    }
}

pub fn load_file_diffs(options: &DiffOptions, backend: &dyn VcsBackend) -> Vec<FileDiff> {
    let refs = DiffRefs::from_options(options, backend);
    get_changed_files(options, backend)
        .into_iter()
        .map(|filename| {
            let old_content = get_old_content(&filename, &refs, backend);
            let new_content = get_new_content(&filename, &refs, backend);
            let status = if old_content.is_empty() && !new_content.is_empty() {
                FileStatus::Added
            } else if !old_content.is_empty() && new_content.is_empty() {
                FileStatus::Deleted
            } else {
                FileStatus::Modified
            };
            FileDiff {
                filename,
                old_content,
                new_content,
                status,
            }
        })
        .collect()
}

pub fn load_pr_file_diffs(pr_info: &PrInfo) -> Result<Vec<FileDiff>, String> {
    let repo_arg = format!("{}/{}", pr_info.repo_owner, pr_info.repo_name);

    // Get PR diff to find changed files
    let output = Command::new("gh")
        .args([
            "pr",
            "diff",
            &pr_info.number.to_string(),
            "--repo",
            &repo_arg,
        ])
        .output()
        .map_err(|e| format!("Failed to run gh pr diff: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh pr diff failed: {}", stderr.trim()));
    }

    let diff_output = String::from_utf8_lossy(&output.stdout);
    let changed_files = parse_changed_files_from_diff(&diff_output);

    // Fetch full file contents for each changed file
    let base_repo = format!("{}/{}", pr_info.base_repo_owner, pr_info.repo_name);
    let head_repo = pr_info
        .head_repo_owner
        .as_ref()
        .map(|owner| format!("{}/{}", owner, pr_info.repo_name))
        .unwrap_or_else(|| base_repo.clone());

    let file_diffs: Vec<FileDiff> = changed_files
        .into_iter()
        .map(|filename| {
            let old_content =
                fetch_file_content_from_github(&base_repo, &pr_info.base_ref, &filename);
            let new_content =
                fetch_file_content_from_github(&head_repo, &pr_info.head_ref, &filename);

            let status = if old_content.is_empty() && !new_content.is_empty() {
                FileStatus::Added
            } else if !old_content.is_empty() && new_content.is_empty() {
                FileStatus::Deleted
            } else {
                FileStatus::Modified
            };

            FileDiff {
                filename,
                old_content,
                new_content,
                status,
            }
        })
        .collect();

    Ok(file_diffs)
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

/// Check if a commit has any file changes
fn commit_has_changes(sha: &str) -> bool {
    let output = Command::new("git")
        .args(["diff-tree", "--no-commit-id", "--name-only", "-r", sha])
        .output();

    match output {
        Ok(o) if o.status.success() => !String::from_utf8_lossy(&o.stdout).trim().is_empty(),
        _ => false,
    }
}

/// Get list of commits in a range for stacked diff mode
/// Filters out merge commits and commits with no file changes
pub fn get_commits_in_range(from: &str, to: &str) -> Vec<CommitInfo> {
    let range = format!("{}..{}", from, to);
    let output = Command::new("git")
        .args(["log", "--reverse", "--format=%H%x00%h%x00%s", &range])
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('\0').collect();
                if parts.len() >= 3 {
                    let sha = parts[0].to_string();
                    // Filter out commits with no changes (like merge commits)
                    if commit_has_changes(&sha) {
                        Some(CommitInfo {
                            sha,
                            short_sha: parts[1].to_string(),
                            message: parts[2].to_string(),
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Load file diffs for a single commit (comparing commit to its parent)
pub fn load_single_commit_diffs(sha: &str, file_filter: &Option<Vec<String>>) -> Vec<FileDiff> {
    // Get the list of changed files for this commit
    let output = Command::new("git")
        .args(["diff-tree", "--no-commit-id", "--name-only", "-r", sha])
        .output()
        .expect("Failed to run git");

    let files: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    let files = if let Some(ref filter) = file_filter {
        files.into_iter().filter(|f| filter.contains(f)).collect()
    } else {
        files
    };

    files
        .into_iter()
        .map(|filename| {
            // Get old content (from parent commit)
            let old_ref = format!("{}^:{}", sha, filename);
            let old_content = Command::new("git")
                .args(["show", &old_ref])
                .output()
                .map(|o| {
                    if o.status.success() {
                        String::from_utf8_lossy(&o.stdout).to_string()
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();

            // Get new content (from the commit itself)
            let new_ref = format!("{}:{}", sha, filename);
            let new_content = Command::new("git")
                .args(["show", &new_ref])
                .output()
                .map(|o| {
                    if o.status.success() {
                        String::from_utf8_lossy(&o.stdout).to_string()
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();

            let status = if old_content.is_empty() && !new_content.is_empty() {
                FileStatus::Added
            } else if !old_content.is_empty() && new_content.is_empty() {
                FileStatus::Deleted
            } else {
                FileStatus::Modified
            };

            FileDiff {
                filename,
                old_content,
                new_content,
                status,
            }
        })
        .collect()
}
