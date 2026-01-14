use std::collections::{HashMap, HashSet};

use crate::command::diff::diff_algo::{compute_side_by_side, find_hunk_starts};
use crate::command::diff::search::SearchState;
use crate::command::diff::types::{
    build_file_tree, DiffFullscreen, DiffViewSettings, FileDiff, FocusedPanel, SidebarItem,
};
use crate::vcs::StackedCommitInfo;

#[derive(Default, Clone, Copy, PartialEq)]
pub enum PendingKey {
    #[default]
    None,
    G,
}

fn sidebar_item_path(item: &SidebarItem) -> &str {
    match item {
        SidebarItem::Directory { path, .. } => path,
        SidebarItem::File { path, .. } => path,
    }
}

fn is_child_path(path: &str, parent: &str) -> bool {
    if parent.is_empty() {
        return false;
    }
    path.starts_with(&format!("{}/", parent))
}

fn build_sidebar_visible_indices(
    items: &[SidebarItem],
    collapsed_dirs: &HashSet<String>,
) -> Vec<usize> {
    let mut visible = Vec::new();
    let mut collapsed_stack: Vec<String> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        let path = sidebar_item_path(item);
        while let Some(last) = collapsed_stack.last() {
            if is_child_path(path, last) {
                break;
            }
            collapsed_stack.pop();
        }

        if let Some(last) = collapsed_stack.last() {
            if is_child_path(path, last) {
                continue;
            }
        }

        visible.push(idx);

        if let SidebarItem::Directory { path, .. } = item {
            if collapsed_dirs.contains(path) {
                collapsed_stack.push(path.clone());
            }
        }
    }

    visible
}

pub struct AppState {
    pub file_diffs: Vec<FileDiff>,
    pub sidebar_items: Vec<SidebarItem>,
    pub sidebar_visible: Vec<usize>,
    pub collapsed_dirs: HashSet<String>,
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
    pub focused_hunk: Option<usize>,
    // Stacked mode fields
    pub stacked_mode: bool,
    pub stacked_commits: Vec<StackedCommitInfo>,
    pub current_commit_index: usize,
    /// Tracks viewed files per commit SHA (commit SHA -> set of viewed filenames)
    stacked_viewed_files: HashMap<String, HashSet<String>>,
    /// VCS backend name ("git" or "jj")
    pub vcs_name: &'static str,
}

impl AppState {
    pub fn new(file_diffs: Vec<FileDiff>) -> Self {
        let sidebar_items = build_file_tree(&file_diffs);
        let collapsed_dirs = HashSet::new();
        let sidebar_visible = build_sidebar_visible_indices(&sidebar_items, &collapsed_dirs);
        let sidebar_selected = sidebar_visible
            .iter()
            .position(|idx| matches!(sidebar_items[*idx], SidebarItem::File { .. }))
            .unwrap_or(0);
        let current_file = sidebar_visible
            .get(sidebar_selected)
            .and_then(|idx| match &sidebar_items[*idx] {
                SidebarItem::File { file_index, .. } => Some(*file_index),
                _ => None,
            })
            .unwrap_or(0);
        let settings = DiffViewSettings::default();
        let (scroll, focused_hunk) = if !file_diffs.is_empty() && current_file < file_diffs.len() {
            let diff = &file_diffs[current_file];
            let side_by_side =
                compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
            let hunks = find_hunk_starts(&side_by_side);
            let scroll = hunks
                .first()
                .map(|&h| (h as u16).saturating_sub(5))
                .unwrap_or(0);
            let focused = if hunks.is_empty() { None } else { Some(0) };
            (scroll, focused)
        } else {
            (0, None)
        };

        Self {
            file_diffs,
            sidebar_items,
            sidebar_visible,
            collapsed_dirs,
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
            focused_hunk,
            stacked_mode: false,
            stacked_commits: Vec::new(),
            current_commit_index: 0,
            stacked_viewed_files: HashMap::new(),
            vcs_name: "git", // Default, will be set by caller
        }
    }

    /// Set the VCS backend name
    pub fn set_vcs_name(&mut self, name: &'static str) {
        self.vcs_name = name;
    }

    pub fn sidebar_visible_len(&self) -> usize {
        self.sidebar_visible.len()
    }

    pub fn sidebar_item_at_visible(&self, visible_index: usize) -> Option<&SidebarItem> {
        self.sidebar_visible
            .get(visible_index)
            .and_then(|idx| self.sidebar_items.get(*idx))
    }

    pub fn sidebar_visible_index_for_file(&self, file_index: usize) -> Option<usize> {
        self.sidebar_visible.iter().position(|idx| {
            matches!(self.sidebar_items[*idx], SidebarItem::File { file_index: fi, .. } if fi == file_index)
        })
    }

    pub fn sidebar_visible_index_for_dir(&self, dir_path: &str) -> Option<usize> {
        self.sidebar_visible.iter().position(|idx| {
            matches!(&self.sidebar_items[*idx], SidebarItem::Directory { path, .. } if path == dir_path)
        })
    }

