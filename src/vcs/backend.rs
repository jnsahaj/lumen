use std::path::Path;
use thiserror::Error;

/// Error types for VCS operations.
#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum VcsError {
    #[error("invalid reference: {0}")]
    InvalidRef(String),

    #[error("file not found: {0}")]
    FileNotFound(String),

    #[error("not a repository")]
    NotARepository,

    #[error("command failed: {0}")]
    CommandFailed(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

/// Information about a commit from any VCS.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CommitInfo {
    /// The commit ID (git SHA or jj commit ID)
    pub commit_id: String,
    /// The change ID (jj only, None for git)
    pub change_id: Option<String>,
    /// Commit message
    pub message: String,
    /// Diff content
    pub diff: String,
    /// Author name and email
    pub author: String,
    /// Commit timestamp formatted for display (YYYY-MM-DD HH:MM:SS)
    pub date: String,
}

/// Abstraction over git and jj backends.
///
/// Note: This trait intentionally does not require `Send + Sync` bounds.
/// The VCS backend is used synchronously from a single thread - there's no
/// cross-thread sharing needed. JjBackend holds jj_lib::Workspace which may
/// not be thread-safe, so adding these bounds would prevent jj support.
#[allow(dead_code)]
pub trait VcsBackend {
    /// Get commit info for a reference (SHA, HEAD, @, etc.)
    fn get_commit(&self, reference: &str) -> Result<CommitInfo, VcsError>;

    /// Get diff of uncommitted changes (working tree vs HEAD/parent).
    /// `staged` is only relevant for git; jj ignores it.
    fn get_working_tree_diff(&self, staged: bool) -> Result<String, VcsError>;

    /// Get diff between two refs (e.g., commit1..commit2).
    fn get_range_diff(&self, from: &str, to: &str, three_dot: bool) -> Result<String, VcsError>;

    /// Get list of changed files for a commit or range.
    fn get_changed_files(&self, reference: &str) -> Result<Vec<String>, VcsError>;

    /// Get file content at a specific ref.
    fn get_file_content_at_ref(&self, reference: &str, path: &Path) -> Result<String, VcsError>;

    /// Get current branch name (or bookmark for jj).
    fn get_current_branch(&self) -> Result<Option<String>, VcsError>;

    /// Get commit log formatted for fzf selection.
    fn get_commit_log_for_fzf(&self) -> Result<String, VcsError>;

    /// Resolve a reference to a canonical commit SHA.
    /// Works with any ref type: git SHA, jj change ID, @, @-, bookmarks, branches, etc.
    fn resolve_ref(&self, reference: &str) -> Result<String, VcsError>;

    /// Get list of files changed in working tree (staged + unstaged + untracked).
    /// For git: combines diff --name-only, diff --cached --name-only, ls-files --others.
    /// For jj: diffs @ tree vs @- tree.
    fn get_working_tree_changed_files(&self) -> Result<Vec<String>, VcsError>;

    /// Get the merge base (common ancestor) of two refs.
    /// Used for triple-dot diffs (A...B).
    /// For git: runs 'git merge-base <ref1> <ref2>'.
    /// For jj: uses revset to find common ancestor.
    fn get_merge_base(&self, ref1: &str, ref2: &str) -> Result<String, VcsError>;

    /// Get the parent reference for working tree comparisons.
    /// For git: returns "HEAD".
    /// For jj: returns "@-".
    fn working_copy_parent_ref(&self) -> &'static str;

    /// Get list of files changed between two refs (range diff).
    /// For git: runs 'git diff --name-only <from> <to>'.
    /// For jj: diffs the trees of the two commits.
    fn get_range_changed_files(&self, from: &str, to: &str) -> Result<Vec<String>, VcsError>;
}
