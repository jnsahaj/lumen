//! Reviewing several repositories from a folder that is not itself a repository.

use std::path::{Path, PathBuf};

use super::app::{new_review_state, open_thread_store};
use super::git::{get_changed_files, load_file_diffs, validate_reference};
use super::state::AppState;
use super::DiffOptions;
use crate::vcs::{self, VcsBackend, VcsBackendType, VcsType};

struct WorkspaceRepository {
    name: String,
    path: PathBuf,
    backend: Result<Box<dyn VcsBackend>, String>,
}

pub struct Workspace {
    repositories: Vec<WorkspaceRepository>,
}

impl Workspace {
    /// Repositories that are immediate child directories of `cwd`, sorted by name.
    /// Symlinks are not followed. Returns `None` when `cwd` is inside a repository
    /// or has no repository directly below it.
    pub fn discover(cwd: &Path, vcs_override: Option<VcsBackendType>) -> Option<Self> {
        let root = cwd.canonicalize().ok()?;
        if vcs::detect_vcs_type(&root) != VcsType::None {
            return None;
        }

        let mut repository_paths: Vec<(String, PathBuf)> = std::fs::read_dir(&root)
            .ok()?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|file_type| file_type.is_dir()))
            .map(|entry| (entry.file_name().to_string_lossy().into_owned(), entry.path()))
            .filter(|(_, path)| vcs::vcs_type_at(path).is_some())
            .collect();
        if repository_paths.is_empty() {
            return None;
        }
        repository_paths.sort();

        let repositories = repository_paths
            .into_iter()
            .map(|(name, path)| WorkspaceRepository {
                backend: vcs::get_backend(&path, vcs_override).map_err(|error| error.to_string()),
                name,
                path,
            })
            .collect();
        Some(Self { repositories })
    }

    /// Name and working tree root of every repository.
    pub fn repository_roots(&self) -> Vec<(String, PathBuf)> {
        self.repositories
            .iter()
            .map(|repository| {
                let root = match &repository.backend {
                    Ok(backend) => backend.working_tree_root().to_path_buf(),
                    Err(_) => repository.path.clone(),
                };
                (repository.name.clone(), root)
            })
            .collect()
    }

    pub fn backend(&self, index: usize) -> Result<&dyn VcsBackend, String> {
        let repository = &self.repositories[index];
        repository
            .backend
            .as_deref()
            .map_err(|error| format!("{}: {}", repository.name, error))
    }

    /// The backend of a repository in which the requested revisions exist.
    fn reviewable_backend(
        &self,
        index: usize,
        options: &DiffOptions,
    ) -> Result<&dyn VcsBackend, String> {
        let backend = self.backend(index)?;
        validate_reference(options.reference.as_ref(), backend)
            .map_err(|error| format!("{}: {}", self.repositories[index].name, error))?;
        Ok(backend)
    }

    /// The first repository with changes, otherwise the first one that can be reviewed.
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
                Err(error) => errors.push(format!("  {}", error)),
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

/// Which repository of a workspace is on screen, and the review state of the others.
pub struct WorkspaceSession<'a> {
    workspace: &'a Workspace,
    active: usize,
    parked_states: Vec<Option<AppState>>,
}

