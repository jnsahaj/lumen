use super::{AIProvider, ProviderError};
use crate::ai_prompt::AIPrompt;
use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct OllamaConfig {
    model: String,
    api_base_url: String,
}

impl OllamaConfig {
    pub fn new(model: String) -> Self {
        Self {
            model,
            api_base_url: "http://localhost:11434/api/generate".to_string(),
        }
    }
}

pub struct OllamaProvider {
    client: reqwest::Client,
    config: OllamaConfig,
}

impl OllamaProvider {
    pub fn new(client: reqwest::Client, config: OllamaConfig) -> Self {
        Self { client, config }
    }

    async fn complete(&self, prompt: AIPrompt) -> Result<String, ProviderError> {
        let payload = json!({
            "model": self.config.model,
            "prompt": format!("{}\n\n{}", prompt.system_prompt, prompt.user_prompt),
            "stream": false
        });

        let response = self
            .client
            .post(&self.config.api_base_url)
            .json(&payload)
            .send()
            .await?;

        let status = response.status();

        match status {
            StatusCode::OK => {
                let response_json: Value = response.json().await?;

                let content = response_json
                    .get("response")
                    .and_then(|response| response.as_str())
                    .ok_or(ProviderError::NoCompletionChoice)?;

                Ok(content.to_string())
            }
            _ => {
                let error_text = response.text().await?;
                Err(ProviderError::APIError(
                    status,
                    format!("response: {error_text}"),
                ))
            }
        }
    }
}

#[async_trait]
impl AIProvider for OllamaProvider {
    async fn complete(&self, prompt: AIPrompt) -> Result<String, ProviderError> {
        self.complete(prompt).await
    }
}
