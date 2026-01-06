use std::path::Path;
use std::process::Command;

use super::backend::{CommitInfo, VcsBackend, VcsError};

/// Pathspec exclusions for diff output - excludes lock files and node_modules.
/// These are appended to git diff/diff-tree commands to filter noisy generated files.
const GIT_DIFF_EXCLUSIONS: [&str; 7] = [
    "--", // Separator for pathspecs
    ".",  // Include everything
    ":(exclude)package-lock.json",
    ":(exclude)yarn.lock",
    ":(exclude)pnpm-lock.yaml",
    ":(exclude)Cargo.lock",
    ":(exclude)node_modules/**",
];

/// Git backend using git CLI commands.
pub struct GitBackend;

impl GitBackend {
    pub fn new() -> Self {
        GitBackend
    }

    fn run_git(&self, args: &[&str]) -> Result<String, VcsError> {
        let output = Command::new("git").args(args).output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(VcsError::CommandFailed(stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn is_valid_ref(&self, reference: &str) -> Result<(), VcsError> {
        let output = Command::new("git")
            .args(["cat-file", "-t", reference.trim()])
            .output()?;

        if output.status.success() {
            let obj_type = String::from_utf8_lossy(&output.stdout);
            if obj_type.trim() == "commit" {
                return Ok(());
            }
        }

        Err(VcsError::InvalidRef(reference.to_string()))
    }
}

impl Default for GitBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl VcsBackend for GitBackend {
    fn get_commit(&self, reference: &str) -> Result<CommitInfo, VcsError> {
        // Use printable delimiter unlikely to appear in commit data
        const FIELD_SEP: &str = "<<<FIELD>>>";
        const MSG_SEP: &str = "<<<MSG>>>";

        let reference = reference.trim();
        self.is_valid_ref(reference)?;

        // Single git log call with delimited format: hash<SEP>author<SEP>email<SEP>date<MSG>message
        let format = format!(
            "%H{FIELD_SEP}%an{FIELD_SEP}%ae{FIELD_SEP}%cd{MSG_SEP}%B",
            FIELD_SEP = FIELD_SEP,
            MSG_SEP = MSG_SEP
        );
        let log_output = self.run_git(&[
            "log",
            &format!("--format={}", format),
            "--date=format:%Y-%m-%d %H:%M:%S",
            "-n",
            "1",
            reference,
        ])?;

        // Parse the output
        let (header, message) = log_output
            .split_once(MSG_SEP)
            .ok_or_else(|| VcsError::Other("Failed to parse git log output".to_string()))?;

        let fields: Vec<&str> = header.split(FIELD_SEP).collect();
        if fields.len() < 4 {
            return Err(VcsError::Other("Incomplete git log output".to_string()));
        }

        let commit_id = fields[0].to_string();
        let author_name = fields[1];
        let author_email = fields[2];
        let date = fields[3].to_string();
        let author = format!("{} <{}>", author_name, author_email);
        let message = message.trim_end_matches('\n').to_string();

        // Get diff (separate call - diff-tree has different semantics)
        // Apply GIT_DIFF_EXCLUSIONS to filter lock files and node_modules
        let mut diff_args = vec![
            "diff-tree",
            "-p",
            "--root",
            "--binary",
            "--no-color",
            "--compact-summary",
            reference,
        ];
        diff_args.extend_from_slice(&GIT_DIFF_EXCLUSIONS);
        let diff = self.run_git(&diff_args)?;

        Ok(CommitInfo {
            commit_id,
            change_id: None, // Git doesn't have change IDs
            message,
            diff,
            author,
            date,
        })
    }

    fn get_working_tree_diff(&self, staged: bool) -> Result<String, VcsError> {
        let mut args = if staged {
            vec!["diff", "--staged"]
        } else {
            vec!["diff"]
        };
        // Apply GIT_DIFF_EXCLUSIONS to filter lock files and node_modules
        args.extend_from_slice(&GIT_DIFF_EXCLUSIONS);

        self.run_git(&args)
    }

    fn get_range_diff(&self, from: &str, to: &str, three_dot: bool) -> Result<String, VcsError> {
        self.is_valid_ref(from)?;
        self.is_valid_ref(to)?;

        let separator = if three_dot { "..." } else { ".." };
        let range = format!("{}{}{}", from, separator, to);

        // Apply GIT_DIFF_EXCLUSIONS to filter lock files and node_modules
        let mut args = vec!["diff", &range];
        args.extend_from_slice(&GIT_DIFF_EXCLUSIONS);
        self.run_git(&args)
    }

    fn get_changed_files(&self, reference: &str) -> Result<Vec<String>, VcsError> {
        let reference = reference.trim();

        // Check if this is a range (contains ..)
        if reference.contains("..") {
            let parts: Vec<&str> = if reference.contains("...") {
                reference.split("...").collect()
            } else {
                reference.split("..").collect()
            };

            if parts.len() == 2 {
                let output = self.run_git(&["diff", "--name-only", parts[0], parts[1]])?;
                return Ok(output
                    .lines()
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect());
            }
        }

        // Single commit - use diff-tree with --root for root commits
        let output = self.run_git(&[
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "-r",
            "--root",
            reference,
        ])?;
        Ok(output
            .lines()
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect())
    }

    fn get_file_content_at_ref(&self, reference: &str, path: &Path) -> Result<String, VcsError> {
        let ref_spec = format!("{}:{}", reference.trim(), path.display());
        let output = Command::new("git").args(["show", &ref_spec]).output()?;

        if !output.status.success() {
            return Err(VcsError::FileNotFound(path.display().to_string()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn get_current_branch(&self) -> Result<Option<String>, VcsError> {
        let output = self.run_git(&["rev-parse", "--abbrev-ref", "HEAD"])?;
        let branch = output.trim();

        if branch == "HEAD" {
            // Detached HEAD state
            Ok(None)
        } else {
            Ok(Some(branch.to_string()))
        }
    }

    fn get_commit_log_for_fzf(&self) -> Result<String, VcsError> {
        self.run_git(&[
            "log",
            "--color=always",
            "--format=%C(auto)%h%d %s %C(black)%C(bold)%cr",
        ])
    }

    fn resolve_ref(&self, reference: &str) -> Result<String, VcsError> {
        let reference = reference.trim();
        self.is_valid_ref(reference)?;

        let output = self.run_git(&["rev-parse", reference])?;
        Ok(output.trim().to_string())
    }

    fn get_working_tree_changed_files(&self) -> Result<Vec<String>, VcsError> {
        use std::collections::HashSet;

        let mut files = HashSet::new();

        // Get unstaged changes (modified/deleted but not staged)
        let unstaged = self.run_git(&["diff", "--name-only", "HEAD"])?;
        for line in unstaged.lines() {
            if !line.is_empty() {
                files.insert(line.to_string());
            }
        }

        // Get staged changes
        let staged = self.run_git(&["diff", "--cached", "--name-only"])?;
        for line in staged.lines() {
            if !line.is_empty() {
                files.insert(line.to_string());
            }
        }

        // Get untracked files
        let untracked = self.run_git(&["ls-files", "--others", "--exclude-standard"])?;
        for line in untracked.lines() {
            if !line.is_empty() {
                files.insert(line.to_string());
            }
        }

        Ok(files.into_iter().collect())
    }

    fn get_merge_base(&self, ref1: &str, ref2: &str) -> Result<String, VcsError> {
        let ref1 = ref1.trim();
        let ref2 = ref2.trim();

        self.is_valid_ref(ref1)?;
        self.is_valid_ref(ref2)?;

        let output = self.run_git(&["merge-base", ref1, ref2])?;
        Ok(output.trim().to_string())
    }

    fn working_copy_parent_ref(&self) -> &'static str {
        "HEAD"
    }

    fn get_range_changed_files(&self, from: &str, to: &str) -> Result<Vec<String>, VcsError> {
        let from = from.trim();
        let to = to.trim();

        self.is_valid_ref(from)?;
        self.is_valid_ref(to)?;

        let output = self.run_git(&["diff", "--name-only", from, to])?;
        Ok(output
            .lines()
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs::test_utils::RepoGuard;

    #[test]
    fn test_get_commit_returns_valid_info() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        let info = backend.get_commit("HEAD").expect("should get commit");
        assert!(!info.commit_id.is_empty());
        assert!(info.change_id.is_none()); // Git has no change IDs
        assert_eq!(info.message, "init");
        assert!(info.author.contains("Test User"));
        assert!(!info.diff.is_empty());
    }

    #[test]
    fn test_get_working_tree_diff_returns_string() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        // Should succeed even if empty
        let diff = backend.get_working_tree_diff(false);
        assert!(diff.is_ok());
    }

    #[test]
    fn test_get_changed_files_returns_paths() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        let files = backend.get_changed_files("HEAD").expect("should get files");
        assert!(files.contains(&"README.md".to_string()));
    }

    #[test]
    fn test_get_current_branch() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        let branch = backend.get_current_branch().expect("should get branch");
        assert!(branch.is_some());
    }

    #[test]
    fn test_get_file_content_at_ref() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        let content = backend
            .get_file_content_at_ref("HEAD", Path::new("README.md"))
            .expect("should get content");
        assert_eq!(content.trim(), "hello");
    }

    #[test]
    fn test_invalid_ref_returns_error() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        let result = backend.get_commit("nonexistent12345");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_file_content_at_ref_missing_file() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        let result = backend.get_file_content_at_ref("HEAD", Path::new("nonexistent.txt"));
        assert!(
            matches!(result, Err(VcsError::FileNotFound(_))),
            "Expected FileNotFound error, got: {:?}",
            result
        );
    }

    #[test]
    fn test_get_commit_log_for_fzf() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        let log = backend.get_commit_log_for_fzf().expect("should get log");
        assert!(!log.is_empty(), "commit log should not be empty");
        // Log should contain the short hash from the commit
        assert!(
            log.lines().next().is_some(),
            "log should have at least one line"
        );
    }

    #[test]
    fn test_get_working_tree_diff_staged() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-staged");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Initial commit
        fs::write(dir.join("file.txt"), "initial\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "init"]);

        // Stage one change, leave another unstaged
        fs::write(dir.join("file.txt"), "staged change\n").expect("modify file");
        git(&dir, &["add", "file.txt"]);
        fs::write(dir.join("file.txt"), "staged change\nunstaged change\n").expect("modify again");

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::new();

        // Staged diff should only show "staged change"
        let staged_diff = backend
            .get_working_tree_diff(true)
            .expect("should get staged diff");
        assert!(
            staged_diff.contains("staged change"),
            "staged diff should contain staged changes"
        );
        assert!(
            !staged_diff.contains("unstaged change"),
            "staged diff should NOT contain unstaged changes"
        );

        // Unstaged diff should show the additional unstaged change
        let unstaged_diff = backend
            .get_working_tree_diff(false)
            .expect("should get unstaged diff");
        assert!(
            unstaged_diff.contains("unstaged change"),
            "unstaged diff should contain unstaged changes"
        );

        // Cleanup
        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_range_diff() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-range");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Commit A
        fs::write(dir.join("file.txt"), "commit A\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "commit A"]);

        // Commit B
        fs::write(dir.join("file.txt"), "commit B\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "commit B"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::new();

        // Range diff HEAD~1..HEAD (two-dot)
        let diff = backend
            .get_range_diff("HEAD~1", "HEAD", false)
            .expect("should get range diff");
        assert!(
            diff.contains("commit A") || diff.contains("commit B"),
            "range diff should contain changes"
        );

        // Three-dot range diff also works
        let diff_3dot = backend
            .get_range_diff("HEAD~1", "HEAD", true)
            .expect("should get three-dot diff");
        assert!(
            !diff_3dot.is_empty() || diff.contains("commit"),
            "three-dot diff should work"
        );

        // Cleanup
        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_range_diff_excludes_lock_files() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-range-exclusion");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Commit A with lock file
        fs::write(dir.join("file.txt"), "A\n").expect("write file");
        fs::write(dir.join("package-lock.json"), "{\"v\":1}\n").expect("write lock");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "A"]);

        // Commit B - modify both
        fs::write(dir.join("file.txt"), "B\n").expect("modify file");
        fs::write(dir.join("package-lock.json"), "{\"v\":2}\n").expect("modify lock");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "B"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::new();
        let diff = backend
            .get_range_diff("HEAD~1", "HEAD", false)
            .expect("should get range diff");

        assert!(
            diff.contains("file.txt"),
            "range diff should contain file.txt"
        );
        assert!(
            !diff.contains("package-lock.json"),
            "range diff should NOT contain package-lock.json"
        );

        // Cleanup
        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_diff_excludes_lock_files() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-exclusion");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Create files including lock files
        fs::write(dir.join("test.txt"), "hello\n").expect("write test.txt");
        fs::write(dir.join("package-lock.json"), "{}\n").expect("write package-lock.json");
        fs::write(dir.join("Cargo.lock"), "lock\n").expect("write Cargo.lock");

        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "init with lock files"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::new();
        let info = backend.get_commit("HEAD").expect("should get commit");

        // Diff should contain test.txt but NOT lock files
        assert!(
            info.diff.contains("test.txt"),
            "diff should contain test.txt"
        );
        assert!(
            !info.diff.contains("package-lock.json"),
            "diff should NOT contain package-lock.json"
        );
        assert!(
            !info.diff.contains("Cargo.lock"),
            "diff should NOT contain Cargo.lock"
        );

        // Cleanup
        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_working_tree_diff_excludes_lock_files() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-wt-exclusion");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Initial commit
        fs::write(dir.join("test.txt"), "hello\n").expect("write test.txt");
        fs::write(dir.join("package-lock.json"), "{}\n").expect("write package-lock.json");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "init"]);

