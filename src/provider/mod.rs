use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatRequest};
use genai::resolver::{AuthData, Endpoint, ServiceTargetResolver};
use genai::{Client, ClientBuilder, ModelIden, ServiceTarget};
use thiserror::Error;

use crate::ai_prompt::{AIPrompt, AIPromptError};
use crate::command::{draft::DraftCommand, explain::ExplainCommand, operate::OperateCommand};
use crate::config::cli::ProviderType;
use crate::config::ProviderInfo;
use crate::error::LumenError;

#[derive(Error, Debug)]
pub enum ProviderError {
    #[error("AI request failed: {0}")]
    GenAIError(#[from] genai::Error),

    #[error("API request failed: {0}")]
    RequestError(#[from] reqwest::Error),

    #[error("No completion content in response")]
    NoCompletionChoice,

    #[error(transparent)]
    AIPromptError(#[from] AIPromptError),
}

enum ProviderBackend {
    GenAI { client: Client, model: String },
}

pub struct LumenProvider {
    backend: ProviderBackend,
    provider_name: String,
}

/// Provider configuration for custom endpoint providers (OpenCode Zen, OpenRouter, Vercel)
struct CustomProviderConfig {
    endpoint: String,
    env_key: &'static str,
    adapter_kind: AdapterKind,
}

impl LumenProvider {
    pub fn new(
        provider_type: ProviderType,
        api_key: Option<String>,
        model: Option<String>,
        base_url: Option<String>,
    ) -> Result<Self, LumenError> {
        if (base_url.is_some())
            && !matches!(
                provider_type,
                ProviderType::Customopenai
            )
        {
            return Err(LumenError::ConfigurationError(
                "Base URL can only be set for CustomOpenAI provider".to_string(),
            ));
        }

        let (backend, provider_name) = match provider_type {
            // Custom endpoint providers (OpenCode Zen, OpenRouter, Vercel, Customopenai) - use ServiceTargetResolver
            ProviderType::OpencodeZen | ProviderType::Openrouter | ProviderType::Vercel | ProviderType::Customopenai => {
                let defaults = ProviderInfo::for_provider(provider_type);
                let config = match provider_type {
                    ProviderType::OpencodeZen => CustomProviderConfig {
                        endpoint: "https://opencode.ai/zen/v1/".to_string(),
                        env_key: defaults.env_key,
                        adapter_kind: AdapterKind::OpenAI,
                    },
                    ProviderType::Openrouter => CustomProviderConfig {
                        endpoint: "https://openrouter.ai/api/v1/".to_string(),
                        env_key: defaults.env_key,
                        adapter_kind: AdapterKind::OpenAI,
                    },
                    ProviderType::Vercel => CustomProviderConfig {
                        // Trailing slash is required for URL joining to work correctly
                        endpoint: "https://ai-gateway.vercel.sh/v1/".to_string(),
                        env_key: defaults.env_key,
                        adapter_kind: AdapterKind::OpenAI,
                    },
                    ProviderType::Customopenai => {
                        let endpoint = base_url.ok_or_else(|| {
                            LumenError::ConfigurationError("A Custom URL is required on CustomOpenAI provider".to_string())
                        })?;

                        CustomProviderConfig {
                            endpoint,
                            env_key: defaults.env_key,
                            adapter_kind: AdapterKind::OpenAI,
                        }
                    }
                    _ => unreachable!(),
                };

                let model = model.unwrap_or_else(|| defaults.default_model.to_string());
                let model_for_resolver = model.clone();

                // Get API key from CLI/config or environment
                let auth_env_key = config.env_key;
                if let Some(key) = api_key {
                    std::env::set_var(auth_env_key, key);
                }

                let endpoint = config.endpoint;
                let adapter_kind = config.adapter_kind;

                let target_resolver = ServiceTargetResolver::from_resolver_fn(
                    move |service_target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
                        let ServiceTarget { model, .. } = service_target;
                        Ok(ServiceTarget {
                            endpoint: Endpoint::from_owned(endpoint.clone()),
                            auth: AuthData::from_env(auth_env_key),
                            model: ModelIden::new(adapter_kind, model.model_name),
                        })
                    },
                );

                let client = ClientBuilder::default()
                    .with_service_target_resolver(target_resolver)
                    .build();

                (
                    ProviderBackend::GenAI {
                        client,
                        model: model_for_resolver,
                    },
                    defaults.display_name.to_string(),
                )
            }
            // Native genai providers
            _ => {
                let defaults = ProviderInfo::for_provider(provider_type);

                let model = model.unwrap_or_else(|| defaults.default_model.to_string());

                // If api_key provided via CLI/config, set it in env so genai picks it up
                if let Some(key) = api_key {
                    if !defaults.env_key.is_empty() {
                        std::env::set_var(defaults.env_key, key);
                    }
                }

                (
                    ProviderBackend::GenAI {
                        client: Client::default(),
                        model,
                    },
                    defaults.display_name.to_string(),
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
                    .first_text()
                    .map(|s| s.to_string())
                    .ok_or(ProviderError::NoCompletionChoice)
            }
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
        }
    }
}

impl std::fmt::Display for LumenProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.provider_name, self.get_model())
    }
}
