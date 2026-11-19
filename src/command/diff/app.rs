use std::collections::VecDeque;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        KeyboardEnhancementFlags, MouseEventKind, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;

/// Writer that drives the TUI. Falls back to /dev/tty when stdout is
/// captured (e.g. when an agent shim does `output=$(lumiere diff)`),
/// so the alternate-screen escapes don't pollute the captured stdout
/// and we can reserve stdout for the annotation payload (`s` keybind).
fn open_tui_writer() -> io::Result<Box<dyn Write + Send>> {
    if io::stdout().is_terminal() {
        return Ok(Box::new(io::stdout()));
    }
    #[cfg(unix)]
    {
        if let Ok(f) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
        {
            return Ok(Box::new(f));
        }
    }
    Ok(Box::new(io::stdout()))
}

use super::agent::{AgentLink, AgentPane, RexCli};
use super::annotation::{AnnotationEditor, AnnotationEditorResult};
use super::coordinates::{extract_selected_text, PanelLayout};
use super::git::{
    get_current_branch, load_file_diffs, load_pr_file_diffs, load_single_commit_diffs,
};
use super::highlight;
use super::render::{
    render_diff, render_empty_state, truncate_path, FilePickerItem, KeyBind, KeyBindSection, Modal,
    ModalContent, ModalFileStatus, ModalResult, ReviewStatus,
};
use super::state::{adjust_scroll_for_hunk, adjust_scroll_to_line, AppState, PendingKey};
use super::theme;
use super::review_thread::{AnnotationTarget, Thread};
use super::thread_store::ThreadStore;
use super::types::{
    ChangeType, CursorPosition, DiffFullscreen, DiffPanelFocus, FileStatus, FocusedPanel,
    SelectionMode, SidebarItem,
};
use super::watcher::{setup_watcher, WatchEvent};
use super::workspace::{Workspace, WorkspaceSession};
use super::{
    fetch_viewed_files, mark_file_as_viewed_async, unmark_file_as_viewed_async, DiffOptions, PrInfo,
};
use spinoff::{spinners, Color, Spinner};

use crate::commit_reference::CommitReference;
use crate::vcs::{StackedCommitInfo, VcsBackend};

/// Navigate to a different commit in stacked mode.
/// Returns true if navigation was successful.
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
        state.load_stacked_viewed_files();
        true
    } else {
        false
    }
}

/// Adjust sidebar scroll to ensure the selected item is visible.
fn ensure_sidebar_visible(state: &mut AppState, visible_height: usize) {
    if state.sidebar_selected >= state.sidebar_scroll + visible_height {
        state.sidebar_scroll = state.sidebar_selected.saturating_sub(visible_height) + 1;
    } else if state.sidebar_selected < state.sidebar_scroll {
        state.sidebar_scroll = state.sidebar_selected;
    }
}

