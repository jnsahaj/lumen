use async_trait::async_trait;
use spinoff::{spinners, Color, Spinner};
use std::io::Read;

use super::{Command, LumenCommand};
use crate::git_entity::commit::Commit;
use crate::git_entity::diff::Diff;
use crate::{error::LumenError, git_entity::GitEntity, provider::LumenProvider};

pub struct ExplainCommand {
    pub git_entity: GitEntity,
    pub query: Option<String>,
}

#[async_trait]
impl Command for ExplainCommand {
    async fn execute(&self, provider: &LumenProvider) -> Result<(), LumenError> {
        LumenCommand::print_with_mdcat(self.git_entity.format_static_details())?;
        if let Some(query) = &self.query {
            LumenCommand::print_with_mdcat(format!("`query`: {query}"))?;
        }

        let spinner_text = match &self.query {
            Some(_) => "Generating answer...".to_string(),
            None => "Generating summary...".to_string(),
        };

        let mut spinner = Spinner::new(spinners::Dots, spinner_text, Color::Blue);
        let result = provider.explain(self).await?;
        spinner.success("Done");

        LumenCommand::print_with_mdcat(result)?;
        Ok(())
    }
}

impl ExplainCommand {
    pub fn new(git_entity: GitEntity, query: Option<String>) -> Result<Self, LumenError> {
        let git_entity = match git_entity {
            GitEntity::Diff(Diff::WorkingTree { staged, diff }) if diff == "-" => {
                // Handle Diff with "-" by reading from stdin
                let mut buffer = String::new();
                std::io::stdin()
                    .read_to_string(&mut buffer)
                    .map_err(LumenError::from)?;

                println!("Replacing '-' with input for Diff: '{}'", buffer.trim());

                GitEntity::Diff(Diff::WorkingTree {
                    staged,
                    diff: buffer.trim().to_string(),
                })
            }

            GitEntity::Commit(commit_sha) if commit_sha.to_string() == "-" => {
                // Handle Commit with "-" by reading from stdin
                let mut buffer = String::new();
                std::io::stdin()
                    .read_to_string(&mut buffer)
                    .map_err(LumenError::from)?;

                println!("Replacing '-' with input for Commit: '{}'", buffer.trim());

                GitEntity::Commit(Commit::new(buffer.trim().to_string())?)
            }
            _ => git_entity,
        };

        Ok(ExplainCommand { git_entity, query })
    }
}
