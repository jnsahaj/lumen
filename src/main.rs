use clap::{command, Parser, Subcommand, ValueEnum};
use configuration::ProjectConfig;
use error::LumenError;
use git_entity::{git_commit::GitCommit, git_diff::GitDiff, GitEntity};
use reqwest;
use std::process;
use std::str::FromStr;
use tokio;

mod ai_prompt;
mod command;
mod configuration;
mod error;
mod git_entity;
mod provider;

#[derive(Parser)]
#[command(name = "lumen")]
#[command(about = "AI-powered CLI tool for git commit summaries", long_about = None)]
struct Cli {
    #[arg(value_enum, short = 'p', long = "provider")]
    provider: Option<ProviderType>,

    #[arg(short = 'k', long = "api-key")]
    api_key: Option<String>,

    #[arg(short = 'm', long = "model")]
    model: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum, Debug)]
enum ProviderType {
    Openai,
    Phind,
    Groq,
    Claude,
    Ollama,
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
            _ => Err(format!("Unknown provider: {}", s)),
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    Explain {
        /// The commit hash to use
        #[arg(group = "target")]
        sha: Option<String>,

        /// Use staged diff
        #[arg(long, group = "target")]
        diff: bool,

        #[arg(long)]
        staged: bool,
    },
    List,
    Draft {
        #[arg(short, long)]
        context: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    let config_path = "./lumen_config.json"; // TODO: Fix hardcoded path!
    let config: ProjectConfig = ProjectConfig::from_file(&config_path.to_string());

    if let Err(e) = run(config).await {
        eprintln!("\x1b[91m\rerror:\x1b[0m {e}");
        process::exit(1);
    }
}

async fn run(config: configuration::ProjectConfig) -> Result<(), LumenError> {
    let cli = Cli::parse();
    let client = reqwest::Client::new();

    let model_provider: ProviderType = cli
        .provider
        .or_else(|| config.model_provider.parse().ok())
        .unwrap_or(ProviderType::Phind);

    let api_key: Option<String> = cli.api_key.or_else(|| Some(config.api_key));

    let model: Option<String> = cli.model.or_else(|| Some(config.model));

    let provider = provider::LumenProvider::new(client, model_provider, api_key, model)?;
    let command = command::LumenCommand::new(provider);

    match cli.command {
        Commands::Explain { sha, diff, staged } => {
            let git_entity = if diff {
                GitEntity::Diff(GitDiff::new(staged)?)
            } else if let Some(sha) = sha {
                GitEntity::Commit(GitCommit::new(sha)?)
            } else {
                return Err(LumenError::InvalidArguments(
                    "`explain` expects SHA-1 or --diff to be present".into(),
                ));
            };

            command.explain(&git_entity).await?
        }
        Commands::List => command.list().await?,
        Commands::Draft { context } => command.draft(context).await?,
    }

    Ok(())
}
