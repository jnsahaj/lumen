use crate::{
    git_entity::{commit::CommitError, diff::DiffError},
    provider::ProviderError,
};
use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LumenError {
    #[error("{0}")]
    GitCommitError(#[from] CommitError),

    #[error("{0}")]
    GitDiffError(#[from] DiffError),

    #[allow(dead_code)]
    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error(transparent)]
    IoError(#[from] io::Error),

    #[error(transparent)]
    Utf8Error(#[from] std::string::FromUtf8Error),

    #[error("{0}")]
    CommandError(String),

    #[error(transparent)]
    ProviderError(#[from] ProviderError),
}
