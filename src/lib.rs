pub mod cli;
pub mod config;
pub mod diff;
pub mod error;
pub mod review;
pub mod search;
pub mod ui;
pub mod vcs;

use std::io::Read;

use clap::Parser;

use crate::cli::{Cli, Commands, ThreadAction, ViewMode};
use crate::config::LumiereConfig;
use crate::diff::DiffOptions;
use crate::error::LumiereError;
use crate::review::{cli as thread_cli, Workspace};
use crate::vcs::VcsBackendType;

pub fn run() -> Result<(), LumiereError> {
    let cli = Cli::parse();
    let config = LumiereConfig::build(&cli)?;

    let cwd = std::env::current_dir()?;
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
            let options = DiffOptions {
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
            match Workspace::discover(&cwd, vcs_override) {
                Some(workspace) => ui::run_workspace_diff_ui(options, &workspace)?,
                None => {
                    let backend = vcs::get_backend(&cwd, vcs_override)?;
                    ui::run_diff_ui(options, backend.as_ref())?
                }
            }
        }
        Commands::Thread { action } => {
            let repositories_at = |repo: Option<std::path::PathBuf>| {
                thread_cli::repositories_at(repo.as_deref().unwrap_or(&cwd), vcs_override)
                    .map_err(LumiereError::CommandError)
            };
            let output = match action {
                ThreadAction::List {
                    repo,
                    json,
                    awaiting,
                } => thread_cli::list(&repositories_at(repo)?, awaiting, json),
                ThreadAction::Show {
                    thread_id,
                    repo,
                    json,
                } => thread_cli::show(&repositories_at(repo)?, &thread_id, json),
                ThreadAction::Reply {
                    thread_id,
                    body,
                    repo,
                    author,
                    ..
                } => {
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
