use std::process::Command;

use super::DiffOptions;
use crate::diff_ui::types::FileDiff;

/// Get the list of files changed in a specific commit (SHA vs SHA^)
pub fn get_changed_files(options: &DiffOptions) -> Vec<String> {
    let output = Command::new("git")
        .args(["diff-tree", "--no-commit-id", "--name-only", "-r", &options.sha])
        .output()
        .expect("Failed to run git");

    let files: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    if let Some(ref filter) = options.file {
        files.into_iter().filter(|f| filter.contains(f)).collect()
    } else {
        files
    }
}

/// Get content of a file at the parent of the given SHA
pub fn get_old_content(filename: &str, sha: &str) -> String {
    let output = Command::new("git")
        .args(["show", &format!("{}^:{}", sha, filename)])
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => String::new(),
    }
}

/// Get content of a file at the given SHA
pub fn get_new_content(filename: &str, sha: &str) -> String {
    let output = Command::new("git")
        .args(["show", &format!("{}:{}", sha, filename)])
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => String::new(),
    }
}

pub fn load_file_diffs(options: &DiffOptions) -> Vec<FileDiff> {
    get_changed_files(options)
        .into_iter()
        .map(|filename| {
            let old_content = get_old_content(&filename, &options.sha);
            let new_content = get_new_content(&filename, &options.sha);
            FileDiff {
                filename,
                old_content,
                new_content,
            }
        })
        .collect()
}
