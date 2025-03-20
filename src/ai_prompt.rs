use crate::{
    command::{draft::DraftCommand, explain::ExplainCommand},
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

        let user_prompt = match &command.query {
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

        let user_prompt = String::from(formatdoc! {"
            Generate a concise git commit message written in present tense for the following code diff with the given specifications below:

            The output response must be in format:
            <type>(<optional scope>): <commit message>
            Choose a type from the type-to-description JSON below that best describes the git diff:
            {commit_types}
            Focus on being accurate and concise.
            {context}
            Commit message must be a maximum of 72 characters.
            Exclude anything unnecessary such as translation. Your entire response will be passed directly into git commit.

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
        You are a Git operations assistant that helps users execute the right Git commands.
        When given a request:
        1. Determine the precise Git command to accomplish the task
        2. Provide a brief, clear explanation of what the command does
        3. Include any warnings about potential data loss or destructive operations
        4. Format your response using the specified XML tags
    "});

        let user_prompt = formatdoc! {"
        Generate the appropriate Git command for this request:

        Request: {query}

        Respond with:
        <command>The exact Git command to execute</command>
        <explanation>A brief explanation of what this command does and why it's appropriate</explanation>
        <warning>Any warnings about potential data loss or side effects (if applicable)</warning>

        Make sure the command works as expected. If the request is ambiguous, choose the most likely interpretation.
        For time-based operations, use standard Git time formats.
        ",
            query = query
        };

        Ok(AIPrompt {
            system_prompt,
            user_prompt,
        })
    }
}