    pub fn rebuild_sidebar_visible(&mut self) {
        let existing_dirs: HashSet<String> = self
            .sidebar_items
            .iter()
            .filter_map(|item| match item {
                SidebarItem::Directory { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        self.collapsed_dirs
            .retain(|path| existing_dirs.contains(path));
        self.sidebar_visible =
            build_sidebar_visible_indices(&self.sidebar_items, &self.collapsed_dirs);

        if self.sidebar_visible.is_empty() {
            self.sidebar_selected = 0;
            self.sidebar_scroll = 0;
            return;
        }

        if let Some(idx) = self.sidebar_visible_index_for_file(self.current_file) {
            self.sidebar_selected = idx;
        } else if self.sidebar_selected >= self.sidebar_visible.len() {
            self.sidebar_selected = self.sidebar_visible.len() - 1;
        }

        if self.sidebar_scroll >= self.sidebar_visible.len() {
            self.sidebar_scroll = self.sidebar_visible.len() - 1;
        }
    }

    pub fn toggle_directory(&mut self, dir_path: &str) {
        let selected_path = self
            .sidebar_item_at_visible(self.sidebar_selected)
            .map(sidebar_item_path)
            .map(str::to_string);
        let collapsing = !self.collapsed_dirs.contains(dir_path);

        if collapsing {
            self.collapsed_dirs.insert(dir_path.to_string());
        } else {
            self.collapsed_dirs.remove(dir_path);
        }

        self.rebuild_sidebar_visible();

        if collapsing {
            if let Some(path) = selected_path {
                if is_child_path(&path, dir_path) {
                    if let Some(idx) = self.sidebar_visible_index_for_dir(dir_path) {
                        self.sidebar_selected = idx;
                    }
                }
            }
        }
    }

    pub fn reveal_file(&mut self, file_index: usize) {
        if file_index >= self.file_diffs.len() {
            return;
        }
        let path = self.file_diffs[file_index].filename.clone();
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() > 1 {
            for i in 0..parts.len() - 1 {
                let dir_path = parts[..=i].join("/");
                self.collapsed_dirs.remove(&dir_path);
            }
        }
        self.rebuild_sidebar_visible();
        if let Some(idx) = self.sidebar_visible_index_for_file(file_index) {
            self.sidebar_selected = idx;
        }
    }

    /// Initialize stacked mode with commits
    pub fn init_stacked_mode(&mut self, commits: Vec<StackedCommitInfo>) {
        self.stacked_mode = true;
        self.stacked_commits = commits;
        self.current_commit_index = 0;
    }

    /// Get the current commit info if in stacked mode
    pub fn current_commit(&self) -> Option<&StackedCommitInfo> {
        if self.stacked_mode {
            self.stacked_commits.get(self.current_commit_index)
        } else {
            None
        }
    }

    /// Save current viewed files for the current commit (stacked mode only)
    pub fn save_stacked_viewed_files(&mut self) {
        if !self.stacked_mode {
            return;
        }
        if let Some(commit) = self.stacked_commits.get(self.current_commit_index) {
            let viewed_filenames: HashSet<String> = self
                .viewed_files
                .iter()
                .filter_map(|&idx| self.file_diffs.get(idx).map(|f| f.filename.clone()))
                .collect();
            self.stacked_viewed_files
                .insert(commit.commit_id.clone(), viewed_filenames);
        }
    }

    /// Load viewed files for the current commit (stacked mode only)
    pub fn load_stacked_viewed_files(&mut self) {
        if !self.stacked_mode {
            return;
        }
        if let Some(commit) = self.stacked_commits.get(self.current_commit_index) {
            if let Some(viewed_filenames) = self.stacked_viewed_files.get(&commit.commit_id) {
                self.viewed_files = self
                    .file_diffs
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| viewed_filenames.contains(&f.filename))
                    .map(|(i, _)| i)
                    .collect();
            } else {
                self.viewed_files.clear();
            }
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

        self.rebuild_sidebar_visible();

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
        let diff = &self.file_diffs[self.current_file];
        let side_by_side = compute_side_by_side(
            &diff.old_content,
            &diff.new_content,
            self.settings.tab_width,
        );
        let hunks = find_hunk_starts(&side_by_side);
        self.scroll = hunks
            .first()
            .map(|&h| (h as u16).saturating_sub(5))
            .unwrap_or(0);
        self.h_scroll = 0;
        self.focused_hunk = if hunks.is_empty() { None } else { Some(0) };
    }
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

/// Adjust scroll for hunk focus - only scrolls if the hunk line is outside the viewport.
/// Uses a larger bottom margin to keep hunks visible with context below.
pub fn adjust_scroll_for_hunk(
    hunk_line: usize,
    scroll: u16,
    visible_height: usize,
    max_scroll: usize,
) -> u16 {
    let top_margin = 5usize;
    let bottom_margin = 25usize;
    let scroll_usize = scroll as usize;
    let content_height = visible_height.saturating_sub(2);

    // Check if hunk is above the viewport (with top margin)
    if hunk_line < scroll_usize + top_margin {
        return (hunk_line.saturating_sub(top_margin) as u16).min(max_scroll as u16);
    }

    // Check if hunk is below the viewport (with bottom margin)
    if hunk_line >= scroll_usize + content_height.saturating_sub(bottom_margin) {
        return (hunk_line.saturating_sub(
            content_height
                .saturating_sub(bottom_margin)
                .saturating_sub(1),
        ) as u16)
            .min(max_scroll as u16);
    }

    // Hunk is within viewport, don't scroll
    scroll
}
