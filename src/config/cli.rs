use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::commit_reference::CommitReference;

/// VCS backend override option
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum, Debug)]
pub enum VcsOverride {
    /// Use git backend
    Git,
    /// Use jj (Jujutsu) backend
    Jj,
}

/// How much of each changed file the diff shows
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    /// The changes, with a few lines around each
    Hunks,
    /// Every line of the file
    Full,
}

#[derive(Parser)]
#[command(name = "lumiere")]
#[command(about = "Review diffs in the terminal and discuss them with your coding agent", long_about = None)]
#[command(version)]
pub struct Cli {
    /// Path to configuration file eg: ./path/to/lumiere.config.json
    #[arg(long)]
    pub config: Option<String>,

    /// Version control system to use (auto-detected if not specified)
    #[arg(value_enum, long = "vcs")]
    pub vcs: Option<VcsOverride>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Launch interactive side-by-side diff viewer
    Diff {
        /// Commit reference: SHA, HEAD, HEAD~3..HEAD, main..feature, main...feature
        /// Can also be a PR number or URL (e.g., 123 or https://github.com/owner/repo/pull/123)
        #[arg(value_parser = clap::value_parser!(CommitReference))]
        reference: Option<CommitReference>,

        /// View a GitHub pull request (number or URL)
        #[arg(long)]
        pr: Option<String>,

        /// Detect the PR associated with the current branch and view it
        #[arg(long = "detect-pr", conflicts_with = "pr")]
        detect_pr: bool,

        /// Filter to specific files
        #[arg(short, long)]
        file: Option<Vec<String>>,

        /// Watch for file changes and auto-reload
        #[arg(short, long)]
        watch: bool,

        /// Color theme (e.g., dracula, nord, gruvbox-dark, catppuccin-mocha)
        #[arg(short, long)]
        theme: Option<String>,

        /// Show commits stacked (commit-by-commit navigation with ctrl+l/h)
        #[arg(long)]
        stacked: bool,

        /// Initially focus on this file path
        #[arg(long)]
        focus: Option<String>,

        /// Origin repository in owner/repo format (default: origin git remote)
        #[arg(long)]
        origin: Option<String>,

        /// Soft-wrap long diff lines instead of scrolling horizontally
        #[arg(long)]
        wrap: bool,

        /// What to show of each file; `z` switches in the review [default: hunks]
        #[arg(value_enum, long)]
        view: Option<ViewMode>,

        /// Rex pane of the coding agent that answers review comments
        /// (default: the latest active pi pane at this repository or folder)
        #[arg(long = "agent-block", value_name = "BLOCK")]
        agent_block: Option<String>,
    },
    /// Read and answer the review threads written in `lumiere diff`
    Thread {
        #[command(subcommand)]
        action: ThreadAction,
    },
}

#[derive(Subcommand)]
pub enum ThreadAction {
    /// Show review threads with their comments
    List {
        /// Repository, or folder of repositories (default: current directory)
        #[arg(long, value_name = "PATH")]
        repo: Option<PathBuf>,

        /// Print JSON instead of text
        #[arg(long)]
        json: bool,

        /// Only threads whose last comment is the reviewer's and that are not resolved
        #[arg(long)]
        awaiting: bool,
    },
    /// Show one review thread with its code and comments
    Show {
        /// Thread id, as shown by `lumiere thread list`
        thread_id: String,

        /// Repository, or folder of repositories (default: current directory)
        #[arg(long, value_name = "PATH")]
        repo: Option<PathBuf>,

        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
    },
    /// Add a reply to a thread
    Reply {
        /// Thread id, as shown by `lumiere thread list`
        thread_id: String,

        /// Text of the reply
        #[arg(long, value_name = "TEXT", conflicts_with = "stdin", required_unless_present = "stdin")]
        body: Option<String>,

        /// Read the reply from standard input
        #[arg(long)]
        stdin: bool,

        /// Repository, or folder of repositories (default: current directory)
        #[arg(long, value_name = "PATH")]
        repo: Option<PathBuf>,

        /// Name shown as the author of the reply
        #[arg(long, value_name = "NAME", default_value = "agent")]
        author: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vcs_git_parses() {
        let cli = Cli::try_parse_from(["lumiere", "--vcs", "git", "diff"]).unwrap();
        assert_eq!(cli.vcs, Some(VcsOverride::Git));
    }

    #[test]
    fn test_vcs_jj_parses() {
        let cli = Cli::try_parse_from(["lumiere", "--vcs", "jj", "diff"]).unwrap();
        assert_eq!(cli.vcs, Some(VcsOverride::Jj));
    }

    #[test]
    fn test_vcs_not_specified() {
        let cli = Cli::try_parse_from(["lumiere", "diff"]).unwrap();
        assert_eq!(cli.vcs, None);
    }

    #[test]
    fn test_thread_reply_needs_a_body_or_stdin_but_not_both() {
        let parse = |args: &[&str]| Cli::try_parse_from([&["lumiere", "thread", "reply", "1a2b"], args].concat());

        assert!(parse(&[]).is_err());
        assert!(parse(&["--body", "text", "--stdin"]).is_err());
        assert!(parse(&["--stdin"]).is_ok());
        assert!(parse(&["--body", "text"]).is_ok());
    }

    #[test]
    fn test_diff_view_takes_hunks_or_full() {
        let parse = |args: &[&str]| Cli::try_parse_from([&["lumiere", "diff"], args].concat());
        let view_of = |args: &[&str]| match parse(args).expect("arguments parse").command {
            Commands::Diff { view, .. } => view,
            _ => panic!("expected diff command"),
        };

        assert_eq!(view_of(&[]), None);
        assert_eq!(view_of(&["--view", "full"]), Some(ViewMode::Full));
        assert_eq!(view_of(&["--view", "hunks"]), Some(ViewMode::Hunks));
        assert!(parse(&["--view", "unified"]).is_err());
    }

    #[test]
    fn test_diff_wrap_flag_parses() {
        let cli = Cli::try_parse_from(["lumiere", "diff", "--wrap"]).unwrap();
        match cli.command {
            Commands::Diff { wrap, .. } => assert!(wrap),
            _ => panic!("expected diff command"),
        }
    }
}
