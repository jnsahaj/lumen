use crate::error::LumenError;

pub enum GitDiffError {
    EmptyDiff,
}

impl std::fmt::Display for GitDiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitDiffError::EmptyDiff => write!(f, "Diff is empty"),
        }
    }
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
        let output = std::process::Command::new("git")
            .args(["diff"])
            .arg(if staged { "--staged" } else { "" })
            .output()?;

        let diff = String::from_utf8(output.stdout)?;
        if diff.is_empty() {
            return Err(GitDiffError::EmptyDiff.into());
        }

        Ok(diff)
    }
}
