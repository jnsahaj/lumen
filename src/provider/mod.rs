use async_trait::async_trait;
use claude::{ClaudeConfig, ClaudeProvider};
use groq::{GroqConfig, GroqProvider};
use ollama::{OllamaConfig, OllamaProvider};
use openai::{OpenAIConfig, OpenAIProvider};
use phind::{PhindConfig, PhindProvider};
use thiserror::Error;

use crate::{ai_prompt::AIPromptError, error::LumenError, git_entity::GitEntity, ProviderType};

pub mod claude;
pub mod groq;
pub mod ollama;
pub mod openai;
pub mod phind;

#[async_trait]
pub trait AIProvider {
    async fn explain(&self, git_entity: GitEntity) -> Result<String, ProviderError>;
    async fn draft(
        &self,
        git_entity: GitEntity,
        context: Option<String>,
    ) -> Result<String, ProviderError>;
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
        }
    }
}

#[async_trait]
impl AIProvider for LumenProvider {
    async fn explain(&self, git_entity: GitEntity) -> Result<String, ProviderError> {
        match self {
            LumenProvider::OpenAI(provider) => provider.explain(git_entity).await,
            LumenProvider::Phind(provider) => provider.explain(git_entity).await,
            LumenProvider::Groq(provider) => provider.explain(git_entity).await,
            LumenProvider::Claude(provider) => provider.explain(git_entity).await,
            LumenProvider::Ollama(provider) => provider.explain(git_entity).await,
        }
    }
    async fn draft(
        &self,
        git_entity: GitEntity,
        context: Option<String>,
    ) -> Result<String, ProviderError> {
        match self {
            LumenProvider::OpenAI(provider) => provider.draft(git_entity, context).await,
            LumenProvider::Phind(provider) => provider.draft(git_entity, context).await,
            LumenProvider::Groq(provider) => provider.draft(git_entity, context).await,
            LumenProvider::Claude(provider) => provider.draft(git_entity, context).await,
            LumenProvider::Ollama(provider) => provider.draft(git_entity, context).await,
        }
    }
}
