use clap::Parser;
use config::cli::{Cli, Commands, ThreadAction, ViewMode};
use config::LumiereConfig;
use error::LumiereError;
use std::io::Read;
use std::process;
use vcs::VcsBackendType;

mod command;
mod commit_reference;
mod config;
mod error;
mod vcs;

fn main() {
    if let Err(e) = run() {
        eprintln!("\x1b[91m\rerror:\x1b[0m {e}");
        process::exit(1);
    }
}

fn run() -> Result<(), LumiereError> {
    let cli = Cli::parse();
    let config = LumiereConfig::build(&cli)?;

    let cwd = std::env::current_dir()?;
    // None: the version control system is detected.
    let vcs_override = cli.vcs.map(VcsBackendType::from);

    match cli.command {
        Commands::Diff {
            reference,
            pr,
            detect_pr,
            file,
            watch,
            theme,
            stacked,
            focus,
            origin,
            wrap,
            view,
            agent_block,
        } => {
            let options = command::diff::DiffOptions {
                reference,
                pr,
                detect_pr,
                file,
                watch,
                theme: theme.or(config.theme.clone()),
                stacked,
                focus,
                origin,
                wrap: wrap || config.wrap.unwrap_or(false),
                hunks_only: view.or(config.view) != Some(ViewMode::Full),
                agent_block,
            };
            // `diff` runs inside a repository, or in a folder of repositories.
            match command::diff::Workspace::discover(&cwd, vcs_override) {
                Some(workspace) => command::diff::run_workspace_diff_ui(options, &workspace)?,
                None => {
                    let backend = vcs::get_backend(&cwd, vcs_override)?;
                    command::diff::run_diff_ui(options, backend.as_ref())?
                }
            }
        }
        Commands::Thread { action } => {
            use command::diff::thread_cli;
            let repositories_at = |repo: Option<std::path::PathBuf>| {
                thread_cli::repositories_at(repo.as_deref().unwrap_or(&cwd), vcs_override)
                    .map_err(LumiereError::CommandError)
            };
            let output = match action {
                ThreadAction::List { repo, json, awaiting } => {
                    thread_cli::list(&repositories_at(repo)?, awaiting, json)
                }
                ThreadAction::Reply { thread_id, body, repo, author, .. } => {
                    let repositories = repositories_at(repo)?;
                    let body = match body {
                        Some(body) => body,
                        None => {
                            let mut buffer = String::new();
                            std::io::stdin().read_to_string(&mut buffer)?;
                            buffer
                        }
                    };
                    thread_cli::reply(&repositories, &thread_id, &body, &author)
                }
            };
            println!("{}", output.map_err(LumiereError::CommandError)?);
        }
    }

    Ok(())
}
