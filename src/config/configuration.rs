use crate::config::cli::ViewMode;
use crate::error::LumiereError;
use dirs::home_dir;
use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use crate::Cli;

/// Settings read from `~/.config/lumiere/lumiere.config.json`, or from the file
/// given with `--config`. Command-line flags take precedence.
#[derive(Debug, Deserialize, Default)]
pub struct LumiereConfig {
    #[serde(default)]
    pub theme: Option<String>,

    #[serde(default)]
    pub wrap: Option<bool>,

    /// `"hunks"` or `"full"`.
    #[serde(default)]
    pub view: Option<ViewMode>,
}

fn default_config_path() -> Option<PathBuf> {
    let path = home_dir()?.join(".config/lumiere/lumiere.config.json");
    path.exists().then_some(path)
}

impl LumiereConfig {
    pub fn build(cli: &Cli) -> Result<Self, LumiereError> {
        match cli.config.as_ref().map(PathBuf::from).or_else(default_config_path) {
            Some(path) => {
                let reader = BufReader::new(File::open(path)?);
                serde_json::from_reader(reader)
                    .map_err(|error| LumiereError::InvalidConfiguration(error.to_string()))
            }
            None => Ok(LumiereConfig::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::path::Path;

    fn build_from(config_file: &Path) -> Result<LumiereConfig, LumiereError> {
        let config_file = config_file.to_str().expect("utf-8 path");
        let cli = Cli::try_parse_from(["lumiere", "--config", config_file, "diff"]).expect("cli");
        LumiereConfig::build(&cli)
    }

    #[test]
    fn test_settings_come_from_the_given_file_and_a_broken_file_is_reported() {
        let directory = tempfile::tempdir().expect("temp dir");
        let settings_file = directory.path().join("lumiere.config.json");

        // Keys Lumiere does not know, such as those of a Lumen settings file, are ignored.
        std::fs::write(
            &settings_file,
            r#"{"theme": "nord", "wrap": true, "view": "full", "provider": "openai"}"#,
        )
        .expect("write settings");
        let config = build_from(&settings_file).expect("settings load");
        assert_eq!(config.theme.as_deref(), Some("nord"));
        assert_eq!(config.wrap, Some(true));
        assert_eq!(config.view, Some(ViewMode::Full));

        std::fs::write(&settings_file, "{}").expect("write settings");
        let config = build_from(&settings_file).expect("empty settings load");
        assert_eq!((config.theme, config.wrap, config.view), (None, None, None));

        std::fs::write(&settings_file, r#"{"view": "unified"}"#).expect("write settings");
        assert!(matches!(
            build_from(&settings_file),
            Err(LumiereError::InvalidConfiguration(_))
        ));

        std::fs::write(&settings_file, "{not json").expect("write settings");
        assert!(matches!(
            build_from(&settings_file),
            Err(LumiereError::InvalidConfiguration(_))
        ));

        assert!(matches!(
            build_from(&directory.path().join("missing.json")),
            Err(LumiereError::IoError(_))
        ));
    }
}
