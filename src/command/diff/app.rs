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

use super::diff_algo::{compute_side_by_side, find_hunk_starts};
use super::git::{get_current_branch, load_file_diffs, load_pr_file_diffs};
use super::highlight;
use super::render::{
    render_diff, render_empty_state, FilePickerItem, KeyBind, KeyBindSection, Modal,
    ModalFileStatus, ModalResult,
};
use super::state::{adjust_scroll_for_hunk, adjust_scroll_to_line, AppState, PendingKey};
use super::theme;
use super::types::{DiffFullscreen, FileStatus, FocusedPanel, SidebarItem};
use super::watcher::{setup_watcher, WatchEvent};
use super::{
    fetch_viewed_files, mark_file_as_viewed_async, unmark_file_as_viewed_async, DiffOptions, PrInfo,
};

pub fn run_app_with_pr(options: DiffOptions, pr_info: PrInfo) -> io::Result<()> {
    match load_pr_file_diffs(&pr_info) {
        Ok(file_diffs) => run_app_internal(options, Some(pr_info), file_diffs),
        Err(e) => {
            eprintln!("\x1b[91merror:\x1b[0m {}", e);
            std::process::exit(1);
        }
    }
}

pub fn run_app(options: DiffOptions, pr_info: Option<PrInfo>) -> io::Result<()> {
    let file_diffs = load_file_diffs(&options);
    run_app_internal(options, pr_info, file_diffs)
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
) -> io::Result<()> {
    theme::init();
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

    let mut state = AppState::new(file_diffs);
    let mut active_modal: Option<Modal> = None;
    let mut pending_watch_event: Option<WatchEvent> = None;

    // Load viewed files from GitHub on startup in PR mode
    if let Some(ref pr) = pr_info {
        sync_viewed_files_from_github(pr, &mut state);
    }

    loop {
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
                    Err(_) => Vec::new(), // On error, show empty state
                }
            } else {
                load_file_diffs(&options)
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
            let branch = get_current_branch();
            terminal.draw(|frame| {
                render_diff(
                    frame,
                    diff,
                    &state.file_diffs,
                    &state.sidebar_items,
                    state.current_file,
                    state.scroll,
                    state.h_scroll,
                    options.watch,
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
                    &branch,
                    pr_info.as_ref(),
                    state.focused_hunk,
                    &hunks,
                );
                if let Some(ref modal) = active_modal {
                    modal.render(frame);
                }
            })?;
        }

        if event::poll(Duration::from_millis(100))? {
            let visible_height = terminal.size()?.height.saturating_sub(2) as usize;
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

            match event::read()? {
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
                Event::Key(key) if key.kind == KeyEventKind::Press && active_modal.is_some() => {
                    if let Some(ref mut modal) = active_modal {
                        if let Some(result) = modal.handle_input(key) {
                            if let ModalResult::FileSelected(file_index) = result {
                                state.select_file(file_index);
                                if let Some(idx) = state.sidebar_items.iter().position(|item| {
                                    matches!(item, SidebarItem::File { file_index: fi, .. } if *fi == state.current_file)
                                }) {
                                    state.sidebar_selected = idx;
                                    let visible_height =
                                        terminal.size()?.height.saturating_sub(5) as usize;
                                    if state.sidebar_selected
                                        >= state.sidebar_scroll + visible_height
                                    {
                                        state.sidebar_scroll = state
                                            .sidebar_selected
                                            .saturating_sub(visible_height)
                                            + 1;
                                    } else if state.sidebar_selected < state.sidebar_scroll {
                                        state.sidebar_scroll = state.sidebar_selected;
                                    }
                                }
                            }
                            active_modal = None;
                        }
                    }
                }
                Event::Mouse(mouse) if active_modal.is_none() => {
                    let term_size = terminal.size()?;
                    let footer_height = 1u16;
                    let sidebar_width = if state.show_sidebar { 40u16 } else { 0u16 };

                    match mouse.kind {
                        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                            if state.show_sidebar
                                && mouse.column < sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height)
                            {
                                let clicked_row =
                                    (mouse.row.saturating_sub(1)) as usize + state.sidebar_scroll;
                                if clicked_row < state.sidebar_items.len() {
                                    if matches!(
                                        state.sidebar_items[clicked_row],
                                        SidebarItem::File { .. }
                                    ) {
                                        state.sidebar_selected = clicked_row;
                                        state.focused_panel = FocusedPanel::DiffView;
                                        if let SidebarItem::File { file_index, .. } =
                                            &state.sidebar_items[state.sidebar_selected]
                                        {
                                            state.select_file(*file_index);
                                        }
                                    }
                                }
                            } else if mouse.column >= sidebar_width {
                                state.focused_panel = FocusedPanel::DiffView;
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            if state.show_sidebar
                                && mouse.column < sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height)
                            {
                                let max_sidebar_scroll =
                                    state.sidebar_items.len().saturating_sub(1);
                                state.sidebar_scroll =
                                    (state.sidebar_scroll + 3).min(max_sidebar_scroll);
                            } else if mouse.column >= sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height)
                            {
                                state.scroll = (state.scroll + 3).min(max_scroll as u16);
                            }
                        }
                        MouseEventKind::ScrollUp => {
                            if state.show_sidebar
                                && mouse.column < sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height)
                            {
                                state.sidebar_scroll = state.sidebar_scroll.saturating_sub(3);
                            } else if mouse.column >= sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height)
                            {
                                state.scroll = state.scroll.saturating_sub(3);
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
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            break
                        }
                        KeyCode::Char('1') => {
                            state.focused_panel = FocusedPanel::Sidebar;
                            state.show_sidebar = true;
                            if !matches!(
                                state.sidebar_items.get(state.sidebar_selected),
                                Some(SidebarItem::File { .. })
                            ) {
                                if let Some(idx) = state
                                    .sidebar_items
                                    .iter()
                                    .position(|item| matches!(item, SidebarItem::File { .. }))
                                {
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
                        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if !state.file_diffs.is_empty() {
                                let mut next = state.sidebar_selected + 1;
                                while next < state.sidebar_items.len() {
                                    if let SidebarItem::File { file_index, .. } =
                                        &state.sidebar_items[next]
                                    {
                                        state.sidebar_selected = next;
                                        state.select_file(*file_index);
                                        let visible_height =
                                            terminal.size()?.height.saturating_sub(5) as usize;
                                        if state.sidebar_selected
                                            >= state.sidebar_scroll + visible_height
                                        {
                                            state.sidebar_scroll = state
                                                .sidebar_selected
                                                .saturating_sub(visible_height)
                                                + 1;
                                        } else if state.sidebar_selected < state.sidebar_scroll {
                                            state.sidebar_scroll = state.sidebar_selected;
                                        }
                                        break;
                                    }
                                    next += 1;
                                }
                            }
                        }
                        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if !state.file_diffs.is_empty() && state.sidebar_selected > 0 {
                                let mut prev = state.sidebar_selected - 1;
                                loop {
                                    if let SidebarItem::File { file_index, .. } =
                                        &state.sidebar_items[prev]
                                    {
                                        state.sidebar_selected = prev;
                                        state.select_file(*file_index);
                                        if state.sidebar_selected < state.sidebar_scroll {
                                            state.sidebar_scroll = state.sidebar_selected;
                                        }
                                        break;
                                    }
                                    if prev == 0 {
                                        break;
                                    }
                                    prev -= 1;
                                }
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
                            if state.focused_panel == FocusedPanel::Sidebar {
                                let mut next = state.sidebar_selected + 1;
                                while next < state.sidebar_items.len() {
                                    if matches!(state.sidebar_items[next], SidebarItem::File { .. })
                                    {
                                        state.sidebar_selected = next;
                                        break;
                                    }
                                    next += 1;
                                }
                                let visible_height =
                                    terminal.size()?.height.saturating_sub(5) as usize;
                                if state.sidebar_selected >= state.sidebar_scroll + visible_height {
                                    state.sidebar_scroll =
                                        state.sidebar_selected.saturating_sub(visible_height) + 1;
                                }
                            } else {
                                state.scroll = (state.scroll + 1).min(max_scroll as u16);
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if state.focused_panel == FocusedPanel::Sidebar {
                                if state.sidebar_selected > 0 {
                                    let mut prev = state.sidebar_selected - 1;
                                    loop {
                                        if matches!(
                                            state.sidebar_items[prev],
                                            SidebarItem::File { .. }
                                        ) {
                                            state.sidebar_selected = prev;
                                            break;
                                        }
                                        if prev == 0 {
                                            break;
                                        }
                                        prev -= 1;
                                    }
                                }
                                if state.sidebar_selected < state.sidebar_scroll {
                                    state.sidebar_scroll = state.sidebar_selected;
                                }
                            } else {
                                state.scroll = state.scroll.saturating_sub(1);
                            }
                        }
                        KeyCode::Char('h') | KeyCode::Left => {
                            if state.focused_panel == FocusedPanel::DiffView {
                                state.h_scroll = state.h_scroll.saturating_sub(4);
                            } else if state.focused_panel == FocusedPanel::Sidebar {
                                state.sidebar_h_scroll = state.sidebar_h_scroll.saturating_sub(4);
                            }
                        }
                        KeyCode::Char('l') | KeyCode::Right => {
                            if state.focused_panel == FocusedPanel::DiffView {
                                state.h_scroll = state.h_scroll.saturating_add(4);
                            } else if state.focused_panel == FocusedPanel::Sidebar {
                                state.sidebar_h_scroll = state.sidebar_h_scroll.saturating_add(4);
                            }
                        }
                        KeyCode::Enter => {
                            if state.focused_panel == FocusedPanel::Sidebar
                                && state.sidebar_selected < state.sidebar_items.len()
                            {
                                if let SidebarItem::File { file_index, .. } =
                                    &state.sidebar_items[state.sidebar_selected]
                                {
                                    state.select_file(*file_index);
                                    state.focused_panel = FocusedPanel::DiffView;
                                }
                            }
                        }
                        KeyCode::Char(' ') => {
                            if state.focused_panel == FocusedPanel::Sidebar
                                && state.sidebar_selected < state.sidebar_items.len()
                            {
                                match &state.sidebar_items[state.sidebar_selected] {
                                    SidebarItem::File { file_index, .. } => {
                                        let file_idx = *file_index;
                                        let filename = state.file_diffs[file_idx].filename.clone();
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
                                    for (idx, item) in state
                                        .sidebar_items
                                        .iter()
                                        .enumerate()
                                        .skip(state.sidebar_selected + 1)
                                    {
                                        if let SidebarItem::File { file_index, .. } = item {
                                            if !state.viewed_files.contains(file_index) {
                                                next_file = Some((idx, *file_index));
                                                break;
                                            }
                                        }
                                    }
                                    if next_file.is_none() {
                                        for (idx, item) in state
                                            .sidebar_items
                                            .iter()
                                            .enumerate()
                                            .take(state.sidebar_selected)
                                        {
                                            if let SidebarItem::File { file_index, .. } = item {
                                                if !state.viewed_files.contains(file_index) {
                                                    next_file = Some((idx, *file_index));
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
                                        if state.sidebar_selected
                                            >= state.sidebar_scroll + visible_height
                                        {
                                            state.sidebar_scroll = state
                                                .sidebar_selected
                                                .saturating_sub(visible_height)
                                                + 1;
                                        } else if state.sidebar_selected < state.sidebar_scroll {
                                            state.sidebar_scroll = state.sidebar_selected;
                                        }
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
                                                description: "Toggle sidebar",
                                            },
                                            KeyBind {
                                                key: "1 / 2",
                                                description: "Focus sidebar / diff",
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
                                                description: "Open file in diff view",
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
