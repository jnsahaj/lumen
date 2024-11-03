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

    pub fn build_draft_prompt(git_entity: &GitEntity) -> Self {
        let GitEntity::Diff(diff) = git_entity else {
            panic!("`draft` is only supported for diffs");
        };

        let conventional_types = r#"{
                    "docs": "Documentation only changes",
                    "style": "Changes that do not affect the meaning of the code (white-space, formatting, missing semi-colons, etc)",
                    "refactor": "A code change that neither fixes a bug nor adds a feature",
                    "perf": "A code change that improves performance",
                    "test": "Adding missing tests or correcting existing tests",
                    "build": "Changes that affect the build system or external dependencies",
                    "ci": "Changes to our CI configuration files and scripts",
                    "chore": "Other changes that don't modify src or test files",
                    "revert": "Reverts a previous commit",
                    "feat": "A new feature",
                    "fix": "A bug fix"
                }"#;

        let system_prompt = String::from(
            "You are a commit message generator that follows these rules:\
                        \n1. Write in present tense\
                        \n2. Be concise and direct\
                        \n3. Output only the commit message without any explanations\
                        \n4. Follow the format: <type>(<optional scope>): <commit message>",
        );

        let user_prompt = format!(
                        "Generate a concise git commit message written in present tense for the following code diff with the given specifications below:\n\
                        \nThe output response must be in format:\n<type>(<optional scope>): <commit message>\
                        \nFocus on being accurate and concise.\
                        \nChoose a type from the type-to-description JSON below that best describes the git diff:\n{}\
                        \nCommit message must be a maximum of 72 characters.\
                        \nAdd a description after the commit message.\
                        \nExclude anything unnecessary such as translation. Your entire response will be passed directly into git commit.\
                        \n\nCode diff:\n```diff\n{}\n```",
                        conventional_types,
                        diff.diff
                    );

        AIPrompt {
            system_prompt,
            user_prompt,
        }
    }
}
