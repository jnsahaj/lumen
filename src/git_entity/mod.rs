use git_commit::GitCommit;
use git_diff::GitDiff;

pub mod git_commit;
pub mod git_diff;

#[derive(Debug, Clone)]
pub enum GitEntity {
    Commit(GitCommit),
    Diff(GitDiff),
}

impl GitEntity {
    pub fn format_static_details(&self) -> String {
        match self {
            GitEntity::Commit(commit) => format!(
                "`commit {}` | {} <{}> | {}\n\n{}\n-----\n",
                commit.full_hash,
                commit.author_name,
                commit.author_email,
                commit.date,
                commit.message,
            ),
            GitEntity::Diff(diff) => {
                format!("Diff{}\n", if diff.staged { " (staged)" } else { "" })
            }
        }
    }
}

impl AsRef<GitCommit> for GitEntity {
    fn as_ref(&self) -> &GitCommit {
        match self {
            GitEntity::Commit(commit) => commit,
            _ => panic!("Not a Commit"),
        }
    }
}

impl AsRef<GitDiff> for GitEntity {
    fn as_ref(&self) -> &GitDiff {
        match self {
            GitEntity::Diff(diff) => diff,
            _ => panic!("Not a Diff"),
        }
    }
}
