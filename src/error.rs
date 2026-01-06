use crate::{git_entity::diff::DiffError, provider::ProviderError, vcs::VcsError};
use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LumenError {
    #[error("{0}")]
    GitDiffError(#[from] DiffError),

    #[error("{0}")]
    VcsError(#[from] VcsError),

    #[allow(dead_code)]
    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    #[error(transparent)]
    IoError(#[from] io::Error),

    #[error(transparent)]
    Utf8Error(#[from] std::string::FromUtf8Error),

    #[error("{0}")]
    CommandError(String),

    #[error(transparent)]
    ProviderError(#[from] ProviderError),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
}
