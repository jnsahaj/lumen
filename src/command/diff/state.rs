use std::collections::HashSet;

use crate::command::diff::diff_algo::{compute_side_by_side, find_hunk_starts};
use crate::command::diff::search::SearchState;
use crate::command::diff::types::{
    build_file_tree, DiffFullscreen, DiffViewSettings, FileDiff, FocusedPanel, SidebarItem,
};

#[derive(Default, Clone, Copy, PartialEq)]
pub enum PendingKey {
    #[default]
    None,
    G,
}

pub struct AppState {
    pub file_diffs: Vec<FileDiff>,
    pub sidebar_items: Vec<SidebarItem>,
    pub current_file: usize,
    pub sidebar_selected: usize,
    pub sidebar_scroll: usize,
    pub sidebar_h_scroll: u16,
    pub scroll: u16,
    pub h_scroll: u16,
    pub focused_panel: FocusedPanel,
    pub viewed_files: HashSet<usize>,
    pub show_sidebar: bool,
    pub settings: DiffViewSettings,
    pub diff_fullscreen: DiffFullscreen,
    pub search_state: SearchState,
    pub pending_key: PendingKey,
    pub needs_reload: bool,
}

impl AppState {
    pub fn new(file_diffs: Vec<FileDiff>) -> Self {
        let sidebar_items = build_file_tree(&file_diffs);
        let sidebar_selected = sidebar_items
            .iter()
            .position(|item| matches!(item, SidebarItem::File { .. }))
            .unwrap_or(0);
        let current_file = sidebar_items
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
        let scroll = if !file_diffs.is_empty() && current_file < file_diffs.len() {
            calc_initial_scroll(&file_diffs[current_file], settings.tab_width)
        } else {
            0
        };

        Self {
            file_diffs,
            sidebar_items,
            current_file,
            sidebar_selected,
            sidebar_scroll: 0,
            sidebar_h_scroll: 0,
            scroll,
            h_scroll: 0,
            focused_panel: FocusedPanel::default(),
            viewed_files: HashSet::new(),
            show_sidebar: true,
            settings,
            diff_fullscreen: DiffFullscreen::default(),
            search_state: SearchState::default(),
            pending_key: PendingKey::default(),
            needs_reload: false,
        }
    }

    /// Reload file diffs, optionally unmarking changed files from viewed set.
    /// Preserves scroll position and current file when possible.
    pub fn reload(&mut self, file_diffs: Vec<FileDiff>, changed_files: Option<&HashSet<String>>) {
        // Store current state to preserve
        let old_filename = self
            .file_diffs
            .get(self.current_file)
            .map(|f| f.filename.clone());
        let old_scroll = self.scroll;
        let old_h_scroll = self.h_scroll;

        // Convert viewed_files indices to filenames (to handle index changes after reload)
        let mut viewed_filenames: HashSet<String> = self
            .viewed_files
            .iter()
            .filter_map(|&idx| self.file_diffs.get(idx).map(|f| f.filename.clone()))
            .collect();

        // Remove changed files from viewed set
        if let Some(changed) = changed_files {
            for filename in changed {
                viewed_filenames.remove(filename);
            }
        }

        self.file_diffs = file_diffs;
        self.sidebar_items = build_file_tree(&self.file_diffs);

        // Convert viewed filenames back to indices in the new file_diffs
        self.viewed_files = self
            .file_diffs
            .iter()
            .enumerate()
            .filter(|(_, f)| viewed_filenames.contains(&f.filename))
            .map(|(i, _)| i)
            .collect();

        // Preserve current file selection
        if let Some(name) = old_filename {
            self.current_file = self
                .file_diffs
                .iter()
                .position(|f| f.filename == name)
                .unwrap_or(0);
        }
        if self.current_file >= self.file_diffs.len() && !self.file_diffs.is_empty() {
            self.current_file = self.file_diffs.len() - 1;
        }

        // Update sidebar selection to match current file
        if let Some(idx) = self.sidebar_items.iter().position(|item| {
            matches!(item, SidebarItem::File { file_index, .. } if *file_index == self.current_file)
        }) {
            self.sidebar_selected = idx;
        } else {
            self.sidebar_selected = self
                .sidebar_items
                .iter()
                .position(|item| matches!(item, SidebarItem::File { .. }))
                .unwrap_or(0);
        }

        // Preserve scroll position instead of resetting
        if !self.file_diffs.is_empty() {
            // Keep the old scroll position, but clamp to valid range
            let diff = &self.file_diffs[self.current_file];
            let side_by_side = compute_side_by_side(
                &diff.old_content,
                &diff.new_content,
                self.settings.tab_width,
            );
            let max_scroll = side_by_side.len().saturating_sub(10);
            self.scroll = old_scroll.min(max_scroll as u16);
            self.h_scroll = old_h_scroll;
        }

        self.needs_reload = false;
    }

    pub fn select_file(&mut self, file_index: usize) {
        self.current_file = file_index;
        self.diff_fullscreen = DiffFullscreen::None;
        self.scroll =
            calc_initial_scroll(&self.file_diffs[self.current_file], self.settings.tab_width);
        self.h_scroll = 0;
    }
}

pub fn calc_initial_scroll(diff: &FileDiff, tab_width: usize) -> u16 {
    let side_by_side = compute_side_by_side(&diff.old_content, &diff.new_content, tab_width);
    let hunks = find_hunk_starts(&side_by_side);
    hunks
        .first()
        .map(|&h| (h as u16).saturating_sub(5))
        .unwrap_or(0)
}

pub fn adjust_scroll_to_line(
    line: usize,
    scroll: u16,
    visible_height: usize,
    max_scroll: usize,
) -> u16 {
    let margin = 10usize;
    let scroll_usize = scroll as usize;
    let content_height = visible_height.saturating_sub(2);

    let new_scroll = if line < scroll_usize + margin {
        line.saturating_sub(margin) as u16
    } else if line >= scroll_usize + content_height.saturating_sub(margin) {
        (line.saturating_sub(content_height.saturating_sub(margin).saturating_sub(1))) as u16
    } else {
        scroll
    };
    new_scroll.min(max_scroll as u16)
}
