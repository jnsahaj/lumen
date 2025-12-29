use std::io::{IsTerminal, Write};

use crate::{
    config::configuration::DraftConfig, error::LumenError, git_entity::GitEntity,
    provider::LumenProvider,
};

pub struct DraftCommand {
    pub git_entity: GitEntity,
    pub context: Option<String>,
    pub draft_config: DraftConfig,
}

impl DraftCommand {
    pub async fn execute(&self, provider: &LumenProvider) -> Result<(), LumenError> {
        let result = provider.draft(self).await?;

        // Only add newline when outputting to terminal, not when piped (e.g., `lumen draft | pbcopy`)
        if std::io::stdout().is_terminal() {
            println!("{result}");
        } else {
            print!("{result}");
        }
        std::io::stdout().flush()?;
        Ok(())
    }
}
