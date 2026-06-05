mod annotation;
mod app;
mod context;
mod coordinates;
mod diff_algo;
pub mod git;
mod global_search;
pub mod highlight;
pub mod pr_provider;
mod render;
mod search;
mod state;
mod sticky_lines;
mod text_edit;
pub mod theme;
mod types;
mod watcher;

use std::io;
use std::process;

use spinoff::{spinners, Color, Spinner};

use crate::commit_reference::CommitReference;
use crate::vcs::VcsBackend;

pub use pr_provider::{
    fetch_viewed_files, mark_file_as_viewed_async, unmark_file_as_viewed_async, PrProvider,
};

pub struct DiffOptions {
    pub reference: Option<CommitReference>,
    pub pr: Option<String>,
    pub detect_pr: bool,
    pub file: Option<Vec<String>>,
    pub watch: bool,
    pub theme: Option<String>,
    pub stacked: bool,
    pub focus: Option<String>,
    pub origin: Option<String>,
    pub wrap: bool,
}

#[derive(Clone)]
pub struct PrInfo {
    pub provider: &'static dyn PrProvider,
    pub number: u64,
    pub node_id: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub base_ref: String,
    pub head_ref: String,
    pub base_repo_owner: String,
    pub head_repo_owner: Option<String>, // None if head repo was deleted (fork deleted)
    /// Azure DevOps project (None for GitHub).
    pub project: Option<String>,
    /// Azure DevOps organisation base URL, e.g. `https://dev.azure.com/org`.
    pub org_url: Option<String>,
}

pub fn run_diff_ui(mut options: DiffOptions, backend: &dyn VcsBackend) -> io::Result<()> {
    // Resolve --detect-pr into options.pr
    if options.detect_pr && options.pr.is_none() {
        let mut spinner = Spinner::new(
            spinners::Dots,
            "Detecting PR for current branch",
            Color::Cyan,
        );
        match pr_provider::detect_current_branch_pr(options.origin.as_deref()) {
            Ok(number) => {
                spinner.success(&format!("Detected PR #{}", number));
                options.pr = Some(number);
            }
            Err(e) => {
                spinner.fail(&e);
                process::exit(1);
            }
        }
    }

    // Handle PR mode
    if let Some(ref pr_input) = options.pr {
        let mut spinner = Spinner::new(spinners::Dots, "Fetching PR metadata", Color::Cyan);
        match pr_provider::fetch_pr_info(pr_input, options.origin.as_deref()) {
            Ok(pr_info) => {
                spinner.success("Fetched PR metadata");
                return app::run_app_with_pr(options, pr_info, backend);
            }
            Err(e) => {
                spinner.fail(&e);
                process::exit(1);
            }
        }
    }

    // Also check if the reference looks like a PR (number or URL)
    if let Some(CommitReference::Single(ref input)) = options.reference {
        if pr_provider::is_pr_reference(input) {
            let mut spinner = Spinner::new(spinners::Dots, "Fetching PR metadata", Color::Cyan);
            match pr_provider::fetch_pr_info(input, options.origin.as_deref()) {
                Ok(pr_info) => {
                    spinner.success("Fetched PR metadata");
                    return app::run_app_with_pr(options, pr_info, backend);
                }
                Err(e) => {
                    spinner.fail(&e);
                    process::exit(1);
                }
            }
        }
    }

    // Handle stacked mode for range references
    if options.stacked {
        if let Some(ref reference) = options.reference {
            let (from, to) = match reference {
                CommitReference::Range { from, to } => (from.clone(), to.clone()),
                CommitReference::TripleDots { from, to } => {
                    // Get merge-base for triple dots
                    let merge_base = backend
                        .get_merge_base(from, to)
                        .unwrap_or_else(|_| from.clone());
                    (merge_base, to.clone())
                }
                CommitReference::Single(_) | CommitReference::RangeToWorkingTree { .. } => {
                    eprintln!(
                        "\x1b[91merror:\x1b[0m --stacked requires a range (e.g., main..feature)"
                    );
                    process::exit(1);
                }
            };

            let commits = match backend.get_commits_in_range(&from, &to) {
                Ok(c) if c.is_empty() => {
                    eprintln!(
                        "\x1b[91merror:\x1b[0m No commits found in range {}..{}",
                        from, to
                    );
                    process::exit(1);
                }
                Ok(c) => c,
                Err(e) => {
                    eprintln!("\x1b[91merror:\x1b[0m {}", e);
                    process::exit(1);
                }
            };

            return app::run_app_stacked(options, commits, backend);
        } else {
            eprintln!("\x1b[91merror:\x1b[0m --stacked requires a range (e.g., main..feature)");
            process::exit(1);
        }
    }

    app::run_app(options, None, backend)
}
