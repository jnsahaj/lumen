use super::{AIProvider, ProviderError};
use crate::{ai_prompt::AIPrompt, git_entity::GitEntity};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContent>,
}

#[derive(Deserialize)]
struct ClaudeContent {
    text: String,
}

// Configuration type to match OpenAI pattern
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
            "messages": [
                {
                    "role": "system",
                    "content": prompt.system_prompt
                },
                {
                    "role": "user",
                    "content": prompt.user_prompt,
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

        let claude_response: ClaudeResponse = response.json().await?;
        claude_response
            .content
            .first()
            .map(|content| content.text.clone())
            .ok_or(ProviderError::NoCompletionChoice)
    }
}

#[async_trait]
impl AIProvider for ClaudeProvider {
    async fn explain(&self, git_entity: GitEntity) -> Result<String, Box<dyn std::error::Error>> {
        let prompt = AIPrompt::build_explain_prompt(&git_entity);
        Ok(self.complete(prompt).await?)
    }

    async fn draft(&self, git_entity: GitEntity) -> Result<String, Box<dyn std::error::Error>> {
        let prompt = AIPrompt::build_draft_prompt(&git_entity)?;
        Ok(self.complete(prompt).await?)
    }
}
