use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::time::sleep;

const QWEN_OAUTH_DEVICE_CODE_ENDPOINT: &str = "https://chat.qwen.ai/api/v1/oauth2/device/code";
const QWEN_OAUTH_TOKEN_ENDPOINT: &str = "https://chat.qwen.ai/api/v1/oauth2/token";
const QWEN_OAUTH_CLIENT_ID: &str = "f0304373b74a44d2b584a3fb70ca9e56";
const QWEN_OAUTH_SCOPE: &str = "openid profile email model.completion";
const QWEN_OAUTH_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
const QWEN_DIR: &str = ".qwen";
const QWEN_CREDENTIAL_FILENAME: &str = "oauth_creds.json";
const TOKEN_REFRESH_BUFFER_MS: u64 = 30_000;
const MAX_POLL_INTERVAL_MS: u64 = 10_000;

pub const DEFAULT_DASHSCOPE_BASE_URL: &str =
    "https://dashscope.aliyuncs.com/compatible-mode/v1";

#[derive(Error, Debug)]
pub enum QwenOAuthError {
    #[error("Qwen OAuth error: {0}")]
    Message(String),

    #[error(transparent)]
    Request(#[from] reqwest::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QwenCredentials {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expiry_date: Option<u64>,
    pub resource_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthorizationData {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct ErrorData {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenData {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    token_type: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    resource_url: Option<String>,
}

enum TokenPollResult {
    Success(TokenData),
    Pending { slow_down: bool },
}

pub struct QwenOAuthManager {
    http: Client,
    cached: Option<QwenCredentials>,
}

impl QwenOAuthManager {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
            cached: None,
        }
    }

    pub async fn ensure_valid_credentials(&mut self) -> Result<QwenCredentials, QwenOAuthError> {
        if self.cached.is_none() {
            self.cached = load_cached_credentials()?;
            if self.cached.is_some() {
                log_debug("loaded cached Qwen OAuth credentials");
            }
        }

        if let Some(creds) = self.cached.clone() {
            if is_token_valid(&creds) {
                log_debug("using cached Qwen OAuth access token");
                return Ok(creds);
            }
        }

        if let Some(creds) = self.cached.clone() {
            if creds.refresh_token.is_some() {
                log_debug("refreshing Qwen OAuth access token");
                if let Ok(refreshed) = self.refresh_access_token(&creds).await {
                    save_credentials(&refreshed)?;
                    log_debug("Qwen OAuth access token refreshed");
                    self.cached = Some(refreshed.clone());
                    return Ok(refreshed);
                }
            }
        }

        log_debug("starting Qwen OAuth device flow");
        let credentials = self.device_authorization_flow().await?;
        save_credentials(&credentials)?;
        log_debug("Qwen OAuth device flow completed");
        self.cached = Some(credentials.clone());
        Ok(credentials)
    }

pub fn endpoint_for(credentials: &QwenCredentials) -> String {
        normalize_endpoint(credentials.resource_url.as_deref())
    }

    async fn device_authorization_flow(&self) -> Result<QwenCredentials, QwenOAuthError> {
        let (code_verifier, code_challenge) = generate_pkce_pair();
        let device_auth = self
            .request_device_authorization(&code_challenge)
            .await?;

        println!("\n=== Qwen OAuth Device Authorization ===");
        println!("Please visit this URL to authorize:\n");
        println!("{}", device_auth.verification_uri_complete);
        println!("\nUser code: {}", device_auth.user_code);
        println!("\nWaiting for authorization...\n");

        if open::that(&device_auth.verification_uri_complete).is_err() {
            println!(
                "Unable to open a browser automatically. Use the URL above to continue."
            );
        }

        let mut poll_interval = Duration::from_secs(2);
        let deadline = Duration::from_secs(device_auth.expires_in);
        let start = std::time::Instant::now();

        while start.elapsed() < deadline {
            match self
                .poll_device_token(&device_auth.device_code, &code_verifier)
                .await?
            {
                TokenPollResult::Success(token) => {
                    return Ok(QwenCredentials {
                        access_token: token.access_token,
                        refresh_token: token.refresh_token,
                        token_type: token.token_type,
                        expiry_date: token
                            .expires_in
                            .map(|expires| now_ms().saturating_add(expires * 1000)),
                        resource_url: token.resource_url,
                    });
                }
                TokenPollResult::Pending { slow_down } => {
                    if slow_down {
                        let next = poll_interval.as_millis() as u64 * 3 / 2;
                        poll_interval =
                            Duration::from_millis(std::cmp::min(next, MAX_POLL_INTERVAL_MS));
                    } else {
                        poll_interval = Duration::from_secs(2);
                    }
                }
            }

            sleep(poll_interval).await;
        }

        Err(QwenOAuthError::Message(
            "Authorization timed out. Please retry.".to_string(),
        ))
    }

    async fn request_device_authorization(
        &self,
        code_challenge: &str,
    ) -> Result<DeviceAuthorizationData, QwenOAuthError> {
        let response = self
            .http
            .post(QWEN_OAUTH_DEVICE_CODE_ENDPOINT)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", QWEN_OAUTH_CLIENT_ID),
                ("scope", QWEN_OAUTH_SCOPE),
                ("code_challenge", code_challenge),
                ("code_challenge_method", "S256"),
            ])
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            if let Ok(error) = serde_json::from_str::<ErrorData>(&body) {
                let description = error.error_description.unwrap_or_default();
                return Err(QwenOAuthError::Message(format!(
                    "Device authorization failed: {} {}",
                    error.error, description
                )));
            }
            return Err(QwenOAuthError::Message(format!(
                "Device authorization failed: {} {}",
                status, body
            )));
        }

