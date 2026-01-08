use crate::error::LumenError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DiffError {
    #[error("diff{} is empty", if *staged { " (staged)" } else { "" })]
    EmptyDiff { staged: bool },
}

#[derive(Clone, Debug)]
pub enum Diff {
    WorkingTree {
        staged: bool,
        diff: String,
    },
    CommitsRange {
        from: String,
        to: String,
        diff: String,
    },
}

impl Diff {
    /// Create a working tree diff from a diff string (for VCS backend integration).
    pub fn from_working_tree_diff(diff: String, staged: bool) -> Result<Self, LumenError> {
        if diff.is_empty() {
            return Err(DiffError::EmptyDiff { staged }.into());
        }
        Ok(Diff::WorkingTree { staged, diff })
    }

    /// Create a range diff from a diff string (for VCS backend integration).
    pub fn from_range_diff(diff: String, from: String, to: String) -> Result<Self, LumenError> {
        if diff.is_empty() {
            return Err(DiffError::EmptyDiff { staged: false }.into());
        }
        Ok(Diff::CommitsRange { from, to, diff })
    }
}
