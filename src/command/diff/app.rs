use std::collections::VecDeque;
use std::io;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseEventKind,
    },
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;

use super::annotation::{AnnotationEditor, AnnotationEditorResult};
use super::diff_algo::{compute_side_by_side, find_hunk_starts};
use super::git::{
    get_current_branch, load_file_diffs, load_pr_file_diffs, load_single_commit_diffs,
};
use super::highlight;
use super::render::{
    render_diff, render_empty_state, truncate_path, FilePickerItem, KeyBind, KeyBindSection, Modal,
    ModalContent, ModalFileStatus, ModalResult,
};
use super::state::{adjust_scroll_for_hunk, adjust_scroll_to_line, AppState, PendingKey};
use super::theme;
use super::types::{
    ChangeType, DiffFullscreen, FilePanelItem, FilePanelPosition, FileStatus, FocusedPanel,
};
use super::watcher::{setup_watcher, WatchEvent};
use super::{
    fetch_viewed_files, mark_file_as_viewed_async, unmark_file_as_viewed_async, DiffOptions, PrInfo,
};
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

/// Adjust file panel scroll to ensure the selected item is visible.
fn ensure_file_panel_visible(state: &mut AppState, term_height: u16) {
    if state.file_panel_selected < state.file_panel_scroll {
        state.file_panel_scroll = state.file_panel_selected;
        return;
    }

    if term_height == u16::MAX {
        return;
    }

    let header_height = if state.stacked_mode { 1 } else { 0 };
    let footer_height = 1;
    let content_height = term_height.saturating_sub(header_height + footer_height);

    let visible_height = match state.settings.file_panel_position {
        FilePanelPosition::Left => content_height.saturating_sub(2) as usize,
        FilePanelPosition::Bottom => {
            let panel_height = (content_height / 3).clamp(5, 12);
            panel_height.saturating_sub(2) as usize
        }
    };

    if state.file_panel_selected >= state.file_panel_scroll + visible_height {
        state.file_panel_scroll = state.file_panel_selected.saturating_sub(visible_height) + 1;
    }
}

/// Format an annotation for display in the annotations list.
fn format_annotation_preview(annotation: &super::state::HunkAnnotation) -> String {
    let preview = annotation.content.lines().next().unwrap_or("");
    let preview = if preview.len() > 40 {
        format!("{}...", &preview[..40])
    } else {
        preview.to_string()
    };
    let truncated_filename = truncate_path(&annotation.filename, 30);
    format!(
        "{}:{}-{} | {} | {}",
        truncated_filename,
        annotation.line_range.0,
        annotation.line_range.1,
        preview,
        annotation.format_time()
    )
}

pub fn run_app_with_pr(
    options: DiffOptions,
    pr_info: PrInfo,
    backend: &dyn VcsBackend,
) -> io::Result<()> {
    match load_pr_file_diffs(&pr_info) {
        Ok(file_diffs) => run_app_internal(options, Some(pr_info), file_diffs, None, backend),
        Err(e) => {
            eprintln!("\x1b[91merror:\x1b[0m {}", e);
            std::process::exit(1);
        }
    }
}

pub fn run_app(
    options: DiffOptions,
    pr_info: Option<PrInfo>,
    backend: &dyn VcsBackend,
) -> io::Result<()> {
    let file_diffs = load_file_diffs(&options, backend);
    run_app_internal(options, pr_info, file_diffs, None, backend)
}

