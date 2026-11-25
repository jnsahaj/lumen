use std::collections::HashSet;
use std::path::{Path, PathBuf};

use git2::{Commit, Repository, StatusOptions, Tree};

use super::backend::{StackedCommitInfo, VcsBackend, VcsError};

const GIT_EMPTY_TREE_SHA: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

pub struct GitBackend {
    repo: Repository,
    working_tree_root: PathBuf,
}

impl GitBackend {
    pub fn new(path: &Path) -> Result<Self, VcsError> {
        let repo = Repository::discover(path).map_err(|_| VcsError::NotARepository)?;
        let working_tree_root = repo.workdir().unwrap_or_else(|| repo.path()).to_path_buf();
        Ok(Self {
            repo,
            working_tree_root,
        })
    }

    fn validate_ref_format(reference: &str) -> Result<(), VcsError> {
        if reference.trim().starts_with('-') {
            return Err(VcsError::InvalidRef(format!(
                "references cannot start with '-': {reference}"
            )));
        }
        Ok(())
    }

    fn resolve_commit(&self, reference: &str) -> Result<Commit<'_>, VcsError> {
        let reference = reference.trim();
        Self::validate_ref_format(reference)?;
        let obj = self
            .repo
            .revparse_single(reference)
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;
        obj.peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))
    }

    fn resolve_tree(&self, reference: &str) -> Result<Tree<'_>, VcsError> {
        self.resolve_commit(reference)?
            .tree()
            .map_err(|e| VcsError::Other(format!("failed to get tree for {reference}: {e}")))
    }

    fn diff_tree_paths(
        &self,
        from: Option<&Tree<'_>>,
        to: Option<&Tree<'_>>,
    ) -> Result<Vec<String>, VcsError> {
        let diff = self
            .repo
            .diff_tree_to_tree(from, to, None)
            .map_err(|e| VcsError::Other(format!("failed to create diff: {e}")))?;

        Ok(diff
            .deltas()
            .filter_map(|d| {
                d.new_file()
                    .path()
                    .and_then(|p| p.to_str().map(String::from))
            })
            .collect())
    }
}

impl VcsBackend for GitBackend {
    fn get_changed_files(&self, reference: &str) -> Result<Vec<String>, VcsError> {
        let reference = reference.trim();

        if let Some((from, to)) = reference
            .split_once("...")
            .or_else(|| reference.split_once(".."))
        {
            return self.get_range_changed_files(from, to);
        }

        let commit = self.resolve_commit(reference)?;
        let tree = commit
            .tree()
            .map_err(|e| VcsError::Other(format!("failed to get commit tree: {e}")))?;
        let parent_tree = if commit.parent_count() > 0 {
            commit.parent(0).ok().and_then(|p| p.tree().ok())
        } else {
            None
        };

        self.diff_tree_paths(parent_tree.as_ref(), Some(&tree))
    }

    fn get_file_content_at_ref(&self, reference: &str, path: &Path) -> Result<String, VcsError> {
        let tree = self.resolve_tree(reference)?;
        let entry = tree
            .get_path(path)
            .map_err(|_| VcsError::FileNotFound(path.display().to_string()))?;
        let blob = self
            .repo
            .find_blob(entry.id())
            .map_err(|_| VcsError::FileNotFound(path.display().to_string()))?;

        Ok(String::from_utf8_lossy(blob.content()).into_owned())
    }

    fn get_files_content_at_ref(&self, reference: &str, paths: &[&Path]) -> Vec<Option<String>> {
        let Ok(tree) = self.resolve_tree(reference) else {
            return vec![None; paths.len()];
        };
        paths
            .iter()
            .map(|path| {
                let entry = tree.get_path(path).ok()?;
                let blob = self.repo.find_blob(entry.id()).ok()?;
                Some(String::from_utf8_lossy(blob.content()).into_owned())
            })
            .collect()
    }

