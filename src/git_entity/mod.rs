use commit::Commit;
use diff::Diff;
use indoc::formatdoc;

use crate::provider::LumenProvider;

pub mod commit;
pub mod diff;

#[derive(Debug, Clone)]
pub enum GitEntity {
    Commit(Commit),
    Diff(Diff),
}

impl GitEntity {
    pub fn format_static_details(&self, provider: &LumenProvider) -> String {
        match self {
            GitEntity::Commit(commit) => formatdoc! {"
                # Entity: Commit
                # Provider: {provider}
                `commit {hash}` | {author} <{email}> | {date}

                {message}
                -----",
                hash = commit.full_hash,
                author = commit.author_name,
                email = commit.author_email,
                date = commit.date,
                message = commit.message,
                provider = provider
            },
            GitEntity::Diff(Diff::WorkingTree { staged, .. }) => formatdoc! {"
                # Entity: Working Tree Diff{staged}
                # Provider: {provider}",
                staged = if *staged { " (staged)" } else { "" }
            },
            GitEntity::Diff(Diff::CommitsRange { from, to, .. }) => formatdoc! {"
                # Entity: Range
                `{from}` -> `{to}`
                # Provider: {provider}
            "},
        }
    }
}

impl AsRef<Commit> for GitEntity {
    fn as_ref(&self) -> &Commit {
        match self {
            GitEntity::Commit(commit) => commit,
            _ => panic!("Not a Commit"),
        }
    }
}

impl AsRef<Diff> for GitEntity {
    fn as_ref(&self) -> &Diff {
        match self {
            GitEntity::Diff(diff) => diff,
            _ => panic!("Not a Diff"),
        }
    }
}
