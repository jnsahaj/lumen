use clap::{command, Parser, Subcommand, ValueEnum};
use std::str::FromStr;

#[derive(Parser)]
#[command(name = "lumen")]
#[command(about = "AI-powered CLI tool for git commit summaries", long_about = None)]
#[command(version)]
pub struct Cli {
    /// Path to configuration file eg: ./path/to/lumen.config.json
    #[arg(long)]
    pub config: Option<String>,

    #[arg(value_enum, short = 'p', long = "provider")]
    pub provider: Option<ProviderType>,

    #[arg(short = 'k', long = "api-key")]
    pub api_key: Option<String>,

    #[arg(short = 'm', long = "model")]
    pub model: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum, Debug)]
pub enum ProviderType {
    Openai,
    Phind,
    Groq,
    Claude,
    Ollama,
    Openrouter
}

impl FromStr for ProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(ProviderType::Openai),
            "phind" => Ok(ProviderType::Phind),
            "groq" => Ok(ProviderType::Groq),
            "claude" => Ok(ProviderType::Claude),
            "ollama" => Ok(ProviderType::Ollama),
            "openrouter" => Ok(ProviderType::Openrouter),
            _ => Err(format!("Unknown provider: {}", s)),
        }
    }
}

#[derive(Subcommand)]
pub enum Commands {
    /// Explain the changes in a commit, or the current diff
    Explain {
        /// The commit hash to use
        #[arg(group = "target")]
        sha: Option<String>,

        /// Explain current diff
        #[arg(long, group = "target")]
        diff: bool,

        /// Use staged diff
        #[arg(long)]
        staged: bool,

        /// Ask a question instead of summary
        #[arg(short, long)]
        query: Option<String>,
    },
    /// List all commits in an interactive fuzzy-finder, and summarize the changes
    List,
    /// Generate a commit message for the staged changes
    Draft {
        /// Add context to communicate intent
        #[arg(short, long)]
        context: Option<String>,
    },
}
