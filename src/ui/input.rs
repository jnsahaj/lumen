use std::time::Instant;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;

use crate::diff::github::{mark_file_as_viewed_async, unmark_file_as_viewed_async};
use crate::diff::watcher::setup_watcher;
use crate::diff::{
    hunk_comment_lines, hunk_lines_on, load_single_commit_diffs, DiffFullscreen, DiffLine,
    DiffOptions, DiffPanelFocus,
};
use crate::review::agent::{AgentLink, AgentPane, AgentRequest};
use crate::review::{AnnotationTarget, Thread};
use crate::search::global::GlobalSearchState;
use crate::search::MatchPanel;
use crate::ui::coordinates::{
    extract_selected_text, rect_contains, CursorPosition, PanelLayout, SelectionMode,
};
use crate::ui::editor::{AnnotationEditor, AnnotationEditorResult};
use crate::ui::keybindings::help_sections;
use crate::ui::state::{
    adjust_scroll_for_hunk, adjust_scroll_to_line, is_child_path, AppState, FocusedPanel,
    PendingKey, SidebarItem,
};
use crate::ui::views::{truncate_path, FilePickerItem, Modal, ModalContent, ModalResult};
use crate::ui::{App, LoopAction, Picker};
use crate::vcs::VcsBackend;

pub fn count_of(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

pub fn editor_words(editor: &str) -> Vec<String> {
    shlex::split(editor)
        .filter(|words| !words.is_empty())
        .unwrap_or_else(|| vec![editor.to_string()])
}

fn changes_file(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::ALT)
        || (matches!(key.code, KeyCode::Char(_)) && key.modifiers.contains(KeyModifiers::CONTROL))
}

fn lines_are_selected(state: &AppState) -> bool {
    state.selection.is_active() && !matches!(state.selection.mode, SelectionMode::None)
}

fn ensure_sidebar_visible(state: &mut AppState, visible_height: usize) {
    if state.sidebar_selected >= state.sidebar_scroll + visible_height {
        state.sidebar_scroll = state.sidebar_selected.saturating_sub(visible_height) + 1;
    } else if state.sidebar_selected < state.sidebar_scroll {
        state.sidebar_scroll = state.sidebar_selected;
    }
}

fn max_h_scroll(state: &mut AppState, term_width: u16) -> u16 {
    if state.settings.wrap || state.file_diffs.is_empty() {
        return 0;
    }
    let diff = &state.file_diffs[state.current_file];
    if diff.is_binary {
        return 0;
    }

    let sidebar_width = if state.show_sidebar {
        (term_width / 4).clamp(20, 35)
    } else {
        0
    };
    let effective_fullscreen = diff.effective_fullscreen(state.diff_fullscreen);
    let layout = PanelLayout::calculate(
        term_width,
        sidebar_width,
        state.show_sidebar,
        effective_fullscreen,
    );

    state.ensure_cache();
    let (max_old, max_new) = state.max_line_widths();

    let gutter = layout.gutter_width as usize;
    let old_overhead = layout.focus_indicator_width as usize + gutter;
    let new_overhead = if layout.old_panel_width > 0 {
        gutter
    } else {
        layout.focus_indicator_width as usize + gutter
    };

    let old_overflow = if layout.old_panel_width > 0 {
        (max_old + old_overhead).saturating_sub(layout.old_panel_width as usize)
    } else {
        0
    };
    let new_overflow = if layout.new_panel_width > 0 {
        (max_new + new_overhead).saturating_sub(layout.new_panel_width as usize)
    } else {
        0
    };

    old_overflow.max(new_overflow).min(u16::MAX as usize) as u16
}

fn clamp_h_scroll(state: &mut AppState, term_width: u16) {
    let max = max_h_scroll(state, term_width);
    if state.h_scroll > max {
        state.h_scroll = max;
    }
}

fn max_sidebar_h_scroll(state: &AppState, term_width: u16) -> u16 {
    if !state.show_sidebar {
        return 0;
    }
    let sidebar_width = (term_width / 4).clamp(20, 35) as usize;
    let inner_width = sidebar_width.saturating_sub(1);
    let max_label = state
        .sidebar_items
        .iter()
        .map(|item| match item {
            SidebarItem::Directory { name, depth, .. } | SidebarItem::File { name, depth, .. } => {
                depth * 2 + 4 + name.chars().count()
            }
        })
        .max()
        .unwrap_or(0);
    max_label.saturating_sub(inner_width).min(u16::MAX as usize) as u16
}

fn clamp_sidebar_h_scroll(state: &mut AppState, term_width: u16) {
    let max = max_sidebar_h_scroll(state, term_width);
    if state.sidebar_h_scroll > max {
        state.sidebar_h_scroll = max;
    }
}

fn find_sbs_index_for_line(
    side_by_side: &[DiffLine],
    panel: DiffPanelFocus,
    line_num: usize,
) -> Option<usize> {
    side_by_side
        .iter()
        .position(|dl| dl.line_number(panel) == Some(line_num))
}

fn navigate_stacked_commit(
    state: &mut AppState,
    new_index: usize,
    options: &DiffOptions,
    backend: &dyn VcsBackend,
) -> bool {
    if new_index >= state.stacked_commits.len() {
        return false;
    }
    state.save_stacked_viewed_files();
    state.current_commit_index = new_index;
    if let Some(commit) = state.stacked_commits.get(new_index) {
        let file_diffs = load_single_commit_diffs(&commit.commit_id, &options.file, backend);
        state.reload(file_diffs, None);
        state.refresh_branch(backend);
        state.load_stacked_viewed_files();
        true
    } else {
        false
    }
}

fn format_annotation_preview(thread: &Thread) -> String {
    let Some(last_comment) = thread.comments.last() else {
        return String::new();
    };
    let states = [
        (thread.resolved, "resolved"),
        (thread.outdated, "outdated"),
        (thread.queued_count() > 0, "queued"),
        (thread.awaiting_agent(), "awaiting agent"),
    ];
    let mut preview: String = states
        .iter()
        .filter(|(applies, _)| *applies)
        .map(|(_, state)| format!("[{state}] "))
        .collect();
    let first_line = last_comment
        .body
        .lines()
        .next()
        .unwrap_or("")
        .replace(" | ", " / ");
    preview.push_str(&format!("{}: {first_line}", last_comment.author_name));
    if preview.chars().count() > 60 {
        preview = format!("{}...", preview.chars().take(60).collect::<String>());
    }

    let truncated_filename = truncate_path(&thread.filename, 30);
    let line_display = thread.line_range_display();
    let location = if line_display.is_empty() {
        truncated_filename
    } else {
        format!("{truncated_filename}:{line_display}")
    };
    format!(
        "{location} [{}] | {preview} | {}",
        thread.target_label(),
        last_comment.format_time()
    )
}

pub fn threads_modal(
    threads: &[Thread],
    selected_thread: Option<&str>,
    show_resolved: bool,
) -> Modal {
    let mut listed_threads: Vec<Thread> = threads
        .iter()
        .filter(|thread| show_resolved || !thread.resolved)
        .cloned()
        .collect();
    listed_threads.sort_by_key(|thread| thread.created_at);
    let items = listed_threads
        .iter()
        .map(format_annotation_preview)
        .collect();
    let selected = selected_thread
        .and_then(|id| listed_threads.iter().position(|thread| thread.id == id))
        .unwrap_or(0);
    let hidden = threads.len() - listed_threads.len();
    let title = if hidden > 0 {
        format!(
            "Threads ({}) · {hidden} resolved hidden",
            listed_threads.len()
        )
    } else {
        format!("Threads ({})", listed_threads.len())
    };
    Modal::annotations(title, items, listed_threads, selected)
}

pub fn agent_picker(agent: &AgentLink<'_>) -> Modal {
    agent_picker_with_selection(agent, None)
}

