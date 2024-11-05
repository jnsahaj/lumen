use super::{AIProvider, ProviderError};
use crate::{ai_prompt::AIPrompt, git_entity::GitEntity};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    content: String,
    role: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PhindRequest {
    additional_extension_context: String,
    allow_magic_buttons: bool,
    is_vscode_extension: bool,
    message_history: Vec<Message>,
    requested_model: String,
    user_input: String,
}

#[derive(Debug, Deserialize)]
struct PhindResponse {
    choices: Option<Vec<Choice>>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    delta: Delta,
}

#[derive(Debug, Deserialize)]
struct Delta {
    content: String,
}

// Configuration type to match other providers
#[derive(Clone)]
pub struct PhindConfig {
    model: String,
    api_base_url: String,
}

impl PhindConfig {
    pub fn new(model: Option<String>) -> Self {
        Self {
            model: model.unwrap_or_else(|| "Phind-70B".to_string()),
            api_base_url: "https://https.extension.phind.com/agent/".to_string(),
        }
    }
}

pub struct PhindProvider {
    client: reqwest::Client,
    config: PhindConfig,
}

impl PhindProvider {
    pub fn new(client: reqwest::Client, config: PhindConfig) -> Self {
        Self { client, config }
    }

    fn create_headers() -> Result<HeaderMap, ProviderError> {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        headers.insert("User-Agent", HeaderValue::from_static(""));
        headers.insert("Accept", HeaderValue::from_static("*/*"));
        headers.insert("Accept-Encoding", HeaderValue::from_static("Identity"));
        Ok(headers)
    }

    async fn complete(&self, prompt: AIPrompt) -> Result<String, ProviderError> {
        // Create the request payload
        let request = PhindRequest {
            additional_extension_context: String::new(),
            allow_magic_buttons: true,
            is_vscode_extension: true,
            message_history: vec![Message {
                content: prompt.user_prompt.clone(),
                role: "user".to_string(),
            }],
            requested_model: self.config.model.clone(),
            user_input: prompt.user_prompt,
        };

        let headers = Self::create_headers()?;

        let response = self
            .client
            .post(&self.config.api_base_url)
            .headers(headers)
            .json(&request)
            .send()
            .await?
            .text()
            .await?;

        // Parse the streaming response
        let lines: Vec<&str> = response.split('\n').collect();
        let mut full_text = String::new();

        for line in lines {
            if line.starts_with("data: ") {
                let obj = line.strip_prefix("data: ").unwrap_or("{}");
                if let Ok(response) = serde_json::from_str::<PhindResponse>(obj) {
                    if let Some(choices) = response.choices {
                        if !choices.is_empty() {
                            full_text.push_str(&choices[0].delta.content);
                        }
                    }
                }
            }
        }

        if full_text.is_empty() {
            return Err(ProviderError::NoCompletionChoice);
        }

        Ok(full_text)
    }
}

#[async_trait]
impl AIProvider for PhindProvider {
    async fn explain(&self, git_entity: GitEntity) -> Result<String, ProviderError> {
        let prompt = AIPrompt::build_explain_prompt(&git_entity)?;
        self.complete(prompt).await
    }

    async fn draft(&self, git_entity: GitEntity) -> Result<String, ProviderError> {
        let prompt = AIPrompt::build_draft_prompt(&git_entity)?;
        self.complete(prompt).await
    }
}
