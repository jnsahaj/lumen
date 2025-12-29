use std::fs;
use std::process::Command;

use super::DiffOptions;
use crate::commit_reference::CommitReference;
use crate::diff_ui::types::{FileDiff, FileStatus};

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
    let output = Command::new("git")
        .args(["show", &ref_spec])
        .output();

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
