use std::io::Write;

use async_trait::async_trait;
use regex::Regex;
use lazy_static::lazy_static;

use crate::{
    config::configuration::DraftConfig, error::LumenError, git_entity::GitEntity,
    provider::LumenProvider,
};

use super::Command;

lazy_static! {
    static ref THINK_TAG_REGEX: Regex = Regex::new(r"(?s)<think>.*?</think>\s*").unwrap();
}

pub struct DraftCommand {
    pub git_entity: GitEntity,
    pub context: Option<String>,
    pub draft_config: DraftConfig,
}

#[async_trait]
impl Command for DraftCommand {
    async fn execute(&self, provider: &LumenProvider) -> Result<(), LumenError> {
        let mut result = provider.draft(self).await?;
        if self.draft_config.trim_thinking_tags {
            result = THINK_TAG_REGEX.replace_all(&result, "").to_string();
            result = result.trim().to_string();
        }
        print!("{result}");
        std::io::stdout().flush()?;
        Ok(())
    }
}
