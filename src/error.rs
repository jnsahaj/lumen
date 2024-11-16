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

    #[error("Missing API key for {0}, use --api-key or LUMEN_API_KEY env variable, or add \"api_key\": \"...\" to configuration file")]
    MissingApiKey(String),

    #[error("Missing Model for {0}, use --model or LUMEN_MODEL env variable, or add \"model\": \"...\" to configuration file")]
    MissingModel(String),

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