impl<'a> WorkspaceSession<'a> {
    pub fn new(workspace: &'a Workspace, active: usize) -> Self {
        let parked_states = workspace.repositories.iter().map(|_| None).collect();
        Self {
            workspace,
            active,
            parked_states,
        }
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn active_name(&self) -> &str {
        &self.workspace.repositories[self.active].name
    }

    fn state_of<'s>(&'s self, index: usize, active_state: &'s AppState) -> Option<&'s AppState> {
        if index == self.active {
            Some(active_state)
        } else {
            self.parked_states[index].as_ref()
        }
    }

    /// Comments not sent yet, across every repository opened in this session.
    pub fn queued_count(&self, active_state: &AppState) -> usize {
        (0..self.parked_states.len())
            .filter_map(|index| self.state_of(index, active_state))
            .map(AppState::queued_count)
            .sum()
    }

    /// The queued comments as text for the agent, grouped by repository: one comment
    /// of the repository on screen, or all of them across the repositories opened.
    pub fn queued_text_for_agent(
        &self,
        active_state: &AppState,
        only: Option<(&str, &str)>,
    ) -> String {
        self.workspace
            .repositories
            .iter()
            .enumerate()
            .filter(|(index, _)| only.is_none() || *index == self.active)
            .filter_map(|(index, repository)| {
                let text = self.state_of(index, active_state)?.queued_text_for_agent(only);
                (!text.is_empty())
                    .then(|| repository_section(&repository.name, &repository.path, &text))
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Marks the queued comments of the repositories that are off screen as sent.
    /// Returns how many there were.
    pub fn mark_parked_sent(&mut self) -> usize {
        self.parked_states
            .iter_mut()
            .flatten()
            .map(|parked_state| parked_state.mark_sent(None))
            .sum()
    }

    pub fn repository_roots(&self) -> Vec<PathBuf> {
        self.workspace
            .repository_roots()
            .into_iter()
            .map(|(_, root)| root)
            .collect()
    }

    pub fn picker_items(&self, active_state: &AppState) -> Vec<String> {
        self.workspace
            .repositories
            .iter()
            .enumerate()
            .map(|(index, repository)| {
                let mut item = repository.name.clone();
                if index == self.active {
                    item.push_str(" · current");
                }
                if repository.backend.is_err() {
                    item.push_str(" · unavailable");
                }
                let thread_count = self
                    .state_of(index, active_state)
                    .map_or(0, |state| state.threads.len());
                if thread_count > 0 {
                    let noun = if thread_count == 1 { "thread" } else { "threads" };
                    item.push_str(&format!(" · {} {}", thread_count, noun));
                }
                item
            })
            .collect()
    }

    /// Put another repository on screen, keeping the review state of the current one.
    /// On error nothing changes.
    pub fn switch_to(
        &mut self,
        index: usize,
        options: &DiffOptions,
        state: &mut AppState,
    ) -> Result<&'a dyn VcsBackend, String> {
        if index == self.active {
            return self.workspace.backend(index);
        }

        let (backend, mut next_state) = match self.parked_states[index].take() {
            Some(mut parked_state) => {
                // File changes made while the repository was off screen were not watched.
                parked_state.needs_reload = state.watching;
                (self.workspace.backend(index)?, parked_state)
            }
            None => {
                let backend = self.workspace.reviewable_backend(index, options)?;
                let file_diffs = load_file_diffs(options, backend);
                let mut review_state = new_review_state(options, file_diffs, backend, None);
                open_thread_store(&mut review_state, backend);
                (backend, review_state)
            }
        };
        next_state.watching = state.watching;
        next_state.set_hunks_only(state.settings.hunks_only, 0);

        self.parked_states[self.active] = Some(std::mem::replace(state, next_state));
        self.active = index;
        Ok(backend)
    }

    pub fn format_threads_for_export(&self, active_state: &AppState, only_queued: bool) -> String {
        let sections: Vec<(&str, &Path, &AppState)> = self
            .workspace
            .repositories
            .iter()
            .enumerate()
            .filter_map(|(index, repository)| {
                let state = self.state_of(index, active_state)?;
                Some((repository.name.as_str(), repository.path.as_path(), state))
            })
            .collect();
        format_repository_sections(&sections, only_queued)
    }
}

/// Threads of every repository that has some to export, each under a heading
/// naming the repository and its absolute path.
fn format_repository_sections(sections: &[(&str, &Path, &AppState)], only_queued: bool) -> String {
    sections
        .iter()
        .filter(|(_, _, state)| {
            state
                .threads
                .iter()
                .any(|thread| !only_queued || thread.queued_count() > 0)
        })
        .map(|(name, path, state)| {
            repository_section(name, path, &state.format_threads_for_export(only_queued))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// `content` under a heading naming the repository and its absolute path.
fn repository_section(name: &str, path: &Path, content: &str) -> String {
    format!("# Repository: {} ({})\n\n{}", name, path.display(), content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::diff::review_thread::AnnotationTarget;
    use crate::command::diff::types::{DiffPanelFocus, FileDiff, FileStatus};
    use crate::commit_reference::CommitReference;
    use crate::vcs::test_utils::{git, make_temp_dir};
    use std::fs;

    fn options_for(reference: Option<CommitReference>) -> DiffOptions {
        DiffOptions {
            reference,
            pr: None,
            detect_pr: false,
            file: None,
            watch: false,
            theme: None,
            stacked: false,
            focus: None,
            origin: None,
            wrap: false,
            hunks_only: true,
            agent_block: None,
        }
    }

    /// A repository with one commit and one uncommitted file named after it.
    fn create_repository(workspace_root: &Path, name: &str) -> PathBuf {
        let dir = workspace_root.join(name);
        fs::create_dir_all(&dir).expect("create repository dir");
        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);
        fs::write(dir.join("README.md"), "hello\n").expect("write README");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "init"]);
        fs::write(dir.join(format!("{}.txt", name)), "uncommitted\n").expect("write change");
        dir
    }

    fn repository_names(workspace: &Workspace) -> Vec<&str> {
        workspace
            .repositories
            .iter()
            .map(|repository| repository.name.as_str())
            .collect()
    }

    fn filenames(state: &AppState) -> Vec<&str> {
        state.file_diffs.iter().map(|diff| diff.filename.as_str()).collect()
    }

    fn open_session<'a>(
        workspace: &'a Workspace,
        options: &DiffOptions,
    ) -> (WorkspaceSession<'a>, AppState) {
        let backend = workspace.backend(0).expect("backend");
        let file_diffs = load_file_diffs(options, backend);
        let state = new_review_state(options, file_diffs, backend, None);
        (WorkspaceSession::new(workspace, 0), state)
    }

    fn annotate(state: &mut AppState, filename: &str, content: &str) {
        state.add_thread(
            filename.to_string(),
            AnnotationTarget::LineRange {
                panel: DiffPanelFocus::New,
                start_line: 1,
                end_line: 1,
            },
            content.to_string(),
        );
    }

    #[test]
    fn test_discover_lists_immediate_child_repositories_sorted_by_name() {
        let root = make_temp_dir("workspace-discover");
        create_repository(&root, "web");
        create_repository(&root, "api");
        fs::create_dir_all(root.join("notes")).expect("create plain folder");
        create_repository(&root.join("group"), "nested");

        let workspace = Workspace::discover(&root, None).expect("workspace");

        assert_eq!(repository_names(&workspace), vec!["api", "web"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn test_discover_ignores_symlinks_to_repositories() {
        let root = make_temp_dir("workspace-symlink");
        let elsewhere = make_temp_dir("workspace-symlink-target");
        create_repository(&root, "api");
        let target = create_repository(&elsewhere, "linked");
        std::os::unix::fs::symlink(&target, root.join("linked")).expect("create symlink");

        let workspace = Workspace::discover(&root, None).expect("workspace");

        assert_eq!(repository_names(&workspace), vec!["api"]);
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&elsewhere);
    }

    #[test]
    fn test_discover_returns_none_when_the_folder_is_not_a_workspace() {
        let root = make_temp_dir("workspace-not-a-workspace");
        let plain_folder = root.join("notes");
        fs::create_dir_all(plain_folder.join("drafts")).expect("create plain folders");
        let repository = create_repository(&root, "api");
        create_repository(&repository, "vendored");

        assert!(Workspace::discover(&plain_folder, None).is_none());
        assert!(Workspace::discover(&repository, None).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_initial_repository_skips_repositories_without_changes() {
        let root = make_temp_dir("workspace-initial");
        let clean = create_repository(&root, "api");
        fs::remove_file(clean.join("api.txt")).expect("remove uncommitted file");
        create_repository(&root, "web");

        let workspace = Workspace::discover(&root, None).expect("workspace");

        assert_eq!(workspace.initial_repository(&options_for(None)), Ok(1));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_initial_repository_skips_repositories_lacking_the_requested_revision() {
        let root = make_temp_dir("workspace-unreviewable");
        create_repository(&root, "api");
        let web = create_repository(&root, "web");
        git(&web, &["checkout", "-b", "only-in-web"]);
        let from_revision = |revision: &str| {
            options_for(Some(CommitReference::MergeBaseToWorkingTree {
                from: revision.to_string(),
            }))
        };
        let workspace = Workspace::discover(&root, None).expect("workspace");

        assert_eq!(workspace.initial_repository(&from_revision("only-in-web")), Ok(1));

        let error = workspace
            .initial_repository(&from_revision("origin/main"))
            .expect_err("no reviewable repository");
        assert!(error.contains("api: invalid reference: origin/main"), "{}", error);
        assert!(error.contains("web: invalid reference: origin/main"), "{}", error);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_switching_repositories_keeps_each_review_state() {
        let root = make_temp_dir("workspace-switch");
        create_repository(&root, "api");
        create_repository(&root, "web");
        let options = options_for(None);
        let workspace = Workspace::discover(&root, None).expect("workspace");
        let (mut session, mut state) = open_session(&workspace, &options);
        annotate(&mut state, "api.txt", "check this");

        let web_backend = session.switch_to(1, &options, &mut state).expect("switch to web");

        assert_eq!(session.active_name(), "web");
        assert_eq!(filenames(&state), vec!["web.txt"]);
        assert!(state.threads.is_empty());
        assert!(web_backend.working_tree_root().ends_with("web"));
        assert_eq!(session.queued_count(&state), 1);
        let text_for_agent = session.queued_text_for_agent(&state, None);
        assert!(text_for_agent.starts_with("# Repository: api ("), "{}", text_for_agent);
        assert!(text_for_agent.ends_with("you: check this"), "{}", text_for_agent);

        session.switch_to(0, &options, &mut state).expect("switch back to api");

        assert_eq!(session.active_name(), "api");
        assert_eq!(filenames(&state), vec!["api.txt"]);
        assert_eq!(state.threads.len(), 1);
        assert_eq!(state.threads[0].comments[0].body, "check this");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_failed_switch_keeps_the_current_repository() {
        let root = make_temp_dir("workspace-failed-switch");
        let api = create_repository(&root, "api");
        git(&api, &["checkout", "-b", "only-in-api"]);
        create_repository(&root, "web");
        let options = options_for(Some(CommitReference::RangeToWorkingTree {
            from: "only-in-api".to_string(),
        }));
        let workspace = Workspace::discover(&root, None).expect("workspace");
        let (mut session, mut state) = open_session(&workspace, &options);

        let error = session
            .switch_to(1, &options, &mut state)
            .err()
            .expect("web lacks the branch");

        assert_eq!(error, "web: invalid reference: only-in-api");
        assert_eq!(session.active_name(), "api");
        assert_eq!(filenames(&state), vec!["api.txt"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn test_export_groups_annotations_by_repository() {
        let file_diff = |filename: &str| FileDiff {
            filename: filename.to_string(),
            old_content: String::new(),
            new_content: "content\n".to_string(),
            status: FileStatus::Added,
            is_binary: false,
        };
        let mut api_state = AppState::new(vec![file_diff("src/api.rs")], None);
        api_state.set_diff_reference(Some("origin/main...-".to_string()));
        annotate(&mut api_state, "src/api.rs", "rename this");
        let untouched_state = AppState::new(vec![file_diff("notes.md")], None);
        let mut web_state = AppState::new(vec![file_diff("app.ts")], None);
        annotate(&mut web_state, "app.ts", "missing test");

        let export = format_repository_sections(
            &[
                ("api", Path::new("/work/api"), &api_state),
                ("docs", Path::new("/work/docs"), &untouched_state),
                ("web", Path::new("/work/web"), &web_state),
            ],
            true,
        );

        assert_eq!(
            export,
            "# Repository: api (/work/api)\n\n\
             # origin/main...-\n\n\
             **src/api.rs** line 1 (RIGHT)\n\n\
             rename this\n\n\
             # Repository: web (/work/web)\n\n\
             **app.ts** line 1 (RIGHT)\n\n\
             missing test"
        );
    }
}
