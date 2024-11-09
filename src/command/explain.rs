use async_trait::async_trait;
use spinoff::{spinners, Color, Spinner};

use crate::{error::LumenError, git_entity::GitEntity, provider::LumenProvider};

use super::{Command, LumenCommand};

pub struct ExplainCommand {
    pub git_entity: GitEntity,
}

#[async_trait]
impl Command for ExplainCommand {
    async fn execute(&self, provider: &LumenProvider) -> Result<(), LumenError> {
        LumenCommand::print_with_mdcat(self.git_entity.format_static_details())?;

        let mut spinner = Spinner::new(spinners::Dots, "Generating Summary...", Color::Blue);
        let result = provider.explain(self).await?;
        spinner.success("Done");

        LumenCommand::print_with_mdcat(result)?;
        Ok(())
    }
}
