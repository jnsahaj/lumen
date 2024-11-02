use git_commit::GitCommit;
use git_diff::GitDiff;

pub mod git_commit;
pub mod git_diff;

#[derive(Debug, Clone)]
pub enum GitEntity {
    Commit(GitCommit),
    StagedDiff(GitDiff),
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
            GitEntity::StagedDiff(_) => "Staged Diff\n".into(),
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
            GitEntity::StagedDiff(diff) => diff,
            _ => panic!("Not a Diff"),
        }
    }
}
