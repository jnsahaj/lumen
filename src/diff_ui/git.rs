use std::fs;

use super::DiffOptions;
use crate::commit_reference::CommitReference;
use crate::diff_ui::types::{FileDiff, FileStatus};
use crate::vcs::Vcs;

pub fn get_current_branch() -> String {
    let vcs = Vcs::detect();
    let output = vcs.get_current_branch().output();

    match output {
        Ok(o) if o.status.success() => {
            let result = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if result.is_empty() {
                "unknown".to_string()
            } else {
                result
            }
        }
        _ => "unknown".to_string(),
    }
}

pub enum DiffRefs {
    WorkingTree,
    Single(String),
    Range { from: String, to: String },
}

impl DiffRefs {
    pub fn from_options(options: &DiffOptions) -> Self {
        let vcs = Vcs::detect();
        
        match &options.reference {
            None => DiffRefs::WorkingTree,
            Some(CommitReference::Single(sha)) => DiffRefs::Single(sha.clone()),
            Some(CommitReference::Range { from, to }) => DiffRefs::Range {
                from: from.clone(),
                to: to.clone(),
            },
            Some(CommitReference::TripleDots { from, to }) => {
                let output = vcs
                    .get_merge_base(from, to)
                    .output()
                    .expect("Failed to get merge base");
                let merge_base = String::from_utf8_lossy(&output.stdout).trim().to_string();
                DiffRefs::Range {
                    from: merge_base,
                    to: to.clone(),
                }
            }
        }
    }
}

pub fn get_changed_files(options: &DiffOptions) -> Vec<String> {
    let vcs = Vcs::detect();
    let refs = DiffRefs::from_options(options);

    let files: Vec<String> = match refs {
        DiffRefs::Single(sha) => {
            let output = vcs
                .get_commit_files(&sha)
                .output()
                .expect("Failed to get commit files");
            vcs.parse_file_list(&String::from_utf8_lossy(&output.stdout))
        }
        DiffRefs::Range { from, to } => {
            let output = vcs
                .get_range_files(&from, &to)
                .output()
                .expect("Failed to get range files");
            vcs.parse_file_list(&String::from_utf8_lossy(&output.stdout))
        }
        DiffRefs::WorkingTree => {
            let mut all_files: std::collections::HashSet<String> = std::collections::HashSet::new();

            let unstaged = vcs
                .get_unstaged_files()
                .output()
                .expect("Failed to get unstaged files");
            for file in vcs.parse_file_list(&String::from_utf8_lossy(&unstaged.stdout)) {
                all_files.insert(file);
            }

            if vcs.supports_staging() {
                let staged = vcs
                    .get_staged_files()
                    .output()
                    .expect("Failed to get staged files");
                for file in vcs.parse_file_list(&String::from_utf8_lossy(&staged.stdout)) {
                    all_files.insert(file);
                }
            }

            let untracked = vcs
                .get_untracked_files()
                .output()
                .expect("Failed to get untracked files");
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

pub fn get_old_content(filename: &str, refs: &DiffRefs) -> String {
    let vcs = Vcs::detect();
    
    let (rev, use_parent) = match refs {
        DiffRefs::Single(sha) => (sha.clone(), true),
        DiffRefs::Range { from, .. } => (from.clone(), false),
        DiffRefs::WorkingTree => {
            match vcs {
                Vcs::Git => ("HEAD".to_string(), false),
                Vcs::Jj => ("@-".to_string(), false),
            }
        }
    };

    let actual_rev = if use_parent {
        match vcs {
            Vcs::Git => format!("{}^", rev),
            Vcs::Jj => format!("{}-", rev),
        }
    } else {
        rev
    };

    let output = vcs.show_file(filename, &actual_rev).output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => String::new(),
    }
}

pub fn get_new_content(filename: &str, refs: &DiffRefs) -> String {
    let vcs = Vcs::detect();
    
    match refs {
        DiffRefs::Single(sha) => {
            let output = vcs.show_file(filename, sha).output();
            match output {
                Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
                _ => String::new(),
            }
        }
        DiffRefs::Range { to, .. } => {
            let output = vcs.show_file(filename, to).output();
            match output {
                Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
                _ => String::new(),
            }
        }
        DiffRefs::WorkingTree => {
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
