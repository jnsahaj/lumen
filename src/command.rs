use std::io::Write;
use std::process::Stdio;

use crate::error::LumenError;
use crate::git_entity::git_commit::GitCommit;
use crate::git_entity::git_diff::GitDiff;
use crate::git_entity::GitEntity;
use crate::provider::AIProvider;
use crate::provider::LumenProvider;

use spinoff::{spinners, Color, Spinner};

pub struct LumenCommand {
    provider: LumenProvider,
}

impl LumenCommand {
    pub fn new(provider: LumenProvider) -> Self {
        LumenCommand { provider }
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

    pub async fn explain(&self, git_entity: &GitEntity) -> Result<(), LumenError> {
        Self::print_with_mdcat(git_entity.format_static_details())?;

        let mut spinner = Spinner::new(spinners::Dots, "Generating Summary...", Color::Blue);
        let result = self.provider.explain(git_entity.clone()).await?;
        spinner.success("Done");

        Self::print_with_mdcat(result)?;

        Ok(())
    }

    pub async fn list(&self) -> Result<(), LumenError> {
        let sha = Self::get_sha_from_fzf()?;
        let git_entity = GitEntity::Commit(GitCommit::new(sha)?);
        self.explain(&git_entity).await
    }

    pub async fn draft(&self) -> Result<(), LumenError> {
        let result = self
            .provider
            .draft(GitEntity::Diff(GitDiff::new(true)?))
            .await?;

        // print without newline
        print!("{result}");
        std::io::stdout().flush()?;

        Ok(())
    }
}
