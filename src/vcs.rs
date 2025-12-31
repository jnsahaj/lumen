use std::path::Path;
use std::process::{Command, Output};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Vcs {
    Git,
    Jj,
}

impl Vcs {
    /// Detect which VCS is managing the current directory
    pub fn detect() -> Self {
        if Path::new(".jj").exists() {
            Vcs::Jj
        } else {
            Vcs::Git
        }
    }

    /// Get working tree diff
    pub fn diff_working_tree(&self, staged: bool) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                if staged {
                    cmd.args(["diff", "--staged"]);
                } else {
                    cmd.arg("diff");
                }
                cmd
            }
            Vcs::Jj => {
                // jj has no staging area - always shows working copy changes
                let mut cmd = Command::new("jj");
                cmd.args(["diff", "--git"]);
                cmd
            }
        }
    }

    /// Get diff between two revisions
    pub fn diff_range(&self, from: &str, to: &str) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["diff", &format!("{}..{}", from, to)]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                cmd.args(["diff", "--git", "--from", from, "--to", to]);
                cmd
            }
        }
    }

    /// Validate that a revision exists
    pub fn validate_revision(&self, rev: &str) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["cat-file", "-t", rev]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                cmd.args(["log", "-r", rev, "--no-graph", "-T", "change_id", "--limit", "1"]);
                cmd
            }
        }
    }

    /// Check if revision validation output indicates a valid commit
    pub fn is_valid_revision_output(&self, output: &Output) -> bool {
        match self {
            Vcs::Git => {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).trim() == "commit"
            }
            Vcs::Jj => output.status.success(),
        }
    }

    /// Get full commit hash
    pub fn get_full_hash(&self, rev: &str) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["rev-parse", rev]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                cmd.args(["log", "-r", rev, "--no-graph", "-T", "commit_id"]);
                cmd
            }
        }
    }

    /// Get diff for a single commit
    pub fn get_commit_diff(&self, rev: &str) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args([
                    "diff-tree",
                    "-p",
                    "--binary",
                    "--no-color",
                    "--compact-summary",
                    rev,
                ]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                // Show diff between parent and this revision
                cmd.args(["diff", "--git", "-r", rev]);
                cmd
            }
        }
    }

    /// Get commit message
    pub fn get_commit_message(&self, rev: &str) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["log", "--format=%B", "-n", "1", rev]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                cmd.args(["log", "-r", rev, "--no-graph", "-T", "description"]);
                cmd
            }
        }
    }

    /// Get author name
    pub fn get_author_name(&self, rev: &str) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["log", "--format=%an", "-n", "1", rev]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                cmd.args(["log", "-r", rev, "--no-graph", "-T", "author.name()"]);
                cmd
            }
        }
    }

    /// Get author email
    pub fn get_author_email(&self, rev: &str) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["log", "--format=%ae", "-n", "1", rev]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                cmd.args(["log", "-r", rev, "--no-graph", "-T", "author.email()"]);
                cmd
            }
        }
    }

    /// Get commit date
    pub fn get_commit_date(&self, rev: &str) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args([
                    "log",
                    "--format=%cd",
                    "--date=format:%Y-%m-%d %H:%M:%S",
                    "-n",
                    "1",
                    rev,
                ]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                cmd.args([
                    "log",
                    "-r",
                    rev,
                    "--no-graph",
                    "-T",
                    "author.timestamp().format(\"%Y-%m-%d %H:%M:%S\")",
                ]);
                cmd
            }
        }
    }

    /// Get current branch name
    pub fn get_current_branch(&self) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["rev-parse", "--abbrev-ref", "HEAD"]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                // Get bookmarks pointing to current revision
                cmd.args([
                    "log",
                    "-r",
                    "@",
                    "--no-graph",
                    "-T",
                    "bookmarks.join(\", \")",
                ]);
                cmd
            }
        }
    }

    /// Get merge base between two revisions
    pub fn get_merge_base(&self, from: &str, to: &str) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["merge-base", from, to]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                // Get the common ancestor
                cmd.args([
                    "log",
                    "-r",
                    &format!("roots({}::{})", from, to),
                    "--no-graph",
                    "-T",
                    "commit_id",
                    "--limit",
                    "1",
                ]);
                cmd
            }
        }
    }

    /// Get list of changed files for a single commit
    pub fn get_commit_files(&self, rev: &str) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["diff-tree", "--no-commit-id", "--name-only", "-r", rev]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                cmd.args(["diff", "-r", rev, "--summary"]);
                cmd
            }
        }
    }

    /// Get list of changed files between two revisions
    pub fn get_range_files(&self, from: &str, to: &str) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["diff", "--name-only", from, to]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                cmd.args(["diff", "--from", from, "--to", to, "--summary"]);
                cmd
            }
        }
    }

    /// Get unstaged changed files (working tree)
    pub fn get_unstaged_files(&self) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["diff", "--name-only", "HEAD"]);
                cmd
            }
            Vcs::Jj => {
                // jj diff --summary shows all working copy changes
                let mut cmd = Command::new("jj");
                cmd.args(["diff", "--summary"]);
                cmd
            }
        }
    }

    /// Get staged files
    pub fn get_staged_files(&self) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["diff", "--cached", "--name-only"]);
                cmd
            }
            Vcs::Jj => {
                // jj has no staging - return empty command that succeeds
                let mut cmd = Command::new("jj");
                cmd.args(["log", "-r", "none()", "--no-graph"]);
                cmd
            }
        }
    }

    /// Get untracked files
    pub fn get_untracked_files(&self) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["ls-files", "--others", "--exclude-standard"]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                cmd.args(["file", "list", "--untracked"]);
                cmd
            }
        }
    }

    /// Show file content at a specific revision
    pub fn show_file(&self, file: &str, rev: &str) -> Command {
        match self {
            Vcs::Git => {
                let mut cmd = Command::new("git");
                cmd.args(["show", &format!("{}:{}", rev, file)]);
                cmd
            }
            Vcs::Jj => {
                let mut cmd = Command::new("jj");
                cmd.args(["file", "show", file, "-r", rev]);
                cmd
            }
        }
    }

    /// Parse file list from summary output (jj uses different format)
    pub fn parse_file_list(&self, output: &str) -> Vec<String> {
        match self {
            Vcs::Git => output
                .lines()
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
            Vcs::Jj => {
                // jj --summary format: "M path/to/file" or "A path" or "D path"
                output
                    .lines()
                    .filter(|s| !s.is_empty())
                    .filter_map(|line| {
                        let parts: Vec<&str> = line.splitn(2, ' ').collect();
                        if parts.len() == 2 {
                            Some(parts[1].to_string())
                        } else {
                            None
                        }
                    })
                    .collect()
            }
        }
    }

    /// Get the fzf command for selecting commits
    pub fn fzf_log_command(&self) -> &'static str {
        match self {
            Vcs::Git => {
                "git log --color=always --format='%C(auto)%h%d %s %C(black)%C(bold)%cr' | fzf --ansi --reverse --bind='enter:become(echo {1})'"
            }
            Vcs::Jj => {
                "jj log --color=always -T 'change_id.shortest() ++ \" \" ++ bookmarks ++ \" \" ++ description.first_line() ++ \" \" ++ author.timestamp().ago()' | fzf --ansi --reverse --bind='enter:become(echo {1})'"
            }
        }
    }

    /// Check if staging is supported
    pub fn supports_staging(&self) -> bool {
        matches!(self, Vcs::Git)
    }
}
