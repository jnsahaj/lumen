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

    // Get PR diff using gh
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
    Ok(parse_unified_diff(&diff_output))
}

fn parse_unified_diff(diff: &str) -> Vec<FileDiff> {
    let mut file_diffs = Vec::new();
    let mut current_file: Option<String> = None;
    let mut old_content = String::new();
    let mut new_content = String::new();
    let mut in_hunk = false;

    for line in diff.lines() {
        if line.starts_with("diff --git") {
            // Save previous file if exists
            if let Some(filename) = current_file.take() {
                let status = determine_file_status(&old_content, &new_content);
                file_diffs.push(FileDiff {
                    filename,
                    old_content: std::mem::take(&mut old_content),
                    new_content: std::mem::take(&mut new_content),
                    status,
                });
            }

            // Parse filename from "diff --git a/path b/path"
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let b_path = parts[3];
                current_file = Some(b_path.strip_prefix("b/").unwrap_or(b_path).to_string());
            }
            in_hunk = false;
        } else if line.starts_with("@@") {
            in_hunk = true;
        } else if in_hunk && current_file.is_some() {
            if let Some(stripped) = line.strip_prefix('-') {
                if !line.starts_with("---") {
                    old_content.push_str(stripped);
                    old_content.push('\n');
                }
            } else if let Some(stripped) = line.strip_prefix('+') {
                if !line.starts_with("+++") {
                    new_content.push_str(stripped);
                    new_content.push('\n');
                }
            } else if let Some(stripped) = line.strip_prefix(' ') {
                old_content.push_str(stripped);
                old_content.push('\n');
                new_content.push_str(stripped);
                new_content.push('\n');
            } else if !line.starts_with('\\') {
                // Handle lines without prefix (context)
                old_content.push_str(line);
                old_content.push('\n');
                new_content.push_str(line);
                new_content.push('\n');
            }
        }
    }

    // Don't forget the last file
    if let Some(filename) = current_file {
        let status = determine_file_status(&old_content, &new_content);
        file_diffs.push(FileDiff {
            filename,
            old_content,
            new_content,
            status,
        });
    }

    file_diffs
}

fn determine_file_status(old_content: &str, new_content: &str) -> FileStatus {
    let old_empty = old_content.trim().is_empty();
    let new_empty = new_content.trim().is_empty();

    if old_empty && !new_empty {
        FileStatus::Added
    } else if !old_empty && new_empty {
        FileStatus::Deleted
    } else {
        FileStatus::Modified
    }
}
