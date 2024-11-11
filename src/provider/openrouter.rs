use super::{AIProvider, ProviderError};
use crate::ai_prompt::AIPrompt;
use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct OpenRouterConfig {
    api_key: String,
    model: String,
    api_base_url: String,
}

impl OpenRouterConfig {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            api_key,
            model: model.unwrap_or_else(|| "anthropic/claude-3.5-sonnet".to_string()),
            api_base_url: "https://openrouter.ai/api/v1/chat/completions".to_string(),
        }
    }
}

pub struct OpenRouterProvider {
    client: reqwest::Client,
    config: OpenRouterConfig,
}

impl OpenRouterProvider {
    pub fn new(client: reqwest::Client, config: OpenRouterConfig) -> Self {
        Self { client, config }
    }

    async fn complete(&self, prompt: AIPrompt) -> Result<String, ProviderError> {
        let payload = json!({
            "model": self.config.model,
            "messages": [
                {
                    "role": "system",
                    "content": prompt.system_prompt
                },
                {
                    "role": "user",
                    "content": prompt.user_prompt
                }
            ]
        });

        let response = self
            .client
            .post(&self.config.api_base_url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("HTTP-Referer", "https://github.com/jnsahaj/lumen")
            .header("X-Title", "Lumen CLI")
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        match status {
            StatusCode::OK => {
                let response_json: Value = response.json().await?;
                let content = response_json
                    .get("choices")
                    .and_then(|choices| choices.get(0))
                    .and_then(|choice| choice.get("message"))
                    .and_then(|message| message.get("content"))
                    .and_then(|content| content.as_str())
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
impl AIProvider for OpenRouterProvider {
    async fn complete(&self, prompt: AIPrompt) -> Result<String, ProviderError> {
        self.complete(prompt).await
    }
}
