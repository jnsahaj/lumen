use genai::chat::{ChatMessage, ChatRequest};
use genai::Client;
use phind::{PhindConfig, PhindProvider};
use thiserror::Error;

use crate::ai_prompt::{AIPrompt, AIPromptError};
use crate::command::{draft::DraftCommand, explain::ExplainCommand, operate::OperateCommand};
use crate::config::cli::ProviderType;
use crate::error::LumenError;

pub mod phind;

#[derive(Error, Debug)]
pub enum ProviderError {
    #[error("AI request failed: {0}")]
    GenAIError(#[from] genai::Error),

    #[error("API request failed: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("No completion content in response")]
    NoCompletionChoice,

    #[error("API request failed with status code {0}: {1}")]
    APIError(reqwest::StatusCode, String),

    #[error("Unexpected response")]
    UnexpectedResponse,

    #[error(transparent)]
    AIPromptError(#[from] AIPromptError),
}

enum ProviderBackend {
    GenAI { client: Client, model: String },
    Phind(PhindProvider),
}

pub struct LumenProvider {
    backend: ProviderBackend,
    provider_name: String,
}

impl LumenProvider {
    pub fn new(
        provider_type: ProviderType,
        api_key: Option<String>,
        model: Option<String>,
    ) -> Result<Self, LumenError> {
        let (backend, provider_name) = match provider_type {
            ProviderType::Phind => {
                let config = PhindConfig::new(model);
                let client = reqwest::Client::new();
                (
                    ProviderBackend::Phind(PhindProvider::new(client, config)),
                    "Phind".to_string(),
                )
            }
            _ => {
                let (default_model, name, env_key) = match provider_type {
                    ProviderType::Openai => ("gpt-4.1-mini", "OpenAI", "OPENAI_API_KEY"),
                    ProviderType::Claude => (
                        "claude-sonnet-4-20250514",
                        "Claude",
                        "ANTHROPIC_API_KEY",
                    ),
                    ProviderType::Groq => ("llama-3.3-70b-versatile", "Groq", "GROQ_API_KEY"),
                    ProviderType::Ollama => ("llama3.2", "Ollama", ""),
                    ProviderType::Deepseek => ("deepseek-chat", "DeepSeek", "DEEPSEEK_API_KEY"),
                    ProviderType::Openrouter => {
                        ("anthropic/claude-sonnet-4", "OpenRouter", "OPENROUTER_API_KEY")
                    }
                    ProviderType::Gemini => ("gemini-2.5-flash", "Gemini", "GEMINI_API_KEY"),
                    ProviderType::Xai => ("grok-3-mini-fast", "xAI", "XAI_API_KEY"),
                    ProviderType::Phind => unreachable!(),
                };

                let model = model.unwrap_or_else(|| default_model.to_string());

                // If api_key provided via CLI/config, set it in env so genai picks it up
                if let Some(key) = api_key {
                    if !env_key.is_empty() {
                        std::env::set_var(env_key, key);
                    }
                }

                (
                    ProviderBackend::GenAI {
                        client: Client::default(),
                        model,
                    },
                    name.to_string(),
                )
            }
        };

        Ok(Self {
            backend,
            provider_name,
        })
    }

    async fn complete(&self, prompt: AIPrompt) -> Result<String, ProviderError> {
        match &self.backend {
            ProviderBackend::GenAI { client, model } => {
                let chat_req = ChatRequest::new(vec![
                    ChatMessage::system(prompt.system_prompt),
                    ChatMessage::user(prompt.user_prompt),
                ]);

                let response = client.exec_chat(model, chat_req, None).await?;

                response
                    .content_text_as_str()
                    .map(|s| s.to_string())
                    .ok_or(ProviderError::NoCompletionChoice)
            }
            ProviderBackend::Phind(provider) => provider.complete(prompt).await,
        }
    }

    pub async fn explain(&self, command: &ExplainCommand) -> Result<String, ProviderError> {
        let prompt = AIPrompt::build_explain_prompt(command)?;
        self.complete(prompt).await
    }

    pub async fn draft(&self, command: &DraftCommand) -> Result<String, ProviderError> {
        let prompt = AIPrompt::build_draft_prompt(command)?;
        self.complete(prompt).await
    }

    pub async fn operate(&self, command: &OperateCommand) -> Result<String, ProviderError> {
        let prompt = AIPrompt::build_operate_prompt(command.query.as_str())?;
        self.complete(prompt).await
    }

    fn get_model(&self) -> String {
        match &self.backend {
            ProviderBackend::GenAI { model, .. } => model.clone(),
            ProviderBackend::Phind(provider) => provider.get_model(),
        }
    }
}

impl std::fmt::Display for LumenProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.provider_name, self.get_model())
    }
}
