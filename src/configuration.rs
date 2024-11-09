use serde::Deserialize;
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
        }
    }
}
