use crate::{
    command::{draft::DraftCommand, explain::ExplainCommand},
    config::configuration::{DraftConfig, ExplainConfig, OperateConfig},
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
    // Helper function to replace variables in custom prompts
    fn format_template(template: &str, vars: &[(&str, &str)]) -> String {
        let mut result = template.to_string();
        for (var, value) in vars {
            result = result.replace(var, value);
        }
        result
    }
    pub fn build_explain_prompt(
        command: &ExplainCommand,
        config: &ExplainConfig,
    ) -> Result<Self, AIPromptError> {
        let default_system_prompt = indoc! {"
            You are a helpful assistant that explains Git changes in a concise way.
            Focus only on the most significant changes and their direct impact.
            When answering specific questions, address them directly and precisely.
            Keep explanations brief but informative and don't ask for further explanations.
            Use markdown for clarity.
        "};

        let system_prompt = config
            .system_prompt
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.as_str())
            .unwrap_or(default_system_prompt)
            .to_string();

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

        let default_user_prompt = match &command.query {
            Some(query) => {
                formatdoc! {"
                    {base_content}

                    Question: {query}

                    Provide a focused answer to the question based on the changes shown above.
                    ",
                    base_content = base_content,
                    query = query
                }
            }
            None => match &command.git_entity {
                GitEntity::Commit(_) => formatdoc! {"
                    {base_content}
                    
                    Provide a short explanation covering:
                    1. Core changes made
                    2. Direct impact
                    ",
                    base_content = base_content
                },
                GitEntity::Diff(Diff::WorkingTree { .. }) => formatdoc! {"
                    {base_content}
                    
                    Provide:
                    1. Key changes
                    2. Notable concerns (if any)
                    ",
                    base_content = base_content
                },
                GitEntity::Diff(Diff::CommitsRange { .. }) => formatdoc! {"
                    {base_content}
                    
                    Provide:
                    1. Core changes made
                    2. Direct impact
                    ",
                    base_content = base_content
                },
            },
        };

        let user_prompt = if let Some(custom_prompt) =
            config.user_prompt.as_ref().filter(|s| !s.trim().is_empty())
        {
            // Use unified template formatting
            Self::format_template(custom_prompt, &[("{base_content}", &base_content)])
        } else {
            default_user_prompt
        };

        Ok(AIPrompt {
            system_prompt,
            user_prompt,
        })
    }

    pub fn build_draft_prompt(
        command: &DraftCommand,
        config: &DraftConfig,
    ) -> Result<Self, AIPromptError> {
        let GitEntity::Diff(Diff::WorkingTree { diff, .. }) = &command.git_entity else {
            return Err(AIPromptError(
                "`draft` is only supported for working tree diffs".into(),
            ));
        };

        let default_system_prompt = indoc! {"
            You are a commit message generator that follows these rules:
            1. Write in present tense
            2. Be concise and direct
            3. Output only the commit message without any explanations
            4. Follow the format: <type>(<optional scope>): <commit message>
        "};

        let system_prompt = config
            .system_prompt
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.as_str())
            .unwrap_or(default_system_prompt)
            .to_string();

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

        let default_user_prompt = formatdoc! {"
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
            context = context,
            diff = diff,
        };

        let user_prompt = if let Some(custom_prompt) =
            config.user_prompt.as_ref().filter(|s| !s.trim().is_empty())
        {
            // Use unified template formatting
            Self::format_template(
                custom_prompt,
                &[
                    ("{diff}", diff),
                    ("{commit_types}", &command.draft_config.commit_types),
                    ("{context}", &context),
                ],
            )
        } else {
            default_user_prompt
        };

        Ok(AIPrompt {
            system_prompt,
            user_prompt,
        })
    }

    pub fn build_operate_prompt(
        query: &str,
        config: &OperateConfig,
    ) -> Result<Self, AIPromptError> {
        let default_system_prompt = indoc! {"
        You're a Git assistant that provides commands with clear explanations.
        - Include warnings ONLY for destructive commands (reset, push --force, clean, etc.)
        - Omit warning tag completely for safe commands
    "};

        let system_prompt = config
            .system_prompt
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.as_str())
            .unwrap_or(default_system_prompt)
            .to_string();

        let default_user_prompt = formatdoc! {"
        Generate Git command for: {query}
        
        <command>Git command</command>
        <explanation>Brief explanation</explanation>
        <warning>Required for destructive commands only - omit for safe commands</warning>
        ",
            query = query
        };

        let user_prompt = if let Some(custom_prompt) =
            config.user_prompt.as_ref().filter(|s| !s.trim().is_empty())
        {
            // Use unified template formatting
            Self::format_template(custom_prompt, &[("{query}", query)])
        } else {
            default_user_prompt
        };

        Ok(AIPrompt {
            system_prompt,
            user_prompt,
        })
    }
}
