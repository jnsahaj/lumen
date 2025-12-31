use genai::adapter::AdapterKind;
use genai::chat::{ChatMessage, ChatRequest};
use genai::resolver::{AuthData, Endpoint, ServiceTargetResolver};
use genai::{Client, ClientBuilder, Headers, ModelIden, ServiceTarget, WebConfig};
use serde_json::Value;
use std::sync::{Arc, Mutex as StdMutex};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::ai_prompt::{AIPrompt, AIPromptError};
use crate::command::{draft::DraftCommand, explain::ExplainCommand, operate::OperateCommand};
use crate::config::cli::ProviderType;
use crate::error::LumenError;
use crate::qwen_oauth::{QwenOAuthError, QwenOAuthManager, DEFAULT_DASHSCOPE_BASE_URL};

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

    #[error("Qwen OAuth error: {0}")]
    QwenOAuthError(String),
}

enum ProviderBackend {
    GenAI { client: Client, model: String },
    QwenOAuth {
        client: Client,
        model: String,
        oauth: Mutex<QwenOAuthManager>,
        overrides: Arc<StdMutex<QwenOverrideState>>,
    },
}

struct QwenOverrideState {
    token: String,
    base_url: String,
    user_agent: String,
}

pub struct LumenProvider {
    backend: ProviderBackend,
    provider_name: String,
}

/// Provider configuration for custom endpoint providers (OpenRouter, Vercel)
struct CustomProviderConfig {
    endpoint: &'static str,
    env_key: &'static str,
    adapter_kind: AdapterKind,
}

impl LumenProvider {
    pub fn new(
        provider_type: ProviderType,
        api_key: Option<String>,
        model: Option<String>,
    ) -> Result<Self, LumenError> {
        let (backend, provider_name) = match provider_type {
            ProviderType::Qwen => {
                let model = model.unwrap_or_else(|| "coder-model".to_string());
                let model_for_resolver = model.clone();
                let user_agent = format!("Lumen/{} ({})", env!("CARGO_PKG_VERSION"), std::env::consts::OS);
                let base_url = {
                    let mut url = DEFAULT_DASHSCOPE_BASE_URL.to_string();
                    if !url.ends_with('/') {
                        url.push('/');
                    }
                    url
                };
                let overrides = Arc::new(StdMutex::new(QwenOverrideState {
                    token: String::new(),
                    base_url,
                    user_agent,
                }));
                let overrides_for_resolver = Arc::clone(&overrides);

                let target_resolver = ServiceTargetResolver::from_resolver_fn(
                    move |service_target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
                        let ServiceTarget { model, .. } = service_target;
                        let overrides = overrides_for_resolver
                            .lock()
                            .expect("Qwen OAuth overrides lock poisoned");
                        let auth_headers = Headers::from([
                            ("Authorization", format!("Bearer {}", overrides.token)),
                            ("User-Agent", overrides.user_agent.clone()),
                            ("X-DashScope-UserAgent", overrides.user_agent.clone()),
                            ("X-DashScope-CacheControl", "enable".to_string()),
                            ("X-DashScope-AuthType", "qwen-oauth".to_string()),
                        ]);
                        let url = format!("{}chat/completions", overrides.base_url);
                        let endpoint = overrides.base_url.clone();
                        Ok(ServiceTarget {
                            endpoint: Endpoint::from_owned(endpoint),
                            auth: AuthData::RequestOverride {
                                url,
                                headers: auth_headers,
                            },
                            model: ModelIden::new(AdapterKind::OpenAI, model.model_name),
                        })
                    },
                );

                let client = ClientBuilder::default()
                    .with_service_target_resolver(target_resolver)
                    .with_web_config(WebConfig::default().with_timeout(std::time::Duration::from_secs(90)))
                    .build();

                (
                    ProviderBackend::QwenOAuth {
                        client,
                        model: model_for_resolver,
                        oauth: Mutex::new(QwenOAuthManager::new()),
                        overrides,
                    },
                    "Qwen OAuth".to_string(),
                )
            }
            // Custom endpoint providers (OpenRouter, Vercel) - use ServiceTargetResolver
            ProviderType::Openrouter | ProviderType::Vercel => {
                let (default_model, name, config) = match provider_type {
                    ProviderType::Openrouter => (
                        "anthropic/claude-sonnet-4.5",
                        "OpenRouter",
                        CustomProviderConfig {
                            endpoint: "https://openrouter.ai/api/v1/",
                            env_key: "OPENROUTER_API_KEY",
                            adapter_kind: AdapterKind::OpenAI,
                        },
                    ),
                    ProviderType::Vercel => (
                        "anthropic/claude-sonnet-4.5",
                        "Vercel",
                        CustomProviderConfig {
                            // Trailing slash is required for URL joining to work correctly
                            endpoint: "https://ai-gateway.vercel.sh/v1/",
                            env_key: "VERCEL_API_KEY",
                            adapter_kind: AdapterKind::OpenAI,
                        },
                    ),
                    _ => unreachable!(),
                };

                let model = model.unwrap_or_else(|| default_model.to_string());
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
                            endpoint: Endpoint::from_static(endpoint),
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
                    name.to_string(),
                )
            }
            // Native genai providers
            _ => {
                let (default_model, name, env_key) = match provider_type {
                    ProviderType::Openai => ("gpt-5-mini", "OpenAI", "OPENAI_API_KEY"),
                    ProviderType::Claude => (
                        "claude-sonnet-4-5-20250930",
                        "Claude",
                        "ANTHROPIC_API_KEY",
                    ),
                    ProviderType::Groq => ("llama-3.3-70b-versatile", "Groq", "GROQ_API_KEY"),
                    ProviderType::Ollama => ("llama3.2", "Ollama", ""),
                    ProviderType::Deepseek => ("deepseek-chat", "DeepSeek", "DEEPSEEK_API_KEY"),
                    ProviderType::Gemini => ("gemini-3-flash", "Gemini", "GEMINI_API_KEY"),
                    ProviderType::Xai => ("grok-4-mini-fast", "xAI", "XAI_API_KEY"),
                    ProviderType::Qwen => unreachable!(),
                    ProviderType::Openrouter | ProviderType::Vercel => {
                        unreachable!()
                    }
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
                    .first_text()
                    .map(|s| s.to_string())
                    .ok_or(ProviderError::NoCompletionChoice)
            }
            ProviderBackend::QwenOAuth {
                model,
                oauth,
                overrides,
                ..
            } => {
                let credentials = {
                    let mut oauth = oauth.lock().await;
                    oauth.ensure_valid_credentials().await?
                };
                if credentials.access_token.trim().is_empty() {
                    return Err(ProviderError::QwenOAuthError(
                        "Qwen OAuth returned empty access token.".to_string(),
                    ));
                }
                let endpoint = QwenOAuthManager::endpoint_for(&credentials);
                let mut overrides = overrides
                    .lock()
                    .expect("Qwen OAuth overrides lock poisoned");
                overrides.token = credentials.access_token;
                overrides.base_url = endpoint;
                log_qwen_debug(&overrides.base_url, &overrides.user_agent, model);

                let effective_model = map_qwen_model(&overrides.base_url, model);
                if effective_model != *model {
                    log_qwen_model_map(model, &effective_model);
                }

                let response = qwen_oauth_chat_completion(
                    &overrides.base_url,
                    &overrides.token,
                    &overrides.user_agent,
                    &effective_model,
                    &prompt.system_prompt,
                    &prompt.user_prompt,
                )
                .await?;

                Ok(response)
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
            ProviderBackend::QwenOAuth { model, .. } => model.clone(),
        }
    }
}

