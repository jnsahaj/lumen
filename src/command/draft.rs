use std::io::Write;

use async_trait::async_trait;

use crate::{
    error::LumenError,
    git_entity::{git_diff::GitDiff, GitEntity},
    provider::{AIProvider, LumenProvider},
};

use super::Command;

pub struct DraftCommand {
    pub context: Option<String>,
}

#[async_trait]
impl Command for DraftCommand {
    async fn execute(&self, provider: &LumenProvider) -> Result<(), LumenError> {
        let result = provider
            .draft(GitEntity::Diff(GitDiff::new(true)?), self.context.clone())
            .await?;

        print!("{result}");
        std::io::stdout().flush()?;
        Ok(())
    }
}
