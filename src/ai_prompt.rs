use crate::git_entity::GitEntity;

pub struct AIPrompt {
    pub system_prompt: String,
    pub user_prompt: String,
}

impl AIPrompt {
    pub fn build_explain_prompt(git_entity: &GitEntity) -> Self {
        let system_prompt = String::from(
            "You are a helpful assistant that explains Git changes in a concise way. \
            Focus only on the most significant changes and their direct impact. \
            Keep explanations brief but informative and don't ask for further explanations. \
            Use markdown for clarity.",
        );

        let user_prompt = match git_entity {
            GitEntity::Commit(commit) => {
                format!(
                    "Explain this commit briefly:\n\
                    \nMessage: {}\
                    \n\nChanges:\n```diff\n{}\n```\
                    \n\nProvide a short explanation covering:\n\
                    1. Core changes made\n\
                    2. Direct impact",
                    commit.message, commit.diff
                )
            }
            GitEntity::Diff(diff) => {
                format!(
                    "Explain these changes concisely:\n\
                    \n```diff\n{}\n```\
                    \n\nProvide:\n\
                    1. Key changes\n\
                    2. Notable concerns (if any)",
                    diff.diff
                )
            }
        };

        AIPrompt {
            system_prompt,
            user_prompt,
        }
    }
}
