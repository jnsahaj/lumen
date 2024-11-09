use super::{AIProvider, ProviderError};
use crate::ai_prompt::AIPrompt;
use async_trait::async_trait;
use reqwest::{
    header::{HeaderMap, HeaderValue},
    StatusCode,
};
use serde_json::{json, Value};

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

    fn parse_line(line: &str) -> Option<String> {
        let data = line.strip_prefix("data: ")?; // Extract data after "data: " prefix
        let json_value: Value = serde_json::from_str(data).ok()?;

        json_value
            .get("choices")?
            .as_array()?
            .first()?
            .get("delta")?
            .get("content")?
            .as_str()
            .map(String::from)
    }

    fn parse_stream_response(response_text: &str) -> String {
        response_text
            .split('\n')
            .filter_map(Self::parse_line)
            .collect()
    }

    async fn complete(&self, prompt: AIPrompt) -> Result<String, ProviderError> {
        let payload = json!({
            "additional_extension_context": "",
            "allow_magic_buttons": true,
            "is_vscode_extension": true,
            "message_history": [{
                "content": prompt.user_prompt,
                "role": "user"
            }],
            "requested_model": self.config.model,
            "user_input": prompt.user_prompt
        });

        let headers = Self::create_headers()?;
        let response = self
            .client
            .post(&self.config.api_base_url)
            .headers(headers)
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        match status {
            StatusCode::OK => {
                let response_text = response.text().await?;
                let full_text = Self::parse_stream_response(&response_text);

                if full_text.is_empty() {
                    return Err(ProviderError::NoCompletionChoice);
                }
                Ok(full_text)
            }
            _ => {
                let error_text = response.text().await?;
                let error_json: Value = serde_json::from_str(&error_text)
                    .unwrap_or_else(|_| json!({"error": {"message": "Unknown error"}}));

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
impl AIProvider for PhindProvider {
    async fn complete(&self, prompt: AIPrompt) -> Result<String, ProviderError> {
        self.complete(prompt).await
    }
}