    fn get_current_branch(&self) -> Result<Option<String>, VcsError> {
        let head = self
            .repo
            .head()
            .map_err(|e| VcsError::Other(format!("failed to get HEAD: {e}")))?;

        Ok(head
            .is_branch()
            .then(|| head.shorthand().map(String::from))
            .flatten())
    }

    fn resolve_ref(&self, reference: &str) -> Result<String, VcsError> {
        Ok(self.resolve_commit(reference)?.id().to_string())
    }

    fn get_working_tree_changed_files(&self) -> Result<Vec<String>, VcsError> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .exclude_submodules(true)
            .include_ignored(false);

        let statuses = self
            .repo
            .statuses(Some(&mut opts))
            .map_err(|e| VcsError::Other(format!("failed to get status: {e}")))?;

        let files: HashSet<String> = statuses
            .iter()
            .filter_map(|s| s.path().map(String::from))
            .collect();

        Ok(files.into_iter().collect())
    }

    fn get_merge_base(&self, ref1: &str, ref2: &str) -> Result<String, VcsError> {
        let oid1 = self.resolve_commit(ref1)?.id();
        let oid2 = self.resolve_commit(ref2)?.id();

        let merge_base = self
            .repo
            .merge_base(oid1, oid2)
            .map_err(|e| VcsError::Other(format!("failed to find merge base: {e}")))?;

        Ok(merge_base.to_string())
    }

    fn working_copy_parent_ref(&self) -> &'static str {
        "HEAD"
    }

    fn get_range_changed_files(&self, from: &str, to: &str) -> Result<Vec<String>, VcsError> {
        let from_tree = self.resolve_tree(from)?;
        let to_tree = self.resolve_tree(to)?;
        self.diff_tree_paths(Some(&from_tree), Some(&to_tree))
    }

    fn get_parent_ref_or_empty(&self, reference: &str) -> Result<String, VcsError> {
        let commit = self.resolve_commit(reference)?;
        if commit.parent_count() > 0 {
            Ok(format!("{}^", reference.trim()))
        } else {
            Ok(GIT_EMPTY_TREE_SHA.to_string())
        }
    }

    fn get_commits_in_range(
        &self,
        from: &str,
        to: &str,
    ) -> Result<Vec<StackedCommitInfo>, VcsError> {
        let from_oid = self.resolve_commit(from)?.id();
        let to_oid = self.resolve_commit(to)?.id();

        let mut revwalk = self
            .repo
            .revwalk()
            .map_err(|e| VcsError::Other(format!("failed to create revwalk: {e}")))?;
        revwalk
            .push(to_oid)
            .map_err(|e| VcsError::Other(format!("failed to push to revwalk: {e}")))?;
        revwalk
            .hide(from_oid)
            .map_err(|e| VcsError::Other(format!("failed to hide from revwalk: {e}")))?;

        let mut commits = Vec::new();
        for oid_result in revwalk {
            let oid = oid_result.map_err(|e| VcsError::Other(format!("revwalk error: {e}")))?;
            let commit = self
                .repo
                .find_commit(oid)
                .map_err(|e| VcsError::Other(format!("failed to find commit: {e}")))?;

            let commit_id = oid.to_string();
            let short_id = commit_id[..7.min(commit_id.len())].to_string();
            let summary = commit.summary().unwrap_or("").to_string();

            if self
                .get_changed_files(&commit_id)
                .is_ok_and(|files| !files.is_empty())
            {
                commits.push(StackedCommitInfo {
                    commit_id,
                    short_id,
                    change_id: None,
                    summary,
                });
            }
        }

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
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::vcs::test_utils::git;

    fn init_repo() -> TempDir {
        let dir = TempDir::new().expect("temp dir");
        git(dir.path(), &["init"]);
        git(dir.path(), &["config", "user.email", "test@example.com"]);
        git(dir.path(), &["config", "user.name", "Test User"]);
        fs::write(dir.path().join("README.md"), "hello\n").expect("write README");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "init"]);
        dir
    }

    #[test]
    fn test_basic_commit_and_content_queries() {
        let dir = init_repo();
        let backend = GitBackend::new(dir.path()).expect("open repo");

        assert!(backend
            .get_changed_files("HEAD")
            .unwrap()
            .contains(&"README.md".to_string()));
        assert!(backend.get_current_branch().unwrap().is_some());
        assert_eq!(
            backend
                .get_file_content_at_ref("HEAD", Path::new("README.md"))
                .unwrap(),
            "hello\n"
        );
        assert!(matches!(
            backend.get_file_content_at_ref("HEAD", Path::new("missing.txt")),
            Err(VcsError::FileNotFound(_))
        ));

        let sha = backend.resolve_ref("HEAD").unwrap();
        assert_eq!(sha.len(), 40);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(backend.resolve_ref("nonexistent_ref_xyz").is_err());

        for reference in ["--upload-pack=evil", "-n"] {
            assert!(matches!(
                backend.resolve_ref(reference),
                Err(VcsError::InvalidRef(_))
            ));
        }
    }

    #[test]
    fn test_working_tree_changed_files() {
        let dir = init_repo();
        let backend = GitBackend::new(dir.path()).expect("open repo");
        assert!(backend.get_working_tree_changed_files().unwrap().is_empty());

        fs::write(dir.path().join("README.md"), "modified\n").unwrap();
        fs::write(dir.path().join("new.txt"), "new file\n").unwrap();

        let files = backend.get_working_tree_changed_files().unwrap();
        assert!(files.contains(&"README.md".to_string()));
        assert!(files.contains(&"new.txt".to_string()));
    }

    #[test]
    fn test_merge_base() {
        let dir = init_repo();
        let p = dir.path();

        git(p, &["checkout", "-b", "branch"]);
        fs::write(p.join("README.md"), "branch\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "branch commit"]);

        let base_branch = GitBackend::new(p).unwrap().resolve_ref("HEAD~1").unwrap();
        git(p, &["checkout", "-b", "other"]);
        fs::write(p.join("other.txt"), "other\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "other commit"]);

        let backend = GitBackend::new(p).unwrap();
        let merge_base = backend.get_merge_base("branch", "other").unwrap();
        assert_eq!(merge_base.len(), 40);
        assert_ne!(merge_base, base_branch);
        assert!(backend.get_merge_base("HEAD", "nonexistent_xyz").is_err());
    }

    #[test]
    fn test_parent_ref_or_empty_for_root_and_child_commits() {
        let dir = init_repo();
        let backend = GitBackend::new(dir.path()).unwrap();

        assert_eq!(
            backend.get_parent_ref_or_empty("HEAD").unwrap(),
            GIT_EMPTY_TREE_SHA
        );

        fs::write(dir.path().join("README.md"), "second\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "second"]);

        assert_eq!(backend.get_parent_ref_or_empty("HEAD").unwrap(), "HEAD^");
    }

    #[test]
    fn test_commits_in_range_ordering_fields_and_empty_filtering() {
        let dir = init_repo();
        let p = dir.path();

        fs::write(p.join("file.txt"), "B\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "commit B"]);

        git(p, &["commit", "--allow-empty", "-m", "empty commit"]);

        fs::write(p.join("file.txt"), "C\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "commit C"]);

        let backend = GitBackend::new(p).unwrap();
        assert!(backend
            .get_commits_in_range("HEAD", "HEAD")
            .unwrap()
            .is_empty());

        let commits = backend.get_commits_in_range("HEAD~3", "HEAD").unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].summary, "commit B");
        assert_eq!(commits[1].summary, "commit C");
        assert_eq!(commits[0].commit_id.len(), 40);
        assert_eq!(commits[0].short_id.len(), 7);
        assert!(commits[0].change_id.is_none());
    }
}
