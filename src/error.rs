use crate::vcs::VcsError;
use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LumiereError {
    #[error("{0}")]
    VcsError(#[from] VcsError),

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error(transparent)]
    IoError(#[from] io::Error),

    #[error("{0}")]
    CommandError(String),
}