        let data: DeviceAuthorizationData = serde_json::from_str(&body)?;
        Ok(data)
    }

    async fn poll_device_token(
        &self,
        device_code: &str,
        code_verifier: &str,
    ) -> Result<TokenPollResult, QwenOAuthError> {
        let response = self
            .http
            .post(QWEN_OAUTH_TOKEN_ENDPOINT)
            .header("Accept", "application/json")
            .form(&[
                ("grant_type", QWEN_OAUTH_GRANT_TYPE),
                ("client_id", QWEN_OAUTH_CLIENT_ID),
                ("device_code", device_code),
                ("code_verifier", code_verifier),
            ])
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            let token: TokenData = serde_json::from_str(&body)?;
            return Ok(TokenPollResult::Success(token));
        }

        if let Ok(error) = serde_json::from_str::<ErrorData>(&body) {
            if status.as_u16() == 400 && error.error == "authorization_pending" {
                return Ok(TokenPollResult::Pending { slow_down: false });
            }

            if status.as_u16() == 429 && error.error == "slow_down" {
                return Ok(TokenPollResult::Pending { slow_down: true });
            }

            let description = error.error_description.unwrap_or_default();
            return Err(QwenOAuthError::Message(format!(
                "Token polling failed: {} {}",
                error.error, description
            )));
        }

        Err(QwenOAuthError::Message(format!(
            "Token polling failed: {} {}",
            status, body
        )))
    }

    async fn refresh_access_token(
        &self,
        credentials: &QwenCredentials,
    ) -> Result<QwenCredentials, QwenOAuthError> {
        let refresh_token = credentials
            .refresh_token
            .as_ref()
            .ok_or_else(|| QwenOAuthError::Message("No refresh token available.".to_string()))?;

        let response = self
            .http
            .post(QWEN_OAUTH_TOKEN_ENDPOINT)
            .header("Accept", "application/json")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", QWEN_OAUTH_CLIENT_ID),
            ])
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            return Err(QwenOAuthError::Message(format!(
                "Token refresh failed: {} {}",
                status, body
            )));
        }

        let token: TokenData = serde_json::from_str(&body)?;

        Ok(QwenCredentials {
            access_token: token.access_token,
            refresh_token: token.refresh_token.or_else(|| credentials.refresh_token.clone()),
            token_type: token.token_type,
            expiry_date: token
                .expires_in
                .map(|expires| now_ms().saturating_add(expires * 1000)),
            resource_url: token.resource_url.or_else(|| credentials.resource_url.clone()),
        })
    }
}

fn normalize_endpoint(resource_url: Option<&str>) -> String {
    if let Some(value) = resource_url {
        log_debug(&format!("resource_url={value}"));
    }

    let raw = resource_url
        .and_then(|value| sanitize_resource_url(value))
        .unwrap_or_else(|| DEFAULT_DASHSCOPE_BASE_URL.to_string());

    let mut base = if raw.starts_with("http") {
        raw
    } else {
        format!("https://{}", raw)
    };

    while base.ends_with('/') {
        base.pop();
    }

    if !is_portal_endpoint(&base) && !base.contains("compatible-mode") {
        base.push_str("/compatible-mode");
    }

    if !base.ends_with("/v1") {
        base.push_str("/v1");
    }

    base.push('/');
    base
}

fn sanitize_resource_url(value: &str) -> Option<String> {
    let parsed = if value.starts_with("http") {
        reqwest::Url::parse(value).ok()
    } else {
        reqwest::Url::parse(&format!("https://{}", value)).ok()
    };
    let host = parsed.as_ref().and_then(|url| url.host_str());

    if let Some(host) = host {
        let allowed = host == "dashscope.aliyuncs.com"
            || host == "dashscope-intl.aliyuncs.com"
            || host == "portal.qwen.ai";
        if allowed {
            return Some(value.to_string());
        }
    }

    None
}

fn is_portal_endpoint(base_url: &str) -> bool {
    if let Ok(parsed) = reqwest::Url::parse(base_url) {
        return parsed.host_str() == Some("portal.qwen.ai");
    }
    false
}

fn generate_pkce_pair() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

    (verifier, challenge)
}

fn is_token_valid(credentials: &QwenCredentials) -> bool {
    if credentials.access_token.is_empty() {
        return false;
    }

    match credentials.expiry_date {
        Some(expiry) => expiry > now_ms().saturating_add(TOKEN_REFRESH_BUFFER_MS),
        None => false,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}

fn credential_path() -> Result<PathBuf, QwenOAuthError> {
    let mut base = dirs::home_dir()
        .ok_or_else(|| QwenOAuthError::Message("Unable to resolve home directory.".to_string()))?;
    base.push(QWEN_DIR);
    base.push(QWEN_CREDENTIAL_FILENAME);
    Ok(base)
}

fn load_cached_credentials() -> Result<Option<QwenCredentials>, QwenOAuthError> {
    let path = credential_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path)?;
    let creds: QwenCredentials = serde_json::from_str(&content)?;
    if creds.access_token.is_empty() {
        return Ok(None);
    }
    Ok(Some(creds))
}

fn save_credentials(credentials: &QwenCredentials) -> Result<(), QwenOAuthError> {
    let path = credential_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(credentials)?;
    fs::write(path, data)?;
    Ok(())
}

fn log_debug(message: &str) {
    if std::env::var("LUMEN_QWEN_DEBUG").ok().as_deref() == Some("1") {
        eprintln!("[qwen-oauth] {message}");
    }
}
