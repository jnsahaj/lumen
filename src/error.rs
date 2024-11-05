use crate::git_entity::{git_commit::GitCommitError, git_diff::GitDiffError};
use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LumenError {
    #[error("{0}")]
    GitCommitError(#[from] GitCommitError),

    #[error("{0}")]
    GitDiffError(#[from] GitDiffError),

    #[error("Missing API key for {0}")]
    MissingApiKey(String),

    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    #[error(transparent)]
    IoError(#[from] io::Error),

    #[error(transparent)]
    Utf8Error(#[from] std::string::FromUtf8Error),

    #[error(transparent)]
    UnknownError(#[from] Box<dyn std::error::Error>),
}
