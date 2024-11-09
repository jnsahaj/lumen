use std::io::Write;

use async_trait::async_trait;

use crate::{error::LumenError, git_entity::GitEntity, provider::LumenProvider};

use super::Command;

pub struct DraftCommand {
    pub git_entity: GitEntity,
    pub context: Option<String>,
}

#[async_trait]
impl Command for DraftCommand {
    async fn execute(&self, provider: &LumenProvider) -> Result<(), LumenError> {
        let result = provider.draft(self).await?;

        print!("{result}");
        std::io::stdout().flush()?;
        Ok(())
    }
}
