use std::fs;
use std::process::Command;

use super::DiffOptions;
use crate::diff_ui::types::FileDiff;

/// Get the list of files changed in a specific commit (SHA vs SHA^) or uncommitted changes
pub fn get_changed_files(options: &DiffOptions) -> Vec<String> {
    let output = match &options.sha {
        Some(sha) => Command::new("git")
            .args(["diff-tree", "--no-commit-id", "--name-only", "-r", sha])
            .output()
            .expect("Failed to run git"),
        None => Command::new("git")
            .args(["diff", "--name-only", "HEAD"])
            .output()
            .expect("Failed to run git"),
    };

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

/// Get content of a file at the parent of the given SHA, or HEAD for uncommitted
pub fn get_old_content(filename: &str, sha: Option<&str>) -> String {
    let ref_spec = match sha {
        Some(s) => format!("{}^:{}", s, filename),
        None => format!("HEAD:{}", filename),
    };
    let output = Command::new("git")
        .args(["show", &ref_spec])
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => String::new(),
    }
}

/// Get content of a file at the given SHA, or current working tree for uncommitted
pub fn get_new_content(filename: &str, sha: Option<&str>) -> String {
    match sha {
        Some(s) => {
            let output = Command::new("git")
                .args(["show", &format!("{}:{}", s, filename)])
                .output();

            match output {
                Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
                _ => String::new(),
            }
        }
        None => {
            // Read from working tree
            fs::read_to_string(filename).unwrap_or_default()
        }
    }
}

pub fn load_file_diffs(options: &DiffOptions) -> Vec<FileDiff> {
    get_changed_files(options)
        .into_iter()
        .map(|filename| {
            let old_content = get_old_content(&filename, options.sha.as_deref());
            let new_content = get_new_content(&filename, options.sha.as_deref());
            FileDiff {
                filename,
                old_content,
                new_content,
            }
        })
        .collect()
}
