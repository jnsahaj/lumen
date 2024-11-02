use git_commit::GitCommit;

pub mod git_commit;

#[derive(Debug, Clone)]
pub enum GitEntity {
    Commit(GitCommit),
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
        }
    }
}

impl AsRef<GitCommit> for GitEntity {
    fn as_ref(&self) -> &GitCommit {
        match self {
            GitEntity::Commit(commit) => commit,
        }
    }
}
