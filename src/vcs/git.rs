use std::path::{Path, PathBuf};

use git2::{Repository, StatusOptions, Tree};

use super::backend::{StackedCommitInfo, VcsBackend, VcsError};

/// Git backend using git2 (libgit2) for repository access.
pub struct GitBackend {
    repo: Repository,
    working_tree_root: PathBuf,
}

impl GitBackend {
    /// Open a git repository at the given path.
    /// Uses git2::Repository::discover to find the repo from any subdirectory.
    pub fn new(path: &Path) -> Result<Self, VcsError> {
        let discover_result = Repository::discover(path);//map_err(|_| VcsError::NotARepository)?;
        return match discover_result {
            Ok(repo) => {
                let working_tree_root = repo.workdir().unwrap_or_else(|| repo.path()).to_path_buf();
                Ok(GitBackend { repo, working_tree_root })
            }
            Err(error) => {
                // Print libgit2 so there is a chance to diagnose any errors with git
                eprintln!("Error on repository discovery: {error:?}");
                return Err(VcsError::NotARepository)
            },
        };

    }

    /// Open a git repository from the current working directory.
    /// Convenience method for tests.
    #[cfg(test)]
    pub fn from_cwd() -> Result<Self, VcsError> {
        Self::new(Path::new("."))
    }

    /// Validate that a reference doesn't look like a flag (defense in depth).
    fn validate_ref_format(reference: &str) -> Result<(), VcsError> {
        if reference.trim().starts_with('-') {
            return Err(VcsError::InvalidRef(format!(
                "references cannot start with '-': {}",
                reference
            )));
        }
        Ok(())
    }

}

impl VcsBackend for GitBackend {
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
                Self::validate_ref_format(parts[0])?;
                Self::validate_ref_format(parts[1])?;

                let from_obj = self
                    .repo
                    .revparse_single(parts[0])
                    .map_err(|_| VcsError::InvalidRef(parts[0].to_string()))?;
                let from_commit = from_obj
                    .peel_to_commit()
                    .map_err(|_| VcsError::InvalidRef(parts[0].to_string()))?;
                let from_tree = from_commit
                    .tree()
                    .map_err(|e| VcsError::Other(format!("failed to get from tree: {}", e)))?;

                let to_obj = self
                    .repo
                    .revparse_single(parts[1])
                    .map_err(|_| VcsError::InvalidRef(parts[1].to_string()))?;
                let to_commit = to_obj
                    .peel_to_commit()
                    .map_err(|_| VcsError::InvalidRef(parts[1].to_string()))?;
                let to_tree = to_commit
                    .tree()
                    .map_err(|e| VcsError::Other(format!("failed to get to tree: {}", e)))?;

                let diff = self
                    .repo
                    .diff_tree_to_tree(Some(&from_tree), Some(&to_tree), None)
                    .map_err(|e| VcsError::Other(format!("failed to create diff: {}", e)))?;

