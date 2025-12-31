use crate::error::LumenError;
use crate::vcs::Vcs;
use thiserror::Error;

use super::{commit::Commit, GIT_DIFF_EXCLUSIONS};

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
    pub fn from_working_tree(staged: bool) -> Result<Self, LumenError> {
        let vcs = Vcs::detect();
        let mut cmd = vcs.diff_working_tree(staged);
        
        if vcs == Vcs::Git {
            cmd.args(GIT_DIFF_EXCLUSIONS);
        }

        let output = cmd.output()?;
        let diff = String::from_utf8(output.stdout)?;
        
        if diff.is_empty() {
            return Err(DiffError::EmptyDiff { staged }.into());
        }

        Ok(Diff::WorkingTree { staged, diff })
    }

    pub fn from_commits_range(from: &str, to: &str, triple_dot: bool) -> Result<Self, LumenError> {
        let _ = Commit::is_valid_commit(from)?;
        let _ = Commit::is_valid_commit(to)?;

        let vcs = Vcs::detect();
        
        let actual_from = if triple_dot {
            let output = vcs.get_merge_base(from, to).output()?;
            String::from_utf8(output.stdout)?.trim().to_string()
        } else {
            from.to_string()
        };

        let mut cmd = vcs.diff_range(&actual_from, to);
        
        if vcs == Vcs::Git {
            cmd.args(GIT_DIFF_EXCLUSIONS);
        }

        let output = cmd.output()?;
        let diff = String::from_utf8(output.stdout)?;

        if diff.is_empty() {
            return Err(DiffError::EmptyDiff { staged: false }.into());
        }

        Ok(Diff::CommitsRange {
            from: from.to_string(),
            to: to.to_string(),
            diff,
        })
    }
}
