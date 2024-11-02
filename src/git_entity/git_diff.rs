use crate::error::LumenError;

pub enum GitDiffError {
    EmptyDiff,
}

#[derive(Clone, Debug)]
pub struct GitDiff {
    pub diff: String,
}

impl GitDiff {
    pub fn new_staged_diff() -> Result<Self, LumenError> {
        Ok(GitDiff {
            diff: Self::get_staged_diff()?,
        })
    }

    fn get_staged_diff() -> Result<String, LumenError> {
        let output = std::process::Command::new("git")
            .args(["diff", "--staged"])
            .output()?;

        let diff = String::from_utf8(output.stdout)?;
        if diff.is_empty() {
            return Err(GitDiffError::EmptyDiff.into());
        }

        Ok(diff)
    }
}