impl From<QwenOAuthError> for ProviderError {
    fn from(error: QwenOAuthError) -> Self {
        ProviderError::QwenOAuthError(error.to_string())
    }
}

fn log_qwen_debug(base_url: &str, user_agent: &str, model: &str) {
    if std::env::var("LUMEN_QWEN_DEBUG").ok().as_deref() != Some("1") {
        return;
    }

    let url = format!("{base_url}chat/completions");
    eprintln!("[qwen-oauth] model={model}");
    eprintln!("[qwen-oauth] base_url={base_url}");
    eprintln!("[qwen-oauth] request_url={url}");
    eprintln!("[qwen-oauth] user_agent={user_agent}");
}

fn map_qwen_model(base_url: &str, model: &str) -> String {
    if base_url.contains("portal.qwen.ai") {
        if model == "coder-model" {
            return model.to_string();
        }
        if model == "vision-model" {
            return model.to_string();
        }
        if model.starts_with("qwen3-coder-plus") {
            return "coder-model".to_string();
        }
        if model.starts_with("qwen3-vl-plus") {
            return "vision-model".to_string();
        }
    }

    model.to_string()
}

fn log_qwen_model_map(original: &str, mapped: &str) {
    if std::env::var("LUMEN_QWEN_DEBUG").ok().as_deref() != Some("1") {
        return;
    }
    eprintln!("[qwen-oauth] model_mapped={original}->{mapped}");
}

async fn qwen_oauth_chat_completion(
    base_url: &str,
    access_token: &str,
    user_agent: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, ProviderError> {
    let url = format!("{base_url}chat/completions");
    if std::env::var("LUMEN_QWEN_DEBUG").ok().as_deref() == Some("1") {
        eprintln!("[qwen-oauth] direct_request_url={url}");
    }

    let body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_prompt }
        ]
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .build()?;

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("User-Agent", user_agent)
        .header("X-DashScope-UserAgent", user_agent)
        .header("X-DashScope-CacheControl", "enable")
        .header("X-DashScope-AuthType", "qwen-oauth")
        .json(&body)
        .send()
        .await?;

    let status = response.status();
    let payload: Value = response.json().await?;

    if !status.is_success() {
        let message = payload.to_string();
        return Err(ProviderError::QwenOAuthError(format!(
            "HTTP {status}: {message}"
        )));
    }

    let content = payload
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|msg| msg.get("content"))
        .cloned()
        .ok_or(ProviderError::NoCompletionChoice)?;

    Ok(extract_openai_content(content)?)
}

fn extract_openai_content(content: Value) -> Result<String, ProviderError> {
    if let Some(text) = content.as_str() {
        return Ok(text.to_string());
    }

    if let Some(parts) = content.as_array() {
        let mut collected = String::new();
        for part in parts {
            if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                collected.push_str(text);
            } else if let Some(text) = part.as_str() {
                collected.push_str(text);
            }
        }
        if !collected.is_empty() {
            return Ok(collected);
        }
    }

    Err(ProviderError::NoCompletionChoice)
}

impl std::fmt::Display for LumenProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.provider_name, self.get_model())
    }
}
