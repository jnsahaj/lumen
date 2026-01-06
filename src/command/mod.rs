use draft::DraftCommand;
use explain::ExplainCommand;
use list::ListCommand;
use operate::OperateCommand;
use std::process::Stdio;

use crate::config::configuration::DraftConfig;
use crate::error::LumenError;
use crate::git_entity::diff::Diff;
use crate::git_entity::GitEntity;
use crate::provider::LumenProvider;

pub mod configure;
pub mod diff;
pub mod draft;
pub mod explain;
pub mod list;
pub mod operate;

#[derive(Debug)]
pub enum CommandType {
    Explain {
        git_entity: GitEntity,
        query: Option<String>,
    },
    List,
    Draft(Option<String>, DraftConfig),
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

    pub async fn execute(&self, command_type: CommandType) -> Result<(), LumenError> {
        match command_type {
            CommandType::Explain { git_entity, query } => {
                ExplainCommand { git_entity, query }
                    .execute(&self.provider)
                    .await
            }
            CommandType::List => ListCommand.execute(&self.provider).await,
            CommandType::Draft(context, draft_config) => {
                DraftCommand {
                    git_entity: GitEntity::Diff(Diff::from_working_tree(true)?),
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

    pub(crate) fn get_sha_from_fzf() -> Result<String, LumenError> {
        let command = "git log --color=always --format='%C(auto)%h%d %s %C(black)%C(bold)%cr' | fzf --ansi --reverse --bind='enter:become(echo {1})'";

        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

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
