use super::PrInfo;
use crate::vcs::{StackedCommitInfo, VcsBackend};

pub(super) enum AppMode<'a> {
    Local {
        backend: &'a dyn VcsBackend,
    },
    PullRequest {
        pr: Box<PrInfo>,
    },
    Stacked {
        backend: &'a dyn VcsBackend,
        initial_commits: Vec<StackedCommitInfo>,
    },
}

impl<'a> AppMode<'a> {
    pub fn pr(&self) -> Option<&PrInfo> {
        match self {
            Self::PullRequest { pr } => Some(pr.as_ref()),
            Self::Local { .. } | Self::Stacked { .. } => None,
        }
    }

    pub fn backend(&self) -> Option<&'a dyn VcsBackend> {
        match self {
            Self::Local { backend } | Self::Stacked { backend, .. } => Some(*backend),
            Self::PullRequest { .. } => None,
        }
    }

    pub fn stacked_backend(&self) -> Option<&'a dyn VcsBackend> {
        match self {
            Self::Stacked { backend, .. } => Some(*backend),
            Self::Local { .. } | Self::PullRequest { .. } => None,
        }
    }

    pub fn is_pull_request(&self) -> bool {
        matches!(self, Self::PullRequest { .. })
    }

    pub fn take_initial_commits(&mut self) -> Option<Vec<StackedCommitInfo>> {
        match self {
            Self::Stacked {
                initial_commits, ..
            } => Some(std::mem::take(initial_commits)),
            Self::Local { .. } | Self::PullRequest { .. } => None,
        }
    }
}
