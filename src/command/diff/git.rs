use std::fs;
use std::path::Path;
use std::process::Command;

use super::types::{is_binary_content, FileDiff, FileStatus};
use super::{DiffOptions, PrInfo};
use crate::commit_reference::CommitReference;
use crate::vcs::VcsBackend;

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
                let merge_base = backend.get_merge_base(from, to).unwrap_or_else(|e| {
                    eprintln!(
                        "Warning: failed to find merge-base for {}...{}: {}. Using '{}' as base.",
                        from, to, e, from
                    );
                    from.clone()
                });
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
    let ref_str = match refs {
        DiffRefs::Single(sha) => {
            // Use get_parent_ref_or_empty to handle root commits gracefully
            backend.get_parent_ref_or_empty(sha).unwrap_or_default()
        }
        DiffRefs::Range { from, .. } => from.clone(),
        DiffRefs::WorkingTree => backend.working_copy_parent_ref().to_string(),
    };

    // Empty ref means root commit with no parent - return empty content
    if ref_str.is_empty() {
        return String::new();
    }

    backend
        .get_file_content_at_ref(&ref_str, Path::new(filename))
        .unwrap_or_default()
}

/// Get content of a file at the "new" side of the diff
pub fn get_new_content(filename: &str, refs: &DiffRefs, backend: &dyn VcsBackend) -> String {
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
            let is_binary = is_binary_content(&old_content) || is_binary_content(&new_content);
            FileDiff {
                filename,
                old_content,
                new_content,
                status,
                is_binary,
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

            let is_binary = is_binary_content(&old_content) || is_binary_content(&new_content);
            FileDiff {
                filename,
                old_content,
                new_content,
                status,
                is_binary,
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

/// Load file diffs for a single commit (comparing commit to its parent).
/// Uses VcsBackend for backend-agnostic file content retrieval.
pub fn load_single_commit_diffs(
    commit_id: &str,
    file_filter: &Option<Vec<String>>,
    backend: &dyn VcsBackend,
) -> Vec<FileDiff> {
    // Get the list of changed files for this commit
    let files = backend.get_changed_files(commit_id).unwrap_or_default();

    let files: Vec<String> = if let Some(ref filter) = file_filter {
        files.into_iter().filter(|f| filter.contains(f)).collect()
    } else {
        files
    };

    // Get parent ref (handles root commits gracefully)
    let parent_ref = backend
        .get_parent_ref_or_empty(commit_id)
        .unwrap_or_default();

    files
        .into_iter()
        .map(|filename| {
            let path = Path::new(&filename);

            // Get old content (from parent commit)
            let old_content = if parent_ref.is_empty() {
                String::new()
            } else {
                backend
                    .get_file_content_at_ref(&parent_ref, path)
                    .unwrap_or_default()
            };

            // Get new content (from the commit itself)
            let new_content = backend
                .get_file_content_at_ref(commit_id, path)
                .unwrap_or_default();

            let status = if old_content.is_empty() && !new_content.is_empty() {
                FileStatus::Added
            } else if !old_content.is_empty() && new_content.is_empty() {
                FileStatus::Deleted
            } else {
                FileStatus::Modified
            };

            let is_binary = is_binary_content(&old_content) || is_binary_content(&new_content);
            FileDiff {
                filename,
                old_content,
                new_content,
                status,
                is_binary,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs::test_utils::{git, make_temp_dir, RepoGuard};
    use crate::vcs::GitBackend;
    use std::fs;

    #[test]
    fn test_load_single_commit_diffs_added_file() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        // HEAD is the initial commit with README.md added
        let diffs = load_single_commit_diffs("HEAD", &None, &backend);

        assert_eq!(diffs.len(), 1, "should have 1 file diff");
        assert_eq!(diffs[0].filename, "README.md");
        assert_eq!(diffs[0].status, FileStatus::Added);
        assert!(
            diffs[0].old_content.is_empty(),
            "old content should be empty for added file"
        );
        assert_eq!(diffs[0].new_content.trim(), "hello");
    }

    #[test]
    fn test_load_single_commit_diffs_modified_file() {
        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-diff-modified");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // First commit
        fs::write(dir.join("file.txt"), "original content\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "first"]);

        // Second commit - modify file
        fs::write(dir.join("file.txt"), "modified content\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "second"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");
        let diffs = load_single_commit_diffs("HEAD", &None, &backend);

        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].filename, "file.txt");
        assert_eq!(diffs[0].status, FileStatus::Modified);
        assert_eq!(diffs[0].old_content, "original content\n");
        assert_eq!(diffs[0].new_content, "modified content\n");

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_single_commit_diffs_multiple_files() {
        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-diff-multi");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Commit with multiple files
        fs::write(dir.join("a.txt"), "file a\n").expect("write a");
        fs::write(dir.join("b.txt"), "file b\n").expect("write b");
        fs::write(dir.join("c.txt"), "file c\n").expect("write c");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "multi"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");
        let diffs = load_single_commit_diffs("HEAD", &None, &backend);

        assert_eq!(diffs.len(), 3, "should have 3 file diffs");

        let filenames: Vec<&str> = diffs.iter().map(|d| d.filename.as_str()).collect();
        assert!(filenames.contains(&"a.txt"));
        assert!(filenames.contains(&"b.txt"));
        assert!(filenames.contains(&"c.txt"));

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_single_commit_diffs_with_filter() {
        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-diff-filter");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        fs::write(dir.join("wanted.txt"), "wanted\n").expect("write wanted");
        fs::write(dir.join("unwanted.txt"), "unwanted\n").expect("write unwanted");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "filter test"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");
        let filter = Some(vec!["wanted.txt".to_string()]);
        let diffs = load_single_commit_diffs("HEAD", &filter, &backend);

        assert_eq!(diffs.len(), 1, "filter should limit to 1 file");
        assert_eq!(diffs[0].filename, "wanted.txt");

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_stacked_diff_integration_git() {
        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-stacked-integration");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Base commit
        fs::write(dir.join("base.txt"), "base\n").expect("write base");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "base"]);

        // Commit A
        fs::write(dir.join("a.txt"), "commit A\n").expect("write a");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "commit A"]);

        // Commit B
        fs::write(dir.join("b.txt"), "commit B\n").expect("write b");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "commit B"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");

        // Get commits in range (simulating stacked diff)
        let commits = backend
            .get_commits_in_range("HEAD~2", "HEAD")
            .expect("should get commits");

        assert_eq!(commits.len(), 2, "should have 2 commits");
        assert_eq!(commits[0].summary, "commit A");
        assert_eq!(commits[1].summary, "commit B");

        // Load diffs for each commit (as stacked diff would do)
        let diffs_a = load_single_commit_diffs(&commits[0].commit_id, &None, &backend);
        assert_eq!(diffs_a.len(), 1);
        assert_eq!(diffs_a[0].filename, "a.txt");
        assert_eq!(diffs_a[0].new_content, "commit A\n");

        let diffs_b = load_single_commit_diffs(&commits[1].commit_id, &None, &backend);
        assert_eq!(diffs_b.len(), 1);
        assert_eq!(diffs_b[0].filename, "b.txt");
        assert_eq!(diffs_b[0].new_content, "commit B\n");

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }
}