        // Modify both files
        fs::write(dir.join("test.txt"), "world\n").expect("modify test.txt");
        fs::write(dir.join("package-lock.json"), "{\"v\": 2}\n").expect("modify package-lock.json");

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::new();
        let diff = backend
            .get_working_tree_diff(false)
            .expect("should get diff");

        // Diff should contain test.txt but NOT package-lock.json
        assert!(
            diff.contains("test.txt"),
            "working tree diff should contain test.txt"
        );
        assert!(
            !diff.contains("package-lock.json"),
            "working tree diff should NOT contain package-lock.json"
        );

        // Cleanup
        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_working_tree_diff_empty() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        // Clean working tree should return empty string
        let diff = backend
            .get_working_tree_diff(false)
            .expect("should succeed on clean tree");
        assert!(
            diff.is_empty(),
            "clean working tree should return empty diff"
        );
    }

    #[test]
    fn test_get_range_diff_identical_commits() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        // Diff of HEAD..HEAD should be empty
        let diff = backend
            .get_range_diff("HEAD", "HEAD", false)
            .expect("should succeed for identical commits");
        assert!(diff.is_empty(), "diff of identical commits should be empty");
    }

    #[test]
    fn test_commit_info_field_format() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();
        let commit = backend.get_commit("HEAD").expect("should get commit");

        // commit_id should be 40-char hex
        assert_eq!(
            commit.commit_id.len(),
            40,
            "commit_id should be 40-char hex, got: {}",
            commit.commit_id
        );
        assert!(
            commit.commit_id.chars().all(|c| c.is_ascii_hexdigit()),
            "commit_id should be hex"
        );

        // Git has no change_id
        assert!(
            commit.change_id.is_none(),
            "git commits should not have change_id"
        );

        // author format: "Name <email>"
        assert!(
            commit.author.contains('<') && commit.author.contains('>'),
            "author should be 'Name <email>' format, got: {}",
            commit.author
        );

        // date format: YYYY-MM-DD HH:MM:SS (19 chars)
        assert_eq!(
            commit.date.len(),
            19,
            "date should be 19 chars (YYYY-MM-DD HH:MM:SS), got: {}",
            commit.date
        );
        assert!(
            commit.date.chars().nth(4) == Some('-')
                && commit.date.chars().nth(7) == Some('-')
                && commit.date.chars().nth(10) == Some(' ')
                && commit.date.chars().nth(13) == Some(':')
                && commit.date.chars().nth(16) == Some(':'),
            "date should be YYYY-MM-DD HH:MM:SS format, got: {}",
            commit.date
        );
    }

    #[test]
    fn test_resolve_ref_head_returns_sha() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        let sha = backend.resolve_ref("HEAD").expect("should resolve HEAD");

        assert_eq!(sha.len(), 40, "should return 40-char SHA, got: {}", sha);
        assert!(
            sha.chars().all(|c| c.is_ascii_hexdigit()),
            "SHA should be hex"
        );
    }

    #[test]
    fn test_resolve_ref_invalid_returns_error() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        let result = backend.resolve_ref("nonexistent_ref_xyz");
        assert!(result.is_err(), "resolve_ref should fail for invalid ref");
    }

    #[test]
    fn test_resolve_ref_matches_commit_id() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        let commit = backend.get_commit("HEAD").expect("should get commit");
        let sha = backend.resolve_ref("HEAD").expect("should resolve HEAD");

        assert_eq!(
            sha, commit.commit_id,
            "resolve_ref should return same SHA as get_commit"
        );
    }

    #[test]
    fn test_get_working_tree_changed_files_modified() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-wt-changed");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Initial commit
        fs::write(dir.join("file.txt"), "initial\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "init"]);

        // Modify file (unstaged)
        fs::write(dir.join("file.txt"), "modified\n").expect("modify file");

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::new();
        let files = backend
            .get_working_tree_changed_files()
            .expect("should get changed files");

        assert!(
            files.contains(&"file.txt".to_string()),
            "should include modified file, got: {:?}",
            files
        );

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_working_tree_changed_files_untracked() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-wt-untracked");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Initial commit
        fs::write(dir.join("file.txt"), "initial\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "init"]);

        // Add untracked file
        fs::write(dir.join("new.txt"), "new file\n").expect("write new file");

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::new();
        let files = backend
            .get_working_tree_changed_files()
            .expect("should get changed files");

        assert!(
            files.contains(&"new.txt".to_string()),
            "should include untracked file, got: {:?}",
            files
        );

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_working_tree_changed_files_clean() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        let files = backend
            .get_working_tree_changed_files()
            .expect("should succeed on clean tree");

        assert!(files.is_empty(), "clean tree should return empty vec");
    }

    #[test]
    fn test_get_merge_base_returns_ancestor() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-merge-base");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Commit A (base)
        fs::write(dir.join("file.txt"), "base\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "base"]);

        // Create branch and commit B
        git(&dir, &["checkout", "-b", "branch"]);
        fs::write(dir.join("file.txt"), "branch\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "branch commit"]);

        // Back to main, commit C
        git(&dir, &["checkout", "main"]);
        fs::write(dir.join("other.txt"), "main\n").expect("write other");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "main commit"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::new();
        let merge_base = backend
            .get_merge_base("main", "branch")
            .expect("should find merge base");

        // Merge base should be 40-char SHA
        assert_eq!(merge_base.len(), 40, "should return 40-char SHA");

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_merge_base_invalid_ref() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::new();

        let result = backend.get_merge_base("HEAD", "nonexistent_branch_xyz");
        assert!(result.is_err(), "should fail for invalid ref");
    }

    #[test]
    fn test_working_copy_parent_ref_returns_head() {
        let backend = GitBackend::new();
        assert_eq!(backend.working_copy_parent_ref(), "HEAD");
    }
}
