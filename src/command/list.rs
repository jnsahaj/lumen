use async_trait::async_trait;

use crate::{
    error::LumenError,
    git_entity::{git_commit::GitCommit, GitEntity},
    provider::LumenProvider,
};

use super::{explain::ExplainCommand, Command, LumenCommand};

pub struct ListCommand;

#[async_trait]
impl Command for ListCommand {
    async fn execute(&self, provider: &LumenProvider) -> Result<(), LumenError> {
        let sha = LumenCommand::get_sha_from_fzf()?;
        let git_entity = GitEntity::Commit(GitCommit::new(sha)?);
        ExplainCommand {
            git_entity,
            query: None,
        }
        .execute(provider)
        .await
    }
}