                return Ok(diff
                    .deltas()
                    .filter_map(|d| {
                        d.new_file()
                            .path()
                            .and_then(|p| p.to_str().map(String::from))
                    })
                    .collect());
            }
        }

        // Single commit - compare to parent tree (or empty tree for root)
        Self::validate_ref_format(reference)?;
        let obj = self
            .repo
            .revparse_single(reference)
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;
        let commit = obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;
        let tree = commit
            .tree()
            .map_err(|e| VcsError::Other(format!("failed to get commit tree: {}", e)))?;

        let parent_tree: Option<Tree> = if commit.parent_count() > 0 {
            commit.parent(0).ok().and_then(|p| p.tree().ok())
        } else {
            None
        };

        let diff = self
            .repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
            .map_err(|e| VcsError::Other(format!("failed to create diff: {}", e)))?;

        Ok(diff
            .deltas()
            .filter_map(|d| {
                d.new_file()
                    .path()
                    .and_then(|p| p.to_str().map(String::from))
            })
            .collect())
    }

    fn get_file_content_at_ref(&self, reference: &str, path: &Path) -> Result<String, VcsError> {
        let reference = reference.trim();
        Self::validate_ref_format(reference)?;

        // Resolve reference to commit
        let obj = self
            .repo
            .revparse_single(reference)
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;
        let commit = obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;
        let tree = commit
            .tree()
            .map_err(|e| VcsError::Other(format!("failed to get tree: {}", e)))?;

        // Look up file in tree
        let entry = tree
            .get_path(path)
            .map_err(|_| VcsError::FileNotFound(path.display().to_string()))?;

        // Get blob content
        let blob = self
            .repo
            .find_blob(entry.id())
            .map_err(|_| VcsError::FileNotFound(path.display().to_string()))?;

        Ok(String::from_utf8_lossy(blob.content()).into_owned())
    }

    fn get_current_branch(&self) -> Result<Option<String>, VcsError> {
        let head = self
            .repo
            .head()
            .map_err(|e| VcsError::Other(format!("failed to get HEAD: {}", e)))?;

        if head.is_branch() {
            Ok(head.shorthand().map(|s| s.to_string()))
        } else {
            // Detached HEAD state
            Ok(None)
        }
    }

    fn resolve_ref(&self, reference: &str) -> Result<String, VcsError> {
        let reference = reference.trim();
        Self::validate_ref_format(reference)?;

        // Use git2 to resolve reference to commit SHA
        let obj = self
            .repo
            .revparse_single(reference)
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;

        let commit = obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;

        Ok(commit.id().to_string())
    }

    fn get_working_tree_changed_files(&self) -> Result<Vec<String>, VcsError> {
        use std::collections::HashSet;

        let mut opts = StatusOptions::new();
        opts.include_untracked(true);
        opts.recurse_untracked_dirs(true);
        opts.exclude_submodules(true);
        opts.include_ignored(false);

        let statuses = self
            .repo
            .statuses(Some(&mut opts))
            .map_err(|e| VcsError::Other(format!("failed to get status: {}", e)))?;

        let files: HashSet<String> = statuses
            .iter()
            .filter_map(|s| s.path().map(String::from))
            .collect();

        Ok(files.into_iter().collect())
    }

    fn get_merge_base(&self, ref1: &str, ref2: &str) -> Result<String, VcsError> {
        let ref1 = ref1.trim();
        let ref2 = ref2.trim();

        Self::validate_ref_format(ref1)?;
        Self::validate_ref_format(ref2)?;

        let obj1 = self
            .repo
            .revparse_single(ref1)
            .map_err(|_| VcsError::InvalidRef(ref1.to_string()))?;
        let oid1 = obj1
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(ref1.to_string()))?
            .id();

        let obj2 = self
            .repo
            .revparse_single(ref2)
            .map_err(|_| VcsError::InvalidRef(ref2.to_string()))?;
        let oid2 = obj2
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(ref2.to_string()))?
            .id();

        let merge_base = self
            .repo
            .merge_base(oid1, oid2)
            .map_err(|e| VcsError::Other(format!("failed to find merge base: {}", e)))?;

        Ok(merge_base.to_string())
    }

    fn working_copy_parent_ref(&self) -> &'static str {
        "HEAD"
    }

    fn get_range_changed_files(&self, from: &str, to: &str) -> Result<Vec<String>, VcsError> {
        let from = from.trim();
        let to = to.trim();

        Self::validate_ref_format(from)?;
        Self::validate_ref_format(to)?;

        let from_obj = self
            .repo
            .revparse_single(from)
            .map_err(|_| VcsError::InvalidRef(from.to_string()))?;
        let from_tree = from_obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(from.to_string()))?
            .tree()
            .map_err(|e| VcsError::Other(format!("failed to get from tree: {}", e)))?;

        let to_obj = self
            .repo
            .revparse_single(to)
            .map_err(|_| VcsError::InvalidRef(to.to_string()))?;
        let to_tree = to_obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(to.to_string()))?
            .tree()
            .map_err(|e| VcsError::Other(format!("failed to get to tree: {}", e)))?;

        let diff = self
            .repo
            .diff_tree_to_tree(Some(&from_tree), Some(&to_tree), None)
            .map_err(|e| VcsError::Other(format!("failed to create diff: {}", e)))?;

        Ok(diff
            .deltas()
            .filter_map(|d| {
                d.new_file()
                    .path()
                    .and_then(|p| p.to_str().map(String::from))
            })
            .collect())
    }

    fn get_parent_ref_or_empty(&self, reference: &str) -> Result<String, VcsError> {
        let reference = reference.trim();
        Self::validate_ref_format(reference)?;

        let obj = self
            .repo
            .revparse_single(reference)
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;
        let commit = obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;

        if commit.parent_count() > 0 {
            // Has parent - return the parent ref
            Ok(format!("{}^", reference))
        } else {
            // No parent (root commit) - return git's empty tree SHA
            // This is a well-known constant: the SHA of an empty tree
            Ok("4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string())
        }
    }

    fn get_commits_in_range(
        &self,
        from: &str,
        to: &str,
    ) -> Result<Vec<StackedCommitInfo>, VcsError> {
        let from = from.trim();
        let to = to.trim();

        Self::validate_ref_format(from)?;
        Self::validate_ref_format(to)?;

        // Resolve refs to OIDs
        let from_obj = self
            .repo
            .revparse_single(from)
            .map_err(|_| VcsError::InvalidRef(from.to_string()))?;
        let from_oid = from_obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(from.to_string()))?
            .id();

        let to_obj = self
            .repo
            .revparse_single(to)
            .map_err(|_| VcsError::InvalidRef(to.to_string()))?;
        let to_oid = to_obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(to.to_string()))?
            .id();

        // Set up revwalk from 'to' to 'from' (exclusive)
        let mut revwalk = self
            .repo
            .revwalk()
            .map_err(|e| VcsError::Other(format!("failed to create revwalk: {}", e)))?;
        revwalk
            .push(to_oid)
            .map_err(|e| VcsError::Other(format!("failed to push to revwalk: {}", e)))?;
        revwalk
            .hide(from_oid)
            .map_err(|e| VcsError::Other(format!("failed to hide from revwalk: {}", e)))?;

        // Collect commits in reverse order (oldest first)
        let mut commits: Vec<StackedCommitInfo> = Vec::new();
        for oid_result in revwalk {
            let oid = oid_result.map_err(|e| VcsError::Other(format!("revwalk error: {}", e)))?;
            let commit = self
                .repo
                .find_commit(oid)
                .map_err(|e| VcsError::Other(format!("failed to find commit: {}", e)))?;

            let commit_id = oid.to_string();
            let short_id = commit_id[..7.min(commit_id.len())].to_string();
            let summary = commit.summary().unwrap_or("").to_string();

            // Filter commits with no file changes (e.g., merge commits)
            if self
                .get_changed_files(&commit_id)
                .map(|f| !f.is_empty())
                .unwrap_or(false)
            {
                commits.push(StackedCommitInfo {
                    commit_id,
                    short_id,
                    change_id: None,
                    summary,
                });
            }
        }

        // Reverse to get oldest first
        commits.reverse();
        Ok(commits)
    }

    fn name(&self) -> &'static str {
        "git"
    }

    fn working_tree_root(&self) -> &Path {
        &self.working_tree_root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs::test_utils::RepoGuard;

    #[test]
    fn test_get_changed_files_returns_paths() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let files = backend.get_changed_files("HEAD").expect("should get files");
        assert!(files.contains(&"README.md".to_string()));
    }

    #[test]
    fn test_get_current_branch() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let branch = backend.get_current_branch().expect("should get branch");
        assert!(branch.is_some());
    }

    #[test]
    fn test_get_file_content_at_ref() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let content = backend
            .get_file_content_at_ref("HEAD", Path::new("README.md"))
            .expect("should get content");
        assert_eq!(content.trim(), "hello");
    }

    #[test]
    fn test_get_file_content_at_ref_missing_file() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let result = backend.get_file_content_at_ref("HEAD", Path::new("nonexistent.txt"));
        assert!(
            matches!(result, Err(VcsError::FileNotFound(_))),
            "Expected FileNotFound error, got: {:?}",
            result
        );
    }

    #[test]
    fn test_resolve_ref_head_returns_sha() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let sha = backend.resolve_ref("HEAD").expect("should resolve HEAD");

        assert_eq!(sha.len(), 40, "should return 40-char SHA, got: {}", sha);
        assert!(
            sha.chars().all(|c| c.is_ascii_hexdigit()),
            "SHA should be hex"
        );
    }

    #[test]
    fn test_ref_starting_with_dash_rejected() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        // Refs starting with - could be interpreted as flags - should be rejected
        for reference in ["--upload-pack=evil", "-n"] {
            assert!(
                matches!(backend.resolve_ref(reference), Err(VcsError::InvalidRef(_))),
                "refs starting with - should be rejected: {}",
                reference
            );
        }
    }

    #[test]
    fn test_resolve_ref_invalid_returns_error() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let result = backend.resolve_ref("nonexistent_ref_xyz");
        assert!(result.is_err(), "resolve_ref should fail for invalid ref");
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

        let backend = GitBackend::from_cwd().expect("should open repo");
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

        let backend = GitBackend::from_cwd().expect("should open repo");
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
        let backend = GitBackend::from_cwd().expect("should open repo");

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

        let backend = GitBackend::from_cwd().expect("should open repo");
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
        let backend = GitBackend::from_cwd().expect("should open repo");

        let result = backend.get_merge_base("HEAD", "nonexistent_branch_xyz");
        assert!(result.is_err(), "should fail for invalid ref");
    }

    #[test]
    fn test_working_copy_parent_ref_returns_head() {
        let backend = GitBackend::from_cwd().expect("should open repo");
        assert_eq!(backend.working_copy_parent_ref(), "HEAD");
    }

    #[test]
    fn test_get_parent_ref_or_empty_root_commit() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        // HEAD is the first (root) commit in RepoGuard - has no parent
        let parent_ref = backend
            .get_parent_ref_or_empty("HEAD")
            .expect("should succeed");

        // Should return empty tree SHA for root commit
        assert_eq!(
            parent_ref, "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
            "root commit should return empty tree SHA"
        );
    }

    #[test]
    fn test_get_parent_ref_or_empty_normal_commit() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-parent-ref");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // First commit (root)
        fs::write(dir.join("file.txt"), "first\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "first"]);

        // Second commit (has parent)
        fs::write(dir.join("file.txt"), "second\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "second"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");
        let parent_ref = backend
            .get_parent_ref_or_empty("HEAD")
            .expect("should succeed");

        // Should return HEAD^ for commit with parent
        assert_eq!(parent_ref, "HEAD^", "commit with parent should return SHA^");

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_commits_in_range_empty_range() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        // HEAD..HEAD is empty range
        let commits = backend
            .get_commits_in_range("HEAD", "HEAD")
            .expect("should succeed");
        assert!(commits.is_empty(), "HEAD..HEAD should return empty vec");
    }

    #[test]
    fn test_get_commits_in_range_with_commits() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-range-commits");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Commit A
        fs::write(dir.join("file.txt"), "A\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "commit A"]);

        // Commit B
        fs::write(dir.join("file.txt"), "B\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "commit B"]);

        // Commit C
        fs::write(dir.join("file.txt"), "C\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "commit C"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");

        // Range HEAD~2..HEAD should return commits B and C (2 commits)
        let commits = backend
            .get_commits_in_range("HEAD~2", "HEAD")
            .expect("should get commits");

        assert_eq!(commits.len(), 2, "should have 2 commits in range");
        assert_eq!(commits[0].summary, "commit B", "first should be B (oldest)");
        assert_eq!(
            commits[1].summary, "commit C",
            "second should be C (newest)"
        );

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_commits_in_range_fields_populated() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-range-fields");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // First commit
        fs::write(dir.join("file.txt"), "first\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "first commit"]);

        // Second commit
        fs::write(dir.join("file.txt"), "second\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "second commit"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");
        let commits = backend
            .get_commits_in_range("HEAD~1", "HEAD")
            .expect("should get commits");

        assert_eq!(commits.len(), 1);
        let commit = &commits[0];

        // commit_id should be 40-char hex
        assert_eq!(commit.commit_id.len(), 40, "commit_id should be 40 chars");
        assert!(
            commit.commit_id.chars().all(|c| c.is_ascii_hexdigit()),
            "commit_id should be hex"
        );

        // short_id should be 7 chars (git default)
        assert!(
            commit.short_id.len() >= 7,
            "short_id should be at least 7 chars"
        );

        // change_id should be None for git
        assert!(commit.change_id.is_none(), "git has no change_id");

        // summary should match commit message
        assert_eq!(commit.summary, "second commit");

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_commits_in_range_excludes_empty_commits() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-range-empty");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // First commit with changes
        fs::write(dir.join("file.txt"), "first\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "first with changes"]);

        // Second commit with changes
        fs::write(dir.join("file.txt"), "second\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "second with changes"]);

        // Empty commit (no file changes)
        git(&dir, &["commit", "--allow-empty", "-m", "empty commit"]);

        // Third commit with changes
        fs::write(dir.join("file.txt"), "third\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "third with changes"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");

        // Get range from first commit to HEAD
        let commits = backend
            .get_commits_in_range("HEAD~3", "HEAD")
            .expect("should get commits");

        // Should have 3 commits (second, empty excluded, third) - but empty is excluded
        // so we get 2 commits
        assert_eq!(
            commits.len(),
            2,
            "should have 2 commits (empty commit excluded)"
        );

        // Verify empty commit is not included
        for commit in &commits {
            assert_ne!(
                commit.summary, "empty commit",
                "empty commit should be excluded"
            );
        }

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }
}
