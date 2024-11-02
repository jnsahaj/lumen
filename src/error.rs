use std::io;

use crate::git_entity::{git_commit::GitCommitError, git_diff::GitDiffError};

pub enum LumenError {
    GitCommitError(GitCommitError),
    GitDiffError(GitDiffError),
    MissingApiKey(String),
    UnknownError(Box<dyn std::error::Error>),
    InvalidArguments(String),
}

impl From<GitCommitError> for LumenError {
    fn from(err: GitCommitError) -> LumenError {
        LumenError::GitCommitError(err)
    }
}

impl From<GitDiffError> for LumenError {
    fn from(err: GitDiffError) -> LumenError {
        LumenError::GitDiffError(err)
    }
}

impl From<Box<dyn std::error::Error>> for LumenError {
    fn from(err: Box<dyn std::error::Error>) -> LumenError {
        LumenError::UnknownError(err)
    }
}

impl From<io::Error> for LumenError {
    fn from(err: io::Error) -> LumenError {
        LumenError::UnknownError(err.into())
    }
}

impl From<std::string::FromUtf8Error> for LumenError {
    fn from(err: std::string::FromUtf8Error) -> LumenError {
        LumenError::UnknownError(err.into())
    }
}

impl std::fmt::Display for LumenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LumenError::GitCommitError(err) => write!(f, "{err}"),
            LumenError::UnknownError(err) => write!(f, "{err}"),
            LumenError::MissingApiKey(provider) => write!(f, "Missing API key for {provider}"),
            LumenError::GitDiffError(err) => write!(f, "{err}"),
            LumenError::InvalidArguments(err) => write!(f, "Invalid arguments: {err}"),
        }
    }
}
