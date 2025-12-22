use std::fs;
use std::process::Command;

use super::DiffOptions;
use crate::diff_ui::types::FileDiff;

pub fn get_changed_files(options: &DiffOptions) -> Vec<String> {
    let mut files = Vec::new();

    let mut args = vec!["diff", "--name-only"];
    if options.staged {
        args.push("--cached");
    }
    if options.base != "HEAD" {
        args.push(&options.base);
    }

    let output = Command::new("git")
        .args(&args)
        .output()
        .expect("Failed to run git");

    files.extend(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|s| !s.is_empty())
            .map(String::from),
    );

    if !options.staged && options.base == "HEAD" {
        let output = Command::new("git")
            .args(["ls-files", "--others", "--exclude-standard"])
            .output()
            .expect("Failed to run git");

        files.extend(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|s| !s.is_empty())
                .map(String::from),
        );
    }

    if let Some(ref filter) = options.file {
        files.into_iter().filter(|f| filter.contains(f)).collect()
    } else {
        files
    }
}

pub fn get_old_content(filename: &str, base: &str) -> String {
    let output = Command::new("git")
        .args(["show", &format!("{}:{}", base, filename)])
        .output();

    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => String::new(),
    }
}

pub fn get_new_content(filename: &str) -> String {
    fs::read_to_string(filename).unwrap_or_default()
}

pub fn load_file_diffs(options: &DiffOptions) -> Vec<FileDiff> {
    get_changed_files(options)
        .into_iter()
        .map(|filename| {
            let old_content = get_old_content(&filename, &options.base);
            let new_content = get_new_content(&filename);
            FileDiff {
                filename,
                old_content,
                new_content,
            }
        })
        .collect()
}
