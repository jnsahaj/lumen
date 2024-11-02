use clap::{command, Parser, Subcommand, ValueEnum};
use error::LumenError;
use git_entity::{git_commit::GitCommit, GitEntity};
use reqwest;
use std::process;
use tokio;

mod command;
mod error;
mod git_entity;
mod provider;

#[derive(Parser)]
#[command(name = "lumen")]
#[command(about = "AI-powered CLI tool for git commit summaries", long_about = None)]
struct Cli {
    #[arg(
        value_enum,
        short = 'p',
        long = "provider",
        env("LUMEN_AI_PROVIDER"),
        default_value = "phind"
    )]
    provider: ProviderType,

    #[arg(short = 'k', long = "api-key", env = "LUMEN_API_KEY")]
    api_key: Option<String>,

    #[arg(short = 'm', long = "model", env = "LUMEN_AI_MODEL")]
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
}

#[derive(Subcommand)]
enum Commands {
    Explain {
        #[arg()]
        sha: String,
    },
    List,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("\x1b[91m\rError: {e}\x1b[0m");
        process::exit(1);
    }
}

async fn run() -> Result<(), LumenError> {
    let cli = Cli::parse();
    let client = reqwest::Client::new();
    let provider = provider::LumenProvider::new(client, cli.provider, cli.api_key, cli.model)?;
    let command = command::LumenCommand::new(provider);

    match cli.command {
        Commands::Explain { sha } => {
            let git_entity = GitEntity::Commit(GitCommit::new(sha)?);
            command.explain(&git_entity).await?
        }
        Commands::List => command.list().await?,
    }

    Ok(())
}