pub fn run_app_stacked(
    options: DiffOptions,
    commits: Vec<StackedCommitInfo>,
    backend: &dyn VcsBackend,
) -> io::Result<()> {
    // Load the first commit's diff
    let first_commit = &commits[0];
    let file_diffs = load_single_commit_diffs(&first_commit.commit_id, &options.file, backend);
    run_app_internal(options, None, file_diffs, Some(commits), backend)
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

fn run_app_internal(
    options: DiffOptions,
    pr_info: Option<PrInfo>,
    file_diffs: Vec<super::types::FileDiff>,
    stacked_commits: Option<Vec<StackedCommitInfo>>,
    backend: &dyn VcsBackend,
) -> io::Result<()> {
    theme::init(options.theme.as_deref());
    highlight::init();

    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    io::stdout().execute(EnableMouseCapture)?;

    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let watch_rx = if options.watch && pr_info.is_none() {
        setup_watcher()
    } else {
        None
    };

    let settings = super::types::DiffViewSettings {
        file_panel_position: options.file_panel_pos.unwrap_or_default(),
        ..Default::default()
    };
    let mut state = AppState::new(file_diffs, settings);
    state.set_vcs_name(backend.name());

    // Set diff reference for annotation export context
    let diff_ref_str = if let Some(pr) = &pr_info {
        Some(format!(
            "PR #{} ({}...{})",
            pr.number, pr.base_ref, pr.head_ref
        ))
    } else {
        options.reference.as_ref().map(|r| match r {
            CommitReference::Single(s) => s.clone(),
            CommitReference::Range { from, to } => format!("{}..{}", from, to),
            CommitReference::TripleDots { from, to } => format!("{}...{}", from, to),
        })
    };
    state.set_diff_reference(diff_ref_str);

    let mut active_modal: Option<Modal> = None;
    let mut annotation_editor: Option<AnnotationEditor> = None;
    let mut pending_watch_event: Option<WatchEvent> = None;
    let mut pending_events: VecDeque<Event> = VecDeque::new();

    // Initialize stacked mode if commits were provided
    if let Some(commits) = stacked_commits {
        state.init_stacked_mode(commits);
    }

    // Load viewed files from GitHub on startup in PR mode
    if let Some(ref pr) = pr_info {
        sync_viewed_files_from_github(pr, &mut state);
    }

    'main: loop {
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
                render_empty_state(frame, options.watch);
                if let Some(ref modal) = active_modal {
                    modal.render(frame);
                }
            })?;
        } else {
            let diff = &state.file_diffs[state.current_file];
            let side_by_side = compute_side_by_side(
                &diff.old_content,
                &diff.new_content,
                state.settings.tab_width,
            );
            let hunks = find_hunk_starts(&side_by_side);
            let hunk_count = hunks.len();
            state
                .search_state
                .update_matches(&side_by_side, state.diff_fullscreen);
            let branch_fallback = get_current_branch(backend);
            let commit_ref = state.diff_reference.as_deref().unwrap_or(&branch_fallback);
            terminal.draw(|frame| {
                render_diff(
                    frame,
                    diff,
                    &state.file_diffs,
                    &state.file_panel_items,
                    &state.file_panel_visible,
                    &state.collapsed_dirs,
                    state.current_file,
                    state.scroll,
                    state.h_scroll,
                    options.watch,
                    state.show_file_panel,
                    state.focused_panel,
                    state.file_panel_selected,
                    state.file_panel_scroll,
                    state.file_panel_h_scroll,
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
                    state.vcs_name,
                    &state.annotations,
                );
                // Render annotation editor (on top of everything except modal)
                if let Some(ref editor) = annotation_editor {
                    editor.render(frame);
                }
                if let Some(ref modal) = active_modal {
                    modal.render(frame);
                }
            })?;
        }

        // Poll for new events if no pending events
        if pending_events.is_empty() && event::poll(Duration::from_millis(100))? {
            pending_events.push_back(event::read()?);
        }

        // Process all pending events
        while let Some(current_event) = pending_events.pop_front() {
            let term_size = terminal.size()?;
            let header_height = if state.stacked_mode { 1u16 } else { 0u16 };
            let footer_height = 1u16;
            let content_area_height = term_size
                .height
                .saturating_sub(header_height + footer_height);

            let diff_view_height = if state.show_file_panel
                && state.settings.file_panel_position == FilePanelPosition::Bottom
            {
                let panel_height = (content_area_height / 3).clamp(5, 12);
                content_area_height.saturating_sub(panel_height)
            } else {
                content_area_height
            };

            let visible_height = diff_view_height as usize;
            let bottom_padding = 5;
            let max_scroll = if !state.file_diffs.is_empty() {
                let diff = &state.file_diffs[state.current_file];
                let total_lines = compute_side_by_side(
                    &diff.old_content,
                    &diff.new_content,
                    state.settings.tab_width,
                )
                .len();
                total_lines.saturating_sub(visible_height.saturating_sub(bottom_padding))
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
                        KeyCode::Backspace => {
                            state.search_state.pop_char();
                        }
                        KeyCode::Char(c) => {
                            state.search_state.push_char(c);
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
                            AnnotationEditorResult::Save => {
                                state.set_annotation(editor.to_annotation());
                                annotation_editor = None;
                            }
                            AnnotationEditorResult::Delete => {
                                state.remove_annotation(editor.file_index, editor.hunk_index);
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
                                        state.file_panel_visible_index_for_file(state.current_file)
                                    {
                                        state.file_panel_selected = idx;
                                        ensure_file_panel_visible(
                                            &mut state,
                                            terminal.size()?.height,
                                        );
                                    }
                                    active_modal = None;
                                }
                                ModalResult::AnnotationJump {
                                    file_index,
                                    hunk_index,
                                } => {
                                    // Jump to the file and hunk
                                    state.select_file(file_index);
                                    state.focused_hunk = Some(hunk_index);
                                    // Scroll to the hunk
                                    let diff = &state.file_diffs[file_index];
                                    let side_by_side = compute_side_by_side(
                                        &diff.old_content,
                                        &diff.new_content,
                                        state.settings.tab_width,
                                    );
                                    let hunks = find_hunk_starts(&side_by_side);
                                    if let Some(&hunk_start) = hunks.get(hunk_index) {
                                        state.scroll = adjust_scroll_for_hunk(
                                            hunk_start,
                                            state.scroll,
                                            visible_height,
                                            max_scroll,
                                        );
                                    }
                                    active_modal = None;
                                }
                                ModalResult::AnnotationEdit {
                                    file_index,
                                    hunk_index,
                                } => {
                                    // Close modal and open annotation editor for editing
                                    if let Some(ann) = state.get_annotation(file_index, hunk_index)
                                    {
                                        let editor = AnnotationEditor::new(
                                            file_index,
                                            hunk_index,
                                            ann.filename.clone(),
                                            ann.line_range,
                                        )
                                        .with_content(&ann.content, ann.created_at);
                                        annotation_editor = Some(editor);
                                        // Also jump to the hunk
                                        state.select_file(file_index);
                                        state.focused_hunk = Some(hunk_index);
                                    }
                                    active_modal = None;
                                }
                                ModalResult::AnnotationDelete {
                                    file_index,
                                    hunk_index,
                                } => {
                                    state.remove_annotation(file_index, hunk_index);
                                    // Refresh the modal if there are still annotations
                                    if !state.annotations.is_empty() {
                                        let mut sorted_annotations = state.annotations.clone();
                                        sorted_annotations.sort_by_key(|a| a.created_at);
                                        let items: Vec<String> = sorted_annotations
                                            .iter()
                                            .map(format_annotation_preview)
                                            .collect();
                                        active_modal = Some(Modal::annotations(
                                            "Annotations",
                                            items,
                                            sorted_annotations,
                                        ));
                                    } else {
                                        active_modal = None;
                                    }
                                }
                                ModalResult::AnnotationCopyAll => {
                                    // Copy all annotations to clipboard
                                    let formatted = state.format_annotations_for_export();
                                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                        let _ = clipboard.set_text(&formatted);
                                    }
                                    active_modal = None;
                                }
                                ModalResult::AnnotationExport(filename) => {
                                    // Write annotations to file
                                    let formatted = state.format_annotations_for_export();
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
                                ModalResult::Dismissed | ModalResult::Selected(_, _) => {
                                    active_modal = None;
                                }
                            }
                        }
                    }
                }
                Event::Mouse(mouse) if active_modal.is_some() => {
                    if let Some(ref mut modal) = active_modal {
                        let term_height = terminal.size()?.height;
                        modal.handle_mouse(mouse, term_height);
                    }
                }
                Event::Mouse(mouse) if active_modal.is_none() => {
                    let term_size = terminal.size()?;
                    let footer_height = 1u16;
                    let header_height = if state.stacked_mode { 1u16 } else { 0u16 };
                    let content_area_height = term_size
                        .height
                        .saturating_sub(header_height + footer_height);

                    let (in_file_panel, clicked_row) = if state.show_file_panel {
                        match state.settings.file_panel_position {
                            FilePanelPosition::Left => {
                                let panel_width = (term_size.width / 4).clamp(20, 35);
                                if mouse.column < panel_width
                                    && mouse.row >= header_height
                                    && mouse.row < term_size.height.saturating_sub(footer_height)
                                {
                                    let row = (mouse.row.saturating_sub(header_height + 1))
                                        as usize
                                        + state.file_panel_scroll;
                                    (true, Some(row))
                                } else {
                                    (false, None)
                                }
                            }
                            FilePanelPosition::Bottom => {
                                let panel_height = (content_area_height / 3).clamp(5, 12);
                                let panel_top = term_size
                                    .height
                                    .saturating_sub(footer_height + panel_height);
                                if mouse.row >= panel_top
                                    && mouse.row < term_size.height.saturating_sub(footer_height)
                                {
                                    let row = (mouse.row.saturating_sub(panel_top + 1)) as usize
                                        + state.file_panel_scroll;
                                    (true, Some(row))
                                } else {
                                    (false, None)
                                }
                            }
                        }
                    } else {
                        (false, None)
                    };

                    match mouse.kind {
                        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
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
                            } else if in_file_panel {
                                if let Some(row) = clicked_row {
                                    if row < state.file_panel_visible_len() {
                                        let item = state.file_panel_item_at_visible(row).cloned();
                                        if let Some(item) = item {
                                            state.file_panel_selected = row;
                                            match item {
                                                FilePanelItem::File { file_index, .. } => {
                                                    state.focused_panel = FocusedPanel::DiffView;
                                                    state.select_file(file_index);
                                                }
                                                FilePanelItem::Directory { path, .. } => {
                                                    state.focused_panel = FocusedPanel::FilePanel;
                                                    state.toggle_directory(&path);
                                                    ensure_file_panel_visible(
                                                        &mut state,
                                                        term_size.height,
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                state.focused_panel = FocusedPanel::DiffView;
                            }
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
                            if in_file_panel {
                                let max_file_panel_scroll =
                                    state.file_panel_visible_len().saturating_sub(1);
                                if scroll_delta > 0 {
                                    state.file_panel_scroll = (state.file_panel_scroll
                                        + scroll_delta as usize)
                                        .min(max_file_panel_scroll);
                                } else {
                                    state.file_panel_scroll = state
                                        .file_panel_scroll
                                        .saturating_sub((-scroll_delta) as usize);
                                }
                            } else {
                                if scroll_delta > 0 {
                                    state.scroll =
                                        (state.scroll + scroll_delta as u16).min(max_scroll as u16);
                                } else {
                                    state.scroll =
                                        state.scroll.saturating_sub((-scroll_delta) as u16);
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
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('c')
                            if (key.code == KeyCode::Esc
                                || key.modifiers.contains(KeyModifiers::CONTROL))
                                && state.search_state.has_query() =>
                        {
                            state.search_state.clear();
                        }
                        KeyCode::Char('q') | KeyCode::Esc => break 'main,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            break 'main
                        }
                        KeyCode::Char('1') => {
                            state.focused_panel = FocusedPanel::FilePanel;
                            state.show_file_panel = true;
                            if !matches!(
                                state.file_panel_item_at_visible(state.file_panel_selected),
                                Some(FilePanelItem::File { .. })
                            ) {
                                if let Some(idx) = state.file_panel_visible.iter().position(|idx| {
                                    matches!(
                                        state.file_panel_items[*idx],
                                        FilePanelItem::File { .. }
                                    )
                                }) {
                                    state.file_panel_selected = idx;
                                }
                            }
                        }
                        KeyCode::Char('2') => {
                            state.focused_panel = FocusedPanel::DiffView;
                        }
                        KeyCode::Tab => {
                            state.show_file_panel = !state.show_file_panel;
                            if !state.show_file_panel {
                                state.focused_panel = FocusedPanel::DiffView;
                            }
                        }
                        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if !state.file_diffs.is_empty() {
                                let mut next = state.file_panel_selected + 1;
                                while next < state.file_panel_visible_len() {
                                    if let Some(FilePanelItem::File { file_index, .. }) =
                                        state.file_panel_item_at_visible(next).cloned()
                                    {
                                        state.file_panel_selected = next;
                                        state.select_file(file_index);
                                        ensure_file_panel_visible(
                                            &mut state,
                                            terminal.size()?.height,
                                        );
                                        break;
                                    }
                                    next += 1;
                                }
                            }
                        }
                        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if !state.file_diffs.is_empty() && state.file_panel_selected > 0 {
                                let mut prev = state.file_panel_selected - 1;
                                loop {
                                    if let Some(FilePanelItem::File { file_index, .. }) =
                                        state.file_panel_item_at_visible(prev).cloned()
                                    {
                                        state.file_panel_selected = prev;
                                        state.select_file(file_index);
                                        ensure_file_panel_visible(&mut state, u16::MAX);
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
                                }
                            }
                        }
                        KeyCode::Char('=') => {
                            state.diff_fullscreen = DiffFullscreen::None;
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
                        KeyCode::Down | KeyCode::Char('j') => {
                            if state.focused_panel == FocusedPanel::FilePanel {
                                if state.file_panel_selected + 1 < state.file_panel_visible_len() {
                                    state.file_panel_selected += 1;
                                }
                                ensure_file_panel_visible(&mut state, terminal.size()?.height);
                            } else {
                                state.scroll = (state.scroll + 1).min(max_scroll as u16);
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if state.focused_panel == FocusedPanel::FilePanel {
                                if state.file_panel_selected > 0 {
                                    state.file_panel_selected =
                                        state.file_panel_selected.saturating_sub(1);
                                }
                                ensure_file_panel_visible(&mut state, u16::MAX);
                            } else {
                                state.scroll = state.scroll.saturating_sub(1);
                            }
                        }
                        KeyCode::Char('h') | KeyCode::Left => {
                            if state.focused_panel == FocusedPanel::DiffView {
                                state.h_scroll = state.h_scroll.saturating_sub(4);
                            } else if state.focused_panel == FocusedPanel::FilePanel {
                                state.file_panel_h_scroll =
                                    state.file_panel_h_scroll.saturating_sub(4);
                            }
                        }
                        KeyCode::Char('l') | KeyCode::Right => {
                            if state.focused_panel == FocusedPanel::DiffView {
                                state.h_scroll = state.h_scroll.saturating_add(4);
                            } else if state.focused_panel == FocusedPanel::FilePanel {
                                state.file_panel_h_scroll =
                                    state.file_panel_h_scroll.saturating_add(4);
                            }
                        }
                        KeyCode::Enter => {
                            if state.focused_panel == FocusedPanel::FilePanel
                                && state.file_panel_selected < state.file_panel_visible_len()
                            {
                                if let Some(item) = state
                                    .file_panel_item_at_visible(state.file_panel_selected)
                                    .cloned()
                                {
                                    match item {
                                        FilePanelItem::File { file_index, .. } => {
                                            state.select_file(file_index);
                                            state.focused_panel = FocusedPanel::DiffView;
                                        }
                                        FilePanelItem::Directory { path, .. } => {
                                            state.toggle_directory(&path);
                                            ensure_file_panel_visible(
                                                &mut state,
                                                terminal.size()?.height,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        KeyCode::Char(' ') => {
                            if state.focused_panel == FocusedPanel::FilePanel
                                && state.file_panel_selected < state.file_panel_visible_len()
                            {
                                let selected = state
                                    .file_panel_item_at_visible(state.file_panel_selected)
                                    .cloned();
                                if let Some(selected) = selected {
                                    match selected {
                                        FilePanelItem::File { file_index, .. } => {
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
                                        FilePanelItem::Directory { path, .. } => {
                                            let dir_prefix = format!("{}/", path);
                                            let child_indices: Vec<usize> = state
                                                .file_panel_items
                                                .iter()
                                                .filter_map(|item| {
                                                    if let FilePanelItem::File {
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
                                        .file_panel_visible
                                        .iter()
                                        .enumerate()
                                        .skip(state.file_panel_selected + 1)
                                    {
                                        if let FilePanelItem::File { file_index, .. } =
                                            &state.file_panel_items[*item_idx]
                                        {
                                            if !state.viewed_files.contains(file_index) {
                                                next_file = Some((visible_idx, *file_index));
                                                break;
                                            }
                                        }
                                    }
                                    if next_file.is_none() {
                                        for (visible_idx, item_idx) in state
                                            .file_panel_visible
                                            .iter()
                                            .enumerate()
                                            .take(state.file_panel_selected)
                                        {
                                            if let FilePanelItem::File { file_index, .. } =
                                                &state.file_panel_items[*item_idx]
                                            {
                                                if !state.viewed_files.contains(file_index) {
                                                    next_file = Some((visible_idx, *file_index));
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    if let Some((idx, file_idx)) = next_file {
                                        state.file_panel_selected = idx;
                                        state.select_file(file_idx);
                                        ensure_file_panel_visible(
                                            &mut state,
                                            terminal.size()?.height,
                                        );
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
                                let diff = &state.file_diffs[state.current_file];
                                let side_by_side = compute_side_by_side(
                                    &diff.old_content,
                                    &diff.new_content,
                                    state.settings.tab_width,
                                );
                                let hunks = find_hunk_starts(&side_by_side);
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
                                let diff = &state.file_diffs[state.current_file];
                                let side_by_side = compute_side_by_side(
                                    &diff.old_content,
                                    &diff.new_content,
                                    state.settings.tab_width,
                                );
                                let hunks = find_hunk_starts(&side_by_side);
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
                        KeyCode::Char('i') => {
                            // Add annotation to focused hunk
                            if let Some(hunk_index) = state.focused_hunk {
                                let file_index = state.current_file;
                                let diff = &state.file_diffs[file_index];

                                // Calculate line range for this hunk
                                let side_by_side = compute_side_by_side(
                                    &diff.old_content,
                                    &diff.new_content,
                                    state.settings.tab_width,
                                );
                                let hunks = find_hunk_starts(&side_by_side);
                                let hunk_start = hunks.get(hunk_index).copied().unwrap_or(0);
                                let next_hunk_start = hunks
                                    .get(hunk_index + 1)
                                    .copied()
                                    .unwrap_or(side_by_side.len());

                                // Find the actual end of the hunk (last changed line, not start of next hunk)
                                let mut actual_hunk_end = hunk_start;
                                for i in hunk_start..next_hunk_start {
                                    if let Some(dl) = side_by_side.get(i) {
                                        if !matches!(dl.change_type, ChangeType::Equal) {
                                            actual_hunk_end = i;
                                        }
                                    }
                                }

                                let start_line = side_by_side
                                    .get(hunk_start)
                                    .and_then(|dl| {
                                        dl.new_line
                                            .as_ref()
                                            .map(|(n, _)| *n)
                                            .or(dl.old_line.as_ref().map(|(n, _)| *n))
                                    })
                                    .unwrap_or(1);
                                let end_line = side_by_side
                                    .get(actual_hunk_end)
                                    .and_then(|dl| {
                                        dl.new_line
                                            .as_ref()
                                            .map(|(n, _)| *n)
                                            .or(dl.old_line.as_ref().map(|(n, _)| *n))
                                    })
                                    .unwrap_or(start_line);

                                let editor = AnnotationEditor::new(
                                    file_index,
                                    hunk_index,
                                    diff.filename.clone(),
                                    (start_line, end_line),
                                );

                                // If editing existing, pre-fill content
                                let editor = if let Some(ann) =
                                    state.get_annotation(file_index, hunk_index)
                                {
                                    editor.with_content(&ann.content, ann.created_at)
                                } else {
                                    editor
                                };

                                annotation_editor = Some(editor);
                            }
                        }
                        KeyCode::Char('I') => {
                            // Open annotations menu
                            if !state.annotations.is_empty() {
                                let mut sorted_annotations = state.annotations.clone();
                                sorted_annotations.sort_by_key(|a| a.created_at);
                                let items: Vec<String> = sorted_annotations
                                    .iter()
                                    .map(format_annotation_preview)
                                    .collect();
                                active_modal = Some(Modal::annotations(
                                    "Annotations",
                                    items,
                                    sorted_annotations,
                                ));
                            }
                        }
                        KeyCode::Char('r') => {
                            state.needs_reload = true;
                        }
                        KeyCode::Char('y') => {
                            if !state.file_diffs.is_empty() {
                                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                    let _ = clipboard
                                        .set_text(&state.file_diffs[state.current_file].filename);
                                }
                            }
                        }
                        KeyCode::Char('e') => {
                            if !state.file_diffs.is_empty() {
                                io::stdout().execute(DisableMouseCapture)?;
                                io::stdout().execute(LeaveAlternateScreen)?;
                                disable_raw_mode()?;

                                let editor =
                                    std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
                                let filename = &state.file_diffs[state.current_file].filename;

                                let line_arg = if let Some(hunk_idx) = state.focused_hunk {
                                    let diff = &state.file_diffs[state.current_file];
                                    let side_by_side = compute_side_by_side(
                                        &diff.old_content,
                                        &diff.new_content,
                                        state.settings.tab_width,
                                    );
                                    let hunks = find_hunk_starts(&side_by_side);
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

                                let status = if let Some(line) = line_arg {
                                    std::process::Command::new(&editor)
                                        .arg(format!("+{}", line))
                                        .arg(filename)
                                        .status()
                                } else {
                                    std::process::Command::new(&editor).arg(filename).status()
                                };
                                let _ = status;

                                enable_raw_mode()?;
                                io::stdout().execute(EnterAlternateScreen)?;
                                io::stdout().execute(EnableMouseCapture)?;
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
                        KeyCode::Char('/') | KeyCode::Char('f')
                            if key.code == KeyCode::Char('/')
                                || key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            state.search_state.start_forward();
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
                                                description: "Toggle file panel",
                                            },
                                            KeyBind {
                                                key: "1 / 2",
                                                description: "Focus file panel / diff",
                                            },
                                            KeyBind {
                                                key: "ctrl+j / ctrl+k",
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
                                        title: "File Panel",
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
                                                key: "/ or ctrl+f",
                                                description: "Start search",
                                            },
                                            KeyBind {
                                                key: "n or down",
                                                description: "Next match",
                                            },
                                            KeyBind {
                                                key: "N or up",
                                                description: "Previous match",
                                            },
                                            KeyBind {
                                                key: "ctrl+c or esc",
                                                description: "Cancel search",
                                            },
                                        ],
                                    },
                                    KeyBindSection {
                                        title: "Annotations",
                                        bindings: vec![
                                            KeyBind {
                                                key: "i",
                                                description: "Add annotation to focused hunk",
                                            },
                                            KeyBind {
                                                key: "I",
                                                description: "View all annotations",
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

    io::stdout().execute(DisableMouseCapture)?;
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

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