/// Compute the largest horizontal scroll offset that still keeps content
/// in view for the current file. Walks `side_by_side` for the longest line
/// on each side and compares against the panel widths from `PanelLayout`.
fn max_h_scroll(state: &mut AppState, term_width: u16) -> u16 {
    if state.settings.wrap {
        return 0;
    }
    if state.file_diffs.is_empty() {
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
    // The renderer collapses to a single panel for added or deleted files,
    // regardless of the diff_fullscreen toggle, so mirror that here.
    let effective_fullscreen = if diff.old_content.is_empty() && !diff.new_content.is_empty() {
        DiffFullscreen::NewOnly
    } else if !diff.old_content.is_empty() && diff.new_content.is_empty() {
        DiffFullscreen::OldOnly
    } else {
        state.diff_fullscreen
    };
    let layout = PanelLayout::calculate(
        term_width,
        sidebar_width,
        state.show_sidebar,
        effective_fullscreen,
    );

    state.ensure_cache();
    let sbs = state.side_by_side_ref();
    let mut max_old = 0usize;
    let mut max_new = 0usize;
    for line in sbs {
        if let Some((_, text)) = &line.old_line {
            max_old = max_old.max(text.chars().count());
        }
        if let Some((_, text)) = &line.new_line {
            max_new = max_new.max(text.chars().count());
        }
    }

    // Per-row overhead before the text: focus indicator (1 col) + line-number
    // gutter (5 cols). The new panel drops its indicator when it's rendered
    // alongside the old panel.
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

/// Clamp `state.h_scroll` against the rightmost meaningful offset for the
/// current file and terminal width.
fn clamp_h_scroll(state: &mut AppState, term_width: u16) {
    let max = max_h_scroll(state, term_width);
    if state.h_scroll > max {
        state.h_scroll = max;
    }
}

/// Largest sidebar horizontal scroll offset that keeps a file/directory entry
/// in view. Computed from the longest visible label minus the sidebar's inner
/// content width.
fn max_sidebar_h_scroll(state: &AppState, term_width: u16) -> u16 {
    if !state.show_sidebar {
        return 0;
    }
    let sidebar_width = (term_width / 4).clamp(20, 35) as usize;
    // Sidebar uses Borders::TOP | LEFT | BOTTOM (the right edge is shared
    // with the diff panel's left border), so only the left border eats width.
    let inner_width = sidebar_width.saturating_sub(1);

    let mut max_label = 0usize;
    for item in &state.sidebar_items {
        let len = match item {
            SidebarItem::Directory { name, depth, .. } => {
                depth * 2 + 2 + 1 + 1 + name.chars().count()
            }
            SidebarItem::File { name, depth, .. } => depth * 2 + 2 + 1 + 1 + name.chars().count(),
        };
        max_label = max_label.max(len);
    }
    max_label.saturating_sub(inner_width).min(u16::MAX as usize) as u16
}

/// Clamp `state.sidebar_h_scroll` against the rightmost meaningful offset.
fn clamp_sidebar_h_scroll(state: &mut AppState, term_width: u16) {
    let max = max_sidebar_h_scroll(state, term_width);
    if state.sidebar_h_scroll > max {
        state.sidebar_h_scroll = max;
    }
}

/// Find the side_by_side array index for a given file line number on a specific panel.
fn find_sbs_index_for_line(
    side_by_side: &[super::types::DiffLine],
    panel: DiffPanelFocus,
    line_num: usize,
) -> Option<usize> {
    side_by_side
        .iter()
        .position(|dl| dl.line_number(panel) == Some(line_num))
}

/// One row of the threads list: where the thread is, its state and latest comment,
/// and when that comment was written. The list splits the row at " | ".
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
        .map(|(_, state)| format!("[{}] ", state))
        .collect();
    let first_line = last_comment.body.lines().next().unwrap_or("").replace(" | ", " / ");
    preview.push_str(&format!("{}: {}", last_comment.author_name, first_line));
    if preview.chars().count() > 60 {
        preview = format!("{}...", preview.chars().take(60).collect::<String>());
    }

    let truncated_filename = truncate_path(&thread.filename, 30);
    let line_display = thread.line_range_display();
    let location = if line_display.is_empty() {
        truncated_filename
    } else {
        format!("{}:{}", truncated_filename, line_display)
    };
    format!(
        "{} [{}] | {} | {}",
        location,
        thread.target_label(),
        preview,
        last_comment.format_time()
    )
}

fn threads_modal(threads: &[Thread]) -> Modal {
    let mut sorted_threads = threads.to_vec();
    sorted_threads.sort_by_key(|thread| thread.created_at);
    let items = sorted_threads.iter().map(format_annotation_preview).collect();
    Modal::annotations("Threads", items, sorted_threads)
}

fn count_of(count: usize, noun: &str) -> String {
    format!("{} {}{}", count, noun, if count == 1 { "" } else { "s" })
}

/// What `i` and `x` act on in the current file: the selected lines, otherwise the
/// focused hunk, otherwise the whole file.
fn target_at_cursor(state: &mut AppState) -> Option<(String, AnnotationTarget)> {
    let diff = state.file_diffs.get(state.current_file)?;
    let filename = diff.filename.clone();
    let is_deleted = !diff.old_content.is_empty() && diff.new_content.is_empty();

    if state.selection.is_active() && !matches!(state.selection.mode, SelectionMode::None) {
        // Line-mode and character-mode selections both cover full lines.
        let panel = state.selection.panel;
        let sel_start = state.selection.anchor.line.min(state.selection.head.line);
        let sel_end = state.selection.anchor.line.max(state.selection.head.line);

        state.ensure_cache();
        let sbs = state.side_by_side_ref();
        let mut line_numbers = (sel_start..=sel_end)
            .filter_map(|idx| sbs.get(idx).and_then(|dl| dl.line_number(panel)));
        let start_line = line_numbers.next()?;
        let end_line = line_numbers.last().unwrap_or(start_line);
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
    let hunk_panel = if is_deleted {
        DiffPanelFocus::Old
    } else {
        DiffPanelFocus::New
    };

    state.ensure_cache();
    let sbs = state.side_by_side_ref();
    let hunks = state.hunks_ref();
    let hunk_start = hunks.get(hunk_index).copied().unwrap_or(0);
    let next_hunk_start = hunks.get(hunk_index + 1).copied().unwrap_or(sbs.len());

    let mut actual_hunk_end = hunk_start;
    for i in hunk_start..next_hunk_start {
        if let Some(dl) = sbs.get(i) {
            if !matches!(dl.change_type, ChangeType::Equal) {
                actual_hunk_end = i;
            }
        }
    }

    let line_num = |dl: &super::types::DiffLine| {
        dl.line_number(hunk_panel)
            .or_else(|| dl.line_number(DiffPanelFocus::Old))
    };
    let start_line = sbs.get(hunk_start).and_then(line_num).unwrap_or(1);
    let end_line = sbs
        .get(actual_hunk_end)
        .and_then(line_num)
        .unwrap_or(start_line);
    Some((
        filename,
        AnnotationTarget::LineRange {
            panel: hunk_panel,
            start_line,
            end_line,
        },
    ))
}

/// Puts a thread's file on screen with the thread's first line as the top row, so
/// that threads above it, however long, cannot push it out of view. Returns whether
/// the file is still in the diff.
fn show_thread(state: &mut AppState, thread_id: &str) -> bool {
    let Some(thread) = state.thread_by_id(thread_id) else {
        return false;
    };
    let filename = thread.filename.clone();
    let target = thread.target.clone();
    let Some(file_index) = state.file_diffs.iter().position(|f| f.filename == filename) else {
        return false;
    };
    state.select_file(file_index);

    // File-level threads are drawn above the first row wherever the file is scrolled.
    if let AnnotationTarget::LineRange { panel, start_line, .. } = target {
        state.ensure_cache();
        let first_row = find_sbs_index_for_line(state.side_by_side_ref(), panel, start_line);
        if let Some(first_row) = first_row {
            state.scroll = first_row as u16;
            // `i` and `x` act on the focused hunk: make it the one the thread is in.
            let hunk = state.hunks_ref().iter().rposition(|&hunk_start| hunk_start <= first_row);
            state.focused_hunk = hunk.or(state.focused_hunk);
        }
    }
    true
}

/// Stores what the editor holds as a queued comment. Returns the thread id and the
/// comment id, or `None` when the thread it belonged to no longer exists.
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

/// Whether an arrow or `j`/`k` key press asks for another file instead of scrolling:
/// with option held, or with ctrl held on a letter.
fn changes_file(key: &crossterm::event::KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::ALT)
        || (matches!(key.code, KeyCode::Char(_)) && key.modifiers.contains(KeyModifiers::CONTROL))
}

/// Removes the comment the editor was changing, when it was changing one.
fn remove_edited_comment(state: &mut AppState, editor: &AnnotationEditor) {
    if let (Some(thread_id), Some(comment_id)) = (&editor.thread_id, &editor.comment_id) {
        state.remove_comment(thread_id, comment_id);
    }
}

fn queued_total(state: &AppState, workspace_session: Option<&WorkspaceSession>) -> usize {
    match workspace_session {
        Some(session) => session.queued_count(state),
        None => state.queued_count(),
    }
}

/// Sends queued comments to the agent, then marks them as sent: the comment `only`,
/// or every queued comment. When the agent cannot be reached they stay queued.
fn send_queued(
    state: &mut AppState,
    workspace_session: Option<&mut WorkspaceSession>,
    agent: &mut AgentLink,
    only: Option<(&str, &str)>,
) -> Result<String, String> {
    let comments = match &workspace_session {
        Some(session) => session.queued_text_for_agent(state, only),
        None => state.queued_text_for_agent(only),
    };
    agent
        .send_comments(&comments)
        .map_err(|error| format!("not sent, still queued: {}", error))?;
    let sent = match only {
        Some(_) => state.mark_sent(only),
        None => {
            state.mark_sent(None)
                + workspace_session.map_or(0, |session| session.mark_parked_sent())
        }
    };
    agent.refresh();
    Ok(format!("sent {} to the agent", count_of(sent, "comment")))
}

/// What the open selection list is for.
enum Picker {
    Repository,
    Agent(Vec<AgentPane>),
    Quit,
}

pub fn run_app_with_pr(
    options: DiffOptions,
    pr_info: PrInfo,
    backend: &dyn VcsBackend,
) -> io::Result<()> {
    match load_pr_file_diffs(&pr_info) {
        Ok(file_diffs) => {
            run_app_internal(options, Some(pr_info), file_diffs, None, backend, None)
        }
        Err(_) => std::process::exit(1),
    }
}

pub fn run_app(
    options: DiffOptions,
    pr_info: Option<PrInfo>,
    backend: &dyn VcsBackend,
) -> io::Result<()> {
    let file_diffs = load_file_diffs(&options, backend);
    run_app_internal(options, pr_info, file_diffs, None, backend, None)
}

pub fn run_app_in_workspace(
    options: DiffOptions,
    workspace: &Workspace,
    initial_repository: usize,
) -> io::Result<()> {
    let backend = workspace
        .backend(initial_repository)
        .map_err(io::Error::other)?;
    let file_diffs = load_file_diffs(&options, backend);
    let session = WorkspaceSession::new(workspace, initial_repository);
    run_app_internal(options, None, file_diffs, None, backend, Some(session))
}

/// Loads the threads of the repository behind `backend` and keeps them saved.
pub(super) fn open_thread_store(state: &mut AppState, backend: &dyn VcsBackend) {
    match ThreadStore::for_repository(backend.working_tree_root()) {
        Ok(store) => state.open_thread_store(store),
        Err(error) => state.thread_store_error = Some(error),
    }
}

/// Review state for one repository, before any user interaction.
pub(super) fn new_review_state(
    options: &DiffOptions,
    file_diffs: Vec<super::types::FileDiff>,
    backend: &dyn VcsBackend,
    focus_file: Option<&str>,
) -> AppState {
    let mut state = AppState::new(file_diffs, focus_file);
    state.settings.wrap = options.wrap;
    state.set_vcs_name(backend.name());
    // Diff reference for annotation export context
    state.set_diff_reference(options.reference.as_ref().map(|r| match r {
        CommitReference::Single(s) => s.clone(),
        CommitReference::Range { from, to } => format!("{}..{}", from, to),
        CommitReference::TripleDots { from, to } => format!("{}...{}", from, to),
        CommitReference::RangeToWorkingTree { from } => format!("{}..-", from),
        CommitReference::MergeBaseToWorkingTree { from } => format!("{}...-", from),
    }));
    state
}

pub fn run_app_stacked(
    options: DiffOptions,
    commits: Vec<StackedCommitInfo>,
    backend: &dyn VcsBackend,
) -> io::Result<()> {
    // Load the first commit's diff
    let first_commit = &commits[0];
    let file_diffs = load_single_commit_diffs(&first_commit.commit_id, &options.file, backend);
    run_app_internal(options, None, file_diffs, Some(commits), backend, None)
}

/// Sync viewed files from GitHub to local state
fn sync_viewed_files_from_github(pr_info: &PrInfo, state: &mut AppState) {
    if let Ok(viewed_paths) = fetch_viewed_files(pr_info) {
        state.viewed_files.clear();
        for (idx, diff) in state.file_diffs.iter().enumerate() {
            if viewed_paths.contains(&diff.filename) {
                state.viewed_files.insert(idx);
            }
        }
    }
}

fn run_app_internal<'a>(
    options: DiffOptions,
    pr_info: Option<PrInfo>,
    file_diffs: Vec<super::types::FileDiff>,
    stacked_commits: Option<Vec<StackedCommitInfo>>,
    mut backend: &'a dyn VcsBackend,
    mut workspace_session: Option<WorkspaceSession<'a>>,
) -> io::Result<()> {
    theme::init(options.theme.as_deref());
    highlight::init();

    // Initialize state before TUI so we can sync viewed files
    let mut state = new_review_state(&options, file_diffs, backend, options.focus.as_deref());
    open_thread_store(&mut state, backend);
    if let Some(pr) = &pr_info {
        state.set_diff_reference(Some(format!(
            "PR #{} ({}...{})",
            pr.number, pr.base_ref, pr.head_ref
        )));
    }

    // Initialize stacked mode if commits were provided
    if let Some(commits) = stacked_commits {
        state.init_stacked_mode(commits);
    }

    // Load viewed files from GitHub on startup in PR mode (before TUI starts)
    if let Some(ref pr) = pr_info {
        let mut spinner = Spinner::new(
            spinners::Dots,
            format!("Syncing viewed status for {} files", state.file_diffs.len()),
            Color::Cyan,
        );
        sync_viewed_files_from_github(pr, &mut state);
        let viewed_count = state.viewed_files.len();
        spinner.success(&format!("{} files marked as viewed", viewed_count));
    }

    // Now enter TUI mode. Use /dev/tty when stdout is captured so the
    // alternate-screen escapes go to the real terminal, not the pipe.
    enable_raw_mode()?;
    let mut tui_writer = open_tui_writer()?;
    execute!(tui_writer, EnterAlternateScreen, EnableMouseCapture)?;
    // Opt into the kitty keyboard protocol so terminals that support it
    // (iTerm2 ≥3.5, kitty, wezterm, alacritty) deliver disambiguated key
    // events — Shift+Enter as KeyCode::Enter+SHIFT, etc. Older terminals
    // ignore this sequence silently, so we tolerate failure.
    let _ = execute!(
        tui_writer,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );

    let mut terminal = Terminal::new(CrosstermBackend::new(tui_writer))?;

    state.watching = options.watch && pr_info.is_none();
    let (mut _watch_handle, mut watch_rx) = if state.watching && pr_info.is_none() {
        if let Some((handle, rx)) = setup_watcher(backend.working_tree_root()) {
            (Some(handle), Some(rx))
        } else {
            state.watching = false;
            (None, None)
        }
    } else {
        (None, None)
    };

    let mut active_modal: Option<Modal> = None;
    let mut open_picker: Option<Picker> = None;
    let mut annotation_editor: Option<AnnotationEditor> = None;
    let mut pending_watch_event: Option<WatchEvent> = None;
    let mut pending_events: VecDeque<Event> = VecDeque::new();
    let mut send_annotations_on_exit = false;

    // The agent may work in any reviewed repository or in the folder lumiere was
    // started from, which is also the folder a workspace review covers.
    let canonical = |path: PathBuf| path.canonicalize().unwrap_or(path);
    let launch_dir = std::env::current_dir().ok().map(canonical);
    let mut review_roots: Vec<PathBuf> = match &workspace_session {
        Some(session) => session.repository_roots(),
        None => vec![backend.working_tree_root().to_path_buf()],
    }
    .into_iter()
    .map(canonical)
    .collect();
    let review_path = match (&workspace_session, &launch_dir) {
        (Some(_), Some(launch_dir)) => launch_dir.clone(),
        _ => review_roots[0].clone(),
    };
    review_roots.extend(launch_dir);
    let rex = RexCli;
    let rex_session = std::env::var("REX_SESSION").ok();
    let mut agent = AgentLink::attach(
        &rex,
        options.agent_block.as_deref(),
        rex_session.as_deref(),
        &review_roots,
        review_path,
    );
    let mut agent_refreshed_at = Instant::now();
    // A short message about the last action, shown in the footer for a few seconds.
    let mut notice: Option<(String, Instant)> = None;

    'main: loop {
        state.reload_threads_if_changed();
        if agent_refreshed_at.elapsed() > Duration::from_secs(3) {
            agent.refresh();
            agent_refreshed_at = Instant::now();
        }
        if notice.as_ref().is_some_and(|(_, shown_at)| shown_at.elapsed() > Duration::from_secs(6)) {
            notice = None;
        }
        if active_modal.is_none() {
            if let Some(error) = state.thread_store_error.take() {
                active_modal = Some(Modal::info(
                    "Threads are not saved",
                    format!("{}\n\nComments written from now on are lost when lumiere exits.", error),
                ));
            }
        }

        if let Some(ref rx) = watch_rx {
            match rx.try_recv() {
                Ok(event) => {
                    state.needs_reload = true;
                    pending_watch_event = Some(event);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {}
            }
        }

        if state.needs_reload {
            let file_diffs = if let Some(ref pr) = pr_info {
                // In PR mode, reload from GitHub
                match load_pr_file_diffs(pr) {
                    Ok(diffs) => diffs,
                    Err(e) => {
                        eprintln!("Warning: failed to reload PR diffs: {}", e);
                        Vec::new()
                    }
                }
            } else {
                load_file_diffs(&options, backend)
            };

            // Pass changed files to reload so it can unmark them from viewed
            let changed_files = pending_watch_event.take().map(|e| e.changed_files);
            state.reload(file_diffs, changed_files.as_ref());

            // Re-sync viewed files from GitHub in PR mode
            if let Some(ref pr) = pr_info {
                sync_viewed_files_from_github(pr, &mut state);
            }
        }

        if state.file_diffs.is_empty() {
            terminal.draw(|frame| {
                render_empty_state(
                    frame,
                    state.watching,
                    workspace_session.as_ref().map(|session| session.active_name()),
                );
                if let Some(ref modal) = active_modal {
                    modal.render(frame);
                }
            })?;
        } else {
            // Use cached side_by_side (avoids recomputing diff every frame during drag etc.)
            state.update_search_matches();
            // Ensure highlighters are cached (only recomputed when file changes)
            state.get_highlighters();
            let diff = &state.file_diffs[state.current_file];
            let side_by_side = state.side_by_side_ref();
            let hunks = state.hunks_ref();
            let (old_hl, new_hl) = state.highlighters_ref().unwrap();
            let hunk_count = hunks.len();
            let empty_viewed_hunks: std::collections::HashSet<usize> =
                std::collections::HashSet::new();
            let viewed_hunks_for_file = state
                .viewed_hunks
                .get(&diff.filename)
                .unwrap_or(&empty_viewed_hunks);
            let branch_fallback = get_current_branch(backend);
            let commit_ref = state.diff_reference.as_deref().unwrap_or(&branch_fallback);
            let workspace_commit_ref = workspace_session
                .as_ref()
                .map(|session| format!("{} · {}", session.active_name(), commit_ref));
            let commit_ref = workspace_commit_ref.as_deref().unwrap_or(commit_ref);
            let agent_label = agent.label();
            let review_status = ReviewStatus {
                agent: &agent_label,
                queued_comments: queued_total(&state, workspace_session.as_ref()),
                notice: notice.as_ref().map(|(text, _)| text.as_str()),
            };
            let row_offset = std::cell::Cell::new(0usize);
            let gaps_cell = std::cell::RefCell::new(Vec::new());
            let rects_cell = std::cell::RefCell::new(Vec::new());
            let editor_rect_cell: std::cell::Cell<Option<ratatui::layout::Rect>> = std::cell::Cell::new(None);
            terminal.draw(|frame| {
                let (offset, gaps, rects, er) = render_diff(
                    frame,
                    diff,
                    &state.file_diffs,
                    &state.cached_trees,
                    &state.sidebar_items,
                    &state.sidebar_visible,
                    &state.collapsed_dirs,
                    state.current_file,
                    state.scroll,
                    if state.settings.wrap {
                        0
                    } else {
                        state.h_scroll
                    },
                    state.watching,
                    state.show_sidebar,
                    state.focused_panel,
                    state.sidebar_selected,
                    state.sidebar_scroll,
                    state.sidebar_h_scroll,
                    &state.viewed_files,
                    &state.settings,
                    hunk_count,
                    state.diff_fullscreen,
                    &state.search_state,
                    commit_ref,
                    pr_info.as_ref(),
                    state.focused_hunk,
                    &hunks,
                    state.stacked_mode,
                    state.current_commit(),
                    state.current_commit_index,
                    state.stacked_commits.len(),
                    &side_by_side,
                    state.vcs_name,
                    &state.threads,
                    &state.selection,
                    old_hl,
                    new_hl,
                    viewed_hunks_for_file,
                    state.total_added,
                    state.total_removed,
                    annotation_editor.as_ref(),
                    &review_status,
                );
                row_offset.set(offset);
                *rects_cell.borrow_mut() = rects;
                editor_rect_cell.set(er);

                // Selection action tooltip (shown after drag completes)
                if state.show_selection_tooltip
                    && state.selection.is_active()
                    && !state.is_dragging
                    && annotation_editor.is_none()
                    && active_modal.is_none()
                {
                    let t = theme::get();
                    let term = frame.area();
                    let header_h: u16 = if state.stacked_mode { 1 } else { 0 };
                    let sidebar_w: u16 = if state.show_sidebar {
                        (term.width / 4).clamp(20, 35)
                    } else {
                        0
                    };
                    let layout = PanelLayout::calculate(
                        term.width,
                        sidebar_w,
                        state.show_sidebar,
                        state.diff_fullscreen,
                    );

                    let sel = &state.selection;
                    let (_, sel_end) = sel.normalized_range();
                    let scroll_usize = state.scroll as usize;

                    if sel_end.line >= scroll_usize {
                        let content_y = sel_end.line - scroll_usize;

                        // Account for annotation overlay gaps
                        let mut cum_gaps: u16 = 0;
                        for &(after_line, gap_h) in &gaps {
                            if after_line < content_y {
                                cum_gaps += gap_h as u16;
                            }
                        }

                        // Position below the selection end
                        let screen_y =
                            header_h + 1 + offset as u16 + content_y as u16 + cum_gaps + 1;

                        let (panel_x, panel_w) = match sel.panel {
                            DiffPanelFocus::Old => (layout.old_panel_x, layout.old_panel_width),
                            DiffPanelFocus::New => (layout.new_panel_x, layout.new_panel_width),
                            _ => (0, 0),
                        };

                        if panel_w > 0 && screen_y < term.height.saturating_sub(1) {
                            let tip_w: u16 = 27;
                            let tip_h: u16 = 1;

                            let cx = layout.content_x_offset(sel.panel);
                            let tip_x =
                                (panel_x + cx).min(panel_x + panel_w.saturating_sub(tip_w + 1));

                            let tip_area = Rect::new(tip_x, screen_y, tip_w.min(panel_w), tip_h);

                            let bg = t.ui.footer_branch_bg;
                            let key_style = Style::default().fg(t.ui.text_primary).bg(bg).bold();
                            let desc_style = Style::default().fg(t.ui.text_muted).bg(bg);
                            let tip_line = Line::from(vec![
                                Span::styled(" i", key_style),
                                Span::styled(" annotate ", desc_style),
                                Span::styled("y", key_style),
                                Span::styled(" copy ", desc_style),
                                Span::styled("esc", key_style),
                                Span::styled("   ", desc_style),
                            ]);

                            frame.render_widget(ratatui::widgets::Clear, tip_area);
                            frame.render_widget(
                                ratatui::widgets::Paragraph::new(tip_line)
                                    .style(Style::default().bg(bg)),
                                tip_area,
                            );
                        }
                    }
                }

                *gaps_cell.borrow_mut() = gaps;
                // Editor is rendered inline by render_diff above; only the modal
                // (annotations list, file picker, etc.) sits on top of everything.
                if let Some(ref modal) = active_modal {
                    modal.render(frame);
                }
            })?;
            state.content_row_offset = row_offset.get();
            state.annotation_overlay_gaps = gaps_cell.into_inner();
            state.annotation_rects = rects_cell.into_inner();
            state.editor_rect = editor_rect_cell.get();
        }

        // Poll for new events if no pending events
        if pending_events.is_empty() && event::poll(Duration::from_millis(100))? {
            pending_events.push_back(event::read()?);
        }

        // Process all pending events
        while let Some(current_event) = pending_events.pop_front() {
            let visible_height = terminal.size()?.height.saturating_sub(2) as usize;
            let bottom_padding = 5;
            let max_scroll = if !state.file_diffs.is_empty() {
                let total_lines = state.total_lines();
                let filename = &state.file_diffs[state.current_file].filename;
                if state.threads.iter().any(|thread| &thread.filename == filename) {
                    // Threads take rows below their lines: let the last line reach
                    // the top, so that a thread under it can be read.
                    total_lines.saturating_sub(1)
                } else {
                    total_lines.saturating_sub(visible_height.saturating_sub(bottom_padding))
                }
            } else {
                0
            };

            match current_event {
                Event::Key(key)
                    if key.kind == KeyEventKind::Press && state.search_state.is_active() =>
                {
                    match key.code {
                        KeyCode::Esc => {
                            state.search_state.cancel();
                            state.mark_search_dirty();
                        }
                        KeyCode::Enter => {
                            state.search_state.confirm();
                            if state.search_state.has_query() {
                                if let Some(line) = state
                                    .search_state
                                    .jump_to_first_match(state.scroll as usize)
                                {
                                    state.scroll = line.saturating_sub(5) as u16;
                                }
                            }
                        }
                        // Word-erase: opt+backspace (macOS) / ctrl+w (readline).
                        // Listed BEFORE plain Backspace so the modified variant
                        // matches first.
                        KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
                            state.search_state.erase_word();
                            state.mark_search_dirty();
                        }
                        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            state.search_state.erase_word();
                            state.mark_search_dirty();
                        }
                        KeyCode::Backspace => {
                            state.search_state.pop_char();
                            state.mark_search_dirty();
                        }
                        // Exclude modified char keys so combos like opt+letter
                        // don't insert the letter into the query.
                        KeyCode::Char(c)
                            if !key.modifiers.contains(KeyModifiers::ALT)
                                && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            state.search_state.push_char(c);
                            state.mark_search_dirty();
                        }
                        _ => {}
                    }
                }
                Event::Key(key)
                    if key.kind == KeyEventKind::Press
                        && annotation_editor.is_some()
                        && active_modal.is_none() =>
                {
                    if let Some(editor) = annotation_editor.as_mut() {
                        match editor.handle_input(key) {
                            AnnotationEditorResult::Continue => {}
                            result @ (AnnotationEditorResult::Send
                            | AnnotationEditorResult::Queue) => {
                                let saved = save_comment(&mut state, editor);
                                let send_now = matches!(result, AnnotationEditorResult::Send);
                                annotation_editor = None;
                                let message = match saved {
                                    None => "the thread no longer exists".to_string(),
                                    Some(_) if !send_now => "comment queued".to_string(),
                                    Some(_) if !agent.is_attached() => {
                                        "no agent attached: comment queued".to_string()
                                    }
                                    Some((thread_id, comment_id)) => send_queued(
                                        &mut state,
                                        None,
                                        &mut agent,
                                        Some((&thread_id, &comment_id)),
                                    )
                                    .unwrap_or_else(|error| error),
                                };
                                notice = Some((message, Instant::now()));
                            }
                            AnnotationEditorResult::Delete => {
                                remove_edited_comment(&mut state, editor);
                                annotation_editor = None;
                            }
                            AnnotationEditorResult::Cancel => {
                                annotation_editor = None;
                            }
                        }
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press && active_modal.is_some() => {
                    if let Some(ref mut modal) = active_modal {
                        let term_height = terminal.size()?.height;
                        if let Some(result) = modal.handle_input(key, term_height) {
                            match result {
                                ModalResult::FileSelected(file_index) => {
                                    state.reveal_file(file_index);
                                    state.select_file(file_index);
                                    if let Some(idx) =
                                        state.sidebar_visible_index_for_file(state.current_file)
                                    {
                                        state.sidebar_selected = idx;
                                        let visible_height =
                                            terminal.size()?.height.saturating_sub(5) as usize;
                                        ensure_sidebar_visible(&mut state, visible_height);
                                    }
                                    active_modal = None;
                                }
                                ModalResult::AnnotationJump { annotation_id } => {
                                    show_thread(&mut state, &annotation_id);
                                    active_modal = None;
                                }
                                ModalResult::AnnotationEdit { annotation_id } => {
                                    if show_thread(&mut state, &annotation_id) {
                                        annotation_editor = state
                                            .thread_by_id(&annotation_id)
                                            .map(AnnotationEditor::for_thread);
                                    }
                                    active_modal = None;
                                }
                                ModalResult::AnnotationDelete { annotation_id } => {
                                    state.remove_thread(&annotation_id);
                                    // Refresh the modal if there are still threads
                                    active_modal = if state.threads.is_empty() {
                                        None
                                    } else {
                                        Some(threads_modal(&state.threads))
                                    };
                                }
                                ModalResult::AnnotationCopyAll => {
                                    // Copy all annotations to clipboard
                                    let formatted = state.format_threads_for_export(false);
                                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                        let _ = clipboard.set_text(&formatted);
                                    }
                                    active_modal = None;
                                }
                                ModalResult::AnnotationExport(filename) => {
                                    // Write annotations to file
                                    let formatted = state.format_threads_for_export(false);
                                    match std::fs::write(&filename, &formatted) {
                                        Ok(_) => {
                                            active_modal = None;
                                        }
                                        Err(e) => {
                                            // Set error message on the modal
                                            if let Some(ref mut modal) = active_modal {
                                                if let ModalContent::Annotations {
                                                    error_message,
                                                    export_input,
                                                    ..
                                                } = &mut modal.content
                                                {
                                                    *error_message =
                                                        Some(format!("Failed to write: {}", e));
                                                    *export_input = None; // Close input, keep modal open
                                                }
                                            }
                                        }
                                    }
                                }
                                ModalResult::Confirmed => {
                                    send_annotations_on_exit = true;
                                    break 'main;
                                }
                                ModalResult::JumpToLine {
                                    file_index,
                                    sbs_line_index,
                                    panel: _panel,
                                } => {
                                    // Reveal + switch to the target file, then pin the
                                    // matched sbs line to the top of the content area.
                                    // The diff view renders sticky-context rows and any
                                    // file-level annotation slots ABOVE scroll=0, so
                                    // setting scroll = sbs_line_index lands the line
                                    // right beneath those — exactly where the user
                                    // expects "pinned to the top" to be.
                                    state.reveal_file(file_index);
                                    state.select_file(file_index);
                                    if let Some(idx) =
                                        state.sidebar_visible_index_for_file(state.current_file)
                                    {
                                        state.sidebar_selected = idx;
                                        let visible_height =
                                            terminal.size()?.height.saturating_sub(5) as usize;
                                        ensure_sidebar_visible(&mut state, visible_height);
                                    }
                                    // Compute max_scroll for the just-switched file
                                    // before pinning scroll, so we don't overscroll
                                    // past the last visible row.
                                    state.ensure_cache();
                                    let sbs_len = state.side_by_side_ref().len();
                                    let vh = terminal.size()?.height.saturating_sub(5) as usize;
                                    let max_scroll = sbs_len.saturating_sub(vh) as u16;
                                    state.scroll = (sbs_line_index as u16).min(max_scroll);
                                    active_modal = None;
                                }
                                ModalResult::Selected(selected_index, _) => {
                                    active_modal = None;
                                    match open_picker.take() {
                                        Some(Picker::Repository) => {
                                            if let Some(session) = workspace_session.as_mut() {
                                                match session.switch_to(
                                                    selected_index,
                                                    &options,
                                                    &mut state,
                                                ) {
                                                    Ok(repository_backend) => {
                                                        backend = repository_backend;
                                                        pending_watch_event = None;
                                                        let watcher = state
                                                            .watching
                                                            .then(|| setup_watcher(backend.working_tree_root()))
                                                            .flatten();
                                                        state.watching = watcher.is_some();
                                                        (_watch_handle, watch_rx) = watcher.unzip();
                                                    }
                                                    Err(error) => {
                                                        active_modal = Some(Modal::info(
                                                            "Cannot open repository",
                                                            error,
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                        // The first item is "none".
                                        Some(Picker::Agent(mut panes)) => {
                                            let pane = (selected_index > 0
                                                && selected_index <= panes.len())
                                            .then(|| panes.remove(selected_index - 1));
                                            agent.set_pane(pane);
                                        }
                                        Some(Picker::Quit) => match selected_index {
                                            0 if agent.is_attached() => {
                                                match send_queued(
                                                    &mut state,
                                                    workspace_session.as_mut(),
                                                    &mut agent,
                                                    None,
                                                ) {
                                                    Ok(_) => break 'main,
                                                    Err(error) => {
                                                        active_modal =
                                                            Some(Modal::info("Not sent", error));
                                                    }
                                                }
                                            }
                                            0 => {
                                                send_annotations_on_exit = true;
                                                break 'main;
                                            }
                                            1 => break 'main,
                                            _ => {}
                                        },
                                        None => {}
                                    }
                                }
                                ModalResult::Dismissed => {
                                    active_modal = None;
                                    open_picker = None;
                                }
                            }
                        }
                    }
                }
                Event::Mouse(mouse) if active_modal.is_some() => {
                    if let Some(ref mut modal) = active_modal {
                        let term_size = terminal.size()?;
                        modal.handle_mouse(mouse, term_size.width, term_size.height);
                    }
                }
                Event::Mouse(mouse) if active_modal.is_none() => {
                    let term_size = terminal.size()?;
                    let footer_height = 1u16;
                    let header_height = if state.stacked_mode { 1u16 } else { 0u16 };
                    let sidebar_width = if state.show_sidebar {
                        (term_size.width / 4).clamp(20, 35)
                    } else {
                        0u16
                    };

                    // For new/deleted files the renderer uses a single full-width panel,
                    // so override diff_fullscreen so PanelLayout matches.
                    let effective_fullscreen = if !state.file_diffs.is_empty() {
                        let d = &state.file_diffs[state.current_file];
                        if d.old_content.is_empty() && !d.new_content.is_empty() {
                            DiffFullscreen::NewOnly
                        } else if !d.old_content.is_empty() && d.new_content.is_empty() {
                            DiffFullscreen::OldOnly
                        } else {
                            state.diff_fullscreen
                        }
                    } else {
                        state.diff_fullscreen
                    };

                    match mouse.kind {
                        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                            // Inline annotation editor: click-outside saves (or cancels if empty).
                            // Click-inside is ignored (textarea cursor placement could be wired later).
                            if annotation_editor.is_some() {
                                let inside = state
                                    .editor_rect
                                    .map(|r| {
                                        mouse.column >= r.x
                                            && mouse.column < r.x + r.width
                                            && mouse.row >= r.y
                                            && mouse.row < r.y + r.height
                                    })
                                    .unwrap_or(false);
                                if !inside {
                                    if let Some(editor) = annotation_editor.as_ref() {
                                        if editor.is_empty() {
                                            remove_edited_comment(&mut state, editor);
                                        } else {
                                            save_comment(&mut state, editor);
                                        }
                                    }
                                    annotation_editor = None;
                                }
                                continue;
                            }
                            // Click on a thread → reply to it, or edit its unsent comment.
                            let hit_thread = state.annotation_rects.iter().find_map(|(id, r)| {
                                (mouse.column >= r.x
                                    && mouse.column < r.x + r.width
                                    && mouse.row >= r.y
                                    && mouse.row < r.y + r.height)
                                    .then(|| id.clone())
                            });
                            if let Some(id) = hit_thread {
                                if let Some(thread) = state.thread_by_id(&id) {
                                    annotation_editor = Some(AnnotationEditor::for_thread(thread));
                                }
                                continue;
                            }
                            // Check for stacked mode header arrow clicks
                            if state.stacked_mode && mouse.row < header_height {
                                // Left arrow click (first 4 columns to cover " < ")
                                if mouse.column < 4 && state.current_commit_index > 0 {
                                    let new_index = state.current_commit_index - 1;
                                    navigate_stacked_commit(
                                        &mut state, new_index, &options, backend,
                                    );
                                }
                                // Right arrow click (last 4 columns to cover " > ")
                                else if mouse.column >= term_size.width.saturating_sub(4)
                                    && state.current_commit_index
                                        < state.stacked_commits.len().saturating_sub(1)
                                {
                                    let new_index = state.current_commit_index + 1;
                                    navigate_stacked_commit(
                                        &mut state, new_index, &options, backend,
                                    );
                                }
                            } else if state.show_sidebar
                                && mouse.column < sidebar_width
                                && mouse.row >= header_height
                                && mouse.row < term_size.height.saturating_sub(footer_height)
                            {
                                state.clear_selection(); // Clear selection when clicking sidebar
                                let clicked_row = (mouse.row.saturating_sub(header_height + 1))
                                    as usize
                                    + state.sidebar_scroll;
                                if clicked_row < state.sidebar_visible_len() {
                                    let item = state.sidebar_item_at_visible(clicked_row).cloned();
                                    if let Some(item) = item {
                                        state.sidebar_selected = clicked_row;
                                        match item {
                                            SidebarItem::File { file_index, .. } => {
                                                state.focused_panel = FocusedPanel::DiffView;
                                                state.select_file(file_index);
                                            }
                                            SidebarItem::Directory { path, .. } => {
                                                state.focused_panel = FocusedPanel::Sidebar;
                                                state.toggle_directory(&path);
                                                let visible_height =
                                                    term_size.height.saturating_sub(5) as usize;
                                                if state.sidebar_selected < state.sidebar_scroll {
                                                    state.sidebar_scroll = state.sidebar_selected;
                                                } else if state.sidebar_selected
                                                    >= state.sidebar_scroll + visible_height
                                                {
                                                    state.sidebar_scroll = state
                                                        .sidebar_selected
                                                        .saturating_sub(visible_height)
                                                        + 1;
                                                }
                                            }
                                        }
                                    }
                                }
                            } else if mouse.column >= sidebar_width
                                && mouse.row >= header_height
                                && mouse.row < term_size.height.saturating_sub(footer_height)
                                && !state.file_diffs.is_empty()
                            {
                                state.focused_panel = FocusedPanel::DiffView;
                                // Any new click in the diff area clears the previous selection
                                state.clear_selection();

                                // Calculate layout for selection
                                let layout = PanelLayout::calculate(
                                    term_size.width,
                                    sidebar_width,
                                    state.show_sidebar,
                                    effective_fullscreen,
                                );

                                if let Some(panel) = layout.panel_at_x(mouse.column) {
                                    let is_gutter = layout.is_in_gutter(mouse.column, panel);
                                    let content_start_y = header_height + 1;

                                    // Coordinate calculation accounting for context lines and annotations
                                    if mouse.row >= content_start_y {
                                        let rel_y = (mouse.row - content_start_y) as usize;

                                        // Skip clicks on context lines or file annotation rows
                                        if rel_y < state.content_row_offset {
                                            continue;
                                        }
                                        let content_y = rel_y - state.content_row_offset;
                                        // Adjust for inline annotation overlay gaps
                                        let adjusted_y =
                                            match state.adjust_for_overlay_gaps(content_y) {
                                                Some(y) => y,
                                                None => continue, // Clicked inside an annotation overlay
                                            };
                                        let line = state.scroll as usize + adjusted_y;
                                        let sbs_len = state.side_by_side_ref().len();
                                        if line >= sbs_len {
                                            continue;
                                        }

                                        let panel_x = match panel {
                                            DiffPanelFocus::Old => layout.old_panel_x,
                                            DiffPanelFocus::New => layout.new_panel_x,
                                            DiffPanelFocus::None => 0,
                                        };

                                        let content_offset = layout.content_x_offset(panel);
                                        let rel_x = mouse.column.saturating_sub(panel_x);
                                        let column = if rel_x >= content_offset {
                                            (rel_x - content_offset + state.h_scroll) as usize
                                        } else {
                                            0
                                        };

                                        let mode = if is_gutter {
                                            SelectionMode::Line
                                        } else {
                                            SelectionMode::Character
                                        };
                                        let pos = CursorPosition { line, column };
                                        state.start_selection(panel, pos, mode);
                                    }
                                }
                            } else if mouse.column >= sidebar_width {
                                state.focused_panel = FocusedPanel::DiffView;
                            }
                        }
                        MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
                            if state.is_dragging && !state.file_diffs.is_empty() {
                                let panel = state.selection.panel;
                                if panel != DiffPanelFocus::None {
                                    let content_start_y = header_height + 1;

                                    if mouse.row >= content_start_y {
                                        let layout = PanelLayout::calculate(
                                            term_size.width,
                                            sidebar_width,
                                            state.show_sidebar,
                                            effective_fullscreen,
                                        );

                                        let rel_y = (mouse.row - content_start_y) as usize;
                                        // Account for context lines and file annotations
                                        let content_y =
                                            rel_y.saturating_sub(state.content_row_offset);
                                        // Adjust for inline annotation overlay gaps (clamped for drag)
                                        let adjusted_y =
                                            state.adjust_for_overlay_gaps_clamped(content_y);
                                        let line = state.scroll as usize + adjusted_y;
                                        // Clamp to valid side_by_side range
                                        let sbs_len = state.side_by_side_ref().len();
                                        let line = line.min(sbs_len.saturating_sub(1));

                                        let panel_x = match panel {
                                            DiffPanelFocus::Old => layout.old_panel_x,
                                            DiffPanelFocus::New => layout.new_panel_x,
                                            DiffPanelFocus::None => 0,
                                        };

                                        let content_offset = layout.content_x_offset(panel);
                                        let rel_x = mouse.column.saturating_sub(panel_x);
                                        let column = if rel_x >= content_offset {
                                            (rel_x - content_offset + state.h_scroll) as usize
                                        } else {
                                            0
                                        };

                                        let pos = CursorPosition { line, column };
                                        state.extend_selection(pos);
                                    }
                                }
                            }
                        }
                        MouseEventKind::Up(crossterm::event::MouseButton::Left) => {
                            state.end_drag();
                        }
                        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                            // Coalesce consecutive scroll events to handle fast scrolling.
                            // Non-scroll events are preserved in pending_events queue.
                            let mut scroll_delta: i32 = match mouse.kind {
                                MouseEventKind::ScrollDown => 3,
                                MouseEventKind::ScrollUp => -3,
                                _ => 0,
                            };

                            // Coalesce scroll events, but preserve non-scroll events
                            while event::poll(Duration::from_millis(0))? {
                                let next_event = event::read()?;
                                match &next_event {
                                    Event::Mouse(m) => match m.kind {
                                        MouseEventKind::ScrollDown => scroll_delta += 3,
                                        MouseEventKind::ScrollUp => scroll_delta -= 3,
                                        _ => {
                                            // Non-scroll mouse event - queue for processing
                                            pending_events.push_back(next_event);
                                            break;
                                        }
                                    },
                                    _ => {
                                        // Non-mouse event - queue for processing
                                        pending_events.push_back(next_event);
                                        break;
                                    }
                                }
                            }

                            // Apply the accumulated scroll delta
                            let in_sidebar = state.show_sidebar
                                && mouse.column < sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height);
                            let in_diff = mouse.column >= sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height);

                            if in_sidebar {
                                let max_sidebar_scroll =
                                    state.sidebar_visible_len().saturating_sub(1);
                                if scroll_delta > 0 {
                                    state.sidebar_scroll = (state.sidebar_scroll
                                        + scroll_delta as usize)
                                        .min(max_sidebar_scroll);
                                } else {
                                    state.sidebar_scroll = state
                                        .sidebar_scroll
                                        .saturating_sub((-scroll_delta) as usize);
                                }
                            } else if in_diff {
                                if scroll_delta > 0 {
                                    state.scroll =
                                        (state.scroll + scroll_delta as u16).min(max_scroll as u16);
                                } else {
                                    state.scroll =
                                        state.scroll.saturating_sub((-scroll_delta) as u16);
                                }
                            }
                        }
                        MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                            // Coalesce consecutive horizontal scroll events
                            let mut h_scroll_delta: i32 = match mouse.kind {
                                MouseEventKind::ScrollRight => 4,
                                MouseEventKind::ScrollLeft => -4,
                                _ => 0,
                            };

                            // Coalesce horizontal scroll events
                            while event::poll(Duration::from_millis(0))? {
                                let next_event = event::read()?;
                                match &next_event {
                                    Event::Mouse(m) => match m.kind {
                                        MouseEventKind::ScrollRight => h_scroll_delta += 4,
                                        MouseEventKind::ScrollLeft => h_scroll_delta -= 4,
                                        _ => {
                                            pending_events.push_back(next_event);
                                            break;
                                        }
                                    },
                                    _ => {
                                        pending_events.push_back(next_event);
                                        break;
                                    }
                                }
                            }

                            // Apply the accumulated horizontal scroll delta
                            let in_sidebar = state.show_sidebar
                                && mouse.column < sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height);
                            let in_diff = mouse.column >= sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height);

                            if in_sidebar {
                                if h_scroll_delta > 0 {
                                    state.sidebar_h_scroll = state
                                        .sidebar_h_scroll
                                        .saturating_add(h_scroll_delta as u16);
                                    clamp_sidebar_h_scroll(&mut state, term_size.width);
                                } else {
                                    state.sidebar_h_scroll = state
                                        .sidebar_h_scroll
                                        .saturating_sub((-h_scroll_delta) as u16);
                                }
                            } else if in_diff {
                                if state.settings.wrap {
                                    state.h_scroll = 0;
                                } else if h_scroll_delta > 0 {
                                    state.h_scroll =
                                        state.h_scroll.saturating_add(h_scroll_delta as u16);
                                    clamp_h_scroll(&mut state, term_size.width);
                                } else {
                                    state.h_scroll =
                                        state.h_scroll.saturating_sub((-h_scroll_delta) as u16);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press && active_modal.is_none() => {
                    if key.code != KeyCode::Char('g') {
                        state.pending_key = PendingKey::None;
                    }
                    state.show_selection_tooltip = false;
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('c')
                            if (key.code == KeyCode::Esc
                                || key.modifiers.contains(KeyModifiers::CONTROL))
                                && state.selection.is_active() =>
                        {
                            // First priority: clear selection
                            state.clear_selection();
                        }
                        KeyCode::Esc | KeyCode::Char('c')
                            if (key.code == KeyCode::Esc
                                || key.modifiers.contains(KeyModifiers::CONTROL))
                                && state.search_state.has_query() =>
                        {
                            state.search_state.clear();
                            state.mark_search_dirty();
                        }
                        KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('c')
                            if key.code != KeyCode::Char('c')
                                || key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            let queued = queued_total(&state, workspace_session.as_ref());
                            if queued == 0 {
                                break 'main;
                            }
                            let queued_comments = count_of(queued, "queued comment");
                            let send_and_exit = if agent.is_attached() {
                                format!("Send {} to the agent and exit", queued_comments)
                            } else {
                                format!("Write {} to stdout and exit", queued_comments)
                            };
                            active_modal = Some(Modal::select(
                                "Queued comments",
                                vec![
                                    send_and_exit,
                                    "Exit and keep them queued".to_string(),
                                    "Cancel".to_string(),
                                ],
                                0,
                            ));
                            open_picker = Some(Picker::Quit);
                        }
                        KeyCode::Char('1') => {
                            state.focused_panel = FocusedPanel::Sidebar;
                            state.show_sidebar = true;
                            if !matches!(
                                state.sidebar_item_at_visible(state.sidebar_selected),
                                Some(SidebarItem::File { .. })
                            ) {
                                if let Some(idx) = state.sidebar_visible.iter().position(|idx| {
                                    matches!(state.sidebar_items[*idx], SidebarItem::File { .. })
                                }) {
                                    state.sidebar_selected = idx;
                                }
                            }
                        }
                        KeyCode::Char('2') => {
                            state.focused_panel = FocusedPanel::DiffView;
                        }
                        KeyCode::Tab => {
                            state.show_sidebar = !state.show_sidebar;
                            if !state.show_sidebar {
                                state.focused_panel = FocusedPanel::DiffView;
                            }
                        }
                        // Next file: option+down, option+k, ctrl+k.
                        KeyCode::Down | KeyCode::Char('k') if changes_file(&key) => {
                            if !state.file_diffs.is_empty() {
                                let mut next = state.sidebar_selected + 1;
                                while next < state.sidebar_visible_len() {
                                    if let Some(SidebarItem::File { file_index, .. }) =
                                        state.sidebar_item_at_visible(next).cloned()
                                    {
                                        state.sidebar_selected = next;
                                        state.select_file(file_index);
                                        let visible_height =
                                            terminal.size()?.height.saturating_sub(5) as usize;
                                        ensure_sidebar_visible(&mut state, visible_height);
                                        break;
                                    }
                                    next += 1;
                                }
                            }
                        }
                        // Previous file: option+up, option+j, ctrl+j.
                        KeyCode::Up | KeyCode::Char('j') if changes_file(&key) => {
                            if !state.file_diffs.is_empty() && state.sidebar_selected > 0 {
                                let mut prev = state.sidebar_selected - 1;
                                loop {
                                    if let Some(SidebarItem::File { file_index, .. }) =
                                        state.sidebar_item_at_visible(prev).cloned()
                                    {
                                        state.sidebar_selected = prev;
                                        state.select_file(file_index);
                                        let visible_height =
                                            terminal.size()?.height.saturating_sub(5) as usize;
                                        ensure_sidebar_visible(&mut state, visible_height);
                                        break;
                                    }
                                    if prev == 0 {
                                        break;
                                    }
                                    prev -= 1;
                                }
                            }
                        }
                        // Stacked mode: navigate to next commit
                        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if state.stacked_mode
                                && state.current_commit_index < state.stacked_commits.len() - 1
                            {
                                let new_index = state.current_commit_index + 1;
                                navigate_stacked_commit(&mut state, new_index, &options, backend);
                            }
                        }
                        // Stacked mode: navigate to previous commit
                        KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if state.stacked_mode && state.current_commit_index > 0 {
                                let new_index = state.current_commit_index - 1;
                                navigate_stacked_commit(&mut state, new_index, &options, backend);
                            }
                        }
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            let half_screen = (visible_height / 2) as u16;
                            state.scroll = (state.scroll + half_screen).min(max_scroll as u16);
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            let half_screen = (visible_height / 2) as u16;
                            state.scroll = state.scroll.saturating_sub(half_screen);
                        }
                        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if !state.file_diffs.is_empty() {
                                let items: Vec<FilePickerItem> = state
                                    .file_diffs
                                    .iter()
                                    .enumerate()
                                    .map(|(i, diff)| {
                                        let status = match diff.status {
                                            FileStatus::Added => ModalFileStatus::Added,
                                            FileStatus::Modified => ModalFileStatus::Modified,
                                            FileStatus::Deleted => ModalFileStatus::Deleted,
                                        };
                                        FilePickerItem {
                                            name: diff.filename.clone(),
                                            file_index: i,
                                            status,
                                            viewed: state.viewed_files.contains(&i),
                                        }
                                    })
                                    .collect();
                                active_modal = Some(Modal::file_picker("Find File", items));
                            }
                        }
                        KeyCode::Char(']') => {
                            if !state.file_diffs.is_empty() {
                                let diff = &state.file_diffs[state.current_file];
                                if !diff.new_content.is_empty() {
                                    state.diff_fullscreen = match state.diff_fullscreen {
                                        DiffFullscreen::NewOnly => DiffFullscreen::None,
                                        _ => DiffFullscreen::NewOnly,
                                    };
                                    state.mark_search_dirty();
                                }
                            }
                        }
                        KeyCode::Char('[') => {
                            if !state.file_diffs.is_empty() {
                                let diff = &state.file_diffs[state.current_file];
                                if !diff.old_content.is_empty() {
                                    state.diff_fullscreen = match state.diff_fullscreen {
                                        DiffFullscreen::OldOnly => DiffFullscreen::None,
                                        _ => DiffFullscreen::OldOnly,
                                    };
                                    state.mark_search_dirty();
                                }
                            }
                        }
                        KeyCode::Char('=') => {
                            state.diff_fullscreen = DiffFullscreen::None;
                            state.mark_search_dirty();
                        }
                        KeyCode::Down
                            if state.search_state.has_query()
                                && state.focused_panel == FocusedPanel::DiffView =>
                        {
                            if let Some(line) = state.search_state.find_next() {
                                state.scroll = adjust_scroll_to_line(
                                    line,
                                    state.scroll,
                                    visible_height,
                                    max_scroll,
                                );
                            }
                        }
                        KeyCode::Up
                            if state.search_state.has_query()
                                && state.focused_panel == FocusedPanel::DiffView =>
                        {
                            if let Some(line) = state.search_state.find_prev() {
                                state.scroll = adjust_scroll_to_line(
                                    line,
                                    state.scroll,
                                    visible_height,
                                    max_scroll,
                                );
                            }
                        }
                        KeyCode::Down | KeyCode::Char('k') => {
                            if state.focused_panel == FocusedPanel::Sidebar {
                                if state.sidebar_selected + 1 < state.sidebar_visible_len() {
                                    state.sidebar_selected += 1;
                                }
                                let visible_height =
                                    terminal.size()?.height.saturating_sub(5) as usize;
                                ensure_sidebar_visible(&mut state, visible_height);
                            } else {
                                state.scroll = (state.scroll + 1).min(max_scroll as u16);
                            }
                        }
                        KeyCode::Up | KeyCode::Char('j') => {
                            if state.focused_panel == FocusedPanel::Sidebar {
                                if state.sidebar_selected > 0 {
                                    state.sidebar_selected =
                                        state.sidebar_selected.saturating_sub(1);
                                }
                                let visible_height =
                                    terminal.size()?.height.saturating_sub(5) as usize;
                                ensure_sidebar_visible(&mut state, visible_height);
                            } else {
                                state.scroll = state.scroll.saturating_sub(1);
                            }
                        }
                        KeyCode::Char('w') => {
                            if pr_info.is_some() {
                                // PR mode doesn't support watching
                            } else {
                                state.watching = !state.watching;
                                if state.watching {
                                    if let Some((handle, rx)) =
                                        setup_watcher(backend.working_tree_root())
                                    {
                                        _watch_handle = Some(handle);
                                        watch_rx = Some(rx);
                                    } else {
                                        state.watching = false;
                                    }
                                } else {
                                    _watch_handle = None;
                                    watch_rx = None;
                                }
                            }
                        }
                        KeyCode::Char('h') | KeyCode::Left => {
                            if state.focused_panel == FocusedPanel::DiffView && !state.settings.wrap
                            {
                                state.h_scroll = state.h_scroll.saturating_sub(4);
                            } else if state.focused_panel == FocusedPanel::Sidebar {
                                state.sidebar_h_scroll = state.sidebar_h_scroll.saturating_sub(4);
                            }
                        }
                        KeyCode::Char('l') | KeyCode::Right => {
                            let term_width = terminal.size()?.width;
                            if state.focused_panel == FocusedPanel::DiffView && !state.settings.wrap
                            {
                                state.h_scroll = state.h_scroll.saturating_add(4);
                                clamp_h_scroll(&mut state, term_width);
                            } else if state.focused_panel == FocusedPanel::Sidebar {
                                state.sidebar_h_scroll = state.sidebar_h_scroll.saturating_add(4);
                                clamp_sidebar_h_scroll(&mut state, term_width);
                            }
                        }
                        KeyCode::Enter => {
                            if state.focused_panel == FocusedPanel::Sidebar
                                && state.sidebar_selected < state.sidebar_visible_len()
                            {
                                if let Some(item) = state
                                    .sidebar_item_at_visible(state.sidebar_selected)
                                    .cloned()
                                {
                                    match item {
                                        SidebarItem::File { file_index, .. } => {
                                            state.select_file(file_index);
                                            state.focused_panel = FocusedPanel::DiffView;
                                        }
                                        SidebarItem::Directory { path, .. } => {
                                            state.toggle_directory(&path);
                                            let visible_height =
                                                terminal.size()?.height.saturating_sub(5) as usize;
                                            if state.sidebar_selected < state.sidebar_scroll {
                                                state.sidebar_scroll = state.sidebar_selected;
                                            } else if state.sidebar_selected
                                                >= state.sidebar_scroll + visible_height
                                            {
                                                state.sidebar_scroll = state
                                                    .sidebar_selected
                                                    .saturating_sub(visible_height)
                                                    + 1;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        KeyCode::Char(' ') => {
                            if state.focused_panel == FocusedPanel::Sidebar
                                && state.sidebar_selected < state.sidebar_visible_len()
                            {
                                let selected = state
                                    .sidebar_item_at_visible(state.sidebar_selected)
                                    .cloned();
                                if let Some(selected) = selected {
                                    match selected {
                                        SidebarItem::File { file_index, .. } => {
                                            let file_idx = file_index;
                                            let filename =
                                                state.file_diffs[file_idx].filename.clone();
                                            let was_viewed = state.viewed_files.contains(&file_idx);

                                            // Optimistic update - update local state immediately
                                            if was_viewed {
                                                state.viewed_files.remove(&file_idx);
                                            } else {
                                                state.viewed_files.insert(file_idx);
                                            }

                                            // Fire off async API call if in PR mode
                                            if let Some(ref pr) = pr_info {
                                                if was_viewed {
                                                    unmark_file_as_viewed_async(pr, &filename);
                                                } else {
                                                    mark_file_as_viewed_async(pr, &filename);
                                                }
                                            }
                                        }
                                        SidebarItem::Directory { path, .. } => {
                                            let dir_prefix = format!("{}/", path);
                                            let child_indices: Vec<usize> = state
                                                .sidebar_items
                                                .iter()
                                                .filter_map(|item| {
                                                    if let SidebarItem::File {
                                                        path: file_path,
                                                        file_index,
                                                        ..
                                                    } = item
                                                    {
                                                        if file_path.starts_with(&dir_prefix) {
                                                            return Some(*file_index);
                                                        }
                                                    }
                                                    None
                                                })
                                                .collect();

                                            let all_viewed = child_indices
                                                .iter()
                                                .all(|i| state.viewed_files.contains(i));

                                            // Optimistic update - update local state immediately
                                            if all_viewed {
                                                for idx in &child_indices {
                                                    state.viewed_files.remove(idx);
                                                }
                                            } else {
                                                for idx in &child_indices {
                                                    state.viewed_files.insert(*idx);
                                                }
                                            }

                                            // Fire off async API calls if in PR mode
                                            if let Some(ref pr) = pr_info {
                                                for &idx in &child_indices {
                                                    let filename = &state.file_diffs[idx].filename;
                                                    if all_viewed {
                                                        unmark_file_as_viewed_async(pr, filename);
                                                    } else {
                                                        mark_file_as_viewed_async(pr, filename);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            } else if state.focused_panel == FocusedPanel::DiffView {
                                let current_file = state.current_file;
                                let filename = state.file_diffs[current_file].filename.clone();
                                let was_viewed = state.viewed_files.contains(&current_file);

                                // Optimistic update - update local state immediately
                                if was_viewed {
                                    state.viewed_files.remove(&current_file);
                                } else {
                                    state.viewed_files.insert(current_file);
                                    // Move to next unviewed file
                                    let mut next_file: Option<(usize, usize)> = None;
                                    for (visible_idx, item_idx) in state
                                        .sidebar_visible
                                        .iter()
                                        .enumerate()
                                        .skip(state.sidebar_selected + 1)
                                    {
                                        if let SidebarItem::File { file_index, .. } =
                                            &state.sidebar_items[*item_idx]
                                        {
                                            if !state.viewed_files.contains(file_index) {
                                                next_file = Some((visible_idx, *file_index));
                                                break;
                                            }
                                        }
                                    }
                                    if next_file.is_none() {
                                        for (visible_idx, item_idx) in state
                                            .sidebar_visible
                                            .iter()
                                            .enumerate()
                                            .take(state.sidebar_selected)
                                        {
                                            if let SidebarItem::File { file_index, .. } =
                                                &state.sidebar_items[*item_idx]
                                            {
                                                if !state.viewed_files.contains(file_index) {
                                                    next_file = Some((visible_idx, *file_index));
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    if let Some((idx, file_idx)) = next_file {
                                        state.sidebar_selected = idx;
                                        state.select_file(file_idx);
                                        let visible_height =
                                            terminal.size()?.height.saturating_sub(5) as usize;
                                        ensure_sidebar_visible(&mut state, visible_height);
                                    }
                                }

                                // Fire off async API call if in PR mode
                                if let Some(ref pr) = pr_info {
                                    if was_viewed {
                                        unmark_file_as_viewed_async(pr, &filename);
                                    } else {
                                        mark_file_as_viewed_async(pr, &filename);
                                    }
                                }
                            }
                        }
                        KeyCode::PageDown => {
                            state.scroll = (state.scroll + 20).min(max_scroll as u16);
                        }
                        KeyCode::PageUp => {
                            state.scroll = state.scroll.saturating_sub(20);
                        }
                        KeyCode::Char('}') => {
                            if !state.file_diffs.is_empty() {
                                state.clear_selection(); // Clear selection on hunk navigation
                                let hunks = state.get_hunks().to_vec();
                                let current_hunk = state.focused_hunk.unwrap_or(0);
                                let next_hunk = if state.focused_hunk.is_none() {
                                    hunks
                                        .iter()
                                        .position(|&h| h > state.scroll as usize + 5)
                                        .unwrap_or(0)
                                } else {
                                    (current_hunk + 1).min(hunks.len().saturating_sub(1))
                                };
                                if !hunks.is_empty() {
                                    state.focused_hunk = Some(next_hunk);
                                    state.scroll = adjust_scroll_for_hunk(
                                        hunks[next_hunk],
                                        state.scroll,
                                        visible_height,
                                        max_scroll,
                                    );
                                }
                            }
                        }
                        KeyCode::Char('{') => {
                            if !state.file_diffs.is_empty() {
                                state.clear_selection(); // Clear selection on hunk navigation
                                let hunks = state.get_hunks().to_vec();
                                let current_hunk = state.focused_hunk.unwrap_or(hunks.len());
                                let prev_hunk = if state.focused_hunk.is_none() {
                                    hunks
                                        .iter()
                                        .rposition(|&h| (h as u16) < state.scroll.saturating_sub(5))
                                        .unwrap_or(hunks.len().saturating_sub(1))
                                } else {
                                    current_hunk.saturating_sub(1)
                                };
                                if !hunks.is_empty() {
                                    state.focused_hunk = Some(prev_hunk);
                                    state.scroll = adjust_scroll_for_hunk(
                                        hunks[prev_hunk],
                                        state.scroll,
                                        visible_height,
                                        max_scroll,
                                    );
                                }
                            }
                        }
                        KeyCode::Char('m') => {
                            if state.focused_panel == FocusedPanel::DiffView
                                && !state.file_diffs.is_empty()
                            {
                                let hunks = state.get_hunks().to_vec();
                                if let Some(hunk_idx) = state.focused_hunk {
                                    if hunk_idx < hunks.len() {
                                        let filename =
                                            state.file_diffs[state.current_file].filename.clone();
                                        let entry = state.viewed_hunks.entry(filename).or_default();
                                        let was_viewed = entry.contains(&hunk_idx);
                                        if was_viewed {
                                            entry.remove(&hunk_idx);
                                        } else {
                                            entry.insert(hunk_idx);
                                        }

                                        // On mark-viewed (not unmark), advance to next hunk.
                                        if !was_viewed && hunk_idx + 1 < hunks.len() {
                                            let next = hunk_idx + 1;
                                            state.focused_hunk = Some(next);
                                            state.scroll = adjust_scroll_for_hunk(
                                                hunks[next],
                                                state.scroll,
                                                visible_height,
                                                max_scroll,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        KeyCode::Char('i') => {
                            let had_selection = state.selection.is_active()
                                && !matches!(state.selection.mode, SelectionMode::None);
                            if let Some((filename, target)) = target_at_cursor(&mut state) {
                                annotation_editor =
                                    Some(match state.thread_at(&filename, &target, false) {
                                        Some(thread) => AnnotationEditor::for_thread(thread),
                                        None => AnnotationEditor::new(filename, target),
                                    });
                            }
                            if had_selection {
                                state.clear_selection();
                            }
                        }
                        KeyCode::Char('x') => {
                            let thread_id = target_at_cursor(&mut state).and_then(|(filename, target)| {
                                let thread = state.thread_at(&filename, &target, true)?;
                                Some(thread.id.clone())
                            });
                            match thread_id {
                                Some(thread_id) => state.toggle_thread_resolved(&thread_id),
                                None => {
                                    notice = Some(("no thread here".to_string(), Instant::now()));
                                }
                            }
                        }
                        KeyCode::Char('I') => {
                            if !state.threads.is_empty() {
                                active_modal = Some(threads_modal(&state.threads));
                            }
                        }
                        KeyCode::Char('r') => {
                            state.needs_reload = true;
                        }
                        KeyCode::Char('R') => {
                            if let Some(session) = &workspace_session {
                                active_modal = Some(Modal::select(
                                    "Repositories",
                                    session.picker_items(&state),
                                    session.active(),
                                ));
                                open_picker = Some(Picker::Repository);
                            }
                        }
                        KeyCode::Char('A') => {
                            let panes = agent.candidates(&review_roots);
                            let attached = panes
                                .iter()
                                .position(|pane| Some(pane.block.as_str()) == agent.attached_block());
                            let mut items = vec!["none".to_string()];
                            items.extend(panes.iter().map(AgentPane::picker_label));
                            active_modal = Some(Modal::select(
                                "Agent that answers comments",
                                items,
                                attached.map_or(0, |index| index + 1),
                            ));
                            open_picker = Some(Picker::Agent(panes));
                        }
                        KeyCode::Char('s') => {
                            let queued = queued_total(&state, workspace_session.as_ref());
                            if queued == 0 {
                                notice = Some(("no queued comments".to_string(), Instant::now()));
                            } else if agent.is_attached() {
                                let message = send_queued(
                                    &mut state,
                                    workspace_session.as_mut(),
                                    &mut agent,
                                    None,
                                )
                                .unwrap_or_else(|error| error);
                                notice = Some((message, Instant::now()));
                            } else {
                                let msg = format!(
                                    "No agent is attached. Exit lumiere and write {} to stdout?\n\n\
                                     Use this to pipe feedback back to a coding agent.",
                                    count_of(queued, "queued comment"),
                                );
                                active_modal = Some(Modal::confirm("Send comments", msg));
                            }
                        }
                        KeyCode::Char('y') => {
                            if !state.file_diffs.is_empty() {
                                // If selection is active, copy selected text
                                if state.selection.is_active() {
                                    state.ensure_cache();
                                    let side_by_side = state.side_by_side_ref();
                                    if let Some(text) =
                                        extract_selected_text(&state.selection, side_by_side)
                                    {
                                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                            let _ = clipboard.set_text(&text);
                                        }
                                    }
                                    state.clear_selection();
                                } else {
                                    // Otherwise copy filename
                                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                        let _ = clipboard.set_text(
                                            &state.file_diffs[state.current_file].filename,
                                        );
                                    }
                                }
                            }
                        }
                        KeyCode::Char('e') => {
                            if !state.file_diffs.is_empty() {
                                let _ = execute!(
                                    terminal.backend_mut(),
                                    PopKeyboardEnhancementFlags
                                );
                                execute!(
                                    terminal.backend_mut(),
                                    DisableMouseCapture,
                                    LeaveAlternateScreen
                                )?;
                                disable_raw_mode()?;

                                let editor =
                                    std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
                                let filename =
                                    state.file_diffs[state.current_file].filename.clone();

                                let line_arg = if let Some(hunk_idx) = state.focused_hunk {
                                    state.ensure_cache();
                                    let side_by_side = state.side_by_side_ref();
                                    let hunks = state.hunks_ref();
                                    if let Some(&hunk_start) = hunks.get(hunk_idx) {
                                        side_by_side.get(hunk_start).and_then(|dl| {
                                            dl.new_line
                                                .as_ref()
                                                .map(|(n, _)| *n)
                                                .or(dl.old_line.as_ref().map(|(n, _)| *n))
                                        })
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                };

                                let mut editor_command = std::process::Command::new(&editor);
                                editor_command.current_dir(backend.working_tree_root());
                                // Captured stdout is reserved for the annotations, so the
                                // editor draws on the terminal, as the TUI does.
                                #[cfg(unix)]
                                if !io::stdout().is_terminal() {
                                    if let Ok(tty) = std::fs::OpenOptions::new()
                                        .read(true)
                                        .write(true)
                                        .open("/dev/tty")
                                    {
                                        editor_command.stdout(tty);
                                    }
                                }
                                if let Some(line) = line_arg {
                                    editor_command.arg(format!("+{}", line));
                                }
                                let _ = editor_command.arg(filename).status();

                                enable_raw_mode()?;
                                execute!(
                                    terminal.backend_mut(),
                                    EnterAlternateScreen,
                                    EnableMouseCapture
                                )?;
                                let _ = execute!(
                                    terminal.backend_mut(),
                                    PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
                                );
                                terminal.clear()?;
                            }
                        }
                        KeyCode::Char('o') => {
                            if let Some(ref pr) = pr_info {
                                if !state.file_diffs.is_empty() {
                                    let filename = &state.file_diffs[state.current_file].filename;
                                    let file_url = format!(
                                        "https://github.com/{}/{}/pull/{}/files#diff-{}",
                                        pr.repo_owner,
                                        pr.repo_name,
                                        pr.number,
                                        generate_file_anchor(filename)
                                    );
                                    let _ = open_url(&file_url);
                                }
                            }
                        }
                        KeyCode::Char('g') => {
                            if state.pending_key == PendingKey::G {
                                state.scroll = 0;
                                state.pending_key = PendingKey::None;
                            } else {
                                state.pending_key = PendingKey::G;
                            }
                        }
                        KeyCode::Char('G') => {
                            state.scroll = max_scroll as u16;
                        }
                        KeyCode::Char('/') => {
                            // In-file incremental search (existing behavior).
                            state.search_state.start_forward();
                            state.mark_search_dirty();
                        }
                        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            // Global telescope-style search across all files.
                            let gs = crate::command::diff::render::GlobalSearchState::build(
                                &state.file_diffs,
                                &state.settings,
                            );
                            active_modal = Some(Modal::global_search("Search", gs));
                        }
                        KeyCode::Char('n') if state.search_state.has_query() => {
                            if let Some(line) = state.search_state.find_next() {
                                state.scroll = adjust_scroll_to_line(
                                    line,
                                    state.scroll,
                                    visible_height,
                                    max_scroll,
                                );
                            }
                        }
                        KeyCode::Char('N') if state.search_state.has_query() => {
                            if let Some(line) = state.search_state.find_prev() {
                                state.scroll = adjust_scroll_to_line(
                                    line,
                                    state.scroll,
                                    visible_height,
                                    max_scroll,
                                );
                            }
                        }
                        KeyCode::Char('?') => {
                            active_modal = Some(Modal::keybindings(
                                "Keybindings",
                                vec![
                                    KeyBindSection {
                                        title: "Global",
                                        bindings: vec![
                                            KeyBind {
                                                key: "q / esc",
                                                description: "Quit",
                                            },
                                            KeyBind {
                                                key: "tab",
                                                description: "Toggle sidebar",
                                            },
                                            KeyBind {
                                                key: "R",
                                                description: "Switch repository (workspace folder)",
                                            },
                                            KeyBind {
                                                key: "A",
                                                description: "Choose the agent that answers comments",
                                            },
                                            KeyBind {
                                                key: "1 / 2",
                                                description: "Focus sidebar / diff",
                                            },
                                            KeyBind {
                                                key: "option+down/k, option+up/j",
                                                description: "Next / previous file",
                                            },
                                            KeyBind {
                                                key: "ctrl+d / ctrl+u",
                                                description: "Scroll half page down / up",
                                            },
                                            KeyBind {
                                                key: "ctrl+p",
                                                description: "Open file picker",
                                            },
                                            KeyBind {
                                                key: "r",
                                                description: "Refresh diff / PR",
                                            },
                                            KeyBind {
                                                key: "y",
                                                description: "Copy current filename",
                                            },
                                            KeyBind {
                                                key: "e",
                                                description: "Edit file (at hunk line if focused)",
                                            },
                                            KeyBind {
                                                key: "o",
                                                description: "Open file in browser (PR mode)",
                                            },
                                            KeyBind {
                                                key: "ctrl+l / ctrl+h",
                                                description: "Next / prev commit (stacked)",
                                            },
                                            KeyBind {
                                                key: "?",
                                                description: "Show keybindings",
                                            },
                                        ],
                                    },
                                    KeyBindSection {
                                        title: "Sidebar",
                                        bindings: vec![
                                            KeyBind {
                                                key: "j/k or up/down",
                                                description: "Navigate files",
                                            },
                                            KeyBind {
                                                key: "h/l or left/right",
                                                description: "Scroll horizontally",
                                            },
                                            KeyBind {
                                                key: "enter",
                                                description:
                                                    "Open file in diff view / toggle directory",
                                            },
                                            KeyBind {
                                                key: "space",
                                                description: "Toggle file as viewed",
                                            },
                                        ],
                                    },
                                    KeyBindSection {
                                        title: "Diff View",
                                        bindings: vec![
                                            KeyBind {
                                                key: "j/k or up/down",
                                                description: "Scroll vertically",
                                            },
                                            KeyBind {
                                                key: "h/l or left/right",
                                                description: "Scroll horizontally",
                                            },
                                        KeyBind {
                                            key: "w",
                                            description: "Toggle watch mode",
                                        },
                                            KeyBind {
                                                key: "gg / G",
                                                description: "Scroll to top / bottom",
                                            },
                                            KeyBind {
                                                key: "{ / }",
                                                description: "Focus prev / next hunk",
                                            },
                                            KeyBind {
                                                key: "pageup / pagedown",
                                                description: "Scroll by page",
                                            },
                                            KeyBind {
                                                key: "space",
                                                description: "Mark viewed & next file",
                                            },
                                            KeyBind {
                                                key: "m",
                                                description: "Mark hunk viewed & next hunk",
                                            },
                                            KeyBind {
                                                key: "]",
                                                description: "Toggle new panel fullscreen",
                                            },
                                            KeyBind {
                                                key: "[",
                                                description: "Toggle old panel fullscreen",
                                            },
                                            KeyBind {
                                                key: "=",
                                                description: "Reset fullscreen to side-by-side",
                                            },
                                        ],
                                    },
                                    KeyBindSection {
                                        title: "Search",
                                        bindings: vec![
                                            KeyBind {
                                                key: "/",
                                                description: "In-file incremental search",
                                            },
                                            KeyBind {
                                                key: "ctrl+f",
                                                description: "Global fuzzy search (all files, with preview)",
                                            },
                                            KeyBind {
                                                key: "n or down",
                                                description: "Next match (in-file)",
                                            },
                                            KeyBind {
                                                key: "N or up",
                                                description: "Previous match (in-file)",
                                            },
                                            KeyBind {
                                                key: "ctrl+c or esc",
                                                description: "Cancel search",
                                            },
                                        ],
                                    },
                                    KeyBindSection {
                                        title: "Selection & Review threads",
                                        bindings: vec![
                                            KeyBind {
                                                key: "y",
                                                description: "Copy selection (or filename)",
                                            },
                                            KeyBind {
                                                key: "i",
                                                description: "Comment on selection / hunk / file, or reply",
                                            },
                                            KeyBind {
                                                key: "enter",
                                                description: "In a comment: save and send to the agent",
                                            },
                                            KeyBind {
                                                key: "alt+enter / ctrl+s",
                                                description: "In a comment: save and queue",
                                            },
                                            KeyBind {
                                                key: "shift+enter / ctrl+j",
                                                description: "In a comment: new line",
                                            },
                                            KeyBind {
                                                key: "x",
                                                description: "Resolve / reopen the thread here",
                                            },
                                            KeyBind {
                                                key: "I",
                                                description: "View all threads",
                                            },
                                            KeyBind {
                                                key: "s",
                                                description: "Send queued comments (exits if no agent)",
                                            },
                                        ],
                                    },
                                ],
                            ));
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    disable_raw_mode()?;

    if send_annotations_on_exit {
        let formatted = match &workspace_session {
            Some(session) => session.format_threads_for_export(&state, true),
            None => state.format_threads_for_export(true),
        };
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        handle.write_all(formatted.as_bytes())?;
        handle.write_all(b"\n")?;
        state.mark_sent(None);
        if let Some(session) = workspace_session.as_mut() {
            session.mark_parked_sent();
        }
    }

    Ok(())
}

fn open_url(url: &str) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", url])
            .spawn()?;
    }
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
    use crate::command::diff::types::FileDiff;

    #[test]
    fn test_showing_a_thread_puts_its_first_line_on_the_top_row() {
        let numbered_lines: String = (1..=80).map(|line| format!("line {}\n", line)).collect();
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
        // A thread above the target whose comment takes more rows than a screen.
        state.add_thread("a.rs".to_string(), on_line(6), "long\n".repeat(60));
        let (thread_id, _) = state.add_thread("a.rs".to_string(), on_line(30), "here".to_string());

        assert!(show_thread(&mut state, &thread_id));

        let top_row = &state.side_by_side_ref()[state.scroll as usize];
        assert_eq!(top_row.line_number(DiffPanelFocus::New), Some(30));
    }
}
