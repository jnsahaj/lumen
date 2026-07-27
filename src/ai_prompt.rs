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

    pub fn build_grouped_explain_prompt(command: &ExplainCommand) -> Result<Self, AIPromptError> {
        let system_prompt = String::from(indoc! {r#"
            You are a code-review assistant that clusters a Git diff into logical groups
            based on purpose (feature, refactor, tests, config, docs, chore, etc.).
            Prefer more, smaller, focused groups over fewer large ones — split unrelated
            changes into separate groups even within the same purpose.
            Test files must be grouped together with the implementation they test, not
            split out into a separate "tests" group.
            Respond with ONLY a single valid JSON object. Do not use markdown code fences.
            Do not include any prose before or after the JSON object.
        "#});

        let diff_block = match &command.git_entity {
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

        let user_prompt = formatdoc! {r#"
            {diff_block}

            Group the changes above into logical clusters and respond with ONLY a JSON object
            matching this exact shape:
            {{"groups":[{{"title":"string","files":["path/to/file",...],"summary":"1-3 sentence prose summary"}}],"overall_summary":"2-4 sentence prose summary, omit or use empty string if the diff is a single logical change"}}

            `files` must be exact repo-relative paths copied verbatim from the diff's file headers.
            Every changed file must appear in exactly one group. If the diff represents a single
            logical change, return exactly one group.
            "#
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

        let user_prompt = formatdoc! {"
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
        };

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
        let user_prompt = formatdoc! {"
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

#[cfg(test)]
mod tests {
    use super::*;

    fn working_tree_command(diff: &str, grouped: bool) -> ExplainCommand {
        ExplainCommand {
            git_entity: GitEntity::Diff(Diff::WorkingTree {
                staged: false,
                diff: diff.to_string(),
            }),
            query: None,
            grouped,
        }
    }

    #[test]
    fn build_explain_prompt_produces_plain_prompt_shape() {
        let command = working_tree_command("diff --git a/src/a.rs b/src/a.rs", false);

        let prompt = AIPrompt::build_explain_prompt(&command).unwrap();

        assert!(prompt
            .user_prompt
            .contains("diff --git a/src/a.rs b/src/a.rs"));
        assert!(prompt.user_prompt.contains("Key changes"));
    }

    #[test]
    fn build_grouped_explain_prompt_embeds_diff_and_json_contract() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\n+++ b/src/a.rs";
        let command = working_tree_command(diff, true);

        let prompt = AIPrompt::build_grouped_explain_prompt(&command).unwrap();

        assert!(prompt.user_prompt.contains(diff));
        assert!(prompt.user_prompt.contains("JSON"));
        assert!(prompt.user_prompt.contains("groups"));
        assert!(prompt.user_prompt.contains("overall_summary"));
    }

    #[test]
    fn build_grouped_explain_prompt_instructs_smaller_groups_and_colocated_tests() {
        let command = working_tree_command("diff --git a/src/a.rs b/src/a.rs", true);

        let prompt = AIPrompt::build_grouped_explain_prompt(&command).unwrap();

        assert!(prompt
            .system_prompt
            .contains("more, smaller, focused groups"));
        assert!(prompt
            .system_prompt
            .contains("Test files must be grouped together with the implementation they test"));
    }
}
