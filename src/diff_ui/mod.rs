mod context;
mod diff;
mod git;
mod highlight;
mod modal;
mod sticky_lines;
mod types;
mod ui;
mod watcher;

use std::collections::HashSet;
use std::io;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
        KeyModifiers, MouseEventKind,
    },
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;

use crate::commit_reference::CommitReference;
use diff::{compute_side_by_side, find_hunk_starts};
use git::load_file_diffs;
pub use modal::{FilePickerItem, FileStatus as ModalFileStatus, KeyBind, KeyBindSection, Modal, ModalResult};
use types::{build_file_tree, DiffFullscreen, DiffViewSettings, FocusedPanel, SidebarItem};
use watcher::setup_watcher;

#[derive(Default, Clone, Copy, PartialEq)]
enum PendingKey {
    #[default]
    None,
    G,
}

pub struct DiffOptions {
    pub reference: Option<CommitReference>,
    pub file: Option<Vec<String>>,
    pub watch: bool,
}

pub fn run_diff_ui(options: DiffOptions) -> io::Result<()> {
    highlight::init();

    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    io::stdout().execute(EnableMouseCapture)?;

    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let watch_rx = if options.watch {
        setup_watcher()
    } else {
        None
    };

    let mut file_diffs = load_file_diffs(&options);
    let mut needs_reload = false;
    let mut focused_panel = FocusedPanel::default();
    let mut viewed_files: HashSet<usize> = HashSet::new();
    let mut show_sidebar = true;
    let mut sidebar_items = build_file_tree(&file_diffs);
    let mut sidebar_selected: usize = sidebar_items
        .iter()
        .position(|item| matches!(item, SidebarItem::File { .. }))
        .unwrap_or(0);
    let mut current_file: usize = sidebar_items
        .get(sidebar_selected)
        .and_then(|item| {
            if let SidebarItem::File { file_index, .. } = item {
                Some(*file_index)
            } else {
                None
            }
        })
        .unwrap_or(0);
    let settings = DiffViewSettings::default();
    let mut diff_fullscreen = DiffFullscreen::default();
    let mut scroll: u16 = if !file_diffs.is_empty() && current_file < file_diffs.len() {
        let diff = &file_diffs[current_file];
        let side_by_side = compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
        let hunks = find_hunk_starts(&side_by_side);
        hunks
            .first()
            .map(|&h| (h as u16).saturating_sub(5))
            .unwrap_or(0)
    } else {
        0
    };
    let mut sidebar_scroll: usize = 0;
    let mut h_scroll: u16 = 0;
    let mut active_modal: Option<Modal> = None;
    let mut pending_key = PendingKey::default();

    loop {
        if let Some(ref rx) = watch_rx {
            match rx.try_recv() {
                Ok(()) => needs_reload = true,
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {}
            }
        }

        if needs_reload {
            let old_filename = file_diffs.get(current_file).map(|f| f.filename.clone());
            file_diffs = load_file_diffs(&options);
            sidebar_items = build_file_tree(&file_diffs);

            if let Some(name) = old_filename {
                current_file = file_diffs
                    .iter()
                    .position(|f| f.filename == name)
                    .unwrap_or(0);
            }
            if current_file >= file_diffs.len() && !file_diffs.is_empty() {
                current_file = file_diffs.len() - 1;
            }
            if let Some(idx) = sidebar_items.iter().position(|item| {
                matches!(item, SidebarItem::File { file_index, .. } if *file_index == current_file)
            }) {
                sidebar_selected = idx;
            } else {
                sidebar_selected = sidebar_items
                    .iter()
                    .position(|item| matches!(item, SidebarItem::File { .. }))
                    .unwrap_or(0);
            }
            if !file_diffs.is_empty() {
                let diff = &file_diffs[current_file];
                let side_by_side = compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
                let hunks = find_hunk_starts(&side_by_side);
                scroll = hunks
                    .first()
                    .map(|&h| (h as u16).saturating_sub(5))
                    .unwrap_or(0);
                h_scroll = 0;
            }
            needs_reload = false;
        }

        if file_diffs.is_empty() {
            terminal.draw(|frame| {
                ui::render_empty_state(frame, options.watch);
                if let Some(ref modal) = active_modal {
                    modal.render(frame);
                }
            })?;
        } else {
            let diff = &file_diffs[current_file];
            let side_by_side = compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
            let hunk_count = find_hunk_starts(&side_by_side).len();
            terminal.draw(|frame| {
                ui::render_diff(
                    frame,
                    diff,
                    &file_diffs,
                    &sidebar_items,
                    current_file,
                    scroll,
                    h_scroll,
                    options.watch,
                    show_sidebar,
                    focused_panel,
                    sidebar_selected,
                    sidebar_scroll,
                    &viewed_files,
                    &settings,
                    hunk_count,
                    diff_fullscreen,
                );
                if let Some(ref modal) = active_modal {
                    modal.render(frame);
                }
            })?;
        }

        if event::poll(Duration::from_millis(100))? {
            let visible_height = terminal.size()?.height.saturating_sub(2) as usize;
            let bottom_padding = 5;
            let max_scroll = if !file_diffs.is_empty() {
                let diff = &file_diffs[current_file];
                let total_lines = compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width).len();
                total_lines.saturating_sub(visible_height.saturating_sub(bottom_padding))
            } else {
                0
            };

            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press && active_modal.is_some() => {
                    if let Some(ref mut modal) = active_modal {
                        if let Some(result) = modal.handle_input(key) {
                            if let ModalResult::FileSelected(file_index) = result {
                                current_file = file_index;
                                diff_fullscreen = DiffFullscreen::None;
                                if let Some(idx) = sidebar_items.iter().position(|item| {
                                    matches!(item, SidebarItem::File { file_index: fi, .. } if *fi == current_file)
                                }) {
                                    sidebar_selected = idx;
                                    let visible_height = terminal.size()?.height.saturating_sub(5) as usize;
                                    if sidebar_selected >= sidebar_scroll + visible_height {
                                        sidebar_scroll = sidebar_selected.saturating_sub(visible_height) + 1;
                                    } else if sidebar_selected < sidebar_scroll {
                                        sidebar_scroll = sidebar_selected;
                                    }
                                }
                                let diff = &file_diffs[current_file];
                                let side_by_side = compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
                                let hunks = find_hunk_starts(&side_by_side);
                                scroll = hunks.first().map(|&h| (h as u16).saturating_sub(5)).unwrap_or(0);
                                h_scroll = 0;
                            }
                            active_modal = None;
                        }
                    }
                }
                Event::Mouse(mouse) if active_modal.is_none() => {
                    let term_size = terminal.size()?;
                    let footer_height = 1u16;
                    let sidebar_width = if show_sidebar { 40u16 } else { 0u16 };

                    match mouse.kind {
                        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                            if show_sidebar
                                && mouse.column < sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height)
                            {
                                let clicked_row =
                                    (mouse.row.saturating_sub(1)) as usize + sidebar_scroll;
                                if clicked_row < sidebar_items.len() {
                                    if matches!(sidebar_items[clicked_row], SidebarItem::File { .. }) {
                                        sidebar_selected = clicked_row;
                                        focused_panel = FocusedPanel::DiffView;
                                        if let SidebarItem::File { file_index, .. } =
                                            &sidebar_items[sidebar_selected]
                                        {
                                            current_file = *file_index;
                                            diff_fullscreen = DiffFullscreen::None;
                                            let diff = &file_diffs[current_file];
                                            let side_by_side = compute_side_by_side(
                                                &diff.old_content,
                                                &diff.new_content,
                                                settings.tab_width,
                                            );
                                            let hunks = find_hunk_starts(&side_by_side);
                                            scroll = hunks
                                                .first()
                                                .map(|&h| (h as u16).saturating_sub(5))
                                                .unwrap_or(0);
                                            h_scroll = 0;
                                        }
                                    }
                                }
                            } else if mouse.column >= sidebar_width {
                                focused_panel = FocusedPanel::DiffView;
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            if show_sidebar
                                && mouse.column < sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height)
                            {
                                let max_sidebar_scroll =
                                    sidebar_items.len().saturating_sub(1);
                                sidebar_scroll = (sidebar_scroll + 3).min(max_sidebar_scroll);
                            } else if mouse.column >= sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height)
                            {
                                scroll = (scroll + 3).min(max_scroll as u16);
                            }
                        }
                        MouseEventKind::ScrollUp => {
                            if show_sidebar
                                && mouse.column < sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height)
                            {
                                sidebar_scroll = sidebar_scroll.saturating_sub(3);
                            } else if mouse.column >= sidebar_width
                                && mouse.row < term_size.height.saturating_sub(footer_height)
                            {
                                scroll = scroll.saturating_sub(3);
                            }
                        }
                        _ => {}
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press && active_modal.is_none() => {
                    if key.code != KeyCode::Char('g') {
                        pending_key = PendingKey::None;
                    }
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                        KeyCode::Char('1') => {
                            focused_panel = FocusedPanel::Sidebar;
                            show_sidebar = true;
                            if !matches!(
                                sidebar_items.get(sidebar_selected),
                                Some(SidebarItem::File { .. })
                            ) {
                                if let Some(idx) = sidebar_items
                                    .iter()
                                    .position(|item| matches!(item, SidebarItem::File { .. }))
                                {
                                    sidebar_selected = idx;
                                }
                            }
                        }
                        KeyCode::Char('2') => {
                            focused_panel = FocusedPanel::DiffView;
                        }
                        KeyCode::Tab => {
                            show_sidebar = !show_sidebar;
                            if !show_sidebar {
                                focused_panel = FocusedPanel::DiffView;
                            }
                        }
                        KeyCode::Char('j')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            // Ctrl+j: next file (in sidebar visual order)
                            if !file_diffs.is_empty() {
                                // Find next file item after current sidebar_selected
                                let mut next = sidebar_selected + 1;
                                while next < sidebar_items.len() {
                                    if let SidebarItem::File { file_index, .. } = &sidebar_items[next] {
                                        sidebar_selected = next;
                                        current_file = *file_index;
                                        diff_fullscreen = DiffFullscreen::None;
                                        // Auto-scroll sidebar
                                        let visible_height =
                                            terminal.size()?.height.saturating_sub(5) as usize;
                                        if sidebar_selected >= sidebar_scroll + visible_height {
                                            sidebar_scroll =
                                                sidebar_selected.saturating_sub(visible_height) + 1;
                                        } else if sidebar_selected < sidebar_scroll {
                                            sidebar_scroll = sidebar_selected;
                                        }
                                        let diff = &file_diffs[current_file];
                                        let side_by_side =
                                            compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
                                        let hunks = find_hunk_starts(&side_by_side);
                                        scroll = hunks
                                            .first()
                                            .map(|&h| (h as u16).saturating_sub(5))
                                            .unwrap_or(0);
                                        h_scroll = 0;
                                        break;
                                    }
                                    next += 1;
                                }
                            }
                        }
                        KeyCode::Char('k')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            // Ctrl+k: previous file (in sidebar visual order)
                            if !file_diffs.is_empty() && sidebar_selected > 0 {
                                // Find previous file item before current sidebar_selected
                                let mut prev = sidebar_selected - 1;
                                loop {
                                    if let SidebarItem::File { file_index, .. } = &sidebar_items[prev] {
                                        sidebar_selected = prev;
                                        current_file = *file_index;
                                        diff_fullscreen = DiffFullscreen::None;
                                        // Auto-scroll sidebar
                                        if sidebar_selected < sidebar_scroll {
                                            sidebar_scroll = sidebar_selected;
                                        }
                                        let diff = &file_diffs[current_file];
                                        let side_by_side =
                                            compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
                                        let hunks = find_hunk_starts(&side_by_side);
                                        scroll = hunks
                                            .first()
                                            .map(|&h| (h as u16).saturating_sub(5))
                                            .unwrap_or(0);
                                        h_scroll = 0;
                                        break;
                                    }
                                    if prev == 0 {
                                        break;
                                    }
                                    prev -= 1;
                                }
                            }
                        }
                        KeyCode::Char('d')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            // Ctrl+d: scroll down half a screen (vim behavior)
                            let half_screen = (visible_height / 2) as u16;
                            scroll = (scroll + half_screen).min(max_scroll as u16);
                        }
                        KeyCode::Char('u')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            // Ctrl+u: scroll up half a screen (vim behavior)
                            let half_screen = (visible_height / 2) as u16;
                            scroll = scroll.saturating_sub(half_screen);
                        }
                        KeyCode::Char('p')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            // Ctrl+p: open file picker (telescope-style fuzzy finder)
                            if !file_diffs.is_empty() {
                                let items: Vec<FilePickerItem> = file_diffs
                                    .iter()
                                    .enumerate()
                                    .map(|(i, diff)| {
                                        let status = match diff.status {
                                            types::FileStatus::Added => ModalFileStatus::Added,
                                            types::FileStatus::Modified => ModalFileStatus::Modified,
                                            types::FileStatus::Deleted => ModalFileStatus::Deleted,
                                        };
                                        FilePickerItem {
                                            name: diff.filename.clone(),
                                            file_index: i,
                                            status,
                                            viewed: viewed_files.contains(&i),
                                        }
                                    })
                                    .collect();
                                active_modal = Some(Modal::file_picker("Find File", items));
                            }
                        }
                        KeyCode::Char(']') => {
                            // Toggle new panel fullscreen (only if new content exists)
                            if !file_diffs.is_empty() {
                                let diff = &file_diffs[current_file];
                                if !diff.new_content.is_empty() {
                                    diff_fullscreen = match diff_fullscreen {
                                        DiffFullscreen::NewOnly => DiffFullscreen::None,
                                        _ => DiffFullscreen::NewOnly,
                                    };
                                }
                            }
                        }
                        KeyCode::Char('[') => {
                            // Toggle old panel fullscreen (only if old content exists)
                            if !file_diffs.is_empty() {
                                let diff = &file_diffs[current_file];
                                if !diff.old_content.is_empty() {
                                    diff_fullscreen = match diff_fullscreen {
                                        DiffFullscreen::OldOnly => DiffFullscreen::None,
                                        _ => DiffFullscreen::OldOnly,
                                    };
                                }
                            }
                        }
                        KeyCode::Char('=') => {
                            // Reset to side-by-side view
                            diff_fullscreen = DiffFullscreen::None;
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if focused_panel == FocusedPanel::Sidebar {
                                let mut next = sidebar_selected + 1;
                                while next < sidebar_items.len() {
                                    if matches!(sidebar_items[next], SidebarItem::File { .. }) {
                                        sidebar_selected = next;
                                        break;
                                    }
                                    next += 1;
                                }
                                // Auto-scroll sidebar to keep selection visible
                                let visible_height =
                                    terminal.size()?.height.saturating_sub(5) as usize;
                                if sidebar_selected >= sidebar_scroll + visible_height {
                                    sidebar_scroll =
                                        sidebar_selected.saturating_sub(visible_height) + 1;
                                }
                            } else {
                                scroll = (scroll + 1).min(max_scroll as u16);
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if focused_panel == FocusedPanel::Sidebar {
                                if sidebar_selected > 0 {
                                    let mut prev = sidebar_selected - 1;
                                    loop {
                                        if matches!(sidebar_items[prev], SidebarItem::File { .. }) {
                                            sidebar_selected = prev;
                                            break;
                                        }
                                        if prev == 0 {
                                            break;
                                        }
                                        prev -= 1;
                                    }
                                }
                                // Auto-scroll sidebar to keep selection visible
                                if sidebar_selected < sidebar_scroll {
                                    sidebar_scroll = sidebar_selected;
                                }
                            } else {
                                scroll = scroll.saturating_sub(1);
                            }
                        }
                        KeyCode::Char('h') | KeyCode::Left => {
                            if focused_panel == FocusedPanel::DiffView {
                                h_scroll = h_scroll.saturating_sub(4);
                            }
                        }
                        KeyCode::Char('l') | KeyCode::Right => {
                            if focused_panel == FocusedPanel::DiffView {
                                h_scroll = h_scroll.saturating_add(4);
                            }
                        }
                        KeyCode::Enter => {
                            if focused_panel == FocusedPanel::Sidebar
                                && sidebar_selected < sidebar_items.len()
                            {
                                if let SidebarItem::File { file_index, .. } =
                                    &sidebar_items[sidebar_selected]
                                {
                                    current_file = *file_index;
                                    diff_fullscreen = DiffFullscreen::None;
                                    let diff = &file_diffs[current_file];
                                    let side_by_side =
                                        compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
                                    let hunks = find_hunk_starts(&side_by_side);
                                    scroll = hunks
                                        .first()
                                        .map(|&h| (h as u16).saturating_sub(5))
                                        .unwrap_or(0);
                                    h_scroll = 0;
                                    focused_panel = FocusedPanel::DiffView;
                                }
                            }
                        }
                        KeyCode::Char(' ') => {
                            if focused_panel == FocusedPanel::Sidebar
                                && sidebar_selected < sidebar_items.len()
                            {
                                match &sidebar_items[sidebar_selected] {
                                    SidebarItem::File { file_index, .. } => {
                                        if viewed_files.contains(file_index) {
                                            viewed_files.remove(file_index);
                                        } else {
                                            viewed_files.insert(*file_index);
                                        }
                                    }
                                    SidebarItem::Directory { path, .. } => {
                                        let dir_prefix = format!("{}/", path);
                                        let child_indices: Vec<usize> = sidebar_items
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

                                        let all_viewed =
                                            child_indices.iter().all(|i| viewed_files.contains(i));
                                        if all_viewed {
                                            for idx in child_indices {
                                                viewed_files.remove(&idx);
                                            }
                                        } else {
                                            for idx in child_indices {
                                                viewed_files.insert(idx);
                                            }
                                        }
                                    }
                                }
                            } else if focused_panel == FocusedPanel::DiffView {
                                // Toggle viewed status; if marking as viewed, move to next non-viewed file
                                if viewed_files.contains(&current_file) {
                                    viewed_files.remove(&current_file);
                                } else {
                                    viewed_files.insert(current_file);
                                    // Find next non-viewed file in sidebar order, wrapping around if needed
                                    // First, try files after current position
                                    let mut next_file: Option<(usize, usize)> = None;
                                    for (idx, item) in sidebar_items.iter().enumerate().skip(sidebar_selected + 1) {
                                        if let SidebarItem::File { file_index, .. } = item {
                                            if !viewed_files.contains(file_index) {
                                                next_file = Some((idx, *file_index));
                                                break;
                                            }
                                        }
                                    }
                                    // If not found, wrap around and search from beginning
                                    if next_file.is_none() {
                                        for (idx, item) in sidebar_items.iter().enumerate().take(sidebar_selected) {
                                            if let SidebarItem::File { file_index, .. } = item {
                                                if !viewed_files.contains(file_index) {
                                                    next_file = Some((idx, *file_index));
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    if let Some((idx, file_idx)) = next_file {
                                        sidebar_selected = idx;
                                        current_file = file_idx;
                                        diff_fullscreen = DiffFullscreen::None;
                                        // Auto-scroll sidebar
                                        let visible_height =
                                            terminal.size()?.height.saturating_sub(5) as usize;
                                        if sidebar_selected >= sidebar_scroll + visible_height {
                                            sidebar_scroll =
                                                sidebar_selected.saturating_sub(visible_height) + 1;
                                        } else if sidebar_selected < sidebar_scroll {
                                            sidebar_scroll = sidebar_selected;
                                        }
                                        let diff = &file_diffs[current_file];
                                        let side_by_side =
                                            compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
                                        let hunks = find_hunk_starts(&side_by_side);
                                        scroll = hunks
                                            .first()
                                            .map(|&h| (h as u16).saturating_sub(5))
                                            .unwrap_or(0);
                                        h_scroll = 0;
                                    }
                                }
                            }
                        }
                        KeyCode::PageDown => {
                            scroll = (scroll + 20).min(max_scroll as u16);
                        }
                        KeyCode::PageUp => {
                            scroll = scroll.saturating_sub(20);
                        }
                        KeyCode::Char('}') => {
                            if !file_diffs.is_empty() {
                                let diff = &file_diffs[current_file];
                                let side_by_side =
                                    compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
                                let hunks = find_hunk_starts(&side_by_side);
                                if let Some(&next) =
                                    hunks.iter().find(|&&h| h > scroll as usize + 5)
                                {
                                    scroll = (next as u16).saturating_sub(5);
                                }
                            }
                        }
                        KeyCode::Char('{') => {
                            if !file_diffs.is_empty() {
                                let diff = &file_diffs[current_file];
                                let side_by_side =
                                    compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
                                let hunks = find_hunk_starts(&side_by_side);
                                if let Some(&prev) = hunks
                                    .iter()
                                    .rev()
                                    .find(|&&h| (h as u16) < scroll.saturating_sub(5))
                                {
                                    scroll = (prev as u16).saturating_sub(5);
                                }
                            }
                        }
                        KeyCode::Char('r') => {
                            needs_reload = true;
                        }
                        KeyCode::Char('y') => {
                            if !file_diffs.is_empty() {
                                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                    let _ = clipboard.set_text(&file_diffs[current_file].filename);
                                }
                            }
                        }
                        KeyCode::Char('e') => {
                            if !file_diffs.is_empty() {
                                io::stdout().execute(DisableMouseCapture)?;
                                io::stdout().execute(LeaveAlternateScreen)?;
                                disable_raw_mode()?;

                                let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
                                let _ = std::process::Command::new(&editor)
                                    .arg(&file_diffs[current_file].filename)
                                    .status();

                                enable_raw_mode()?;
                                io::stdout().execute(EnterAlternateScreen)?;
                                io::stdout().execute(EnableMouseCapture)?;
                                terminal.clear()?;
                            }
                        }
                        KeyCode::Char('g') => {
                            if pending_key == PendingKey::G {
                                scroll = 0;
                                pending_key = PendingKey::None;
                            } else {
                                pending_key = PendingKey::G;
                            }
                        }
                        KeyCode::Char('G') => {
                            scroll = max_scroll as u16;
                        }
                        KeyCode::Char('?') => {
                            active_modal = Some(Modal::keybindings(
                                "Keybindings",
                                vec![
                                    KeyBindSection {
                                        title: "Global",
                                        bindings: vec![
                                            KeyBind { key: "q / esc", description: "Quit" },
                                            KeyBind { key: "tab", description: "Toggle sidebar" },
                                            KeyBind { key: "1 / 2", description: "Focus sidebar / diff" },
                                            KeyBind { key: "ctrl+j / ctrl+k", description: "Next / previous file" },
                                            KeyBind { key: "ctrl+d / ctrl+u", description: "Scroll half page down / up" },
                                            KeyBind { key: "ctrl+p", description: "Open file picker" },
                                            KeyBind { key: "r", description: "Reload diff" },
                                            KeyBind { key: "y", description: "Copy current filename" },
                                            KeyBind { key: "e", description: "Open current file in editor" },
                                            KeyBind { key: "?", description: "Show keybindings" },
                                        ],
                                    },
                                    KeyBindSection {
                                        title: "Sidebar",
                                        bindings: vec![
                                            KeyBind { key: "j/k or up/down", description: "Navigate files" },
                                            KeyBind { key: "enter", description: "Open file in diff view" },
                                            KeyBind { key: "space", description: "Toggle file as viewed" },
                                        ],
                                    },
                                    KeyBindSection {
                                        title: "Diff View",
                                        bindings: vec![
                                            KeyBind { key: "j/k or up/down", description: "Scroll vertically" },
                                            KeyBind { key: "h/l or left/right", description: "Scroll horizontally" },
                                            KeyBind { key: "gg / G", description: "Scroll to top / bottom" },
                                            KeyBind { key: "{ / }", description: "Previous / next hunk" },
                                            KeyBind { key: "pageup / pagedown", description: "Scroll by page" },
                                            KeyBind { key: "space", description: "Mark viewed & next file" },
                                            KeyBind { key: "]", description: "Toggle new panel fullscreen" },
                                            KeyBind { key: "[", description: "Toggle old panel fullscreen" },
                                            KeyBind { key: "=", description: "Reset fullscreen to side-by-side" },
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
