//! Session state for reviewing several repositories in a workspace folder, keeping each
//! repository's `AppState` parked while another is on screen.

use std::path::{Path, PathBuf};

use crate::diff::{load_file_diffs, DiffOptions};
use crate::review::Workspace;
use crate::ui::state::AppState;
use crate::vcs::VcsBackend;

pub struct WorkspaceSession<'a> {
    workspace: &'a Workspace,
    active: usize,
    parked_states: Vec<Option<AppState>>,
}

impl<'a> WorkspaceSession<'a> {
    pub fn new(workspace: &'a Workspace, active: usize) -> Self {
        let parked_states = workspace.repositories().iter().map(|_| None).collect();
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
        &self.workspace.repositories()[self.active].name
    }

    fn state_of<'s>(&'s self, index: usize, active_state: &'s AppState) -> Option<&'s AppState> {
        if index == self.active {
            Some(active_state)
        } else {
            self.parked_states[index].as_ref()
        }
    }

    pub fn queued_count(&self, active_state: &AppState) -> usize {
        (0..self.parked_states.len())
            .filter_map(|index| self.state_of(index, active_state))
            .map(AppState::queued_count)
            .sum()
    }

    pub fn queued_text_for_agent(
        &self,
        active_state: &AppState,
        only: Option<(&str, &str)>,
    ) -> String {
        self.workspace
            .repositories()
            .iter()
            .enumerate()
            .filter(|(index, _)| only.is_none() || *index == self.active)
            .filter_map(|(index, repository)| {
                let text = self
                    .state_of(index, active_state)?
                    .queued_text_for_agent(only);
                (!text.is_empty())
                    .then(|| repository_section(&repository.name, &repository.path, &text))
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

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
            .repositories()
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
                    let noun = if thread_count == 1 {
                        "thread"
                    } else {
                        "threads"
                    };
                    item.push_str(&format!(" · {thread_count} {noun}"));
                }
                item
            })
            .collect()
    }

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
                parked_state.needs_reload = state.watching;
                let backend = self.workspace.backend(index)?;
                parked_state.refresh_branch(backend);
                (backend, parked_state)
            }
            None => {
                let backend = self.workspace.reviewable_backend(index, options)?;
                let file_diffs = load_file_diffs(options, backend);
                let mut review_state = AppState::for_repo(options, file_diffs, backend, None);
                review_state.attach_thread_store(backend);
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
            .repositories()
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

fn repository_section(name: &str, path: &Path, content: &str) -> String {
    format!("# Repository: {name} ({})\n\n{content}", path.display())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::diff::{DiffPanelFocus, FileDiff, FileStatus};
    use crate::review::AnnotationTarget;
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

    fn filenames(state: &AppState) -> Vec<&str> {
        state
            .file_diffs
            .iter()
            .map(|diff| diff.filename.as_str())
            .collect()
    }

    fn open_session<'a>(
        workspace: &'a Workspace,
        options: &DiffOptions,
    ) -> (WorkspaceSession<'a>, AppState) {
        let backend = workspace.backend(0).expect("backend");
        let file_diffs = load_file_diffs(options, backend);
        let state = AppState::for_repo(options, file_diffs, backend, None);
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
    fn test_switching_repositories_keeps_each_review_state() {
        let root = TempDir::new().unwrap();
        create_repository(root.path(), "api");
        create_repository(root.path(), "web");
        let options = options_for(None);
        let workspace = Workspace::discover(root.path(), None).expect("workspace");
        let (mut session, mut state) = open_session(&workspace, &options);
        annotate(&mut state, "api.txt", "check this");

        let web_backend = session.switch_to(1, &options, &mut state).expect("switch");
        assert_eq!(session.active_name(), "web");
        assert_eq!(filenames(&state), vec!["web.txt"]);
        assert!(state.threads.is_empty());
        assert!(web_backend.working_tree_root().ends_with("web"));
        assert_eq!(session.queued_count(&state), 1);
        let text_for_agent = session.queued_text_for_agent(&state, None);
        assert!(text_for_agent.starts_with("# Repository: api ("));
        assert!(text_for_agent.ends_with("you: check this"));

        session
            .switch_to(0, &options, &mut state)
            .expect("switch back");
        assert_eq!(session.active_name(), "api");
        assert_eq!(filenames(&state), vec!["api.txt"]);
        assert_eq!(state.threads.len(), 1);
        assert_eq!(state.threads[0].comments[0].body, "check this");
    }

    #[test]
    fn test_failed_switch_keeps_the_current_repository() {
        let root = TempDir::new().unwrap();
        let api = create_repository(root.path(), "api");
        git(&api, &["checkout", "-b", "only-in-api"]);
        create_repository(root.path(), "web");
        let options = options_for(Some(CommitReference::RangeToWorkingTree {
            from: "only-in-api".to_string(),
        }));
        let workspace = Workspace::discover(root.path(), None).expect("workspace");
        let (mut session, mut state) = open_session(&workspace, &options);

        let error = session
            .switch_to(1, &options, &mut state)
            .err()
            .expect("web lacks branch");
        assert_eq!(error, "web: invalid reference: only-in-api");
        assert_eq!(session.active_name(), "api");
        assert_eq!(filenames(&state), vec!["api.txt"]);
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
