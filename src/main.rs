use clap::Parser;
use command::LumenCommand;
use commit_reference::CommitReference;
use config::cli::{Cli, Commands};
use config::LumenConfig;
use error::LumenError;
use git_entity::{commit::Commit, diff::Diff, GitEntity};
use std::io::Read;
use std::process;
use vcs::{NoopBackend, VcsBackend, VcsBackendType};

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

    let config = match LumenConfig::build(&cli) {
        Ok(config) => config,
        Err(e) => return Err(e),
    };

    let provider = provider::LumenProvider::new(config.provider, config.api_key, config.model)?;
    let command = command::LumenCommand::new(provider);

    let cwd = std::env::current_dir()?;
    let vcs_override = cli.vcs.map(VcsBackendType::from);
    let backend: Box<dyn VcsBackend> = match external_diff_label(&cli.command) {
        Some(label) => Box::new(NoopBackend::new(label)),
        None => vcs::get_backend(&cwd, vcs_override)?,
    };

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
                    Some(CommitReference::RangeToWorkingTree { from }) => {
                        let head_ref = backend.working_copy_parent_ref();
                        let range_diff = backend
                            .get_range_diff(&from, head_ref, false)
                            .unwrap_or_default();
                        let wt_diff = backend.get_working_tree_diff(false).unwrap_or_default();
                        let combined = format!("{}{}", range_diff, wt_diff);
                        GitEntity::Diff(Diff::from_range_diff(
                            combined,
                            from,
                            "working tree".to_string(),
                        )?)
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
            detect_pr,
            file,
            watch,
            theme,
            stacked,
            focus,
            origin,
            wrap,
            stdin,
            files,
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
                stdin,
                files,
            };
            command::diff::run_diff_ui(options, backend.as_ref())?;
        }
        Commands::Configure => {
            command::configure::ConfigureCommand::execute()?;
        }
    }

    Ok(())
}

fn external_diff_label(command: &Commands) -> Option<Option<String>> {
    let Commands::Diff {
        reference,
        stdin,
        files,
        ..
    } = command
    else {
        return None;
    };

    if let Some(paths) = files {
        let label = if paths.len() == 2 {
            Some(format!("{} → {}", paths[0], paths[1]))
        } else {
            None
        };
        return Some(label);
    }

    let from_stdin = *stdin || matches!(reference, Some(CommitReference::Single(s)) if s == "-");
    if from_stdin {
        return Some(Some("stdin".to_string()));
    }

    None
}

fn read_from_stdin() -> Result<String, LumenError> {
    let mut buffer = String::new();
    std::io::stdin().read_to_string(&mut buffer)?;

    eprintln!("Reading commit SHA from stdin: '{}'", buffer.trim());
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff_cmd(stdin: bool, files: Option<Vec<String>>, reference: Option<&str>) -> Commands {
        Commands::Diff {
            reference: reference.map(|s| CommitReference::Single(s.to_string())),
            pr: None,
            detect_pr: false,
            file: None,
            watch: false,
            theme: None,
            stacked: false,
            focus: None,
            origin: None,
            wrap: false,
            stdin,
            files,
        }
    }

    #[test]
    fn test_external_label_stdin() {
        assert_eq!(
            external_diff_label(&diff_cmd(true, None, None)),
            Some(Some("stdin".to_string()))
        );
    }

    #[test]
    fn test_external_label_dash_reference() {
        assert_eq!(
            external_diff_label(&diff_cmd(false, None, Some("-"))),
            Some(Some("stdin".to_string()))
        );
    }

    #[test]
    fn test_external_label_two_files() {
        let files = Some(vec!["old".to_string(), "new".to_string()]);
        assert_eq!(
            external_diff_label(&diff_cmd(false, files, None)),
            Some(Some("old → new".to_string()))
        );
    }

    #[test]
    fn test_external_label_three_files_has_no_label_but_is_external() {
        let files = Some(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(external_diff_label(&diff_cmd(false, files, None)), Some(None));
    }

    #[test]
    fn test_external_label_none_for_regular_diff() {
        assert_eq!(external_diff_label(&diff_cmd(false, None, Some("HEAD"))), None);
        assert_eq!(external_diff_label(&diff_cmd(false, None, None)), None);
    }

    #[test]
    fn test_external_label_none_for_non_diff_command() {
        assert_eq!(external_diff_label(&Commands::List), None);
    }
}
