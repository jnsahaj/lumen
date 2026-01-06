use std::path::Path;

/// Type of version control system detected in a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcsType {
    /// Git repository (has .git/)
    Git,
    /// Jujutsu repository (has .jj/), includes colocated repos
    Jj,
    /// No VCS detected
    None,
}

/// Detect the VCS type for a directory.
///
/// Prefers Jj when both .jj/ and .git/ are present (colocated repo).
/// Walks up the directory tree to find the repo root.
pub fn detect_vcs_type(start_dir: &Path) -> VcsType {
    let mut current = start_dir;

    loop {
        // Check for .jj/ first (prefer jj in colocated repos)
        if current.join(".jj").is_dir() {
            return VcsType::Jj;
        }

        // Check for .git/
        if current.join(".git").exists() {
            return VcsType::Git;
        }

        // Walk up to parent
        match current.parent() {
            Some(parent) => current = parent,
            None => return VcsType::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_detect_git_repo() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();

        assert_eq!(detect_vcs_type(temp.path()), VcsType::Git);
    }

    #[test]
    fn test_detect_jj_repo() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".jj")).unwrap();

        assert_eq!(detect_vcs_type(temp.path()), VcsType::Jj);
    }

    #[test]
    fn test_detect_colocated_prefers_jj() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        fs::create_dir(temp.path().join(".jj")).unwrap();

        assert_eq!(detect_vcs_type(temp.path()), VcsType::Jj);
    }

    #[test]
    fn test_detect_no_vcs() {
        let temp = TempDir::new().unwrap();

        assert_eq!(detect_vcs_type(temp.path()), VcsType::None);
    }

    #[test]
    fn test_detect_from_subdirectory() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        let subdir = temp.path().join("src").join("nested");
        fs::create_dir_all(&subdir).unwrap();

        assert_eq!(detect_vcs_type(&subdir), VcsType::Git);
    }
}
