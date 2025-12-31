use crate::error::LumenError;
use dirs::home_dir;
use inquire::{Select, Text};
use serde_json::{json, Value};
use std::fmt;
use std::fs;

struct ProviderOption {
    id: &'static str,
    display_name: &'static str,
    env_var: &'static str,
    needs_api_key: bool,
}

impl fmt::Display for ProviderOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name)
    }
}

const PROVIDERS: &[ProviderOption] = &[
    ProviderOption {
        id: "openai",
        display_name: "OpenAI",
        env_var: "OPENAI_API_KEY",
        needs_api_key: true,
    },
    ProviderOption {
        id: "groq",
        display_name: "Groq",
        env_var: "GROQ_API_KEY",
        needs_api_key: true,
    },
    ProviderOption {
        id: "claude",
        display_name: "Claude (Anthropic)",
        env_var: "ANTHROPIC_API_KEY",
        needs_api_key: true,
    },
    ProviderOption {
        id: "ollama",
        display_name: "Ollama (local)",
        env_var: "",
        needs_api_key: false,
    },
    ProviderOption {
        id: "openrouter",
        display_name: "OpenRouter",
        env_var: "OPENROUTER_API_KEY",
        needs_api_key: true,
    },
    ProviderOption {
        id: "deepseek",
        display_name: "DeepSeek",
        env_var: "DEEPSEEK_API_KEY",
        needs_api_key: true,
    },
    ProviderOption {
        id: "gemini",
        display_name: "Gemini (Google)",
        env_var: "GEMINI_API_KEY",
        needs_api_key: true,
    },
    ProviderOption {
        id: "xai",
        display_name: "xAI (Grok)",
        env_var: "XAI_API_KEY",
        needs_api_key: true,
    },
    ProviderOption {
        id: "vercel",
        display_name: "Vercel AI Gateway",
        env_var: "VERCEL_API_KEY",
        needs_api_key: true,
    },
    ProviderOption {
        id: "qwen",
        display_name: "Qwen (OAuth)",
        env_var: "",
        needs_api_key: false,
    },
];

pub struct ConfigureCommand;

impl ConfigureCommand {
    pub fn execute() -> Result<(), LumenError> {
        println!("\n  \x1b[1;36mLumen Configuration\x1b[0m\n");

        let provider = Self::select_provider()?;
        let api_key = Self::get_api_key(&provider)?;

        Self::save_config(&provider, api_key.as_deref())?;

        let config_path = Self::get_config_path()?;
        println!(
            "\n  \x1b[1;32m✓\x1b[0m Configuration saved to \x1b[2m{}\x1b[0m\n",
            config_path.join("lumen.config.json").display()
        );

        Ok(())
    }

    fn select_provider() -> Result<&'static ProviderOption, LumenError> {
        let options: Vec<&ProviderOption> = PROVIDERS.iter().collect();

        let selection = Select::new("Select your default AI provider:", options)
            .with_help_message("↑↓ to move, enter to select, type to filter")
            .prompt()
            .map_err(|e| LumenError::ConfigurationError(e.to_string()))?;

        Ok(selection)
    }

    fn get_api_key(provider: &ProviderOption) -> Result<Option<String>, LumenError> {
        if !provider.needs_api_key {
            if provider.id == "qwen" {
                println!(
                    "\n  \x1b[2mQwen OAuth will open a browser on first use to authorize your account.\x1b[0m"
                );
            } else {
                println!(
                    "\n  \x1b[2mOllama runs locally — no API key needed.\x1b[0m"
                );
            }
            return Ok(None);
        }

        let prompt = format!(
            "Enter your API key (or leave empty to use {}):",
            provider.env_var
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

    fn get_config_path() -> Result<std::path::PathBuf, LumenError> {
        let mut path = home_dir().ok_or_else(|| {
            LumenError::ConfigurationError("Could not determine home directory".to_string())
        })?;
        path.push(".config");
        path.push("lumen");
        Ok(path)
    }

    fn save_config(provider: &ProviderOption, api_key: Option<&str>) -> Result<(), LumenError> {
        let config_dir = Self::get_config_path()?;
        fs::create_dir_all(&config_dir)?;

        let config_file = config_dir.join("lumen.config.json");

        let mut config: Value = if config_file.exists() {
            let content = fs::read_to_string(&config_file)?;
            serde_json::from_str(&content).unwrap_or_else(|_| json!({}))
        } else {
            json!({})
        };

        config["provider"] = json!(provider.id);

        if let Some(key) = api_key {
            config["api_key"] = json!(key);
        } else if provider.id == "qwen" {
            if let Some(obj) = config.as_object_mut() {
                obj.remove("api_key");
            }
        }

        let content = serde_json::to_string_pretty(&config)?;
        fs::write(&config_file, content)?;

        Ok(())
    }
}
