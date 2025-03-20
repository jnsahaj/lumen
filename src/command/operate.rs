use async_trait::async_trait;
use spinoff::{spinners, Color, Spinner};

use crate::{error::LumenError, provider::LumenProvider};

use super::{Command, LumenCommand};

pub struct OperateCommand {
    pub query: String,
}

#[async_trait]
impl Command for OperateCommand {
    async fn execute(&self, provider: &LumenProvider) -> Result<(), LumenError> {
        LumenCommand::print_with_mdcat(format!("`query`: {}", &self.query))?;

        let spinner_text = "Generating answer...".to_string();

        let mut spinner = Spinner::new(spinners::Dots, spinner_text, Color::Blue);
        let result = provider.operate(self).await?;
        spinner.success("Done");

        LumenCommand::print_with_mdcat(result)?;
        Ok(())
    }
}
