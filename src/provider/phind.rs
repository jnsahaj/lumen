use crate::{ai_prompt::AIPrompt, git_entity::GitEntity};

use super::AIProvider;
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

pub struct PhindProvider {
    client: reqwest::Client,
    model: String,
}

impl PhindProvider {
    pub fn new(client: reqwest::Client, model: Option<String>) -> Self {
        PhindProvider {
            client,
            model: model.unwrap_or_else(|| "Phind-70B".to_string()),
        }
    }

    async fn create_request(
        &self,
        ai_prompt: AIPrompt,
    ) -> Result<PhindRequest, Box<dyn std::error::Error>> {
        let user_input = ai_prompt.user_prompt;

        Ok(PhindRequest {
            additional_extension_context: String::new(),
            allow_magic_buttons: true,
            is_vscode_extension: true,
            message_history: vec![Message {
                content: user_input.clone(),
                role: "user".to_string(),
            }],
            requested_model: self.model.clone(),
            user_input,
        })
    }

    fn create_headers() -> Result<HeaderMap, Box<dyn std::error::Error>> {
        let mut headers = HeaderMap::new();
        headers.insert("Content-Type", HeaderValue::from_static("application/json"));
        headers.insert("User-Agent", HeaderValue::from_static(""));
        headers.insert("Accept", HeaderValue::from_static("*/*"));
        headers.insert("Accept-Encoding", HeaderValue::from_static("Identity"));
        Ok(headers)
    }

    async fn get_main_text(response: &str) -> Result<String, Box<dyn std::error::Error>> {
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

        Ok(full_text)
    }
}

#[async_trait]
impl AIProvider for PhindProvider {
    async fn explain(&self, git_entity: GitEntity) -> Result<String, Box<dyn std::error::Error>> {
        let request = self
            .create_request(AIPrompt::build_explain_prompt(&git_entity))
            .await?;

        let headers = Self::create_headers()?;

        let response = self
            .client
            .post("https://https.extension.phind.com/agent/")
            .headers(headers)
            .json(&request)
            .send()
            .await?
            .text()
            .await?;

        let res = Self::get_main_text(&response).await?;
        Ok(res)
    }

    async fn draft(&self, git_entity: GitEntity) -> Result<String, Box<dyn std::error::Error>> {
        let request = self
            .create_request(AIPrompt::build_draft_prompt(&git_entity))
            .await?;

        let headers = Self::create_headers()?;

        let response = self
            .client
            .post("https://https.extension.phind.com/agent/")
            .headers(headers)
            .json(&request)
            .send()
            .await?
            .text()
            .await?;

        let res = Self::get_main_text(&response).await?;
        Ok(res)
    }
}
