use super::{AIProvider, ProviderError};
use crate::{ai_prompt::AIPrompt, git_entity::GitEntity};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

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

// Configuration type to match other providers
#[derive(Clone)]
pub struct GroqConfig {
    api_key: String,
    model: String,
    api_base_url: String,
}

impl GroqConfig {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            api_key,
            model: model.unwrap_or_else(|| "mixtral-8x7b-32768".to_string()),
            api_base_url: "https://api.groq.com/openai/v1/chat/completions".to_string(),
        }
    }
}

pub struct GroqProvider {
    client: reqwest::Client,
    config: GroqConfig,
}

impl GroqProvider {
    pub fn new(client: reqwest::Client, config: GroqConfig) -> Self {
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
            .json(&payload)
            .send()
            .await?;

        let groq_response: GroqResponse = response.json().await?;
        groq_response
            .choices
            .get(0)
            .map(|choice| choice.message.content.clone())
            .ok_or(ProviderError::NoCompletionChoice)
    }
}

#[async_trait]
impl AIProvider for GroqProvider {
    async fn explain(&self, git_entity: GitEntity) -> Result<String, ProviderError> {
        let prompt = AIPrompt::build_explain_prompt(&git_entity)?;
        self.complete(prompt).await
    }

    async fn draft(&self, git_entity: GitEntity) -> Result<String, ProviderError> {
        let prompt = AIPrompt::build_draft_prompt(&git_entity)?;
        self.complete(prompt).await
    }
}
