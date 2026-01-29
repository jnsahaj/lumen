use draft::DraftCommand;
use explain::ExplainCommand;
use list::ListCommand;
use operate::OperateCommand;
use std::process::Stdio;

use crate::config::configuration::DraftConfig;
use crate::error::LumenError;
use crate::git_entity::GitEntity;
use crate::provider::LumenProvider;
use crate::vcs::VcsBackend;

pub mod completion;
pub mod configure;
pub mod diff;
pub mod draft;
pub mod explain;
pub mod list;
pub mod operate;

pub enum CommandType<'a> {
    Explain {
        git_entity: GitEntity,
        query: Option<String>,
    },
    List {
        backend: &'a dyn VcsBackend,
    },
    Draft {
        git_entity: GitEntity,
        context: Option<String>,
        draft_config: DraftConfig,
    },
    Operate {
        query: String,
    },
}

pub struct LumenCommand {
    provider: LumenProvider,
}

impl LumenCommand {
    pub fn new(provider: LumenProvider) -> Self {
        LumenCommand { provider }
    }

    pub async fn execute(&self, command_type: CommandType<'_>) -> Result<(), LumenError> {
        match command_type {
            CommandType::Explain { git_entity, query } => {
                ExplainCommand { git_entity, query }
                    .execute(&self.provider)
                    .await
            }
            CommandType::List { backend } => ListCommand.execute(&self.provider, backend).await,
            CommandType::Draft {
                git_entity,
                context,
                draft_config,
            } => {
                DraftCommand {
                    git_entity,
                    draft_config,
                    context,
                }
                .execute(&self.provider)
                .await
            }
            CommandType::Operate { query } => {
                OperateCommand { query }.execute(&self.provider).await
            }
        }
    }

    pub(crate) fn get_sha_from_fzf(backend: &dyn VcsBackend) -> Result<String, LumenError> {
        // Get commit log from backend (supports both git and jj)
        let log = backend.get_commit_log_for_fzf()?;

        // Pipe to fzf for selection
        let mut fzf = std::process::Command::new("fzf")
            .args(["--ansi", "--reverse", "--bind=enter:become(echo {1})"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        // Write log to fzf stdin
        if let Some(mut stdin) = fzf.stdin.take() {
            use std::io::Write;
            stdin.write_all(log.as_bytes())?;
        }

        let output = fzf.wait_with_output()?;

        if !output.status.success() {
            let mut stderr = String::from_utf8(output.stderr)?;
            stderr.pop();

            let hint = match &stderr {
                stderr if stderr.contains("fzf: command not found") => {
                    Some("`list` command requires fzf")
                }
                _ => None,
            };

            let hint = match hint {
                Some(hint) => format!("(hint: {})", hint),
                None => String::new(),
            };

            return Err(LumenError::CommandError(format!("{} {}", stderr, hint)));
        }

        let mut sha = String::from_utf8(output.stdout)?;
        sha.pop(); // remove trailing newline from echo

        Ok(sha)
    }

    fn print_with_mdcat(content: String) -> Result<(), LumenError> {
        match std::process::Command::new("mdcat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
        {
            Ok(mut mdcat) => {
                if let Some(stdin) = mdcat.stdin.take() {
                    std::process::Command::new("echo")
                        .arg(&content)
                        .stdout(stdin)
                        .spawn()?
                        .wait()?;
                }
                let output = mdcat.wait_with_output()?;
                println!("{}", String::from_utf8(output.stdout)?);
            }
            Err(_) => {
                println!("{}", content);
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn execute_bash_command(command: &str) -> Result<(), LumenError> {
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()?;

        if !output.status.success() {
            let mut stderr = String::from_utf8(output.stderr)?;
            stderr.pop();
            return Err(LumenError::CommandError(stderr));
        }
        println!("{}", String::from_utf8(output.stdout)?);
        Ok(())
    }

    #[allow(dead_code)]
    fn execute_bash_command_with_confirmation(command: &str) -> Result<(), LumenError> {
        let mut input = String::new();
        println!("{} (y/N)", command);
        std::io::stdin().read_line(&mut input)?;
        if input.trim().to_lowercase() != "y" {
            return Err(LumenError::CommandError("Aborted".to_string()));
        }
        LumenCommand::execute_bash_command(command)
    }
}
