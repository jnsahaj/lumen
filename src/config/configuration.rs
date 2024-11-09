use crate::config::cli::ProviderType;
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::env;
use std::fs;

use crate::Cli;

#[derive(Debug, Deserialize)]
pub struct LumenConfig {
    #[serde(
        default = "default_ai_provider",
        deserialize_with = "deserialize_ai_provider"
    )]
    pub ai_provider: ProviderType,

    #[serde(default = "default_model")]
    pub model: String,

    #[serde(default = "default_api_key")]
    pub api_key: String,

    #[serde(
        default = "default_commit_prefix",
        deserialize_with = "deserialize_commit_types"
    )]
    pub commit_types: String,
}

fn default_ai_provider() -> ProviderType {
    env::var("LUMEN_AI_PROVIDER")
        .unwrap_or_else(|_| "phind".to_string())
        .parse()
        .unwrap_or(ProviderType::Phind)
}

fn deserialize_ai_provider<'de, D>(deserializer: D) -> Result<ProviderType, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse().map_err(serde::de::Error::custom)
}

fn default_model() -> String {
    env::var("LUMEN_AI_MODEL").unwrap_or_else(|_| "".to_string())
}

fn default_api_key() -> String {
    env::var("LUMEN_API_KEY").unwrap_or_else(|_| "".to_string())
}

fn default_commit_prefix() -> String {
    r#"{
        "docs": "Documentation only changes",
        "style": "Changes that do not affect the meaning of the code",
        "refactor": "A code change that neither fixes a bug nor adds a feature",
        "perf": "A code change that improves performance",
        "test": "Adding missing tests or correcting existing tests",
        "build": "Changes that affect the build system or external dependencies",
        "ci": "Changes to our CI configuration files and scripts",
        "chore": "Other changes that don't modify src or test files",
        "revert": "Reverts a previous commit",
        "feat": "A new feature",
        "fix": "A bug fix"
    }"#
    .to_string()
}

fn deserialize_commit_types<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let commit_types_map: HashMap<String, String> = HashMap::deserialize(deserializer)?;
    serde_json::to_string(&commit_types_map).map_err(serde::de::Error::custom)
}

impl LumenConfig {
    pub fn Build(cli: &Cli) -> Self {
        let config_path = "./lumen.config.json";
        let config = LumenConfig::from_file(&config_path.to_string());

        let ai_provider: ProviderType = cli
            .provider
            .or_else(|| Some(config.ai_provider))
            .unwrap_or(default_ai_provider());

        let api_key: String = cli.api_key.clone().unwrap_or(config.api_key);
        let model: String = cli.model.clone().unwrap_or(config.model);

        LumenConfig {
            ai_provider,
            model,
            api_key,
            commit_types: config.commit_types,
        }
    }

    pub fn from_file(file_path: &String) -> Self {
        let content = match fs::read_to_string(file_path) {
            Ok(content) => content,
            Err(_) => return LumenConfig::default(),
        };

        match serde_json::from_str::<LumenConfig>(&content) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("Failed to parse JSON: {}", e);
                LumenConfig::default()
            }
        }
    }
}

impl Default for LumenConfig {
    fn default() -> Self {
        LumenConfig {
            ai_provider: default_ai_provider(),
            model: default_model(),
            api_key: default_api_key(),
            commit_types: default_commit_prefix(),
        }
    }
}
