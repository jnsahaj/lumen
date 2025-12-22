mod diff;
mod git;
mod highlight;
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

use diff::{compute_side_by_side, find_hunk_starts};
use git::load_file_diffs;
use types::{build_file_tree, DiffViewSettings, FocusedPanel, SidebarItem};
use watcher::setup_watcher;

pub struct DiffOptions {
    pub sha: String,
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
    let mut current_file: usize = 0;
    let mut scroll: u16 = if !file_diffs.is_empty() {
        let diff = &file_diffs[0];
        let side_by_side = compute_side_by_side(&diff.old_content, &diff.new_content);
        let hunks = find_hunk_starts(&side_by_side);
        hunks
            .first()
            .map(|&h| (h as u16).saturating_sub(5))
            .unwrap_or(0)
    } else {
        0
    };
    let mut needs_reload = false;
    let mut focused_panel = FocusedPanel::default();
    let mut viewed_files: HashSet<usize> = HashSet::new();
    let mut show_sidebar = true;
    let mut sidebar_items = build_file_tree(&file_diffs);
    let mut sidebar_selected: usize = sidebar_items
        .iter()
        .position(|item| matches!(item, SidebarItem::File { .. }))
        .unwrap_or(0);
    let mut sidebar_scroll: usize = 0;
    let mut h_scroll: u16 = 0;
    let settings = DiffViewSettings::default();

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
                let side_by_side = compute_side_by_side(&diff.old_content, &diff.new_content);
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
            })?;
        } else {
            let diff = &file_diffs[current_file];
            let side_by_side = compute_side_by_side(&diff.old_content, &diff.new_content);
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
                );
            })?;
        }

        if event::poll(Duration::from_millis(100))? {
            let max_scroll = if !file_diffs.is_empty() {
                let diff = &file_diffs[current_file];
                compute_side_by_side(&diff.old_content, &diff.new_content)
                    .len()
                    .saturating_sub(1)
            } else {
                0
            };

            match event::read()? {
                Event::Mouse(mouse) => {
                    let term_size = terminal.size()?;
                    let header_height = 3u16;
                    let sidebar_width = if show_sidebar { 40u16 } else { 0u16 };

                    match mouse.kind {
                        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                            if show_sidebar
                                && mouse.column < sidebar_width
                                && mouse.row >= header_height
                            {
                                let clicked_row =
                                    (mouse.row - header_height - 1) as usize + sidebar_scroll;
                                if clicked_row < sidebar_items.len() {
                                    if matches!(sidebar_items[clicked_row], SidebarItem::File { .. }) {
                                        sidebar_selected = clicked_row;
                                        focused_panel = FocusedPanel::DiffView;
                                        if let SidebarItem::File { file_index, .. } =
                                            &sidebar_items[sidebar_selected]
                                        {
                                            current_file = *file_index;
                                            let diff = &file_diffs[current_file];
                                            let side_by_side = compute_side_by_side(
                                                &diff.old_content,
                                                &diff.new_content,
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
                                && mouse.row >= header_height
                            {
                                let max_sidebar_scroll =
                                    sidebar_items.len().saturating_sub(1);
                                sidebar_scroll = (sidebar_scroll + 3).min(max_sidebar_scroll);
                            } else if mouse.column >= sidebar_width
                                && mouse.row >= header_height
                                && mouse.row < term_size.height
                            {
                                scroll = (scroll + 3).min(max_scroll as u16);
                            }
                        }
                        MouseEventKind::ScrollUp => {
                            if show_sidebar
                                && mouse.column < sidebar_width
                                && mouse.row >= header_height
                            {
                                sidebar_scroll = sidebar_scroll.saturating_sub(3);
                            } else if mouse.column >= sidebar_width
                                && mouse.row >= header_height
                                && mouse.row < term_size.height
                            {
                                scroll = scroll.saturating_sub(3);
                            }
                        }
                        _ => {}
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
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
                            // Ctrl+j: next file
                            if !file_diffs.is_empty() && current_file < file_diffs.len() - 1 {
                                current_file += 1;
                                if let Some(idx) = sidebar_items.iter().position(|item| {
                                    matches!(item, SidebarItem::File { file_index, .. } if *file_index == current_file)
                                }) {
                                    sidebar_selected = idx;
                                    // Auto-scroll sidebar
                                    let visible_height =
                                        terminal.size()?.height.saturating_sub(5) as usize;
                                    if sidebar_selected >= sidebar_scroll + visible_height {
                                        sidebar_scroll =
                                            sidebar_selected.saturating_sub(visible_height) + 1;
                                    } else if sidebar_selected < sidebar_scroll {
                                        sidebar_scroll = sidebar_selected;
                                    }
                                }
                                let diff = &file_diffs[current_file];
                                let side_by_side =
                                    compute_side_by_side(&diff.old_content, &diff.new_content);
                                let hunks = find_hunk_starts(&side_by_side);
                                scroll = hunks
                                    .first()
                                    .map(|&h| (h as u16).saturating_sub(5))
                                    .unwrap_or(0);
                                h_scroll = 0;
                            }
                        }
                        KeyCode::Char('k')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            // Ctrl+k: previous file
                            if current_file > 0 {
                                current_file -= 1;
                                if let Some(idx) = sidebar_items.iter().position(|item| {
                                    matches!(item, SidebarItem::File { file_index, .. } if *file_index == current_file)
                                }) {
                                    sidebar_selected = idx;
                                    // Auto-scroll sidebar
                                    if sidebar_selected < sidebar_scroll {
                                        sidebar_scroll = sidebar_selected;
                                    }
                                }
                                let diff = &file_diffs[current_file];
                                let side_by_side =
                                    compute_side_by_side(&diff.old_content, &diff.new_content);
                                let hunks = find_hunk_starts(&side_by_side);
                                scroll = hunks
                                    .first()
                                    .map(|&h| (h as u16).saturating_sub(5))
                                    .unwrap_or(0);
                                h_scroll = 0;
                            }
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
                                    let diff = &file_diffs[current_file];
                                    let side_by_side =
                                        compute_side_by_side(&diff.old_content, &diff.new_content);
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
                                // Toggle viewed status; if marking as viewed, move to next
                                if viewed_files.contains(&current_file) {
                                    viewed_files.remove(&current_file);
                                } else {
                                    viewed_files.insert(current_file);
                                    if current_file < file_diffs.len() - 1 {
                                        current_file += 1;
                                        if let Some(idx) = sidebar_items.iter().position(|item| {
                                            matches!(item, SidebarItem::File { file_index, .. } if *file_index == current_file)
                                        }) {
                                            sidebar_selected = idx;
                                            // Auto-scroll sidebar
                                            let visible_height =
                                                terminal.size()?.height.saturating_sub(5) as usize;
                                            if sidebar_selected >= sidebar_scroll + visible_height {
                                                sidebar_scroll =
                                                    sidebar_selected.saturating_sub(visible_height)
                                                        + 1;
                                            }
                                        }
                                        let diff = &file_diffs[current_file];
                                        let side_by_side =
                                            compute_side_by_side(&diff.old_content, &diff.new_content);
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
                                    compute_side_by_side(&diff.old_content, &diff.new_content);
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
                                    compute_side_by_side(&diff.old_content, &diff.new_content);
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
