use crate::config::cli::ProviderType;
use crate::config::{ProviderInfo, ALL_PROVIDERS};
use crate::error::LumenError;
use crate::provider::lmstudio_endpoint;
use dirs::home_dir;
use inquire::{Select, Text};
use serde_json::{json, Value};
use std::fmt;
use std::fs;
use std::time::Duration;

/// Wrapper for display in the selection prompt
struct ProviderChoice(&'static ProviderInfo);

impl fmt::Display for ProviderChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.display_name)
    }
}

/// Command to handle interactive configuration of Lumen features.
pub struct ConfigureCommand;

impl ConfigureCommand {
    /// Executes the interactive configuration wizard.
    ///
    /// This process:
    /// 1. Prompts the user to select an AI provider
    /// 2. Asks for an API key (if needed)
    /// 3. Allows specifying a custom model name
    /// 4. Saves the configuration to `~/.config/lumen/lumen.config.json`
    pub async fn execute() -> Result<(), LumenError> {
        println!("\n  \x1b[1;36mLumen Configuration\x1b[0m\n");

        let provider = Self::select_provider()?;
        let api_key = Self::get_api_key(provider)?;
        let current_model = Self::existing_model();
        let model = Self::get_model_name(provider, current_model.as_deref()).await?;

        Self::save_config(provider, api_key.as_deref(), model.as_deref())?;

        let config_path = Self::get_config_path()?;
        println!(
            "\n  \x1b[1;32m✓\x1b[0m Configuration saved to \x1b[2m{}\x1b[0m\n",
            config_path.join("lumen.config.json").display()
        );

        Ok(())
    }

    /// Prompts the user to select an AI provider from the supported list.
    fn select_provider() -> Result<&'static ProviderInfo, LumenError> {
        let options: Vec<ProviderChoice> = ALL_PROVIDERS.iter().map(ProviderChoice).collect();

        let selection = Select::new("Select your default AI provider:", options)
            .with_help_message("↑↓ to move, enter to select, type to filter")
            .prompt()
            .map_err(|e| LumenError::ConfigurationError(e.to_string()))?;

