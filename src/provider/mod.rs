use crate::config::cli::ProviderType;
use async_trait::async_trait;
use claude::{ClaudeConfig, ClaudeProvider};
use deepseek::{DeepSeekConfig, DeepSeekProvider};
use groq::{GroqConfig, GroqProvider};
use ollama::{OllamaConfig, OllamaProvider};
use openai::{OpenAIConfig, OpenAIProvider};
use openrouter::{OpenRouterConfig, OpenRouterProvider};
use phind::{PhindConfig, PhindProvider};
use thiserror::Error;

use crate::{
    ai_prompt::{AIPrompt, AIPromptError},
    command::{draft::DraftCommand, explain::ExplainCommand},
    error::LumenError,
};

pub mod claude;
pub mod deepseek;
pub mod groq;
pub mod ollama;
pub mod openai;
pub mod openrouter;
pub mod phind;

#[async_trait]
pub trait AIProvider {
    async fn complete(&self, prompt: AIPrompt) -> Result<String, ProviderError>;
}

#[derive(Error, Debug)]
pub enum ProviderError {
    #[error("API request failed: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("No completion choice available")]
    NoCompletionChoice,

    #[error(transparent)]
    AIPromptError(#[from] AIPromptError),

    #[error("API request failed with status code {0}: {1}")]
    APIError(reqwest::StatusCode, String),

    #[error("Unexpected response")]
    UnexpectedResponse,
}

pub enum LumenProvider {
    OpenAI(Box<OpenAIProvider>),
    Phind(Box<PhindProvider>),
    Groq(Box<GroqProvider>),
    Claude(Box<ClaudeProvider>),
    Ollama(Box<OllamaProvider>),
    OpenRouter(Box<OpenRouterProvider>),
    DeepSeek(Box<DeepSeekProvider>),
}

impl LumenProvider {
    pub fn new(
        client: reqwest::Client,
        provider_type: ProviderType,
        api_key: Option<String>,
        model: Option<String>,
    ) -> Result<Self, LumenError> {
        match provider_type {
            ProviderType::Openai => {
                let api_key = api_key.ok_or(LumenError::MissingApiKey("OpenAI".to_string()))?;
                let config = OpenAIConfig::new(api_key, model);
                let provider = LumenProvider::OpenAI(Box::new(OpenAIProvider::new(client, config)));
                Ok(provider)
            }
            ProviderType::Phind => Ok(LumenProvider::Phind(Box::new(PhindProvider::new(
                client,
                PhindConfig::new(model),
            )))),
            ProviderType::Groq => {
                let api_key = api_key.ok_or(LumenError::MissingApiKey("Groq".to_string()))?;
                let config = GroqConfig::new(api_key, model);
                let provider = LumenProvider::Groq(Box::new(GroqProvider::new(client, config)));
                Ok(provider)
            }
            ProviderType::Claude => {
                let api_key = api_key.ok_or(LumenError::MissingApiKey("Claude".to_string()))?;
                let config = ClaudeConfig::new(api_key, model);
                let provider = LumenProvider::Claude(Box::new(ClaudeProvider::new(client, config)));
                Ok(provider)
            }
            ProviderType::Ollama => {
                let model = model.ok_or(LumenError::MissingModel("Ollama".to_string()))?;
                let config = OllamaConfig::new(model);
                let provider = LumenProvider::Ollama(Box::new(OllamaProvider::new(client, config)));
                Ok(provider)
            }
            ProviderType::Openrouter => {
                let api_key = api_key.ok_or(LumenError::MissingApiKey("OpenRouter".to_string()))?;
                let config = OpenRouterConfig::new(api_key, model);
                let provider =
                    LumenProvider::OpenRouter(Box::new(OpenRouterProvider::new(client, config)));
                Ok(provider)
            }
            ProviderType::Deepseek => {
                let api_key = api_key.ok_or(LumenError::MissingApiKey("DeepSeek".to_string()))?;
                let config = DeepSeekConfig::new(api_key, model);
                let provider =
                    LumenProvider::DeepSeek(Box::new(DeepSeekProvider::new(client, config)));
                Ok(provider)
            }
        }
    }

    pub async fn explain(&self, command: &ExplainCommand) -> Result<String, ProviderError> {
        let prompt = AIPrompt::build_explain_prompt(command)?;
        match self {
            LumenProvider::OpenAI(provider) => provider.complete(prompt).await,
            LumenProvider::Phind(provider) => provider.complete(prompt).await,
            LumenProvider::Groq(provider) => provider.complete(prompt).await,
            LumenProvider::Claude(provider) => provider.complete(prompt).await,
            LumenProvider::Ollama(provider) => provider.complete(prompt).await,
            LumenProvider::OpenRouter(provider) => provider.complete(prompt).await,
            LumenProvider::DeepSeek(provider) => provider.complete(prompt).await,
        }
    }

    pub async fn draft(&self, command: &DraftCommand) -> Result<String, ProviderError> {
        let prompt = AIPrompt::build_draft_prompt(command)?;
        match self {
            LumenProvider::OpenAI(provider) => provider.complete(prompt).await,
            LumenProvider::Phind(provider) => provider.complete(prompt).await,
            LumenProvider::Groq(provider) => provider.complete(prompt).await,
            LumenProvider::Claude(provider) => provider.complete(prompt).await,
            LumenProvider::Ollama(provider) => provider.complete(prompt).await,
            LumenProvider::OpenRouter(provider) => provider.complete(prompt).await,
            LumenProvider::DeepSeek(provider) => provider.complete(prompt).await,
        }
    }
}
