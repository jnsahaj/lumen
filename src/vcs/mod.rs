//! VCS backend abstraction module supporting Git and Jujutsu repositories.

mod backend;
mod detection;
mod git;
#[cfg(feature = "jj")]
mod jj;
mod reference;
#[cfg(test)]
pub mod test_utils;

pub use backend::{StackedCommitInfo, VcsBackend, VcsError};
pub use detection::{detect_vcs_type, vcs_type_at, VcsType};
pub use git::GitBackend;
#[cfg(feature = "jj")]
pub use jj::JjBackend;
pub use reference::{CommitReference, ReferenceParseError};

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcsBackendType {
    Git,
    Jj,
}

/// Get the VCS backend for `path`, preferring Jujutsu in colocated repositories
/// unless `override_type` forces a specific backend.
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
        VcsType::Git => GitBackend::new(path).map(|b| Box::new(b) as Box<dyn VcsBackend>),
        VcsType::Jj => {
            #[cfg(feature = "jj")]
            {
                JjBackend::new(path).map(|b| Box::new(b) as Box<dyn VcsBackend>)
            }
            #[cfg(not(feature = "jj"))]
            {
                eprintln!(
                    "Warning: jj repository detected but jj support not compiled in. Using git backend."
                );
                GitBackend::new(path).map(|b| Box::new(b) as Box<dyn VcsBackend>)
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
        assert_eq!(backend.name(), "git");
    }

    #[test]
    #[cfg(feature = "jj")]
    fn test_get_backend_in_jj_repo() {
        let Some(repo) = JjRepoGuard::new() else {
            return;
        };

        let backend = get_backend(&repo.dir, None).expect("should get backend");
        assert_eq!(backend.name(), "jj");
    }

    #[test]
    #[cfg(feature = "jj")]
    fn test_vcs_override_git_in_jj_repo() {
        let Some(repo) = JjRepoGuard::new() else {
            return;
        };

        git(&repo.dir, &["add", "."]);
        git(&repo.dir, &["commit", "-m", "init"]);

        let backend =
            get_backend(&repo.dir, Some(VcsBackendType::Git)).expect("should get backend");
        assert_eq!(backend.name(), "git");
        backend
            .resolve_ref("HEAD")
            .expect("git backend resolves HEAD");
    }

    #[test]
    fn test_get_backend_in_non_repo_fails() {
        let temp = tempfile::TempDir::new().unwrap();
        let result = get_backend(temp.path(), None);
        assert!(matches!(result, Err(VcsError::NotARepository)));
    }
}