pub fn agent_picker_with_selection(
    agent: &AgentLink<'_>,
    highlighted_target: Option<Option<(&str, &str)>>,
) -> Modal {
    let panes = agent.known_candidates();
    let selected = match highlighted_target {
        Some(None) => 0,
        Some(Some((session, block))) => panes
            .iter()
            .position(|pane| pane.session == session && pane.block == block)
            .map_or_else(
                || {
                    panes
                        .iter()
                        .position(|pane| Some(pane.block.as_str()) == agent.attached_block())
                        .map_or(0, |index| index + 1)
                },
                |index| index + 1,
            ),
        None => panes
            .iter()
            .position(|pane| Some(pane.block.as_str()) == agent.attached_block())
            .map_or(0, |index| index + 1),
    };
    let mut items = vec!["none".to_string()];
    items.extend(panes.iter().map(AgentPane::picker_label));
    Modal::select("Agents", items, selected)
}

pub fn target_at_cursor(state: &mut AppState) -> Option<(String, AnnotationTarget)> {
    let filename = state.file_diffs.get(state.current_file)?.filename.clone();

    if lines_are_selected(state) {
        let panel = state.selection.panel;
        let sel_start = state.selection.anchor.line.min(state.selection.head.line);
        let sel_end = state.selection.anchor.line.max(state.selection.head.line);

        state.ensure_cache();
        let sbs = state.side_by_side_ref();
        let mut line_numbers = (sel_start..=sel_end)
            .filter_map(|idx| sbs.get(idx).and_then(|dl| dl.line_number(panel)));
        let start_line = line_numbers.next()?;
        let end_line = line_numbers.next_back().unwrap_or(start_line);
        return Some((
            filename,
            AnnotationTarget::LineRange {
                panel,
                start_line,
                end_line,
            },
        ));
    }

    let Some(hunk_index) = state.focused_hunk else {
        return Some((filename, AnnotationTarget::File));
    };

    state.ensure_cache();
    let hunk_start = state.hunks_ref().get(hunk_index).copied().unwrap_or(0);
    let (panel, start_line, end_line) =
        hunk_comment_lines(state.side_by_side_ref(), hunk_start, state.diff_fullscreen)
            .unwrap_or((DiffPanelFocus::New, 1, 1));
    Some((
        filename,
        AnnotationTarget::LineRange {
            panel,
            start_line,
            end_line,
        },
    ))
}

pub fn thread_at_cursor<'a>(
    state: &'a AppState,
    filename: &str,
    target: &AnnotationTarget,
    or_resolved: bool,
) -> Option<&'a Thread> {
    if let Some(thread) = state.thread_at(filename, target, or_resolved) {
        return Some(thread);
    }
    let AnnotationTarget::LineRange { panel, .. } = target else {
        return None;
    };
    if lines_are_selected(state) {
        return None;
    }
    let hunk_start = *state.hunks_ref().get(state.focused_hunk?)?;
    let other_side = match panel {
        DiffPanelFocus::Old => DiffPanelFocus::New,
        _ => DiffPanelFocus::Old,
    };
    let (start_line, end_line) = hunk_lines_on(state.side_by_side_ref(), hunk_start, other_side)?;
    let other_side_of_hunk = AnnotationTarget::LineRange {
        panel: other_side,
        start_line,
        end_line,
    };
    state.thread_at(filename, &other_side_of_hunk, or_resolved)
}

pub fn show_thread(state: &mut AppState, thread_id: &str) -> bool {
    let Some(thread) = state.thread_by_id(thread_id) else {
        return false;
    };
    let filename = thread.filename.clone();
    let target = thread.target.clone();
    let Some(file_index) = state.file_diffs.iter().position(|f| f.filename == filename) else {
        return false;
    };
    state.select_file(file_index);

    if let AnnotationTarget::LineRange {
        panel, start_line, ..
    } = target
    {
        state.ensure_cache();
        if let Some(first_row) =
            find_sbs_index_for_line(state.side_by_side_ref(), panel, start_line)
        {
            state.scroll = first_row as u16;
            let hunk = state
                .hunks_ref()
                .iter()
                .rposition(|&hunk_start| hunk_start <= first_row);
            state.focused_hunk = hunk.or(state.focused_hunk);
        }
    }
    true
}

fn save_comment(state: &mut AppState, editor: &AnnotationEditor) -> Option<(String, String)> {
    let body = editor.content();
    match (&editor.thread_id, &editor.comment_id) {
        (Some(thread_id), Some(comment_id)) => {
            state.update_comment(thread_id, comment_id, body);
            Some((thread_id.clone(), comment_id.clone()))
        }
        (Some(thread_id), None) => state
            .add_reply(thread_id, body)
            .map(|comment_id| (thread_id.clone(), comment_id)),
        (None, _) => Some(state.add_thread(editor.filename.clone(), editor.target.clone(), body)),
    }
}

fn remove_edited_comment(state: &mut AppState, editor: &AnnotationEditor) {
    if let (Some(thread_id), Some(comment_id)) = (&editor.thread_id, &editor.comment_id) {
        state.remove_comment(thread_id, comment_id);
    }
}

pub fn close_deleted_thread_modal(
    state: &AppState,
    active_modal: &mut Option<Modal>,
    annotation_editor: &mut Option<AnnotationEditor>,
    show_resolved_threads: bool,
) -> bool {
    let Some(Modal {
        content: ModalContent::ThreadView(view),
    }) = active_modal.as_ref()
    else {
        return false;
    };
    if state.thread_by_id(&view.thread_id).is_some() {
        return false;
    }
    let closed_id = view.thread_id.clone();
    let reopen_list = view.return_to_list && !state.threads.is_empty();
    *active_modal = reopen_list.then(|| threads_modal(&state.threads, None, show_resolved_threads));
    if annotation_editor
        .as_ref()
        .and_then(|e| e.thread_id.as_deref())
        == Some(closed_id.as_str())
    {
        *annotation_editor = None;
    }
    true
}

pub fn dismiss_modal(
    state: &AppState,
    active_modal: &mut Option<Modal>,
    show_resolved_threads: bool,
) {
    let reopen_selected = match active_modal.as_ref() {
        Some(Modal {
            content: ModalContent::ThreadView(view),
        }) if view.return_to_list && !state.threads.is_empty() => Some(view.thread_id.clone()),
        _ => None,
    };
    *active_modal =
        reopen_selected.map(|id| threads_modal(&state.threads, Some(&id), show_resolved_threads));
}

fn dismiss_editor_on_outside_click(
    state: &mut AppState,
    annotation_editor: &mut Option<AnnotationEditor>,
    active_modal: &mut Option<Modal>,
    column: u16,
    row: u16,
) {
    let inside = state
        .editor_rect
        .is_some_and(|r| rect_contains(r, column, row));
    if !inside {
        if let Some(editor) = annotation_editor.as_ref() {
            if editor.is_empty() {
                remove_edited_comment(state, editor);
            } else {
                save_comment(state, editor);
                if let Some(modal) = active_modal.as_mut() {
                    modal.pin_thread_to_bottom();
                }
            }
        }
        *annotation_editor = None;
    }
}

impl<'a> App<'a> {
    fn max_scroll(&mut self, term_height: u16) -> usize {
        if self.state.file_diffs.is_empty() {
            return 0;
        }
        let visible_height = term_height.saturating_sub(2) as usize;
        let total_lines = self.state.total_lines();
        let filename = &self.state.file_diffs[self.state.current_file].filename;
        if self.state.threads.iter().any(|t| &t.filename == filename) {
            total_lines.saturating_sub(1)
        } else {
            total_lines.saturating_sub(visible_height.saturating_sub(5))
        }
    }

