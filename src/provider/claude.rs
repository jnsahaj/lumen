use super::AIProvider;
use crate::{ai_prompt::AIPrompt, git_entity::GitEntity};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

pub struct ClaudeProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

#[derive(Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContent>,
}

#[derive(Deserialize)]
struct ClaudeContent {
    text: String,
}

impl ClaudeProvider {
    pub fn new(client: reqwest::Client, api_key: String, model: Option<String>) -> Self {
        ClaudeProvider {
            client,
            api_key,
            model: model.unwrap_or_else(|| "claude-3-5-sonnet-20241022".to_string()),
        }
    }
}

async fn get_completion_result(
    client: &reqwest::Client,
    api_key: &str,
    payload: serde_json::Value,
) -> Result<String, Box<dyn std::error::Error>> {
    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await?;

    let claude_response: ClaudeResponse = response.json().await?;
    Ok(claude_response
        .content
        .first()
        .map(|content| content.text.clone())
        .unwrap_or_default())
}

#[async_trait]
impl AIProvider for ClaudeProvider {
    async fn explain(&self, git_entity: GitEntity) -> Result<String, Box<dyn std::error::Error>> {
        let AIPrompt {
            system_prompt,
            user_prompt,
        } = AIPrompt::build_explain_prompt(&git_entity);

        let payload = json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": [
                {
                    "role": "system",
                    "content": system_prompt
                },
                {
                    "role": "user",
                    "content": user_prompt,
                }
            ]
        });

        let res = get_completion_result(&self.client, &self.api_key, payload).await?;
        Ok(res)
    }

    async fn draft(&self, git_entity: GitEntity) -> Result<String, Box<dyn std::error::Error>> {
        todo!()
    }
}
