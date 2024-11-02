use super::AIProvider;
use crate::git_entity::GitEntity;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

pub struct GroqProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

#[derive(Deserialize)]
struct GroqResponse {
    choices: Vec<GroqChoice>,
}

#[derive(Deserialize)]
struct GroqChoice {
    message: GroqMessage,
}

#[derive(Deserialize)]
struct GroqMessage {
    content: String,
}

impl GroqProvider {
    pub fn new(client: reqwest::Client, api_key: String, model: Option<String>) -> Self {
        GroqProvider {
            client,
            api_key,
            model: model.unwrap_or_else(|| "mixtral-8x7b-32768".to_string()),
        }
    }
}

async fn get_completion_result(
    client: &reqwest::Client,
    api_key: &str,
    payload: serde_json::Value,
) -> Result<String, Box<dyn std::error::Error>> {
    let response = client
        .post("https://api.groq.com/openai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&payload)
        .send()
        .await?;

    let groq_response: GroqResponse = response.json().await?;
    Ok(groq_response
        .choices
        .get(0)
        .map(|choice| choice.message.content.clone())
        .unwrap_or_default())
}

#[async_trait]
impl AIProvider for GroqProvider {
    async fn explain(&self, git_entity: GitEntity) -> Result<String, Box<dyn std::error::Error>> {
        let commit = match git_entity {
            GitEntity::Commit(commit) => commit,
        };

        let user_input = format!(
            "Please analyze this git commit and provide a summary.\n\nCommit Message:\n{}\n\nDiff Content:\n{}",
            commit.message, commit.diff
        );

        let payload = json!({
            "model": self.model,
            "messages": [
                {
                    "role": "system",
                    "content": r#"You are a helpful assistant that analyzes git commits. \
                                 Provide a concise summary of the changes based on the commit message and diff content. \
                                 Focus on the impact and purpose of the changes."#
                },
                {
                    "role": "user",
                    "content": user_input,
                }
            ]
        });

        let res = get_completion_result(&self.client, &self.api_key, payload).await?;
        Ok(res)
    }
}
