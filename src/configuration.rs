use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::env;
use std::fs;

#[derive(Deserialize)]
pub struct ProjectConfig {
    #[serde(default = "default_model_provider")]
    pub model_provider: String,

    #[serde(default = "default_model")]
    pub model: String,

    #[serde(default = "default_api_key")]
    pub api_key: String,

    #[serde(
        default = "default_commit_prefix",
        deserialize_with = "deserialize_prefix"
    )]
    pub prefix: String,
}

fn default_model_provider() -> String {
    env::var("LUMEN_AI_PROVIDER").unwrap_or_else(|_| "phind".to_string())
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

fn deserialize_prefix<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let prefix_map: HashMap<String, String> = HashMap::deserialize(deserializer)?;
    serde_json::to_string(&prefix_map).map_err(serde::de::Error::custom)
}

impl ProjectConfig {
    pub fn from_file(file_path: &String) -> Self {
        let content = match fs::read_to_string(file_path) {
            Ok(content) => content,
            Err(_) => return ProjectConfig::default(),
        };

        serde_json::from_str(&content).unwrap_or_else(|_| ProjectConfig::default())
    }
}

impl Default for ProjectConfig {
    fn default() -> Self {
        ProjectConfig {
            model_provider: default_model_provider(),
            model: default_model(),
            api_key: default_api_key(),
            prefix: default_commit_prefix(),
        }
    }
}
