/// Single source of truth for all provider configurations.
///
/// Add new providers here - they will automatically appear in:
/// - The `lumen configure` interactive prompt
/// - The provider initialization in provider/mod.rs
use crate::config::cli::ProviderType;

/// A notice displayed when a provider doesn't require an API key.
pub struct NoAuthNotice(&'static str);

impl std::fmt::Display for NoAuthNotice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Provider metadata with display name, default model, and environment variable key
pub struct ProviderInfo {
    pub id: &'static str,
    pub provider_type: ProviderType,
    pub display_name: &'static str,
    pub default_model: &'static str,
    pub env_key: &'static str,
    /// Optional notice printed when no API key is needed (local providers).
    /// Note: This notice only appears during `lumen configure`, not at runtime.
    pub no_auth_notice: Option<NoAuthNotice>,
}

/// All supported providers - single source of truth.
/// Add new providers here to make them available everywhere.
pub const ALL_PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        id: "openai",
        provider_type: ProviderType::Openai,
        display_name: "OpenAI",
        default_model: "gpt-5-mini",
        env_key: "OPENAI_API_KEY",
        no_auth_notice: None,
    },
    ProviderInfo {
        id: "groq",
        provider_type: ProviderType::Groq,
        display_name: "Groq",
        default_model: "llama-3.3-70b-versatile",
        env_key: "GROQ_API_KEY",
        no_auth_notice: None,
    },
    ProviderInfo {
        id: "claude",
        provider_type: ProviderType::Claude,
        display_name: "Claude (Anthropic)",
        default_model: "claude-sonnet-4-5-20250930",
        env_key: "ANTHROPIC_API_KEY",
        no_auth_notice: None,
    },
    ProviderInfo {
        id: "ollama",
        provider_type: ProviderType::Ollama,
        display_name: "Ollama (local)",
        default_model: "llama3.2",
        env_key: "",
        no_auth_notice: Some(NoAuthNotice("Ollama runs locally — no API key needed.")),
    },
    ProviderInfo {
        id: "lmstudio",
        provider_type: ProviderType::LmStudio,
        display_name: "LM Studio (local)",
        // No default: model IDs are arbitrary and must be loaded in LM Studio.
        // The configure wizard discovers models from the server; at runtime an
        // empty model produces a clear error (see provider/mod.rs).
        // Note: The no_auth_notice only appears during lumen configure, not at runtime.
        default_model: "",
        env_key: "",
        no_auth_notice: Some(
            NoAuthNotice(
                "LM Studio runs locally — no API key needed (use -k if you enabled server auth).",
            ),
        ),
    },
    ProviderInfo {
        id: "opencode-zen",
        provider_type: ProviderType::OpencodeZen,
        display_name: "OpenCode Zen",
        default_model: "claude-sonnet-4-5",
        env_key: "OPENCODE_API_KEY",
        no_auth_notice: None,
    },
    ProviderInfo {
        id: "openrouter",
        provider_type: ProviderType::Openrouter,
        display_name: "OpenRouter",
        default_model: "anthropic/claude-sonnet-4.5",
        env_key: "OPENROUTER_API_KEY",
        no_auth_notice: None,
    },
    ProviderInfo {
        id: "deepseek",
        provider_type: ProviderType::Deepseek,
        display_name: "DeepSeek",
        default_model: "deepseek-chat",
        env_key: "DEEPSEEK_API_KEY",
        no_auth_notice: None,
    },
    ProviderInfo {
        id: "gemini",
        provider_type: ProviderType::Gemini,
        display_name: "Gemini (Google)",
        default_model: "gemini-2.5-flash",
        env_key: "GEMINI_API_KEY",
        no_auth_notice: None,
    },
    ProviderInfo {
        id: "xai",
        provider_type: ProviderType::Xai,
        display_name: "xAI (Grok)",
        default_model: "grok-4-mini-fast",
        env_key: "XAI_API_KEY",
        no_auth_notice: None,
    },
    ProviderInfo {
        id: "vercel",
        provider_type: ProviderType::Vercel,
        display_name: "Vercel AI Gateway",
        default_model: "anthropic/claude-sonnet-4.5",
        env_key: "VERCEL_API_KEY",
        no_auth_notice: None,
    },
];

impl ProviderInfo {
    /// Get provider info by type
    pub fn for_provider(provider: ProviderType) -> &'static ProviderInfo {
        ALL_PROVIDERS
            .iter()
            .find(|p| p.provider_type == provider)
            .expect("All provider types must be defined in ALL_PROVIDERS")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_auth_notice_display_implements_fmt() {
        let notice = NoAuthNotice("Test notice");
        assert_eq!(format!("{}", notice), "Test notice");
    }

    /// Verify local providers have no_auth_notice set
    #[test]
    fn local_providers_have_no_auth_notice() {
        // Ollama and LM Studio are the only local providers (empty env_key)
        for provider in ALL_PROVIDERS {
            if provider.env_key.is_empty() {
                assert!(
                    provider.no_auth_notice.is_some(),
                    "Local provider '{}' should have no_auth_notice",
                    provider.display_name
                );
            }
        }
    }

    /// Verify remote providers don't have no_auth_notice set
    #[test]
    fn remote_providers_have_no_no_auth_notice() {
        // All providers with API keys should not have notices
        for provider in ALL_PROVIDERS {
            if !provider.env_key.is_empty() {
                assert!(
                    provider.no_auth_notice.is_none(),
                    "Remote provider '{}' should not have no_auth_notice",
                    provider.display_name
                );
            }
        }
    }
}
