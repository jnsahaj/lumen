use clap::Parser;
use command::LumenCommand;
use commit_reference::CommitReference;
use config::cli::{Cli, Commands};
use config::LumenConfig;
use error::LumenError;
use git_entity::{commit::Commit, diff::Diff, GitEntity};
use std::io::Read;
use std::process;
use vcs::VcsBackendType;

mod ai_prompt;
mod command;
mod commit_reference;
mod config;
mod error;
mod git_entity;
mod provider;
mod vcs;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("\x1b[91m\rerror:\x1b[0m {e}");
        process::exit(1);
    }
}

async fn run() -> Result<(), LumenError> {
    let cli = Cli::parse();

    if let Commands::Completion { shell } = &cli.command {
        command::completion::generate_completions(*shell);
        return Ok(());
    }

    let config = match LumenConfig::build(&cli) {
        Ok(config) => config,
        Err(e) => return Err(e),
    };

    let provider = provider::LumenProvider::new(config.provider, config.api_key, config.model)?;
    let command = command::LumenCommand::new(provider);

    // Get VCS backend based on CLI override or auto-detection
    let cwd = std::env::current_dir()?;
    let vcs_override = cli.vcs.map(VcsBackendType::from);
    let backend = vcs::get_backend(&cwd, vcs_override)?;

    match cli.command {
        Commands::Explain {
            reference,
            staged,
            query,
            list,
        } => {
            let git_entity = if list {
                let sha = LumenCommand::get_sha_from_fzf(backend.as_ref())?;
                let info = backend.get_commit(&sha)?;
                GitEntity::Commit(Commit::from_commit_info(info))
            } else {
                match reference {
                    Some(CommitReference::Single(input)) => {
                        let sha = if input == "-" {
                            read_from_stdin()?
                        } else {
                            input
                        };
                        let info = backend.get_commit(&sha)?;
                        GitEntity::Commit(Commit::from_commit_info(info))
                    }
                    Some(CommitReference::Range { from, to }) => {
                        let diff = backend.get_range_diff(&from, &to, false)?;
                        GitEntity::Diff(Diff::from_range_diff(diff, from, to)?)
                    }
                    Some(CommitReference::TripleDots { from, to }) => {
                        let diff = backend.get_range_diff(&from, &to, true)?;
                        GitEntity::Diff(Diff::from_range_diff(diff, from, to)?)
                    }
                    None => {
                        // Default: show uncommitted diff
                        let diff = backend.get_working_tree_diff(staged)?;
                        GitEntity::Diff(Diff::from_working_tree_diff(diff, staged)?)
                    }
                }
            };

            command
                .execute(command::CommandType::Explain { git_entity, query })
                .await?;
        }
        Commands::List => {
            eprintln!("Warning: 'lumen list' is deprecated. Use 'lumen explain --list' instead.");
            command
                .execute(command::CommandType::List {
                    backend: backend.as_ref(),
                })
                .await?
        }
        Commands::Draft { context } => {
            // Draft always uses staged diff (git convention)
            let diff = backend.get_working_tree_diff(true)?;
            let git_entity = GitEntity::Diff(Diff::from_working_tree_diff(diff, true)?);
            command
                .execute(command::CommandType::Draft {
                    git_entity,
                    context,
                    draft_config: config.draft,
                })
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
            theme,
            stacked,
        } => {
            let options = command::diff::DiffOptions {
                reference,
                pr,
                file,
                watch,
                theme: theme.or(config.theme.clone()),
                stacked,
            };
            command::diff::run_diff_ui(options, backend.as_ref())?;
        }
        Commands::Configure => {
            command::configure::ConfigureCommand::execute()?;
        }
        Commands::Completion { .. } => unreachable!("completion handled before config"),
    }

    Ok(())
}

fn read_from_stdin() -> Result<String, LumenError> {
    let mut buffer = String::new();
    std::io::stdin().read_to_string(&mut buffer)?;

    eprintln!("Reading commit SHA from stdin: '{}'", buffer.trim());
    Ok(buffer)
}
