use clap::CommandFactory;
use clap_complete::{generate, Shell};
use std::io;

use crate::config::cli::{Cli, CompletionShell};

pub fn generate_completions(shell: CompletionShell) {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    let shell = match shell {
        CompletionShell::Bash => Shell::Bash,
        CompletionShell::Zsh => Shell::Zsh,
        CompletionShell::Fish => Shell::Fish,
        CompletionShell::PowerShell => Shell::PowerShell,
        CompletionShell::Elvish => Shell::Elvish,
    };
    let mut out = io::stdout();
    generate(shell, &mut cmd, name, &mut out);
}
