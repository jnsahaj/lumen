use crate::{
    command::{draft::DraftCommand, explain::ExplainCommand},
    git_entity::GitEntity,
};
use thiserror::Error;

#[derive(Error, Debug)]
#[error("{0}")]
pub struct AIPromptError(String);

pub struct AIPrompt {
    pub system_prompt: String,
    pub user_prompt: String,
}

impl AIPrompt {
    pub fn build_explain_prompt(command: &ExplainCommand) -> Result<Self, AIPromptError> {
        let system_prompt = String::from(
            "You are a helpful assistant that explains Git changes in a concise way. \
        Focus only on the most significant changes and their direct impact. \
        When answering specific questions, address them directly and precisely. \
        Keep explanations brief but informative and don't ask for further explanations. \
        Use markdown for clarity.",
        );

        let base_content = match &command.git_entity {
            GitEntity::Commit(commit) => {
                format!(
                    "Context - Commit:\n\
                \nMessage: {}\
                \nChanges:\n```diff\n{}\n```",
                    commit.message, commit.diff
                )
            }
            GitEntity::Diff(diff) => {
                format!(
                    "Context - Changes:\n\
                \n```diff\n{}\n```",
                    diff.diff
                )
            }
        };

        let user_prompt = match &command.query {
            Some(query) => {
                format!(
                    "{}\n\nQuestion: {}\
                \nProvide a focused answer to the question based on the changes shown above.",
                    base_content, query
                )
            }
            None => {
                let analysis_points = match &command.git_entity {
                    GitEntity::Commit(_) => {
                        "\n\nProvide a short explanation covering:\n\
                    1. Core changes made\n\
                    2. Direct impact"
                    }
                    GitEntity::Diff(_) => {
                        "\n\nProvide:\n\
                    1. Key changes\n\
                    2. Notable concerns (if any)"
                    }
                };
                format!("{}{}", base_content, analysis_points)
            }
        };

        Ok(AIPrompt {
            system_prompt,
            user_prompt,
        })
    }

    pub fn build_draft_prompt(command: &DraftCommand) -> Result<Self, AIPromptError> {
        let GitEntity::Diff(diff) = &command.git_entity else {
            return Err(AIPromptError("`draft` is only supported for diffs".into()));
        };

        let system_prompt = String::from(
            "You are a commit message generator that follows these rules:\
                        \n1. Write in present tense\
                        \n2. Be concise and direct\
                        \n3. Output only the commit message without any explanations\
                        \n4. Follow the format: <type>(<optional scope>): <commit message>",
        );

        let context = if let Some(context) = &command.context {
            format!("Use the following context to understand intent:\n{context}")
        } else {
            "".to_string()
        };

        let user_prompt = format!(
                "Generate a concise git commit message written in present tense for the following code diff with the given specifications below:\n\
                \nThe output response must be in format:\n<type>(<optional scope>): <commit message>\
                \nChoose a type from the type-to-description JSON below that best describes the git diff:\n{}\
                \nFocus on being accurate and concise.\
                \n{}\
                \nCommit message must be a maximum of 72 characters.\
                \nExclude anything unnecessary such as translation. Your entire response will be passed directly into git commit.\
                \n\nCode diff:\n```diff\n{}\n```",
                command.draft_config.commit_types,
                context,
                diff.diff
            );

        Ok(AIPrompt {
            system_prompt,
            user_prompt,
        })
    }
}
