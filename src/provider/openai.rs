use super::{AIProvider, ProviderError};
use crate::{ai_prompt::AIPrompt, git_entity::GitEntity};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct Message {
    content: String,
}

// Configuration type
#[derive(Clone)]
pub struct OpenAIConfig {
    api_key: String,
    model: String,
    api_base_url: String,
}

impl OpenAIConfig {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            api_key,
            model: model.unwrap_or_else(|| "gpt-4o-mini".to_string()),
            api_base_url: "https://api.openai.com/v1/chat/completions".to_string(),
        }
    }
}

pub struct OpenAIProvider {
    client: reqwest::Client,
    config: OpenAIConfig,
}

impl OpenAIProvider {
    pub fn new(client: reqwest::Client, config: OpenAIConfig) -> Self {
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
                    "content": prompt.user_prompt,
                }
            ]
        });

        let response = self
            .client
            .post(&self.config.api_base_url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&payload)
            .send()
            .await?;

        let openai_response: OpenAIResponse = response.json().await?;

        openai_response
            .choices
            .get(0)
            .map(|choice| choice.message.content.clone())
            .ok_or(ProviderError::NoCompletionChoice)
    }
}

#[async_trait]
impl AIProvider for OpenAIProvider {
    async fn explain(&self, git_entity: GitEntity) -> Result<String, ProviderError> {
        let prompt = AIPrompt::build_explain_prompt(&git_entity)?;
        self.complete(prompt).await
    }

    async fn draft(&self, git_entity: GitEntity) -> Result<String, ProviderError> {
        let prompt = AIPrompt::build_draft_prompt(&git_entity)?;
        self.complete(prompt).await
    }
}
