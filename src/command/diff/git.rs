use std::fs;
use std::process::Command;

use super::types::{FileDiff, FileStatus};
use super::{DiffOptions, PrInfo};
use crate::commit_reference::CommitReference;

pub fn get_current_branch() -> String {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => "unknown".to_string(),
    }
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
    pub fn from_options(options: &DiffOptions) -> Self {
        match &options.reference {
            None => DiffRefs::WorkingTree,
            Some(CommitReference::Single(sha)) => DiffRefs::Single(sha.clone()),
            Some(CommitReference::Range { from, to }) => DiffRefs::Range {
                from: from.clone(),
                to: to.clone(),
            },
            Some(CommitReference::TripleDots { from, to }) => {
                // Get merge-base for triple dots
                let output = Command::new("git")
                    .args(["merge-base", from, to])
                    .output()
                    .expect("Failed to run git merge-base");
                let merge_base = String::from_utf8_lossy(&output.stdout).trim().to_string();
                DiffRefs::Range {
                    from: merge_base,
                    to: to.clone(),
                }
            }
        }
    }
}

/// Get the list of files changed
pub fn get_changed_files(options: &DiffOptions) -> Vec<String> {
    let refs = DiffRefs::from_options(options);

    let files: Vec<String> = match refs {
        DiffRefs::Single(sha) => {
            let output = Command::new("git")
                .args(["diff-tree", "--no-commit-id", "--name-only", "-r", &sha])
                .output()
                .expect("Failed to run git");
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        }
        DiffRefs::Range { from, to } => {
            let output = Command::new("git")
                .args(["diff", "--name-only", &from, &to])
                .output()
                .expect("Failed to run git");
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        }
        DiffRefs::WorkingTree => {
            // Get unstaged changes (tracked files modified in working tree)
            let unstaged = Command::new("git")
                .args(["diff", "--name-only", "HEAD"])
                .output()
                .expect("Failed to run git");

            // Get staged changes (including newly added files)
            let staged = Command::new("git")
                .args(["diff", "--cached", "--name-only"])
                .output()
                .expect("Failed to run git");

            // Get untracked files (new files not yet added to git)
            let untracked = Command::new("git")
                .args(["ls-files", "--others", "--exclude-standard"])
                .output()
                .expect("Failed to run git");

            let mut all_files: std::collections::HashSet<String> = std::collections::HashSet::new();

            for line in String::from_utf8_lossy(&unstaged.stdout).lines() {
                if !line.is_empty() {
                    all_files.insert(line.to_string());
                }
            }
            for line in String::from_utf8_lossy(&staged.stdout).lines() {
                if !line.is_empty() {
                    all_files.insert(line.to_string());
                }
            }
            for line in String::from_utf8_lossy(&untracked.stdout).lines() {
                if !line.is_empty() {
                    all_files.insert(line.to_string());
                }
            }

            all_files.into_iter().collect()
        }
    };

    if let Some(ref filter) = options.file {
        files.into_iter().filter(|f| filter.contains(f)).collect()
    } else {
        files
    }
}

/// Get content of a file at the "old" side of the diff
pub fn get_old_content(filename: &str, refs: &DiffRefs) -> String {
    let ref_spec = match refs {
        DiffRefs::Single(sha) => format!("{}^:{}", sha, filename),
        DiffRefs::Range { from, .. } => format!("{}:{}", from, filename),
        DiffRefs::WorkingTree => format!("HEAD:{}", filename),
    };
    let output = Command::new("git").args(["show", &ref_spec]).output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => String::new(),
    }
}

/// Get content of a file at the "new" side of the diff
pub fn get_new_content(filename: &str, refs: &DiffRefs) -> String {
    match refs {
        DiffRefs::Single(sha) => {
            let output = Command::new("git")
                .args(["show", &format!("{}:{}", sha, filename)])
                .output();

            match output {
                Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
                _ => String::new(),
            }
        }
        DiffRefs::Range { to, .. } => {
            let output = Command::new("git")
                .args(["show", &format!("{}:{}", to, filename)])
                .output();

            match output {
                Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
                _ => String::new(),
            }
        }
        DiffRefs::WorkingTree => {
            // Read from working tree
            fs::read_to_string(filename).unwrap_or_default()
        }
    }
}

pub fn load_file_diffs(options: &DiffOptions) -> Vec<FileDiff> {
    let refs = DiffRefs::from_options(options);
    get_changed_files(options)
        .into_iter()
        .map(|filename| {
            let old_content = get_old_content(&filename, &refs);
            let new_content = get_new_content(&filename, &refs);
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
