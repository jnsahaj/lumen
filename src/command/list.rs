use crate::{
    error::LumenError,
    git_entity::{commit::Commit, GitEntity},
    provider::LumenProvider,
    vcs::VcsBackend,
};

use super::{explain::ExplainCommand, LumenCommand};

pub struct ListCommand;

impl ListCommand {
    pub async fn execute(
        &self,
        provider: &LumenProvider,
        backend: &dyn VcsBackend,
    ) -> Result<(), LumenError> {
        let sha = LumenCommand::get_sha_from_fzf(backend)?;
        let info = backend.get_commit(&sha)?;
        let git_entity = GitEntity::Commit(Commit::from_commit_info(info));
        ExplainCommand {
            git_entity,
            query: None,
        }
        .execute(provider)
        .await
    }
}