    pub fn handle_event(&mut self, event: Event, term_size: Rect) -> LoopAction {
        match event {
            Event::Resize(_, _) => {
                self.dirty = true;
                LoopAction::Continue
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                self.dirty = true;
                if self.state.search_state.is_active() {
                    self.handle_search_key(key);
                    LoopAction::Continue
                } else if self.annotation_editor.is_some()
                    && self.active_modal.as_ref().is_none_or(Modal::is_thread_view)
                {
                    self.handle_editor_key(key);
                    LoopAction::Continue
                } else if self.active_modal.is_some() {
                    self.handle_modal_key(key, term_size.height)
                } else {
                    self.handle_normal_key(key, term_size)
                }
            }
            Event::Mouse(mouse) => {
                if !matches!(mouse.kind, MouseEventKind::Moved) {
                    self.dirty = true;
                    self.handle_mouse(mouse, term_size);
                }
                LoopAction::Continue
            }
            _ => LoopAction::Continue,
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.state.search_state.cancel();
                self.state.mark_search_dirty();
            }
            KeyCode::Enter => {
                self.state.search_state.confirm();
                if self.state.search_state.has_query() {
                    if let Some(line) = self
                        .state
                        .search_state
                        .jump_to_first_match(self.state.scroll as usize)
                    {
                        self.state.scroll = line.saturating_sub(5) as u16;
                    }
                }
            }
            KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
                self.state.search_state.erase_word();
                self.state.mark_search_dirty();
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.search_state.erase_word();
                self.state.mark_search_dirty();
            }
            KeyCode::Backspace => {
                self.state.search_state.pop_char();
                self.state.mark_search_dirty();
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.state.search_state.push_char(c);
                self.state.mark_search_dirty();
            }
            _ => {}
        }
    }

    fn handle_editor_key(&mut self, key: KeyEvent) {
        let Some(editor) = self.annotation_editor.as_mut() else {
            return;
        };
        match editor.handle_input(key) {
            AnnotationEditorResult::Continue => {}
            result @ (AnnotationEditorResult::Send | AnnotationEditorResult::Queue) => {
                let saved = save_comment(&mut self.state, editor);
                let send_now = matches!(result, AnnotationEditorResult::Send);
                self.annotation_editor = None;
                if let Some(ref mut modal) = self.active_modal {
                    modal.pin_thread_to_bottom();
                }
                let message = match saved {
                    None => "the thread no longer exists".to_string(),
                    Some(_) if !send_now => "comment queued".to_string(),
                    Some(_) if !self.agent.is_attached() => {
                        "no agent attached: comment queued".to_string()
                    }
                    Some((thread_id, comment_id)) => self
                        .send_queued(Some((&thread_id, &comment_id)))
                        .unwrap_or_else(|error| error),
                };
                self.set_notice(message);
            }
            AnnotationEditorResult::Delete => {
                remove_edited_comment(&mut self.state, editor);
                self.annotation_editor = None;
            }
            AnnotationEditorResult::Cancel => {
                self.annotation_editor = None;
            }
        }
    }

    fn handle_modal_key(&mut self, key: KeyEvent, term_height: u16) -> LoopAction {
        let Some(result) = self
            .active_modal
            .as_mut()
            .and_then(|modal| modal.handle_input(key, term_height))
        else {
            return LoopAction::Continue;
        };

        match result {
            ModalResult::FileSelected(file_index) => {
                self.focus_file_in_sidebar(file_index, term_height);
                self.active_modal = None;
            }
            ModalResult::AnnotationOpen { annotation_id } => {
                self.active_modal = self.state.thread_by_id(&annotation_id).map(|thread| {
                    Modal::thread_view(thread.id.clone(), self.state.settings.tab_width, true)
                });
            }
            ModalResult::AnnotationJump { annotation_id } => {
                show_thread(&mut self.state, &annotation_id);
                self.active_modal = None;
            }
            ModalResult::AnnotationEdit { annotation_id } => {
                if show_thread(&mut self.state, &annotation_id) {
                    self.annotation_editor = self
                        .state
                        .thread_by_id(&annotation_id)
                        .map(AnnotationEditor::for_thread);
                }
                self.active_modal = None;
            }
            ModalResult::AnnotationDelete { annotation_id } => {
                self.state.remove_thread(&annotation_id);
                self.active_modal = (!self.state.threads.is_empty())
                    .then(|| threads_modal(&self.state.threads, None, self.show_resolved_threads));
            }
            ModalResult::AnnotationToggleResolved { annotation_id } => {
                self.state.toggle_thread_resolved(&annotation_id);
                self.active_modal = Some(threads_modal(
                    &self.state.threads,
                    Some(&annotation_id),
                    self.show_resolved_threads,
                ));
            }
            ModalResult::AnnotationsToggleResolvedShown { selected_id } => {
                self.show_resolved_threads = !self.show_resolved_threads;
                self.active_modal = Some(threads_modal(
                    &self.state.threads,
                    selected_id.as_deref(),
                    self.show_resolved_threads,
                ));
            }
            ModalResult::ThreadReply { thread_id } => {
                if let Some(thread) = self.state.thread_by_id(&thread_id) {
                    self.annotation_editor = Some(AnnotationEditor::for_thread(thread));
                }
            }
            ModalResult::ThreadToggleResolved { thread_id } => {
                self.state.toggle_thread_resolved(&thread_id);
            }
            ModalResult::Confirmed => {
                self.send_annotations_on_exit = true;
                return LoopAction::Quit;
            }
            ModalResult::JumpToLine {
                file_index,
                line_no,
                panel,
            } => {
                self.focus_file_in_sidebar(file_index, term_height);
                let panel = match panel {
                    MatchPanel::Old => DiffPanelFocus::Old,
                    MatchPanel::New => DiffPanelFocus::New,
                };
                self.state.ensure_cache();
                let mut row =
                    find_sbs_index_for_line(self.state.side_by_side_ref(), panel, line_no);
                if row.is_none() && self.state.settings.hunks_only {
                    self.state.set_hunks_only(false, 0);
                    row = find_sbs_index_for_line(self.state.side_by_side_ref(), panel, line_no);
                    self.set_notice("full file: the match is outside the hunks");
                }
                let sbs_len = self.state.side_by_side_ref().len();
                let vh = term_height.saturating_sub(5) as usize;
                let max_scroll = sbs_len.saturating_sub(vh).min(u16::MAX as usize) as u16;
                self.state.scroll =
                    (row.unwrap_or(0).min(u16::MAX as usize) as u16).min(max_scroll);
                self.active_modal = None;
            }
            ModalResult::Selected(selected_index, _) => {
                self.active_modal = None;
                match self.open_picker.take() {
                    Some(Picker::Repository) => {
                        if let Some(session) = self.workspace_session.as_mut() {
                            match session.switch_to(selected_index, &self.options, &mut self.state)
                            {
                                Ok(repository_backend) => {
                                    self.backend = repository_backend;
                                    self.pending_watch_event = None;
                                    let watcher = self
                                        .state
                                        .watching
                                        .then(|| setup_watcher(self.backend.working_tree_root()))
                                        .flatten();
                                    self.state.watching = watcher.is_some();
                                    (self.watch_handle, self.watch_rx) = watcher.unzip();
                                }
                                Err(error) => {
                                    self.active_modal =
                                        Some(Modal::info("Cannot open repository", error));
                                }
                            }
                        }
                    }
                    Some(Picker::Agent) => {
                        let pane = selected_index
                            .checked_sub(1)
                            .and_then(|idx| self.agent.known_candidates().get(idx).cloned());
                        self.agent.set_pane(pane);
                    }
                    Some(Picker::Quit) => match selected_index {
                        0 if self.agent.is_attached() => match self.send_queued(None) {
                            Ok(_) => return LoopAction::Quit,
                            Err(error) => {
                                self.active_modal = Some(Modal::info("Not sent", error));
                            }
                        },
                        0 => {
                            self.send_annotations_on_exit = true;
                            return LoopAction::Quit;
                        }
                        1 => return LoopAction::Quit,
                        _ => {}
                    },
                    None => {}
                }
            }
            ModalResult::Dismissed => {
                dismiss_modal(
                    &self.state,
                    &mut self.active_modal,
                    self.show_resolved_threads,
                );
                self.open_picker = None;
            }
        }
        LoopAction::Continue
    }

    fn focus_file_in_sidebar(&mut self, file_index: usize, term_height: u16) {
        self.state.reveal_file(file_index);
        self.state.select_file(file_index);
        if let Some(idx) = self
            .state
            .sidebar_visible_index_for_file(self.state.current_file)
        {
            self.state.sidebar_selected = idx;
            let visible_height = term_height.saturating_sub(5) as usize;
            ensure_sidebar_visible(&mut self.state, visible_height);
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, term_size: Rect) {
        if self.active_modal.is_some() {
            if self.annotation_editor.is_some()
                && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            {
                dismiss_editor_on_outside_click(
                    &mut self.state,
                    &mut self.annotation_editor,
                    &mut self.active_modal,
                    mouse.column,
                    mouse.row,
                );
                return;
            }
            if let Some(ref mut modal) = self.active_modal {
                modal.handle_mouse(mouse, term_size.width, term_size.height);
            }
            return;
        }

        let footer_height = 1u16;
        let header_height = u16::from(self.state.stacked_mode);
        let sidebar_width = if self.state.show_sidebar {
            (term_size.width / 4).clamp(20, 35)
        } else {
            0
        };
        let effective_fullscreen = self
            .state
            .file_diffs
            .get(self.state.current_file)
            .map_or(self.state.diff_fullscreen, |d| {
                d.effective_fullscreen(self.state.diff_fullscreen)
            });

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.annotation_editor.is_some() {
                    dismiss_editor_on_outside_click(
                        &mut self.state,
                        &mut self.annotation_editor,
                        &mut self.active_modal,
                        mouse.column,
                        mouse.row,
                    );
                    return;
                }

                let hit_thread = self.state.annotation_rects.iter().find_map(|hit| {
                    rect_contains(hit.area, mouse.column, mouse.row)
                        .then(|| (hit.thread_id.clone(), hit.expand_rows.contains(&mouse.row)))
                });
                if let Some((id, clicked_expand)) = hit_thread {
                    if let Some(thread) = self.state.thread_by_id(&id) {
                        if clicked_expand {
                            self.active_modal = Some(Modal::thread_view(
                                thread.id.clone(),
                                self.state.settings.tab_width,
                                false,
                            ));
                        } else {
                            self.annotation_editor = Some(AnnotationEditor::for_thread(thread));
                        }
                    }
                    return;
                }

                if self.state.stacked_mode && mouse.row < header_height {
                    if mouse.column < 4 && self.state.current_commit_index > 0 {
                        let new_index = self.state.current_commit_index - 1;
                        navigate_stacked_commit(
                            &mut self.state,
                            new_index,
                            &self.options,
                            self.backend,
                        );
                    } else if mouse.column >= term_size.width.saturating_sub(4)
                        && self.state.current_commit_index
                            < self.state.stacked_commits.len().saturating_sub(1)
                    {
                        let new_index = self.state.current_commit_index + 1;
                        navigate_stacked_commit(
                            &mut self.state,
                            new_index,
                            &self.options,
                            self.backend,
                        );
                    }
                } else if self.state.show_sidebar
                    && mouse.column < sidebar_width
                    && mouse.row >= header_height
                    && mouse.row < term_size.height.saturating_sub(footer_height)
                {
                    self.state.clear_selection();
                    let clicked_row = (mouse.row.saturating_sub(header_height + 1)) as usize
                        + self.state.sidebar_scroll;
                    if let Some(item) = self.state.sidebar_item_at_visible(clicked_row).cloned() {
                        self.state.sidebar_selected = clicked_row;
                        match item {
                            SidebarItem::File { file_index, .. } => {
                                self.state.focused_panel = FocusedPanel::DiffView;
                                self.state.select_file(file_index);
                            }
                            SidebarItem::Directory { path, .. } => {
                                self.state.focused_panel = FocusedPanel::Sidebar;
                                self.state.toggle_directory(&path);
                                ensure_sidebar_visible(
                                    &mut self.state,
                                    term_size.height.saturating_sub(5) as usize,
                                );
                            }
                        }
                    }
                } else if mouse.column >= sidebar_width
                    && mouse.row >= header_height
                    && mouse.row < term_size.height.saturating_sub(footer_height)
                    && !self.state.file_diffs.is_empty()
                {
                    self.state.focused_panel = FocusedPanel::DiffView;
                    self.state.clear_selection();
                    let layout = PanelLayout::calculate(
                        term_size.width,
                        sidebar_width,
                        self.state.show_sidebar,
                        effective_fullscreen,
                    );
                    let content_start_y = header_height + 1;
                    if let Some(panel) = layout.panel_at_x(mouse.column) {
                        if mouse.row >= content_start_y {
                            let rel_y = (mouse.row - content_start_y) as usize;
                            if rel_y < self.state.content_row_offset {
                                return;
                            }
                            let content_y = rel_y - self.state.content_row_offset;
                            let Some(adjusted_y) = self.state.adjust_for_overlay_gaps(content_y)
                            else {
                                return;
                            };
                            let line = self.state.scroll as usize + adjusted_y;
                            if line >= self.state.side_by_side_ref().len() {
                                return;
                            }
                            let column = self.mouse_column_to_content(&layout, panel, mouse.column);
                            let mode = if layout.is_in_gutter(mouse.column, panel) {
                                SelectionMode::Line
                            } else {
                                SelectionMode::Character
                            };
                            self.state.start_selection(
                                panel,
                                CursorPosition { line, column },
                                mode,
                            );
                        }
                    }
                } else if mouse.column >= sidebar_width {
                    self.state.focused_panel = FocusedPanel::DiffView;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if !self.state.is_dragging || self.state.file_diffs.is_empty() {
                    return;
                }
                let panel = self.state.selection.panel;
                let content_start_y = header_height + 1;
                if panel == DiffPanelFocus::None || mouse.row < content_start_y {
                    return;
                }
                let layout = PanelLayout::calculate(
                    term_size.width,
                    sidebar_width,
                    self.state.show_sidebar,
                    effective_fullscreen,
                );
                let rel_y = (mouse.row - content_start_y) as usize;
                let content_y = rel_y.saturating_sub(self.state.content_row_offset);
                let adjusted_y = self.state.adjust_for_overlay_gaps_clamped(content_y);
                let sbs_len = self.state.side_by_side_ref().len();
                let line = (self.state.scroll as usize + adjusted_y).min(sbs_len.saturating_sub(1));
                let column = self.mouse_column_to_content(&layout, panel, mouse.column);
                self.state.extend_selection(CursorPosition { line, column });
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.state.end_drag();
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let mut scroll_delta: i32 = match mouse.kind {
                    MouseEventKind::ScrollDown => 3,
                    MouseEventKind::ScrollUp => -3,
                    _ => 0,
                };
                while let Some(Event::Mouse(next)) = self.pending_events.front() {
                    match next.kind {
                        MouseEventKind::ScrollDown => {
                            scroll_delta += 3;
                            self.pending_events.pop_front();
                        }
                        MouseEventKind::ScrollUp => {
                            scroll_delta -= 3;
                            self.pending_events.pop_front();
                        }
                        _ => break,
                    }
                }

                let in_sidebar = self.state.show_sidebar
                    && mouse.column < sidebar_width
                    && mouse.row < term_size.height.saturating_sub(footer_height);
                let in_diff = mouse.column >= sidebar_width
                    && mouse.row < term_size.height.saturating_sub(footer_height);

                if in_sidebar {
                    let max_sidebar_scroll = self.state.sidebar_visible_len().saturating_sub(1);
                    if scroll_delta > 0 {
                        self.state.sidebar_scroll = (self.state.sidebar_scroll
                            + scroll_delta as usize)
                            .min(max_sidebar_scroll);
                    } else {
                        self.state.sidebar_scroll = self
                            .state
                            .sidebar_scroll
                            .saturating_sub((-scroll_delta) as usize);
                    }
                } else if in_diff {
                    let max_scroll =
                        self.max_scroll(term_size.height).min(u16::MAX as usize) as u16;
                    if scroll_delta > 0 {
                        self.state.scroll = self
                            .state
                            .scroll
                            .saturating_add(scroll_delta as u16)
                            .min(max_scroll);
                    } else {
                        self.state.scroll =
                            self.state.scroll.saturating_sub((-scroll_delta) as u16);
                    }
                }
            }
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                let mut h_delta: i32 = match mouse.kind {
                    MouseEventKind::ScrollRight => 4,
                    MouseEventKind::ScrollLeft => -4,
                    _ => 0,
                };
                while let Some(Event::Mouse(next)) = self.pending_events.front() {
                    match next.kind {
                        MouseEventKind::ScrollRight => {
                            h_delta += 4;
                            self.pending_events.pop_front();
                        }
                        MouseEventKind::ScrollLeft => {
                            h_delta -= 4;
                            self.pending_events.pop_front();
                        }
                        _ => break,
                    }
                }

                let in_sidebar = self.state.show_sidebar
                    && mouse.column < sidebar_width
                    && mouse.row < term_size.height.saturating_sub(footer_height);
                let in_diff = mouse.column >= sidebar_width
                    && mouse.row < term_size.height.saturating_sub(footer_height);

                if in_sidebar {
                    if h_delta > 0 {
                        self.state.sidebar_h_scroll =
                            self.state.sidebar_h_scroll.saturating_add(h_delta as u16);
                        clamp_sidebar_h_scroll(&mut self.state, term_size.width);
                    } else {
                        self.state.sidebar_h_scroll = self
                            .state
                            .sidebar_h_scroll
                            .saturating_sub((-h_delta) as u16);
                    }
                } else if in_diff {
                    if self.state.settings.wrap {
                        self.state.h_scroll = 0;
                    } else if h_delta > 0 {
                        self.state.h_scroll = self.state.h_scroll.saturating_add(h_delta as u16);
                        clamp_h_scroll(&mut self.state, term_size.width);
                    } else {
                        self.state.h_scroll = self.state.h_scroll.saturating_sub((-h_delta) as u16);
                    }
                }
            }
            _ => {}
        }
    }

    fn mouse_column_to_content(
        &self,
        layout: &PanelLayout,
        panel: DiffPanelFocus,
        column: u16,
    ) -> usize {
        let rel_x = column.saturating_sub(layout.panel_x(panel));
        let content_offset = layout.content_x_offset(panel);
        if rel_x >= content_offset {
            (rel_x - content_offset + self.state.h_scroll) as usize
        } else {
            0
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent, term_size: Rect) -> LoopAction {
        let visible_height = term_size.height.saturating_sub(2) as usize;
        let max_scroll = self.max_scroll(term_size.height);

        if key.code != KeyCode::Char('g') {
            self.state.pending_key = PendingKey::None;
        }
        self.state.show_selection_tooltip = false;

        match key.code {
            KeyCode::Esc | KeyCode::Char('c')
                if (key.code == KeyCode::Esc || key.modifiers.contains(KeyModifiers::CONTROL))
                    && self.state.selection.is_active() =>
            {
                self.state.clear_selection();
            }
            KeyCode::Esc | KeyCode::Char('c')
                if (key.code == KeyCode::Esc || key.modifiers.contains(KeyModifiers::CONTROL))
                    && self.state.search_state.has_query() =>
            {
                self.state.search_state.clear();
                self.state.mark_search_dirty();
            }
            KeyCode::Char('q') | KeyCode::Char('c')
                if key.code == KeyCode::Char('q')
                    || key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                let queued = self.queued_total();
                if queued == 0 {
                    return LoopAction::Quit;
                }
                let queued_comments = count_of(queued, "queued comment");
                let send_and_exit = if self.agent.is_attached() {
                    format!("Send {queued_comments} to the agent and exit")
                } else {
                    format!("Write {queued_comments} to stdout and exit")
                };
                self.active_modal = Some(Modal::select(
                    "Queued comments",
                    vec![
                        send_and_exit,
                        "Exit and keep them queued".to_string(),
                        "Cancel".to_string(),
                    ],
                    0,
                ));
                self.open_picker = Some(Picker::Quit);
            }
            KeyCode::Char('1') => {
                self.state.focused_panel = FocusedPanel::Sidebar;
                self.state.show_sidebar = true;
                if !matches!(
                    self.state
                        .sidebar_item_at_visible(self.state.sidebar_selected),
                    Some(SidebarItem::File { .. })
                ) {
                    if let Some(idx) = self.state.sidebar_visible.iter().position(|idx| {
                        matches!(self.state.sidebar_items[*idx], SidebarItem::File { .. })
                    }) {
                        self.state.sidebar_selected = idx;
                    }
                }
            }
            KeyCode::Char('2') => {
                self.state.focused_panel = FocusedPanel::DiffView;
            }
            KeyCode::Tab => {
                self.state.show_sidebar = !self.state.show_sidebar;
                if !self.state.show_sidebar {
                    self.state.focused_panel = FocusedPanel::DiffView;
                }
            }
            KeyCode::Down | KeyCode::Char('k') if changes_file(&key) => {
                self.select_adjacent_file(true, term_size.height);
            }
            KeyCode::Up | KeyCode::Char('j') if changes_file(&key) => {
                self.select_adjacent_file(false, term_size.height);
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.state.stacked_mode
                    && self.state.current_commit_index
                        < self.state.stacked_commits.len().saturating_sub(1)
                {
                    let next = self.state.current_commit_index + 1;
                    navigate_stacked_commit(&mut self.state, next, &self.options, self.backend);
                }
            }
            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.state.stacked_mode && self.state.current_commit_index > 0 {
                    let prev = self.state.current_commit_index - 1;
                    navigate_stacked_commit(&mut self.state, prev, &self.options, self.backend);
                }
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let half_screen = (visible_height / 2) as u16;
                let max_scroll_u16 = max_scroll.min(u16::MAX as usize) as u16;
                self.state.scroll = self
                    .state
                    .scroll
                    .saturating_add(half_screen)
                    .min(max_scroll_u16);
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let half_screen = (visible_height / 2) as u16;
                self.state.scroll = self.state.scroll.saturating_sub(half_screen);
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if !self.state.file_diffs.is_empty() {
                    let items: Vec<FilePickerItem> = self
                        .state
                        .file_diffs
                        .iter()
                        .enumerate()
                        .map(|(i, diff)| FilePickerItem {
                            name: diff.filename.clone(),
                            file_index: i,
                            status: diff.status,
                            viewed: self.state.viewed_files.contains(&i),
                        })
                        .collect();
                    self.active_modal = Some(Modal::file_picker("Find File", items));
                }
            }
            KeyCode::Char('-') => {
                if let Some(diff) = self.state.file_diffs.get(self.state.current_file) {
                    if !diff.new_content.is_empty() {
                        self.state.diff_fullscreen = match self.state.diff_fullscreen {
                            DiffFullscreen::NewOnly => DiffFullscreen::None,
                            _ => DiffFullscreen::NewOnly,
                        };
                        self.state.mark_search_dirty();
                    }
                }
            }
            KeyCode::Char('0') => {
                if let Some(diff) = self.state.file_diffs.get(self.state.current_file) {
                    if !diff.old_content.is_empty() {
                        self.state.diff_fullscreen = match self.state.diff_fullscreen {
                            DiffFullscreen::OldOnly => DiffFullscreen::None,
                            _ => DiffFullscreen::OldOnly,
                        };
                        self.state.mark_search_dirty();
                    }
                }
            }
            KeyCode::Char('=') => {
                self.state.diff_fullscreen = DiffFullscreen::None;
                self.state.mark_search_dirty();
            }
            KeyCode::Down
                if self.state.search_state.has_query()
                    && self.state.focused_panel == FocusedPanel::DiffView =>
            {
                if let Some(line) = self.state.search_state.find_next() {
                    self.state.scroll =
                        adjust_scroll_to_line(line, self.state.scroll, visible_height, max_scroll);
                }
            }
            KeyCode::Up
                if self.state.search_state.has_query()
                    && self.state.focused_panel == FocusedPanel::DiffView =>
            {
                if let Some(line) = self.state.search_state.find_prev() {
                    self.state.scroll =
                        adjust_scroll_to_line(line, self.state.scroll, visible_height, max_scroll);
                }
            }
            KeyCode::Down | KeyCode::Char('k') => {
                if self.state.focused_panel == FocusedPanel::Sidebar {
                    if self.state.sidebar_selected + 1 < self.state.sidebar_visible_len() {
                        self.state.sidebar_selected += 1;
                    }
                    ensure_sidebar_visible(
                        &mut self.state,
                        term_size.height.saturating_sub(5) as usize,
                    );
                } else {
                    let max_scroll_u16 = max_scroll.min(u16::MAX as usize) as u16;
                    self.state.scroll = self.state.scroll.saturating_add(1).min(max_scroll_u16);
                }
            }
            KeyCode::Up | KeyCode::Char('j') => {
                if self.state.focused_panel == FocusedPanel::Sidebar {
                    self.state.sidebar_selected = self.state.sidebar_selected.saturating_sub(1);
                    ensure_sidebar_visible(
                        &mut self.state,
                        term_size.height.saturating_sub(5) as usize,
                    );
                } else {
                    self.state.scroll = self.state.scroll.saturating_sub(1);
                }
            }
            KeyCode::Char('z') => {
                let hunks_only = !self.state.settings.hunks_only;
                self.state.set_hunks_only(hunks_only, visible_height);
                self.set_notice(if hunks_only {
                    "hunks only"
                } else {
                    "full file"
                });
            }
            KeyCode::Char('w') => {
                if self.pr_info.is_none() {
                    self.state.watching = !self.state.watching;
                    if self.state.watching {
                        if let Some((handle, rx)) = setup_watcher(self.backend.working_tree_root())
                        {
                            self.watch_handle = Some(handle);
                            self.watch_rx = Some(rx);
                        } else {
                            self.state.watching = false;
                        }
                    } else {
                        self.watch_handle = None;
                        self.watch_rx = None;
                    }
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if self.state.focused_panel == FocusedPanel::DiffView && !self.state.settings.wrap {
                    self.state.h_scroll = self.state.h_scroll.saturating_sub(4);
                } else if self.state.focused_panel == FocusedPanel::Sidebar {
                    self.state.sidebar_h_scroll = self.state.sidebar_h_scroll.saturating_sub(4);
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if self.state.focused_panel == FocusedPanel::DiffView && !self.state.settings.wrap {
                    self.state.h_scroll = self.state.h_scroll.saturating_add(4);
                    clamp_h_scroll(&mut self.state, term_size.width);
                } else if self.state.focused_panel == FocusedPanel::Sidebar {
                    self.state.sidebar_h_scroll = self.state.sidebar_h_scroll.saturating_add(4);
                    clamp_sidebar_h_scroll(&mut self.state, term_size.width);
                }
            }
            KeyCode::Enter => {
                if self.state.focused_panel == FocusedPanel::Sidebar {
                    if let Some(item) = self
                        .state
                        .sidebar_item_at_visible(self.state.sidebar_selected)
                        .cloned()
                    {
                        match item {
                            SidebarItem::File { file_index, .. } => {
                                self.state.select_file(file_index);
                                self.state.focused_panel = FocusedPanel::DiffView;
                            }
                            SidebarItem::Directory { path, .. } => {
                                self.state.toggle_directory(&path);
                                ensure_sidebar_visible(
                                    &mut self.state,
                                    term_size.height.saturating_sub(5) as usize,
                                );
                            }
                        }
                    }
                }
            }
            KeyCode::Char(' ') => self.handle_toggle_viewed(term_size.height),
            KeyCode::PageDown => {
                let max_scroll_u16 = max_scroll.min(u16::MAX as usize) as u16;
                self.state.scroll = self.state.scroll.saturating_add(20).min(max_scroll_u16);
            }
            KeyCode::PageUp => {
                self.state.scroll = self.state.scroll.saturating_sub(20);
            }
            KeyCode::Char(']') => self.step_hunk(true, visible_height, max_scroll),
            KeyCode::Char('[') => self.step_hunk(false, visible_height, max_scroll),
            KeyCode::Char('m') => self.toggle_hunk_viewed(visible_height, max_scroll),
            KeyCode::Char('i') => {
                let had_selection = lines_are_selected(&self.state);
                if let Some((filename, target)) = target_at_cursor(&mut self.state) {
                    self.annotation_editor = Some(
                        match thread_at_cursor(&self.state, &filename, &target, false) {
                            Some(thread) => AnnotationEditor::for_thread(thread),
                            None => AnnotationEditor::new(filename, target),
                        },
                    );
                }
                if had_selection {
                    self.state.clear_selection();
                }
            }
            KeyCode::Char('t') => {
                let thread_id = target_at_cursor(&mut self.state).and_then(|(filename, target)| {
                    thread_at_cursor(&self.state, &filename, &target, true)
                        .map(|thread| thread.id.clone())
                });
                match thread_id {
                    Some(thread_id) => {
                        self.active_modal = Some(Modal::thread_view(
                            thread_id,
                            self.state.settings.tab_width,
                            false,
                        ));
                    }
                    None => self.set_notice("no thread here"),
                }
            }
            KeyCode::Char('x') => {
                let thread_id = target_at_cursor(&mut self.state).and_then(|(filename, target)| {
                    thread_at_cursor(&self.state, &filename, &target, true)
                        .map(|thread| thread.id.clone())
                });
                match thread_id {
                    Some(thread_id) => self.state.toggle_thread_resolved(&thread_id),
                    None => self.set_notice("no thread here"),
                }
            }
            KeyCode::Char('I') => {
                if !self.state.threads.is_empty() {
                    self.active_modal = Some(threads_modal(
                        &self.state.threads,
                        None,
                        self.show_resolved_threads,
                    ));
                }
            }
            KeyCode::Char('r') => {
                self.state.needs_reload = true;
            }
            KeyCode::Char('R') => {
                if let Some(session) = &self.workspace_session {
                    self.active_modal = Some(Modal::select(
                        "Repositories",
                        session.picker_items(&self.state),
                        session.active(),
                    ));
                    self.open_picker = Some(Picker::Repository);
                }
            }
            KeyCode::Char('A') => {
                self.active_modal = Some(agent_picker(&self.agent));
                self.open_picker = Some(Picker::Agent);
                self.agent_worker.request(AgentRequest::FindCandidates {
                    review_roots: self.review_roots.clone(),
                });
            }
            KeyCode::Char('s') => {
                let queued = self.queued_total();
                if queued == 0 {
                    self.set_notice("no queued comments");
                } else if self.agent.is_attached() {
                    let message = self.send_queued(None).unwrap_or_else(|error| error);
                    self.set_notice(message);
                } else {
                    let msg = format!(
                        "No agent is attached. Exit lumiere and write {} to stdout?\n\n\
                         Use this to pipe feedback back to a coding agent.",
                        count_of(queued, "queued comment"),
                    );
                    self.active_modal = Some(Modal::confirm("Send comments", msg));
                }
            }
            KeyCode::Char('y') => {
                if !self.state.file_diffs.is_empty() {
                    if self.state.selection.is_active() {
                        self.state.ensure_cache();
                        if let Some(text) = extract_selected_text(
                            &self.state.selection,
                            self.state.side_by_side_ref(),
                        ) {
                            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                let _ = clipboard.set_text(&text);
                            }
                        }
                        self.state.clear_selection();
                    } else if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard
                            .set_text(&self.state.file_diffs[self.state.current_file].filename);
                    }
                }
            }
            KeyCode::Char('e') => {
                if !self.state.file_diffs.is_empty() {
                    return LoopAction::OpenExternalEditor;
                }
            }
            KeyCode::Char('o') => {
                if let Some(ref pr) = self.pr_info {
                    if let Some(diff) = self.state.file_diffs.get(self.state.current_file) {
                        let file_url = format!(
                            "https://github.com/{}/{}/pull/{}/files#diff-{}",
                            pr.repo_owner,
                            pr.repo_name,
                            pr.number,
                            generate_file_anchor(&diff.filename)
                        );
                        let _ = open_url(&file_url);
                    }
                }
            }
            KeyCode::Char('g') => {
                if self.state.pending_key == PendingKey::G {
                    self.state.scroll = 0;
                    self.state.pending_key = PendingKey::None;
                } else {
                    self.state.pending_key = PendingKey::G;
                }
            }
            KeyCode::Char('G') => {
                self.state.scroll = max_scroll.min(u16::MAX as usize) as u16;
            }
            KeyCode::Char('/') => {
                self.state.search_state.start_forward();
                self.state.mark_search_dirty();
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let gs = GlobalSearchState::build(&self.state.file_diffs, &self.state.settings);
                self.active_modal = Some(Modal::global_search("Search", gs));
            }
            KeyCode::Char('n') if self.state.search_state.has_query() => {
                if let Some(line) = self.state.search_state.find_next() {
                    self.state.scroll =
                        adjust_scroll_to_line(line, self.state.scroll, visible_height, max_scroll);
                }
            }
            KeyCode::Char('N') if self.state.search_state.has_query() => {
                if let Some(line) = self.state.search_state.find_prev() {
                    self.state.scroll =
                        adjust_scroll_to_line(line, self.state.scroll, visible_height, max_scroll);
                }
            }
            KeyCode::Char('?') => {
                self.active_modal = Some(Modal::keybindings("Keybindings", help_sections()));
            }
            _ => {}
        }

        LoopAction::Continue
    }

    fn select_adjacent_file(&mut self, forward: bool, term_height: u16) {
        if self.state.file_diffs.is_empty() {
            return;
        }
        let found = if forward {
            ((self.state.sidebar_selected + 1)..self.state.sidebar_visible_len()).find_map(|idx| {
                match self.state.sidebar_item_at_visible(idx) {
                    Some(SidebarItem::File { file_index, .. }) => Some((idx, *file_index)),
                    _ => None,
                }
            })
        } else {
            (0..self.state.sidebar_selected).rev().find_map(|idx| {
                match self.state.sidebar_item_at_visible(idx) {
                    Some(SidebarItem::File { file_index, .. }) => Some((idx, *file_index)),
                    _ => None,
                }
            })
        };
        if let Some((visible_idx, file_index)) = found {
            self.state.sidebar_selected = visible_idx;
            self.state.select_file(file_index);
            ensure_sidebar_visible(&mut self.state, term_height.saturating_sub(5) as usize);
        }
    }

    fn handle_toggle_viewed(&mut self, term_height: u16) {
        if self.state.file_diffs.is_empty() {
            return;
        }
        if self.state.focused_panel == FocusedPanel::Sidebar {
            let Some(selected) = self
                .state
                .sidebar_item_at_visible(self.state.sidebar_selected)
                .cloned()
            else {
                return;
            };
            match selected {
                SidebarItem::File { file_index, .. } => {
                    let filename = self.state.file_diffs[file_index].filename.clone();
                    let was_viewed = self.state.viewed_files.contains(&file_index);
                    if was_viewed {
                        self.state.viewed_files.remove(&file_index);
                    } else {
                        self.state.viewed_files.insert(file_index);
                    }
                    self.sync_pr_file_viewed(&filename, !was_viewed);
                }
                SidebarItem::Directory { path, .. } => {
                    let child_indices: Vec<usize> = self
                        .state
                        .sidebar_items
                        .iter()
                        .filter_map(|item| match item {
                            SidebarItem::File {
                                path: file_path,
                                file_index,
                                ..
                            } if is_child_path(file_path, &path) => Some(*file_index),
                            _ => None,
                        })
                        .collect();
                    let all_viewed = child_indices
                        .iter()
                        .all(|i| self.state.viewed_files.contains(i));
                    for &idx in &child_indices {
                        if all_viewed {
                            self.state.viewed_files.remove(&idx);
                        } else {
                            self.state.viewed_files.insert(idx);
                        }
                        let filename = self.state.file_diffs[idx].filename.clone();
                        self.sync_pr_file_viewed(&filename, !all_viewed);
                    }
                }
            }
        } else {
            let current_file = self.state.current_file;
            let filename = self.state.file_diffs[current_file].filename.clone();
            let was_viewed = self.state.viewed_files.contains(&current_file);
            if was_viewed {
                self.state.viewed_files.remove(&current_file);
            } else {
                self.state.viewed_files.insert(current_file);
                let next_unviewed = ((self.state.sidebar_selected + 1)
                    ..self.state.sidebar_visible_len())
                    .chain(0..self.state.sidebar_selected)
                    .find_map(|visible_idx| {
                        let item_idx = self.state.sidebar_visible[visible_idx];
                        match &self.state.sidebar_items[item_idx] {
                            SidebarItem::File { file_index, .. }
                                if !self.state.viewed_files.contains(file_index) =>
                            {
                                Some((visible_idx, *file_index))
                            }
                            _ => None,
                        }
                    });
                if let Some((idx, file_idx)) = next_unviewed {
                    self.state.sidebar_selected = idx;
                    self.state.select_file(file_idx);
                    ensure_sidebar_visible(&mut self.state, term_height.saturating_sub(5) as usize);
                }
            }
            self.sync_pr_file_viewed(&filename, !was_viewed);
        }
    }

    fn sync_pr_file_viewed(&self, filename: &str, viewed: bool) {
        if let Some(ref pr) = self.pr_info {
            if viewed {
                mark_file_as_viewed_async(pr, filename);
            } else {
                unmark_file_as_viewed_async(pr, filename);
            }
        }
    }

    fn step_hunk(&mut self, forward: bool, visible_height: usize, max_scroll: usize) {
        if self.state.file_diffs.is_empty() {
            return;
        }
        self.state.clear_selection();
        let hunks = self.state.get_hunks().to_vec();
        if hunks.is_empty() {
            return;
        }
        let target = if forward {
            match self.state.focused_hunk {
                None => hunks
                    .iter()
                    .position(|&h| h > self.state.scroll as usize + 5)
                    .unwrap_or(0),
                Some(cur) => cur.saturating_add(1).min(hunks.len().saturating_sub(1)),
            }
        } else {
            match self.state.focused_hunk {
                None => hunks
                    .iter()
                    .rposition(|&h| (h as u16) < self.state.scroll.saturating_sub(5))
                    .unwrap_or(hunks.len().saturating_sub(1)),
                Some(cur) => cur.min(hunks.len().saturating_sub(1)).saturating_sub(1),
            }
        };
        self.state.focused_hunk = Some(target);
        self.state.scroll =
            adjust_scroll_for_hunk(hunks[target], self.state.scroll, visible_height, max_scroll);
    }

    fn toggle_hunk_viewed(&mut self, visible_height: usize, max_scroll: usize) {
        if self.state.focused_panel != FocusedPanel::DiffView || self.state.file_diffs.is_empty() {
            return;
        }
        let hunks = self.state.get_hunks().to_vec();
        let Some(hunk_idx) = self.state.focused_hunk.filter(|&i| i < hunks.len()) else {
            return;
        };
        let filename = self.state.file_diffs[self.state.current_file]
            .filename
            .clone();
        let entry = self.state.viewed_hunks.entry(filename).or_default();
        let was_viewed = entry.contains(&hunk_idx);
        if was_viewed {
            entry.remove(&hunk_idx);
        } else {
            entry.insert(hunk_idx);
            if hunk_idx + 1 < hunks.len() {
                let next = hunk_idx + 1;
                self.state.focused_hunk = Some(next);
                self.state.scroll = adjust_scroll_for_hunk(
                    hunks[next],
                    self.state.scroll,
                    visible_height,
                    max_scroll,
                );
            }
        }
    }

    pub fn set_notice(&mut self, message: impl Into<String>) {
        self.notice = Some((message.into(), Instant::now()));
    }
}

fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg(url).spawn()?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open").arg(url).spawn()?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .args(["/C", "start", url])
        .spawn()?;
    Ok(())
}

fn generate_file_anchor(filename: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(filename.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileDiff, FileStatus};

    #[test]
    fn test_an_editor_value_is_split_into_its_program_and_arguments() {
        assert_eq!(editor_words("hx"), ["hx"]);
        assert_eq!(editor_words("code --wait"), ["code", "--wait"]);
        assert_eq!(
            editor_words("'/opt/My Editor/bin/edit' -w"),
            ["/opt/My Editor/bin/edit", "-w"]
        );
        assert_eq!(editor_words("it's"), ["it's"]);
    }

    fn state_with_file(old_content: &str, new_content: &str) -> AppState {
        AppState::new(
            vec![FileDiff {
                filename: "a.rs".to_string(),
                old_content: old_content.to_string(),
                new_content: new_content.to_string(),
                status: FileStatus::Modified,
                is_binary: false,
            }],
            None,
        )
    }

    #[test]
    fn test_a_comment_on_the_focused_hunk_covers_the_lines_of_that_hunk() {
        let hunk_target = |old_content: &str, new_content: &str| {
            let mut state = state_with_file(old_content, new_content);
            assert_eq!(state.focused_hunk, Some(0));
            target_at_cursor(&mut state).expect("target").1
        };
        let lines = |panel, start_line, end_line| AnnotationTarget::LineRange {
            panel,
            start_line,
            end_line,
        };

        assert_eq!(
            hunk_target("keep\nkeep\na\nb\nc\nd\nend\n", "keep\nkeep\nA\nB\nend\n"),
            lines(DiffPanelFocus::New, 3, 4)
        );
        assert_eq!(
            hunk_target("keep\nend\n", "keep\nnew 1\nnew 2\nend\n"),
            lines(DiffPanelFocus::New, 2, 3)
        );
        assert_eq!(
            hunk_target("keep\ngone 1\ngone 2\nend\n", "keep\nend\n"),
            lines(DiffPanelFocus::Old, 2, 3)
        );
        assert_eq!(
            hunk_target("gone 1\ngone 2\n", ""),
            lines(DiffPanelFocus::Old, 1, 2)
        );

        let old_panel_target = |old_content: &str, new_content: &str| {
            let mut state = state_with_file(old_content, new_content);
            state.diff_fullscreen = DiffFullscreen::OldOnly;
            target_at_cursor(&mut state).expect("target").1
        };
        assert_eq!(
            old_panel_target("keep\nkeep\na\nb\nc\nd\nend\n", "keep\nkeep\nA\nB\nend\n"),
            lines(DiffPanelFocus::Old, 3, 6)
        );
        assert_eq!(
            old_panel_target("keep\nend\n", "keep\nnew 1\nnew 2\nend\n"),
            lines(DiffPanelFocus::New, 2, 3)
        );
    }

    #[test]
    fn test_a_hunk_thread_started_from_the_old_panel_is_found_with_both_panels_shown() {
        let mut state = state_with_file("keep\na\nb\nc\nend\n", "keep\nA\nB\nend\n");
        state.diff_fullscreen = DiffFullscreen::OldOnly;
        let (filename, old_lines) = target_at_cursor(&mut state).expect("target");
        let (thread_id, _) = state.add_thread(filename, old_lines, "why?".to_string());

        state.diff_fullscreen = DiffFullscreen::None;
        let (filename, target) = target_at_cursor(&mut state).expect("target");

        assert!(matches!(
            target,
            AnnotationTarget::LineRange {
                panel: DiffPanelFocus::New,
                ..
            }
        ));
        let found = thread_at_cursor(&state, &filename, &target, false).map(|thread| &thread.id);
        assert_eq!(found, Some(&thread_id));
    }

    #[test]
    fn test_threads_list_resolving_filtering_and_modal_navigation() {
        let mut state = state_with_file("a\n", "b\n");
        let (first_id, _) = state.add_thread(
            "a.rs".to_string(),
            AnnotationTarget::File,
            "first".to_string(),
        );
        let (second_id, _) = state.add_thread(
            "a.rs".to_string(),
            AnnotationTarget::File,
            "second".to_string(),
        );
        let mut list = threads_modal(&state.threads, Some(&second_id), true);

        let x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        let Some(ModalResult::AnnotationToggleResolved { annotation_id }) =
            list.handle_input(x, 40)
        else {
            panic!("x resolves selected thread");
        };
        assert_eq!(annotation_id, second_id);
        state.toggle_thread_resolved(&annotation_id);
        assert!(matches!(
            threads_modal(&state.threads, Some(&annotation_id), true).content,
            ModalContent::Annotations { selected: 1, .. }
        ));

        // Resolved threads stay hidden until `r` toggles them visible.
        let mut open_only = threads_modal(&state.threads, None, false);
        let ModalContent::Annotations {
            title, annotations, ..
        } = &open_only.content
        else {
            panic!("expected annotations modal");
        };
        assert_eq!(title, "Threads (1) · 1 resolved hidden");
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].id, first_id);

        let r = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE);
        let Some(ModalResult::AnnotationsToggleResolvedShown { selected_id }) =
            open_only.handle_input(r, 40)
        else {
            panic!("r toggles resolved visibility");
        };
        let with_resolved = threads_modal(&state.threads, selected_id.as_deref(), true);
        assert!(matches!(
            with_resolved.content,
            ModalContent::Annotations { selected: 0, ref annotations, .. } if annotations.len() == 2
        ));

        // Enter opens thread view, g jumps, y/o are unbound.
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert!(matches!(
            open_only.handle_input(enter, 40),
            Some(ModalResult::AnnotationOpen { ref annotation_id }) if annotation_id == &first_id
        ));
        let g = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
        assert!(matches!(
            open_only.handle_input(g, 40),
            Some(ModalResult::AnnotationJump { ref annotation_id }) if annotation_id == &first_id
        ));
        assert!(open_only
            .handle_input(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE), 40)
            .is_none());
        assert!(open_only
            .handle_input(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE), 40)
            .is_none());
    }

    #[test]
    fn test_showing_a_thread_puts_its_first_line_on_the_top_row() {
        let numbered_lines: String = (1..=80).map(|line| format!("line {line}\n")).collect();
        let mut state = AppState::new(
            vec![FileDiff {
                filename: "a.rs".to_string(),
                old_content: String::new(),
                new_content: numbered_lines,
                status: FileStatus::Added,
                is_binary: false,
            }],
            None,
        );
        let on_line = |line: usize| AnnotationTarget::LineRange {
            panel: DiffPanelFocus::New,
            start_line: line,
            end_line: line,
        };
        state.add_thread("a.rs".to_string(), on_line(6), "long\n".repeat(60));
        let (thread_id, _) = state.add_thread("a.rs".to_string(), on_line(30), "here".to_string());

        assert!(show_thread(&mut state, &thread_id));
        let top_row = &state.side_by_side_ref()[state.scroll as usize];
        assert_eq!(top_row.line_number(DiffPanelFocus::New), Some(30));
    }

    #[test]
    fn test_thread_modal_dismiss_and_delete_cleanup() {
        let mut state = state_with_file("a\n", "b\n");
        let (first_id, _) = state.add_thread(
            "a.rs".to_string(),
            AnnotationTarget::File,
            "first".to_string(),
        );
        let (second_id, _) = state.add_thread(
            "a.rs".to_string(),
            AnnotationTarget::File,
            "second".to_string(),
        );

        let mut from_diff = Some(Modal::thread_view(second_id.clone(), 4, false));
        dismiss_modal(&state, &mut from_diff, false);
        assert!(from_diff.is_none());

        let mut from_list = Some(Modal::thread_view(second_id.clone(), 4, true));
        dismiss_modal(&state, &mut from_list, false);
        assert!(matches!(
            from_list,
            Some(Modal {
                content: ModalContent::Annotations { selected: 1, .. }
            })
        ));

        let mut active_modal = Some(Modal::thread_view(second_id.clone(), 4, true));
        let mut editor = Some(AnnotationEditor::for_thread(
            state.thread_by_id(&second_id).unwrap(),
        ));
        state.remove_thread(&second_id);
        assert!(close_deleted_thread_modal(
            &state,
            &mut active_modal,
            &mut editor,
            false
        ));
        assert!(editor.is_none());
        assert!(matches!(
            active_modal,
            Some(Modal {
                content: ModalContent::Annotations { ref annotations, .. }
            }) if annotations.len() == 1 && annotations[0].id == first_id
        ));

        let mut last_modal = Some(Modal::thread_view(first_id.clone(), 4, true));
        state.remove_thread(&first_id);
        close_deleted_thread_modal(&state, &mut last_modal, &mut editor, false);
        assert!(last_modal.is_none());
    }
}
