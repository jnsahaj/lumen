use crate::config::cli::ProviderType;
use crate::error::LumenError;
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

    #[serde(default = "default_draft_config")]
    pub draft: DraftConfig,

    #[serde(default)]
    pub explain: Option<ExplainConfig>,

    #[serde(default)]
    pub list: Option<ListConfig>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DraftConfig {
    #[serde(
        default = "default_commit_types",
        deserialize_with = "deserialize_commit_types"
    )]
    pub commit_types: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct ExplainConfig {
    // Add explain-specific settings
}

#[derive(Debug, Deserialize, Default)]
pub struct ListConfig {
    // Add list-specific settings
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

fn default_commit_types() -> String {
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

fn default_draft_config() -> DraftConfig {
    DraftConfig {
        commit_types: default_commit_types(),
    }
}

impl LumenConfig {
    pub fn Build(cli: &Cli) -> Result<Self, LumenError> {
        let config_path = "./lumen.config.json";
        let config = match LumenConfig::from_file(config_path) {
            Ok(config) => config,
            Err(e) => return Err(e),
        };

        let ai_provider: ProviderType = cli
            .provider
            .or_else(|| Some(config.ai_provider))
            .unwrap_or(default_ai_provider());

        let api_key: String = cli.api_key.clone().unwrap_or(config.api_key);
        let model: String = cli.model.clone().unwrap_or(config.model);

        Ok(LumenConfig {
            ai_provider,
            model,
            api_key,
            draft: config.draft,
            explain: None,
            list: None,
        })
    }

    pub fn from_file(file_path: &str) -> Result<Self, LumenError> {
        let content = match fs::read_to_string(file_path) {
            Ok(content) => content,
            // FILE DOSENT EXIST
            Err(_) => return Ok(LumenConfig::default()),
        };

        match serde_json::from_str::<LumenConfig>(&content) {
            Ok(config) => Ok(config),
            Err(e) => {
                Err(LumenError::InvalidConfiguration(e.to_string()))
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
            draft: default_draft_config(), 
            explain: None,
            list: None,
        }
    }
}
