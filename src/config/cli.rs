use clap::{Parser, Subcommand, ValueEnum, ValueHint};
use std::str::FromStr;

use crate::commit_reference::CommitReference;

/// VCS backend override option
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum, Debug)]
pub enum VcsOverride {
    /// Use git backend
    Git,
    /// Use jj (Jujutsu) backend
    Jj,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum, Debug)]
pub enum CompletionShell {
    #[value(name = "bash")]
    Bash,
    #[value(name = "zsh")]
    Zsh,
    #[value(name = "fish")]
    Fish,
    #[value(name = "powershell")]
    PowerShell,
    #[value(name = "elvish")]
    Elvish,
}

#[derive(Parser)]
#[command(name = "lumen")]
#[command(about = "AI-powered CLI tool for git commit summaries", long_about = None)]
#[command(version)]
pub struct Cli {
    /// Path to configuration file eg: ./path/to/lumen.config.json
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub config: Option<String>,

    #[arg(value_enum, short = 'p', long = "provider")]
    pub provider: Option<ProviderType>,

    #[arg(short = 'k', long = "api-key", value_hint = ValueHint::Other)]
    pub api_key: Option<String>,

    #[arg(short = 'm', long = "model", value_hint = ValueHint::Other)]
    pub model: Option<String>,

    /// Version control system to use (auto-detected if not specified)
    #[arg(value_enum, long = "vcs")]
    pub vcs: Option<VcsOverride>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum, Debug)]
pub enum ProviderType {
    Openai,
    Groq,
    Claude,
    Ollama,
    OpencodeZen,
    Openrouter,
    Deepseek,
    Gemini,
    Xai,
    Vercel,
}

impl FromStr for ProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(ProviderType::Openai),
            "groq" => Ok(ProviderType::Groq),
            "claude" => Ok(ProviderType::Claude),
            "ollama" => Ok(ProviderType::Ollama),
            "opencode-zen" => Ok(ProviderType::OpencodeZen),
            "openrouter" => Ok(ProviderType::Openrouter),
            "deepseek" => Ok(ProviderType::Deepseek),
            "gemini" => Ok(ProviderType::Gemini),
            "xai" => Ok(ProviderType::Xai),
            "vercel" => Ok(ProviderType::Vercel),
            _ => Err(format!("Unknown provider: {}", s)),
        }
    }
}

#[derive(Subcommand)]
pub enum Commands {
    /// Explain the changes in a commit, or the current diff (default). Use --list to select commit interactively
    Explain {
        /// Commit reference: SHA, HEAD, HEAD~3..HEAD, main..feature, main...feature
        #[arg(value_parser = clap::value_parser!(CommitReference), value_hint = ValueHint::Other)]
        reference: Option<CommitReference>,

        /// Use staged diff only (when showing uncommitted changes)
        #[arg(long)]
        staged: bool,

        /// Ask a question instead of summary
        #[arg(short, long, value_hint = ValueHint::Other)]
        query: Option<String>,

        /// Select commit interactively using fuzzy finder
        #[arg(long)]
        list: bool,
    },
    /// List all commits in an interactive fuzzy-finder, and summarize the changes
    List,
    /// Generate a commit message for the staged changes
    Draft {
        /// Add context to communicate intent
        #[arg(short, long, value_hint = ValueHint::Other)]
        context: Option<String>,
    },

    Operate {
        #[arg(value_hint = ValueHint::Other)]
        query: String,
    },
    /// Launch interactive side-by-side diff viewer
    Diff {
        /// Commit reference: SHA, HEAD, HEAD~3..HEAD, main..feature, main...feature
        /// Can also be a PR number or URL (e.g., 123 or https://github.com/owner/repo/pull/123)
        #[arg(value_parser = clap::value_parser!(CommitReference), value_hint = ValueHint::Other)]
        reference: Option<CommitReference>,

        /// View a GitHub pull request (number or URL)
        #[arg(long, value_hint = ValueHint::Url)]
        pr: Option<String>,

        /// Filter to specific files
        #[arg(short, long, value_hint = ValueHint::FilePath)]
        file: Option<Vec<String>>,

        /// Watch for file changes and auto-reload
        #[arg(short, long)]
        watch: bool,

        /// Color theme (e.g., dracula, nord, gruvbox-dark, catppuccin-mocha)
        #[arg(short, long, value_hint = ValueHint::Other)]
        theme: Option<String>,

        /// Show commits stacked (commit-by-commit navigation with ctrl+l/h)
        #[arg(long)]
        stacked: bool,

        /// Initially focus on this file path
        #[arg(long)]
        focus: Option<String>,
    },
    /// Interactively configure Lumen (provider, API key)
    Configure,
    /// Generate shell completions for a given shell
    Completion {
        /// Shell to generate completions for
        #[arg(value_enum, long)]
        shell: CompletionShell,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vcs_git_parses() {
        let cli = Cli::try_parse_from(["lumen", "--vcs", "git", "diff"]).unwrap();
        assert_eq!(cli.vcs, Some(VcsOverride::Git));
    }

    #[test]
    fn test_vcs_jj_parses() {
        let cli = Cli::try_parse_from(["lumen", "--vcs", "jj", "diff"]).unwrap();
        assert_eq!(cli.vcs, Some(VcsOverride::Jj));
    }

    #[test]
    fn test_vcs_not_specified() {
        let cli = Cli::try_parse_from(["lumen", "diff"]).unwrap();
        assert_eq!(cli.vcs, None);
    }
}