        Ok(selection.0)
    }

    /// Prompts the user for an API key if the provider requires one.
    /// Returns `None` if the user leaves the input empty (to use env var) or if the provider
    /// is local (e.g. Ollama, LM Studio).
    fn get_api_key(provider: &ProviderInfo) -> Result<Option<String>, LumenError> {
        if provider.env_key.is_empty() {
            if let Some(note) = &provider.no_auth_notice {
                println!("\n  \x1b[2m{note}\x1b[0m");
            }
            return Ok(None);
        }

        let prompt = format!(
            "Enter your API key (or leave empty to use {}):",
            provider.env_key
        );

        let api_key = Text::new(&prompt)
            .prompt()
            .map_err(|e| LumenError::ConfigurationError(e.to_string()))?;

        if api_key.is_empty() {
            Ok(None)
        } else {
            Ok(Some(api_key))
        }
    }

    /// Prompts the user for a model.
    ///
    /// For LM Studio, discovers models from the running server and lets the user pick
    /// (falling back to free text if the server is unreachable). For other providers,
    /// prompts for a model name; returns `None` if the user accepts the default.
    async fn get_model_name(
        provider: &ProviderInfo,
        current_model: Option<&str>,
    ) -> Result<Option<String>, LumenError> {
        if provider.provider_type == ProviderType::LmStudio {
            return Self::get_lmstudio_model(current_model).await;
        }

        let prompt = format!(
            "Enter model name (leave empty for default: {}):",
            provider.default_model
        );

        let model = Text::new(&prompt)
            .with_help_message("Press Enter to use the default model")
            .prompt()
            .map_err(|e| LumenError::ConfigurationError(e.to_string()))?;

        if model.is_empty() {
            Ok(None)
        } else {
            Ok(Some(model))
        }
    }

    /// Model selection for LM Studio: fetch the model list from the server and let the
    /// user pick. Falls back to free-text input if the server is down or has no models.
    async fn get_lmstudio_model(current_model: Option<&str>) -> Result<Option<String>, LumenError> {
        match Self::fetch_lmstudio_models().await {
            Ok(models) if !models.is_empty() => {
                let initial = current_model
                    .and_then(|current| models.iter().position(|m| m == current))
                    .unwrap_or(0);

                // Warn if configured model isn't loaded, or confirm it is
                if let Some(current) = current_model {
                    if !models.contains(&current.to_string()) {
                        println!(
                            "\n  \x1b[33m⚠ Model '{current}' is not loaded in LM Studio. \
                             Selecting first available model.\x1b[0m"
                        );
                    } else {
                        println!(
                            "\n  \x1b[32m✓\x1b[0m Model '{current}' is loaded and ready to use."
                        );
                    }
                }

                let selected = Select::new(
                    "Select a model (it must be loaded in LM Studio):",
                    models,
                )
                .with_help_message("↑↓ to move, enter to select, type to filter")
                .with_starting_cursor(initial)
                .prompt()
                .map_err(|e| LumenError::ConfigurationError(e.to_string()))?;
                Ok(Some(selected))
            }
            Ok(_) => {
                println!(
                    "\n  \x1b[33m⚠ LM Studio has no models loaded. Load one in LM Studio, then enter its ID:\x1b[0m"
                );
                Self::prompt_model_text()
            }
            Err(e) => {
                println!(
                    "\n  \x1b[33m⚠ Could not reach LM Studio at {} ({}). Enter a model ID manually:\x1b[0m",
                    lmstudio_endpoint(),
                    e
                );
                Self::prompt_model_text()
            }
        }
    }

    /// Free-text model prompt (fallback when LM Studio model discovery fails).
    /// Loops until the user provides a non-empty model ID — LM Studio has no default,
    /// so saving an empty model would cause a guaranteed runtime failure.
    fn prompt_model_text() -> Result<Option<String>, LumenError> {
        loop {
            let model = Text::new("Model ID (e.g. qwen3-8b)")
                .with_help_message("Required — the model must be loaded in LM Studio")
                .prompt()
                .map_err(|e| LumenError::ConfigurationError(e.to_string()))?;

            if !model.is_empty() {
                return Ok(Some(model));
            }
            println!("\n  \x1b[33m⚠ A model ID is required for LM Studio. Please enter one.\x1b[0m");
        }
    }

    /// Fetches available model IDs from the LM Studio server (`GET {base}/models`, OpenAI format).
    async fn fetch_lmstudio_models() -> Result<Vec<String>, LumenError> {
        #[derive(serde::Deserialize)]
        struct ModelsResponse {
            data: Vec<ModelEntry>,
        }

        #[derive(serde::Deserialize)]
        struct ModelEntry {
            id: String,
        }

        let url = format!("{}models", lmstudio_endpoint());

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| LumenError::ConfigurationError(e.to_string()))?;

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| LumenError::ConfigurationError(format!("{url}: {e}")))?;

        if !response.status().is_success() {
            return Err(LumenError::ConfigurationError(format!(
                "{url} returned {}",
                response.status()
            )));
        }

        let body: ModelsResponse = response
            .json()
            .await
            .map_err(|e| LumenError::ConfigurationError(format!("invalid model list: {e}")))?;

        Ok(body.data.into_iter().map(|m| m.id).collect())
    }

    /// Reads the model from the existing config file, if any (used to preselect in prompts).
    fn existing_model() -> Option<String> {
        let path = Self::get_config_path().ok()?.join("lumen.config.json");
        let content = fs::read_to_string(&path).ok()?;
        let value: Value = serde_json::from_str(&content).ok()?;
        value
            .get("model")
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
    }

    /// Resolves the path to the configuration directory (`~/.config/lumen`).
    fn get_config_path() -> Result<std::path::PathBuf, LumenError> {
        let mut path = home_dir().ok_or_else(|| {
            LumenError::ConfigurationError("Could not determine home directory".to_string())
        })?;
        path.push(".config");
        path.push("lumen");
        Ok(path)
    }

    /// Saves the selected configuration to the JSON config file.
    /// If `model` is `None`, any existing `model` key in the config is removed to ensure
    /// the provider's default is used. For LM Studio, prompts again if a model was previously
    /// configured (since LM Studio has no default and an empty model causes runtime errors).
    fn save_config(
        provider: &ProviderInfo,
        api_key: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), LumenError> {
        let config_dir = Self::get_config_path()?;
        fs::create_dir_all(&config_dir)?;

        let config_file = config_dir.join("lumen.config.json");

        let mut config: Value = if config_file.exists() {
            let content = fs::read_to_string(&config_file)?;
            serde_json::from_str(&content).unwrap_or_else(|_| json!({}))
        } else {
            json!({})
        };

        // Get provider ID from the type
        config["provider"] = json!(provider.id);

        if let Some(key) = api_key {
            config["api_key"] = json!(key);
        }

        // LM Studio requires a model - prompt again if empty and one was previously configured
        if provider.provider_type == ProviderType::LmStudio && model.is_none() {
            if let Some(existing_model) = config.get("model").and_then(|m| m.as_str()) {
                println!(
                    "\n  \x1b[33m⚠ LM Studio requires a model to be configured. \
                     Previous model '{existing_model}' will be retained.\x1b[0m"
                );
            }
        } else if let Some(m) = model {
            config["model"] = json!(m);
        } else {
            // Remove model key to use provider default
            config.as_object_mut().map(|obj| obj.remove("model"));
        }

        let content = serde_json::to_string_pretty(&config)?;
        fs::write(&config_file, content)?;

        Ok(())
    }
}
