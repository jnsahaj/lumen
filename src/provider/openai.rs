use crate::{ai_prompt::AIPrompt, git_entity::GitEntity};

use super::AIProvider;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

pub struct OpenAIProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

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

impl OpenAIProvider {
    pub fn new(client: reqwest::Client, api_key: String, model: Option<String>) -> Self {
        OpenAIProvider {
            client,
            api_key,
            model: model.unwrap_or_else(|| "gpt-4o-mini".to_string()),
        }
    }
}

async fn get_completion_result(
    client: &reqwest::Client,
    api_key: &str,
    payload: serde_json::Value,
) -> Result<String, Box<dyn std::error::Error>> {
    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&payload)
        .send()
        .await?;

    let openai_response: OpenAIResponse = response.json().await?;
    Ok(openai_response
        .choices
        .get(0)
        .map(|choice| choice.message.content.clone())
        .unwrap_or_default())
}

#[async_trait]
impl AIProvider for OpenAIProvider {
    async fn explain(&self, git_entity: GitEntity) -> Result<String, Box<dyn std::error::Error>> {
        let AIPrompt {
            system_prompt,
            user_prompt,
        } = AIPrompt::build_explain_prompt(&git_entity);

        let payload = json!({
            "model": self.model,
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
        let AIPrompt {
            system_prompt,
            user_prompt,
        } = AIPrompt::build_draft_prompt(&git_entity)?;

        let payload = json!({
            "model": self.model,
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
}
