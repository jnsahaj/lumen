use crate::error::LumenError;
use crate::vcs::Vcs;
use thiserror::Error;

use super::GIT_DIFF_EXCLUSIONS;

#[derive(Error, Debug, Clone)]
pub enum CommitError {
    #[error("Commit '{0}' not found")]
    InvalidCommit(String),

    #[error("Diff for commit '{0}' is empty")]
    EmptyDiff(String),
}

#[derive(Clone, Debug)]
pub struct Commit {
    pub full_hash: String,
    pub message: String,
    pub diff: String,
    pub author_name: String,
    pub author_email: String,
    pub date: String,
}

impl Commit {
    pub fn new(sha: String) -> Result<Self, LumenError> {
        Self::is_valid_commit(&sha)?;

        Ok(Commit {
            full_hash: Self::get_full_hash(&sha)?,
            message: Self::get_message(&sha)?,
            diff: Self::get_diff(&sha)?,
            author_name: Self::get_author_name(&sha)?,
            author_email: Self::get_author_email(&sha)?,
            date: Self::get_date(&sha)?,
        })
    }

    pub fn is_valid_commit(sha: &str) -> Result<(), LumenError> {
        let vcs = Vcs::detect();
        let output = vcs.validate_revision(sha).output()?;

        if vcs.is_valid_revision_output(&output) {
            return Ok(());
        }

        Err(CommitError::InvalidCommit(sha.to_string()).into())
    }

    fn get_full_hash(sha: &str) -> Result<String, LumenError> {
        let vcs = Vcs::detect();
        let output = vcs.get_full_hash(sha).output()?;

        let full_hash = String::from_utf8(output.stdout)?.trim_end().to_string();
        Ok(full_hash)
    }

    fn get_diff(sha: &str) -> Result<String, LumenError> {
        let vcs = Vcs::detect();
        let mut cmd = vcs.get_commit_diff(sha);
        
        if vcs == Vcs::Git {
            cmd.args(GIT_DIFF_EXCLUSIONS);
        }
        
        let output = cmd.output()?;
        let diff = String::from_utf8(output.stdout)?;
        
        if diff.is_empty() {
            return Err(CommitError::EmptyDiff(sha.to_string()).into());
        }

        Ok(diff)
    }

    fn get_message(sha: &str) -> Result<String, LumenError> {
        let vcs = Vcs::detect();
        let output = vcs.get_commit_message(sha).output()?;

        let message = String::from_utf8(output.stdout)?.trim_end_matches('\n').to_string();
        Ok(message)
    }

    fn get_author_name(sha: &str) -> Result<String, LumenError> {
        let vcs = Vcs::detect();
        let output = vcs.get_author_name(sha).output()?;

        let name = String::from_utf8(output.stdout)?.trim_end().to_string();
        Ok(name)
    }

    fn get_author_email(sha: &str) -> Result<String, LumenError> {
        let vcs = Vcs::detect();
        let output = vcs.get_author_email(sha).output()?;

        let email = String::from_utf8(output.stdout)?.trim_end().to_string();
        Ok(email)
    }

    fn get_date(sha: &str) -> Result<String, LumenError> {
        let vcs = Vcs::detect();
        let output = vcs.get_commit_date(sha).output()?;

        let date = String::from_utf8(output.stdout)?.trim_end().to_string();
        Ok(date)
    }
}
