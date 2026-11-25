//! Workspace discovery: finding immediate child repositories under a folder that is
//! not itself a repository.

use std::path::{Path, PathBuf};

use crate::diff::{get_changed_files, validate_reference, DiffOptions};
use crate::vcs::{self, VcsBackend, VcsBackendType, VcsType};

pub struct WorkspaceRepository {
    pub name: String,
    pub path: PathBuf,
    pub backend: Result<Box<dyn VcsBackend>, String>,
}

pub struct Workspace {
    repositories: Vec<WorkspaceRepository>,
}

impl Workspace {
    /// Immediate child directories of `cwd` that are repository roots, sorted by name.
    /// Symlinks are ignored. Returns `None` if `cwd` is inside a repository or has none.
    pub fn discover(cwd: &Path, vcs_override: Option<VcsBackendType>) -> Option<Self> {
        let root = cwd.canonicalize().ok()?;
        if vcs::detect_vcs_type(&root) != VcsType::None {
            return None;
        }

        let mut repository_paths: Vec<(String, PathBuf)> = std::fs::read_dir(&root)
            .ok()?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|ft| ft.is_dir()))
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                )
            })
            .filter(|(_, path)| vcs::vcs_type_at(path).is_some())
            .collect();
        if repository_paths.is_empty() {
            return None;
        }
        repository_paths.sort();

        let repositories = repository_paths
            .into_iter()
            .map(|(name, path)| WorkspaceRepository {
                backend: vcs::get_backend(&path, vcs_override).map_err(|e| e.to_string()),
                name,
                path,
            })
            .collect();
        Some(Self { repositories })
    }

    pub fn repositories(&self) -> &[WorkspaceRepository] {
        &self.repositories
    }

    pub fn repository_roots(&self) -> Vec<(String, PathBuf)> {
        self.repositories
            .iter()
            .map(|repo| {
                let root = match &repo.backend {
                    Ok(backend) => backend.working_tree_root().to_path_buf(),
                    Err(_) => repo.path.clone(),
                };
                (repo.name.clone(), root)
            })
            .collect()
    }

    pub fn backend(&self, index: usize) -> Result<&dyn VcsBackend, String> {
        let repository = &self.repositories[index];
        repository
            .backend
            .as_deref()
            .map_err(|error| format!("{}: {error}", repository.name))
    }

    pub fn reviewable_backend(
        &self,
        index: usize,
        options: &DiffOptions,
    ) -> Result<&dyn VcsBackend, String> {
        let backend = self.backend(index)?;
        validate_reference(options.reference.as_ref(), backend)
            .map_err(|error| format!("{}: {error}", self.repositories[index].name))?;
        Ok(backend)
    }

    pub fn initial_repository(&self, options: &DiffOptions) -> Result<usize, String> {
        let mut first_reviewable = None;
        let mut errors = Vec::new();
        for index in 0..self.repositories.len() {
            match self.reviewable_backend(index, options) {
                Ok(backend) => {
                    if !get_changed_files(options, backend).is_empty() {
                        return Ok(index);
                    }
                    first_reviewable.get_or_insert(index);
                }
                Err(error) => errors.push(format!("  {error}")),
            }
        }
        first_reviewable.ok_or_else(|| {
            format!(
                "no repository in this folder can be reviewed:\n{}",
                errors.join("\n")
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::vcs::test_utils::git;
    use crate::vcs::CommitReference;

    fn options_for(reference: Option<CommitReference>) -> DiffOptions {
        DiffOptions {
            reference,
            hunks_only: true,
            ..DiffOptions::default()
        }
    }

    fn create_repository(workspace_root: &Path, name: &str) -> PathBuf {
        let dir = workspace_root.join(name);
        fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);
        fs::write(dir.join("README.md"), "hello\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "init"]);
        fs::write(dir.join(format!("{name}.txt")), "uncommitted\n").unwrap();
        dir
    }

    fn repository_names(workspace: &Workspace) -> Vec<&str> {
        workspace
            .repositories()
            .iter()
            .map(|r| r.name.as_str())
            .collect()
    }

    #[test]
    fn test_discover_lists_immediate_child_repositories_sorted_and_skips_plain_or_nested() {
        let root = TempDir::new().unwrap();
        create_repository(root.path(), "web");
        create_repository(root.path(), "api");
        fs::create_dir_all(root.path().join("notes")).unwrap();
        create_repository(&root.path().join("group"), "nested");

        let workspace = Workspace::discover(root.path(), None).expect("workspace");
        assert_eq!(repository_names(&workspace), vec!["api", "web"]);

        assert!(Workspace::discover(&root.path().join("notes"), None).is_none());
        assert!(Workspace::discover(&root.path().join("api"), None).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn test_discover_ignores_symlinks_to_repositories() {
        let root = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        create_repository(root.path(), "api");
        let target = create_repository(elsewhere.path(), "linked");
        std::os::unix::fs::symlink(&target, root.path().join("linked")).unwrap();

        let workspace = Workspace::discover(root.path(), None).expect("workspace");
        assert_eq!(repository_names(&workspace), vec!["api"]);
    }

    #[test]
    fn test_initial_repository_skips_clean_and_missing_revision_repos() {
        let root = TempDir::new().unwrap();
        let clean = create_repository(root.path(), "api");
        fs::remove_file(clean.join("api.txt")).unwrap();
        let web = create_repository(root.path(), "web");
        git(&web, &["checkout", "-b", "only-in-web"]);

        let workspace = Workspace::discover(root.path(), None).expect("workspace");
        assert_eq!(workspace.initial_repository(&options_for(None)), Ok(1));

        let from_rev = |rev: &str| {
            options_for(Some(CommitReference::MergeBaseToWorkingTree {
                from: rev.to_string(),
            }))
        };
        assert_eq!(
            workspace.initial_repository(&from_rev("only-in-web")),
            Ok(1)
        );
        let err = workspace
            .initial_repository(&from_rev("origin/main"))
            .unwrap_err();
        assert!(err.contains("api: invalid reference: origin/main"));
        assert!(err.contains("web: invalid reference: origin/main"));
    }
}
