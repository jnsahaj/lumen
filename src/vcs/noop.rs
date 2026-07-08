use std::path::Path;

use super::backend::{CommitInfo, StackedCommitInfo, VcsBackend, VcsError};

pub struct NoopBackend {
    label: Option<String>,
}

impl NoopBackend {
    pub fn new(label: Option<String>) -> Self {
        Self { label }
    }
}

impl VcsBackend for NoopBackend {
    fn get_commit(&self, _reference: &str) -> Result<CommitInfo, VcsError> {
        Err(VcsError::NotARepository)
    }

    fn get_working_tree_diff(&self, _staged: bool) -> Result<String, VcsError> {
        Err(VcsError::NotARepository)
    }

    fn get_range_diff(&self, _from: &str, _to: &str, _three_dot: bool) -> Result<String, VcsError> {
        Err(VcsError::NotARepository)
    }

    fn get_changed_files(&self, _reference: &str) -> Result<Vec<String>, VcsError> {
        Ok(Vec::new())
    }

    fn get_file_content_at_ref(&self, _reference: &str, _path: &Path) -> Result<String, VcsError> {
        Ok(String::new())
    }

    fn get_current_branch(&self) -> Result<Option<String>, VcsError> {
        Ok(self.label.clone())
    }

    fn get_commit_log_for_fzf(&self) -> Result<String, VcsError> {
        Err(VcsError::NotARepository)
    }

    fn resolve_ref(&self, _reference: &str) -> Result<String, VcsError> {
        Err(VcsError::NotARepository)
    }

    fn get_working_tree_changed_files(&self) -> Result<Vec<String>, VcsError> {
        Ok(Vec::new())
    }

    fn get_merge_base(&self, _ref1: &str, _ref2: &str) -> Result<String, VcsError> {
        Err(VcsError::NotARepository)
    }

    fn working_copy_parent_ref(&self) -> &'static str {
        ""
    }

    fn get_range_changed_files(&self, _from: &str, _to: &str) -> Result<Vec<String>, VcsError> {
        Ok(Vec::new())
    }

    fn get_parent_ref_or_empty(&self, _reference: &str) -> Result<String, VcsError> {
        Ok(String::new())
    }

    fn get_commits_in_range(
        &self,
        _from: &str,
        _to: &str,
    ) -> Result<Vec<StackedCommitInfo>, VcsError> {
        Ok(Vec::new())
    }

    fn name(&self) -> &'static str {
        "diff"
    }
}
