use clap::Parser;
use command::LumenCommand;
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

    let config = match LumenConfig::build(&cli) {
        Ok(config) => config,
        Err(e) => return Err(e),
    };

    let provider =
        provider::LumenProvider::new(config.provider, config.api_key, config.model)?;
    let command = command::LumenCommand::new(provider);

    match cli.command {
        Commands::Explain {
            reference,
            staged,
            query,
            list,
        } => {
            let git_entity = if list {
                let sha = LumenCommand::get_sha_from_fzf()?;
                GitEntity::Commit(Commit::new(sha)?)
            } else {
                match reference {
                    Some(CommitReference::Single(input)) => {
                        let sha = if input == "-" {
                            read_from_stdin()?
                        } else {
                            input
                        };
                        GitEntity::Commit(Commit::new(sha)?)
                    }
                    Some(CommitReference::Range { from, to }) => {
                        GitEntity::Diff(Diff::from_commits_range(&from, &to, false)?)
                    }
                    Some(CommitReference::TripleDots { from, to }) => {
                        GitEntity::Diff(Diff::from_commits_range(&from, &to, true)?)
                    }
                    None => {
                        // Default: show uncommitted diff
                        GitEntity::Diff(Diff::from_working_tree(staged)?)
                    }
                }
            };

            command
                .execute(command::CommandType::Explain { git_entity, query })
                .await?;
        }
        Commands::List => {
            eprintln!("Warning: 'lumen list' is deprecated. Use 'lumen explain --list' instead.");
            command.execute(command::CommandType::List).await?
        }
        Commands::Draft { context } => {
            command
                .execute(command::CommandType::Draft(context, config.draft))
                .await?
        }
        Commands::Operate { query } => {
            command
                .execute(command::CommandType::Operate { query })
                .await?;
        }
        Commands::Diff {
            reference,
            pr,
            file,
            watch,
        } => {
            let options = command::diff::DiffOptions {
                reference,
                pr,
                file,
                watch,
            };
            command::diff::run_diff_ui(options)?;
        }
        Commands::Configure => {
            command::configure::ConfigureCommand::execute()?;
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
