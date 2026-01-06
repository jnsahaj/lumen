//! VCS backend abstraction module.
//!
//! Provides a unified interface for working with git and jj repositories.

mod backend;
mod detection;
mod git;
#[cfg(feature = "jj")]
mod jj;
#[cfg(test)]
pub mod test_utils;

pub use backend::{CommitInfo, VcsBackend, VcsError};
pub use detection::{detect_vcs_type, VcsType};
pub use git::GitBackend;
#[cfg(feature = "jj")]
pub use jj::JjBackend;

use std::path::Path;

use crate::config::cli::VcsOverride;

/// VCS backend type for explicit selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcsBackendType {
    Git,
    Jj,
}

impl From<VcsOverride> for VcsBackendType {
    fn from(vcs: VcsOverride) -> Self {
        match vcs {
            VcsOverride::Git => VcsBackendType::Git,
            VcsOverride::Jj => VcsBackendType::Jj,
        }
    }
}

/// Get the appropriate VCS backend for the current directory.
///
/// If `override_type` is provided, uses that backend type explicitly.
/// Otherwise auto-detects jj vs git repositories. Prefers jj when both are present (colocated).
pub fn get_backend(
    path: &Path,
    override_type: Option<VcsBackendType>,
) -> Result<Box<dyn VcsBackend>, VcsError> {
    let vcs_type = override_type.map_or_else(
        || detect_vcs_type(path),
        |ot| match ot {
            VcsBackendType::Git => VcsType::Git,
            VcsBackendType::Jj => VcsType::Jj,
        },
    );

    match vcs_type {
        VcsType::Git => Ok(Box::new(GitBackend::new())),
        VcsType::Jj => {
            #[cfg(feature = "jj")]
            {
                JjBackend::new(path).map(|b| Box::new(b) as Box<dyn VcsBackend>)
            }
            #[cfg(not(feature = "jj"))]
            {
                // jj feature not enabled, fall back to git for colocated repos
                eprintln!("Warning: jj repository detected but jj support not compiled in. Using git backend.");
                Ok(Box::new(GitBackend::new()))
            }
        }
        VcsType::None => Err(VcsError::NotARepository),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_utils::RepoGuard;
    #[cfg(feature = "jj")]
    use test_utils::{git, JjRepoGuard};

    #[test]
    fn test_get_backend_in_git_repo() {
        let repo = RepoGuard::new();
        let backend = get_backend(&repo.dir, None).expect("should get backend");
        let commit = backend.get_commit("HEAD").expect("should get commit");
        assert!(!commit.commit_id.is_empty());
    }

    #[test]
    fn test_get_backend_in_non_repo_fails() {
        let temp = tempfile::TempDir::new().unwrap();
        let result = get_backend(temp.path(), None);
        assert!(matches!(result, Err(VcsError::NotARepository)));
    }

    #[test]
    #[cfg(feature = "jj")]
    fn test_get_backend_in_jj_repo() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = get_backend(&repo.dir, None).expect("should get backend");
        let commit = backend.get_commit("@").expect("should get commit");
        assert!(!commit.commit_id.is_empty());
    }

    #[test]
    #[cfg(feature = "jj")]
    fn test_vcs_override_git_in_jj_repo() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        // Also need a git commit for git backend to work
        git(&repo.dir, &["add", "."]);
        git(&repo.dir, &["commit", "-m", "init"]);

        // Override to git backend in a colocated repo
        let backend =
            get_backend(&repo.dir, Some(VcsBackendType::Git)).expect("should get backend");
        // Git backend uses HEAD, not @
        let commit = backend.get_commit("HEAD").expect("should get commit");
        assert!(!commit.commit_id.is_empty());
    }

    #[test]
    fn test_vcs_override_to_backend_type_conversion() {
        assert_eq!(VcsBackendType::from(VcsOverride::Git), VcsBackendType::Git);
        assert_eq!(VcsBackendType::from(VcsOverride::Jj), VcsBackendType::Jj);
    }
}
