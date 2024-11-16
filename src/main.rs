use clap::Parser;
use config::cli::{Cli, Commands};
use config::LumenConfig;
use error::LumenError;
use git_entity::{commit::Commit, diff::Diff, GitEntity};
use std::process;

mod ai_prompt;
mod command;
mod config;
mod error;
mod git_entity;
mod provider;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("\x1b[91m\rerror:\x1b[0m {e}");
        process::exit(1);
    }
}

async fn run() -> Result<(), LumenError> {
    let cli = Cli::parse();
    let client = reqwest::Client::new();

    let config = match LumenConfig::build(&cli) {
        Ok(config) => config,
        Err(e) => return Err(e),
    };

    let provider =
        provider::LumenProvider::new(client, config.provider, config.api_key, config.model)?;
    let command = command::LumenCommand::new(provider);

    match cli.command {
        Commands::Explain {
            sha,
            diff,
            staged,
            query,
        } => {
            let git_entity = if diff {
                GitEntity::Diff(Diff::from_working_tree(staged)?)
            } else if let Some(sha) = sha {
                GitEntity::Commit(Commit::new(sha)?)
            } else {
                return Err(LumenError::InvalidArguments(
                    "`explain` expects SHA-1 or --diff to be present".into(),
                ));
            };

            command
                .execute(command::CommandType::Explain { git_entity, query })
                .await?;
        }
        Commands::List => command.execute(command::CommandType::List).await?,
        Commands::Draft { context } => {
            command
                .execute(command::CommandType::Draft(context, config.draft))
                .await?
        }
    }

    Ok(())
}
