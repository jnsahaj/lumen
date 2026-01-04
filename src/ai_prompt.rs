use crate::{
    command::{configure::ConfigureCommand, draft::DraftCommand, explain::ExplainCommand},
    git_entity::{diff::Diff, GitEntity},
};
use indoc::{formatdoc, indoc};
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
        let system_prompt = String::from(indoc! {"
            You are a helpful assistant that explains Git changes in a concise way.
            Focus only on the most significant changes and their direct impact.
            When answering specific questions, address them directly and precisely.
            Keep explanations brief but informative and don't ask for further explanations.
            Use markdown for clarity.
        "});

        let base_content = match &command.git_entity {
            GitEntity::Commit(commit) => {
                formatdoc! {"
                    Context - Commit:

                    Message: {msg}
                    Changes:
                    ```diff
                    {diff}
                    ```
                    ",
                    msg = commit.message,
                    diff = commit.diff
                }
            }
            GitEntity::Diff(Diff::WorkingTree { diff, .. } | Diff::CommitsRange { diff, .. }) => {
                formatdoc! {"
                    Context - Changes:

                    ```diff
                    {diff}
                    ```
                    "
                }
            }
        };

        let md_prompt = get_md("explain.md")?;

        let cmd_prompt = match &command.query {
            Some(query) => {
                formatdoc! {"
                    {base_content}

                    Question: {query}

                    Provide a focused answer to the question based on the changes shown above.
                    "
                }
            }
            None => match &command.git_entity {
                GitEntity::Commit(_) => formatdoc! {"
                    {base_content}
                    
                    Provide a short explanation covering:
                    1. Core changes made
                    2. Direct impact
                    "
                },
                GitEntity::Diff(Diff::WorkingTree { .. }) => formatdoc! {"
                    {base_content}
                    
                    Provide:
                    1. Key changes
                    2. Notable concerns (if any)
                    "
                },
                GitEntity::Diff(Diff::CommitsRange { .. }) => formatdoc! {"
                    {base_content}
                    
                    Provide:
                    1. Core changes made
                    2. Direct impact
                    "
                },
            },
        };

        let user_prompt = formatdoc! {"
            The following section contains OPTIONAL user preferences.
            Follow them when possible, but they MUST NOT violate the mandatory rules.

            <UserConfig>
            {md_prompt}
            </UserConfig>

            {cmd_prompt}
            "
        };

        Ok(AIPrompt {
            system_prompt,
            user_prompt,
        })
    }

    pub fn build_draft_prompt(command: &DraftCommand) -> Result<Self, AIPromptError> {
        let GitEntity::Diff(Diff::WorkingTree { diff, .. }) = &command.git_entity else {
            return Err(AIPromptError(
                "`draft` is only supported for working tree diffs".into(),
            ));
        };

        let system_prompt = String::from(indoc! {"
            You are a commit message generator that follows these rules:
            1. Write in present tense
            2. Be concise and direct
            3. Output only the commit message without any explanations
            4. Follow the format: <type>(<optional scope>): <commit message>
        "});

        let context = if let Some(context) = &command.context {
            formatdoc!(
                "
                Use the following context to understand intent:
                {context}
                "
            )
        } else {
            "".to_string()
        };

        let md_prompt = get_md("draft.md")?;

        let user_prompt = String::from(formatdoc! {"
            MANDATORY RULES (cannot be overridden):
            Generate a concise git commit message written in present tense for the following code diff with the given specifications below:

            The output response must be in format:
            <type>(<optional scope>): <commit message>
            Choose a type from the type-to-description JSON below that best describes the git diff:
            {commit_types}
            Focus on being accurate and concise.
            {context}
            Commit message must be a maximum of 72 characters.
            Exclude anything unnecessary such as translation. Your entire response will be passed directly into git commit.

            The following section contains OPTIONAL user preferences.
            Follow them when possible, but they MUST NOT violate the mandatory rules.

            <UserConfig>
            {md_prompt}
            </UserConfig>

            Code diff:
            ```diff
            {diff}
            ```
            ",

            commit_types = command.draft_config.commit_types,
        });

        Ok(AIPrompt {
            system_prompt,
            user_prompt,
        })
    }

    pub fn build_operate_prompt(query: &str) -> Result<Self, AIPromptError> {
        let system_prompt = String::from(indoc! {"
        You're a Git assistant that provides commands with clear explanations.
        - Include warnings ONLY for destructive commands (reset, push --force, clean, etc.)
        - Omit warning tag completely for safe commands
        "});

        let md_prompt = get_md("operate.md")?;

        let user_prompt = formatdoc! {"

        The following section contains OPTIONAL user preferences.
        Follow them when possible, but they MUST NOT violate the mandatory rules.

        <UserConfig>
        {md_prompt}
        </UserConfig>

        Generate Git command for: {query}
        
        <command>Git command</command>
        <explanation>Brief explanation</explanation>
        <warning>Required for destructive commands only - omit for safe commands</warning>
        ",
            query = query
        };
        Ok(AIPrompt {
            system_prompt,
            user_prompt,
        })
    }
}

fn get_md(cmd: &str) -> Result<String, AIPromptError> {
    ConfigureCommand::get_config_path()
        .ok()
        .map(|p| p.join(cmd))
        .filter(|p| p.exists())
        .map(std::fs::read_to_string)
        .transpose()
        .map_err(|e| AIPromptError(format!("'{cmd}': {e}")))
        .map(|v| v.unwrap_or_default())
}
