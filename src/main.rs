use clap::Parser;
use commit_reference::CommitReference;
use config::cli::{Cli, Commands};
use config::LumenConfig;
use error::LumenError;
use git_entity::{commit::Commit, diff::Diff, GitEntity};
use std::io::Read;
use std::process;

mod ai_prompt;
mod command;
mod commit_reference;
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
            reference,
            diff,
            staged,
            query,
            instruction,
        } => {
            let git_entity = if diff {
                GitEntity::Diff(Diff::from_working_tree(staged)?)
            } else if let Some(CommitReference::Single(input)) = reference {
                let sha = if input == "-" {
                    read_from_stdin()?
                } else {
                    input
                };
                GitEntity::Commit(Commit::new(sha)?)
            } else if let Some(CommitReference::Range { from, to }) = reference {
                GitEntity::Diff(Diff::from_commits_range(&from, &to, false)?)
            } else if let Some(CommitReference::TripleDots { from, to }) = reference {
                GitEntity::Diff(Diff::from_commits_range(&from, &to, true)?)
            } else {
                return Err(LumenError::InvalidArguments(
                    "`explain` expects SHA-1 or --diff to be present".into(),
                ));
            };

            command
                .execute(command::CommandType::Explain {
                    git_entity,
                    query,
                    instruction,
                })
                .await?;
        }
        Commands::List => command.execute(command::CommandType::List).await?,
        Commands::Draft {
            context,
            instruction,
        } => {
            command
                .execute(command::CommandType::Draft(
                    context,
                    config.draft,
                    instruction,
                ))
                .await?
        }
    }

    Ok(())
}

fn read_from_stdin() -> Result<String, LumenError> {
    let mut buffer = String::new();
    std::io::stdin().read_to_string(&mut buffer)?;

    eprintln!("Reading commit SHA from stdin: '{}'", buffer.trim());
    Ok(buffer)
}
