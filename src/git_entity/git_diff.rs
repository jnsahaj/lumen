use crate::error::LumenError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitDiffError {
    #[error("diff{} is empty", if *staged { " (staged)" } else { "" })]
    EmptyDiff { staged: bool },
}

#[derive(Clone, Debug)]
pub struct GitDiff {
    pub staged: bool,
    pub diff: String,
}

impl GitDiff {
    pub fn new(staged: bool) -> Result<Self, LumenError> {
        Ok(GitDiff {
            staged,
            diff: Self::get_diff(staged)?,
        })
    }

    fn get_diff(staged: bool) -> Result<String, LumenError> {
        let args = if staged {
            vec!["diff", "--staged"]
        } else {
            vec!["diff"]
        };

        let output = std::process::Command::new("git").args(args).output()?;

        let diff = String::from_utf8(output.stdout)?;
        if diff.is_empty() {
            return Err(GitDiffError::EmptyDiff { staged }.into());
        }

        Ok(diff)
    }
}
