use crate::error::LumenError;
use thiserror::Error;

use super::commit::Commit;

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
        let args = if staged {
            vec!["diff", "--staged"]
        } else {
            vec!["diff"]
        };

        let output = std::process::Command::new("git").args(args).output()?;

        let diff = String::from_utf8(output.stdout)?;
        if diff.is_empty() {
            return Err(DiffError::EmptyDiff { staged }.into());
        }

        Ok(Diff::WorkingTree { staged, diff })
    }

    pub fn from_commits_range(from: &str, to: &str) -> Result<Self, LumenError> {
        let _ = Commit::is_valid_commit(from)?;
        let _ = Commit::is_valid_commit(to)?;

        let output = std::process::Command::new("git")
            .args(["diff", from, to])
            .output()?;

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
