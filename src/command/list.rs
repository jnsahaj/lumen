use crate::{
    error::LumenError,
    git_entity::{commit::Commit, GitEntity},
    provider::LumenProvider,
};

use super::{explain::ExplainCommand, LumenCommand};

pub struct ListCommand;

impl ListCommand {
    pub async fn execute(&self, provider: &LumenProvider) -> Result<(), LumenError> {
        let sha = LumenCommand::get_sha_from_fzf()?;
        let git_entity = GitEntity::Commit(Commit::new(sha)?);
        ExplainCommand {
            git_entity,
            query: None,
        }
        .execute(provider)
        .await
    }
}
