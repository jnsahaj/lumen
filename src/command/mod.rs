use async_trait::async_trait;
use draft::DraftCommand;
use explain::ExplainCommand;
use list::ListCommand;
use std::process::Stdio;

use crate::error::LumenError;
use crate::git_entity::git_diff::GitDiff;
use crate::git_entity::GitEntity;
use crate::provider::LumenProvider;

pub mod draft;
pub mod explain;
pub mod list;

#[derive(Debug)]
pub enum CommandType {
    Explain(GitEntity),
    List,
    Draft(Option<String>),
}

#[async_trait]
pub trait Command {
    async fn execute(&self, provider: &LumenProvider) -> Result<(), LumenError>;
}

impl CommandType {
    pub fn create_command(self) -> Result<Box<dyn Command>, LumenError> {
        Ok(match self {
            CommandType::Explain(git_entity) => Box::new(ExplainCommand { git_entity }),
            CommandType::List => Box::new(ListCommand),
            CommandType::Draft(context) => Box::new(DraftCommand {
                context,
                git_entity: GitEntity::Diff(GitDiff::new(true)?),
            }),
        })
    }
}

pub struct LumenCommand {
    provider: LumenProvider,
}

impl LumenCommand {
    pub fn new(provider: LumenProvider) -> Self {
        LumenCommand { provider }
    }

    pub async fn execute(&self, command_type: CommandType) -> Result<(), LumenError> {
        command_type.create_command()?.execute(&self.provider).await
    }

    fn get_sha_from_fzf() -> Result<String, LumenError> {
        let command = "git log --color=always --format='%C(auto)%h%d %s %C(black)%C(bold)%cr' | fzf --ansi --reverse --bind='enter:become(echo {1})' --wrap";

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

            return Err(LumenError::CommandError(
                format!("{} {}", stderr, hint).into(),
            ));
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
}
