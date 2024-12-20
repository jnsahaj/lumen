use super::{AIProvider, ProviderError};
use crate::ai_prompt::AIPrompt;
use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct ClaudeConfig {
    api_key: String,
    model: String,
    api_base_url: String,
}

impl ClaudeConfig {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            api_key,
            model: model.unwrap_or_else(|| "claude-3-5-sonnet-20241022".to_string()),
            api_base_url: "https://api.anthropic.com/v1/messages".to_string(),
        }
    }
}

pub struct ClaudeProvider {
    client: reqwest::Client,
    config: ClaudeConfig,
}

impl ClaudeProvider {
    pub fn new(client: reqwest::Client, config: ClaudeConfig) -> Self {
        Self { client, config }
    }

    async fn complete(&self, prompt: AIPrompt) -> Result<String, ProviderError> {
        let payload = json!({
            "model": self.config.model,
            "max_tokens": 4096,
            "system": prompt.system_prompt,
            "messages": [
                {
                    "role": "user",
                    "content": prompt.user_prompt
                }
            ]
        });

        let response = self
            .client
            .post(&self.config.api_base_url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        match status {
            StatusCode::OK => {
                let response_json: Value = response.json().await?;
                let content = response_json
                    .get("content")
                    .and_then(|content| content.get(0))
                    .and_then(|message| message.get("text"))
                    .and_then(|text| text.as_str())
                    .ok_or(ProviderError::NoCompletionChoice)?;
                Ok(content.to_string())
            }
            _ => {
                let error_json: Value = response.json().await?;
                let error_message = error_json
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(|msg| msg.as_str())
                    .ok_or(ProviderError::UnexpectedResponse)?
                    .into();
                Err(ProviderError::APIError(status, error_message))
            }
        }
    }
}

#[async_trait]
impl AIProvider for ClaudeProvider {
    async fn complete(&self, prompt: AIPrompt) -> Result<String, ProviderError> {
        self.complete(prompt).await
    }
}
