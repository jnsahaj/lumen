use std::collections::{HashMap, HashSet};

use tree_sitter::{Parser, Tree};

use crate::command::diff::context::get_language_context;
use crate::command::diff::diff_algo::{compute_side_by_side, count_added_removed, find_hunk_starts};
use crate::command::diff::highlight::FileHighlighter;

use crate::command::diff::search::SearchState;
use crate::command::diff::review_thread::{
    anchor_text_for, refresh_anchors, AnnotationTarget, Comment, Thread,
};
use crate::command::diff::thread_store::{StoreStamp, ThreadStore};
use crate::command::diff::types::{
    build_file_tree, CursorPosition, DiffFullscreen, DiffLine, DiffPanelFocus,
    DiffViewSettings, FileDiff, FocusedPanel, Selection, SelectionMode, SidebarItem,
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

#[derive(Debug, Default)]
pub struct TreeCache {
    pub old_file_trees: HashMap<String, Tree>,
    pub new_file_trees: HashMap<String, Tree>,
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
    /// Hunks marked as viewed, keyed by filename (stable across reloads).
    /// Values are hunk indices within that file's `find_hunk_starts` result.
    pub viewed_hunks: HashMap<String, HashSet<usize>>,
    pub show_sidebar: bool,
    pub settings: DiffViewSettings,
    pub diff_fullscreen: DiffFullscreen,
    pub search_state: SearchState,
    pub pending_key: PendingKey,
    pub needs_reload: bool,
    pub watching: bool,
    pub focused_hunk: Option<usize>,
    /// Review threads of this repository, as last read from or written to the store.
    pub threads: Vec<Thread>,
    /// Where threads are kept between sessions. Without it they live in memory only.
    thread_store: Option<ThreadStore>,
    thread_store_stamp: Option<StoreStamp>,
    /// Why threads stopped being saved, until the UI has shown it.
    pub thread_store_error: Option<String>,
    // Stacked mode fields
    pub stacked_mode: bool,
    pub stacked_commits: Vec<StackedCommitInfo>,
    pub current_commit_index: usize,
    /// Tracks viewed files per commit SHA (commit SHA -> set of viewed filenames)
    stacked_viewed_files: HashMap<String, HashSet<String>>,
    /// VCS backend name ("git" or "jj")
    pub vcs_name: &'static str,
    /// The commit reference used to open the diff (e.g., "HEAD~2..HEAD", "main..feature")
    pub diff_reference: Option<String>,
    // Selection state
    /// Which panel has selection focus
    pub diff_panel_focus: DiffPanelFocus,
    /// Current text selection
    pub selection: Selection,
    /// Whether a mouse drag is in progress
    pub is_dragging: bool,
    /// Whether to show the selection action tooltip
    pub show_selection_tooltip: bool,
    // Cached diff computation
    /// Cached side_by_side diff for current file (invalidated on file change)
    cached_side_by_side: Option<(usize, Vec<DiffLine>)>,
    /// Cached hunk starts for current file
    cached_hunks: Option<(usize, Vec<usize>)>,
    /// Cached total line count for current file (avoids recomputing diff for max_scroll)
    cached_total_lines: Option<(usize, usize)>,
    /// Cached syntax highlighters for current file (avoids re-parsing with tree-sitter every frame)
    cached_highlighters: Option<(usize, FileHighlighter, FileHighlighter)>,
    /// Cached parsed syntax tree
    pub cached_trees: TreeCache,
    /// Whether search matches need to be recomputed
    search_dirty: bool,
    /// Number of non-content rows at the top of the rendered diff (context lines + file annotations).
    /// Set by render_diff each frame, used by mouse handlers for coordinate mapping.
    pub content_row_offset: usize,
    /// Annotation overlay gaps within the content area. Each entry is
    /// `(content_line_after, gap_height)` where `content_line_after` is the 0-based
    /// visible content line index after which an overlay gap of `gap_height` rows appears.
    /// Used by mouse handlers to correctly map screen rows to side_by_side indices.
    pub annotation_overlay_gaps: Vec<(usize, usize)>,
    /// Screen rectangles of currently-rendered annotation overlays.
    /// Set by render_diff each frame; used by mouse handlers to open the inline
    /// editor when the user clicks an existing annotation.
    pub annotation_rects: Vec<(String, ratatui::layout::Rect)>,
    /// Screen rectangle of the active inline annotation editor, when present.
    /// Set by render_diff each frame; used by mouse handlers to detect clicks
    /// outside the editor (save if non-empty, cancel if empty).
    pub editor_rect: Option<ratatui::layout::Rect>,
    /// Total added lines across all files in the current diff. Recomputed on reload.
    pub total_added: usize,
    /// Total removed lines across all files in the current diff. Recomputed on reload.
    pub total_removed: usize,
}

fn compute_total_line_stats(file_diffs: &[FileDiff]) -> (usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;
    for diff in file_diffs {
        if diff.is_binary {
            continue;
        }
        let (a, r) = count_added_removed(&diff.old_content, &diff.new_content);
        added += a;
        removed += r;
    }
    (added, removed)
}

impl AppState {
    pub fn new(file_diffs: Vec<FileDiff>, focus_file: Option<&str>) -> Self {
        let sidebar_items = build_file_tree(&file_diffs);
        let collapsed_dirs = HashSet::new();
        let sidebar_visible = build_sidebar_visible_indices(&sidebar_items, &collapsed_dirs);
        let (sidebar_selected, current_file) = if let Some(focus_path) = focus_file {
            if let Some(file_idx) = file_diffs.iter().position(|f| f.filename == focus_path) {
                let sidebar_idx = sidebar_visible
                    .iter()
                    .position(|&idx| {
                        matches!(sidebar_items[idx], SidebarItem::File { file_index, .. } if file_index == file_idx)
                    })
                    .unwrap_or(0);
                (sidebar_idx, file_idx)
            } else {
                eprintln!(
                    "\x1b[93mwarning:\x1b[0m --focus file '{}' not found in diff, using first file",
                    focus_path
                );
                Self::find_first_file(&sidebar_items, &sidebar_visible)
            }
        } else {
            Self::find_first_file(&sidebar_items, &sidebar_visible)
        };
        let settings = DiffViewSettings::default();
        let (total_added, total_removed) = compute_total_line_stats(&file_diffs);
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
            viewed_hunks: HashMap::new(),
            show_sidebar: true,
            settings,
            diff_fullscreen: DiffFullscreen::default(),
            search_state: SearchState::default(),
            pending_key: PendingKey::default(),
            needs_reload: false,
            watching: false,
            focused_hunk,
            threads: Vec::new(),
            thread_store: None,
            thread_store_stamp: None,
            thread_store_error: None,
            stacked_mode: false,
            stacked_commits: Vec::new(),
            current_commit_index: 0,
            stacked_viewed_files: HashMap::new(),
            vcs_name: "git", // Default, will be set by caller
            diff_reference: None,
            diff_panel_focus: DiffPanelFocus::default(),
            selection: Selection::default(),
            is_dragging: false,
            show_selection_tooltip: false,
            cached_side_by_side: None,
            cached_hunks: None,
            cached_total_lines: None,
            cached_highlighters: None,
            cached_trees: TreeCache::default(),
            search_dirty: true,
            content_row_offset: 0,
            annotation_overlay_gaps: Vec::new(),
            annotation_rects: Vec::new(),
            editor_rect: None,
            total_added,
            total_removed,
        }
    }

    fn find_first_file(sidebar_items: &[SidebarItem], sidebar_visible: &[usize]) -> (usize, usize) {
        for (visible_idx, &item_idx) in sidebar_visible.iter().enumerate() {
            if let SidebarItem::File { file_index, .. } = &sidebar_items[item_idx] {
                return (visible_idx, *file_index);
            }
        }
        (0, 0)
    }

    /// Get cached side_by_side diff for current file, computing if necessary
    pub fn get_side_by_side(&mut self) -> &[DiffLine] {
        if self.file_diffs.is_empty() {
            return &[];
        }

        let current = self.current_file;
        let needs_recompute = match &self.cached_side_by_side {
            Some((cached_file, _)) => *cached_file != current,
            None => true,
        };

        if needs_recompute {
            let diff = &self.file_diffs[current];
            let side_by_side = compute_side_by_side(
                &diff.old_content,
                &diff.new_content,
                self.settings.tab_width,
            );
            let hunks = find_hunk_starts(&side_by_side);
            let total = side_by_side.len();
            self.cached_side_by_side = Some((current, side_by_side));
            self.cached_hunks = Some((current, hunks));
            self.cached_total_lines = Some((current, total));
            let filename = self.file_diffs[current].filename.clone();
            let old_source = &self.file_diffs[current].old_content;
            let new_source = &self.file_diffs[current].new_content;
            let Some(lang_ctx) = get_language_context(&filename) else {
                return &self.cached_side_by_side.as_ref().unwrap().1;
            };
            let mut parser = Parser::new();
            if parser.set_language(&lang_ctx.language).is_ok() {
                let old_tree = self.cached_trees.old_file_trees.get(&filename);
                if let Some(tree) = parser.parse(old_source, old_tree) {
                    self.cached_trees
                        .old_file_trees
                        .insert(filename.clone(), tree);
                }
                let new_tree = self.cached_trees.new_file_trees.get(&filename);
                if let Some(tree) = parser.parse(new_source, new_tree) {
                    self.cached_trees.new_file_trees.insert(filename, tree);
                }
            }
        }

        &self.cached_side_by_side.as_ref().unwrap().1
    }

    /// Get cached hunk starts for current file
    pub fn get_hunks(&mut self) -> &[usize] {
        // Ensure side_by_side is computed (which also computes hunks)
        let _ = self.get_side_by_side();
        &self.cached_hunks.as_ref().unwrap().1
    }

    /// Ensure the side_by_side and hunk caches are populated for the current file.
    /// Call this before using `side_by_side_ref()` or `hunks_ref()`.
    pub fn ensure_cache(&mut self) {
        if self.file_diffs.is_empty() {
            return;
        }
        let current = self.current_file;
        let needs_recompute = match &self.cached_side_by_side {
            Some((cached_file, _)) => *cached_file != current,
            None => true,
        };
        if needs_recompute {
            let diff = &self.file_diffs[current];
            let sbs = compute_side_by_side(
                &diff.old_content,
                &diff.new_content,
                self.settings.tab_width,
            );
            let hnks = find_hunk_starts(&sbs);
            let total = sbs.len();
            self.cached_side_by_side = Some((current, sbs));
            self.cached_hunks = Some((current, hnks));
            self.cached_total_lines = Some((current, total));
            let filename = self.file_diffs[current].filename.clone();
            let old_source = &self.file_diffs[current].old_content;
            let new_source = &self.file_diffs[current].new_content;
            let Some(lang_ctx) = get_language_context(&filename) else {
                return;
            };
            let mut parser = Parser::new();
            if parser.set_language(&lang_ctx.language).is_ok() {
                let old_tree = self.cached_trees.old_file_trees.get(&filename);
                if let Some(tree) = parser.parse(old_source, old_tree) {
                    self.cached_trees
                        .old_file_trees
                        .insert(filename.clone(), tree);
                }
                let new_tree = self.cached_trees.new_file_trees.get(&filename);
                if let Some(tree) = parser.parse(new_source, new_tree) {
                    self.cached_trees.new_file_trees.insert(filename, tree);
                }
            }
        }
    }

    /// Get an immutable reference to the cached side_by_side data.
    /// Must call `ensure_cache()` first.
    pub fn side_by_side_ref(&self) -> &[DiffLine] {
        match &self.cached_side_by_side {
            Some((_, ref data)) => data,
            None => &[],
        }
    }

    /// Get an immutable reference to the cached hunk starts.
    /// Must call `ensure_cache()` first.
    pub fn hunks_ref(&self) -> &[usize] {
        match &self.cached_hunks {
            Some((_, ref data)) => data,
            None => &[],
        }
    }

    /// Ensure cache is populated and update search matches.
    /// Combines the mutable cache population with search update to avoid borrow conflicts.
    pub fn update_search_matches(&mut self) {
        self.ensure_cache();
        if self.search_dirty {
            let sbs = match &self.cached_side_by_side {
                Some((_, data)) => data.as_slice(),
                None => &[],
            };
            self.search_state.update_matches(sbs, self.diff_fullscreen);
            self.search_dirty = false;
        }
    }

    /// Mark search matches as needing recomputation
    pub fn mark_search_dirty(&mut self) {
        self.search_dirty = true;
    }

    /// Get the cached total line count for the current file.
    /// Ensures cache is populated first.
    pub fn total_lines(&mut self) -> usize {
        self.ensure_cache();
        self.cached_total_lines
            .as_ref()
            .map(|(_, n)| *n)
            .unwrap_or(0)
    }

    /// Get cached FileHighlighters for the current file, creating them if needed.
    /// Returns (old_highlighter, new_highlighter).
    pub fn get_highlighters(&mut self) -> (&FileHighlighter, &FileHighlighter) {
        let current = self.current_file;
        let needs_recompute = match &self.cached_highlighters {
            Some((cached_file, _, _)) => *cached_file != current,
            None => true,
        };
        if needs_recompute {
            let diff = &self.file_diffs[current];
            let old_hl = FileHighlighter::new(&diff.old_content, &diff.filename);
            let new_hl = FileHighlighter::new(&diff.new_content, &diff.filename);
            self.cached_highlighters = Some((current, old_hl, new_hl));
        }
        let (_, old_hl, new_hl) = self.cached_highlighters.as_ref().unwrap();
        (old_hl, new_hl)
    }

    /// Get cached highlighters without triggering recompute.
    /// Must call get_highlighters() at least once first for current file.
    pub fn highlighters_ref(&self) -> Option<(&FileHighlighter, &FileHighlighter)> {
        self.cached_highlighters
            .as_ref()
            .map(|(_, old_hl, new_hl)| (old_hl, new_hl))
    }

    /// Invalidate the cache (call when file changes)
    pub fn invalidate_cache(&mut self) {
        self.cached_side_by_side = None;
        self.cached_hunks = None;
        self.cached_total_lines = None;
        self.cached_highlighters = None;
        self.search_dirty = true;
        self.content_row_offset = 0;
        self.annotation_overlay_gaps.clear();
        self.annotation_rects.clear();
        self.editor_rect = None;
    }

    /// Adjust a screen-relative content row for annotation overlay gaps.
    /// Returns `Some(adjusted_content_y)` for content rows, or `None` if the
    /// click landed inside an annotation overlay.
    pub fn adjust_for_overlay_gaps(&self, content_y: usize) -> Option<usize> {
        let mut cumulative = 0;
        for &(after_line, gap_height) in &self.annotation_overlay_gaps {
            let gap_screen_start = after_line + 1 + cumulative;
            let gap_screen_end = gap_screen_start + gap_height;
            if content_y < gap_screen_start {
                break; // Before this gap
            }
            if content_y < gap_screen_end {
                return None; // Inside an overlay gap
            }
            cumulative += gap_height;
        }
        Some(content_y - cumulative)
    }

    /// Like `adjust_for_overlay_gaps`, but if the position is inside a gap,
    /// returns the content line just before the gap instead of None.
    /// Used for drag operations where we always need a valid line.
    pub fn adjust_for_overlay_gaps_clamped(&self, content_y: usize) -> usize {
        let mut cumulative = 0;
        for &(after_line, gap_height) in &self.annotation_overlay_gaps {
            let gap_screen_start = after_line + 1 + cumulative;
            let gap_screen_end = gap_screen_start + gap_height;
            if content_y < gap_screen_start {
                break;
            }
            if content_y < gap_screen_end {
                return after_line; // Map to the content line just before the gap
            }
            cumulative += gap_height;
        }
        content_y - cumulative
    }

    /// Clear all selection state
    pub fn clear_selection(&mut self) {
        self.diff_panel_focus = DiffPanelFocus::None;
        self.selection = Selection::default();
        self.is_dragging = false;
        self.show_selection_tooltip = false;
    }

    /// Start a new selection
    pub fn start_selection(&mut self, panel: DiffPanelFocus, pos: CursorPosition, mode: SelectionMode) {
        self.diff_panel_focus = panel;
        self.selection = Selection {
            panel,
            anchor: pos,
            head: pos,
            mode,
        };
        self.is_dragging = true;
    }

    /// Extend the current selection to a new position
    pub fn extend_selection(&mut self, pos: CursorPosition) {
        if self.is_dragging {
            self.selection.head = pos;
        }
    }

    /// End the drag operation but keep the selection
    pub fn end_drag(&mut self) {
        self.is_dragging = false;
        // Show tooltip only if there's an actual selection (not just a click)
        if self.selection.is_active() && self.selection.anchor != self.selection.head {
            self.show_selection_tooltip = true;
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
        let selected_item = self.sidebar_item_at_visible(self.sidebar_selected).cloned();
        let collapsing = !self.collapsed_dirs.contains(dir_path);

        if collapsing {
            self.collapsed_dirs.insert(dir_path.to_string());
        } else {
            self.collapsed_dirs.remove(dir_path);
        }

        self.rebuild_sidebar_visible();

        if collapsing {
            if let Some(item) = &selected_item {
                let path = sidebar_item_path(item);
                if is_child_path(path, dir_path) {
                    if let Some(idx) = self.sidebar_visible_index_for_dir(dir_path) {
                        self.sidebar_selected = idx;
                        return;
                    }
                }
            }
        }

        if let Some(item) = selected_item {
            match item {
                SidebarItem::Directory { path, .. } => {
                    if let Some(idx) = self.sidebar_visible_index_for_dir(&path) {
                        self.sidebar_selected = idx;
                    }
                }
                SidebarItem::File { file_index, .. } => {
                    if let Some(idx) = self.sidebar_visible_index_for_file(file_index) {
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

    /// Set the diff reference string (e.g., "HEAD~2..HEAD")
    pub fn set_diff_reference(&mut self, reference: Option<String>) {
        self.diff_reference = reference;
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
                self.viewed_hunks.remove(filename);
            }
        }

        self.file_diffs = file_diffs;
        let (total_added, total_removed) = compute_total_line_stats(&self.file_diffs);
        self.total_added = total_added;
        self.total_removed = total_removed;
        self.sidebar_items = build_file_tree(&self.file_diffs);

        let filenames: HashSet<&str> = self.file_diffs.iter().map(|f| f.filename.as_str()).collect();
        self.viewed_hunks.retain(|fname, _| filenames.contains(fname.as_str()));

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

        self.needs_reload = false;
        self.invalidate_cache(); // Clear cache after reload

        // Preserve scroll position instead of resetting
        if !self.file_diffs.is_empty() {
            // Keep the old scroll position, but clamp to valid range
            let total = self.total_lines();
            let max_scroll = total.saturating_sub(10);
            self.scroll = old_scroll.min(max_scroll as u16);
            self.h_scroll = old_h_scroll;
        }

        self.refresh_thread_anchors();
    }

    pub fn select_file(&mut self, file_index: usize) {
        self.current_file = file_index;
        self.diff_fullscreen = DiffFullscreen::None;
        self.clear_selection(); // Clear selection when changing files
        self.invalidate_cache(); // Clear cache for new file

        // Use cached computation
        let hunks = self.get_hunks().to_vec();
        self.scroll = hunks
            .first()
            .map(|&h| (h as u16).saturating_sub(5))
            .unwrap_or(0);
        self.h_scroll = 0;
        self.focused_hunk = if hunks.is_empty() { None } else { Some(0) };
    }

    /// Keeps this repository's threads in `store` from now on, starting from what it holds.
    pub fn open_thread_store(&mut self, store: ThreadStore) {
        // Taken before reading, so that a write landing in between is noticed later.
        let stamp = store.stamp();
        match store.load() {
            Ok(threads) => {
                self.threads = threads;
                self.thread_store_stamp = stamp;
                self.thread_store = Some(store);
                self.refresh_thread_anchors();
            }
            Err(error) => self.thread_store_error = Some(error),
        }
    }

    /// Picks up threads changed by another process. Returns whether anything was read.
    pub fn reload_threads_if_changed(&mut self) -> bool {
        let Some(store) = &self.thread_store else {
            return false;
        };
        let stamp = store.stamp();
        if stamp == self.thread_store_stamp {
            return false;
        }
        match store.load() {
            Ok(threads) => {
                self.threads = threads;
                self.thread_store_stamp = stamp;
            }
            Err(error) => self.stop_saving_threads(error),
        }
        true
    }

    fn stop_saving_threads(&mut self, error: String) {
        self.thread_store = None;
        self.thread_store_stamp = None;
        self.thread_store_error = Some(error);
    }

    /// Applies `change` to the threads, through the store when there is one so that
    /// replies written by another process in the meantime are kept.
    fn change_threads<T>(&mut self, change: impl Fn(&mut Vec<Thread>) -> T) -> T {
        if let Some(store) = &self.thread_store {
            match store.update(&change) {
                // The stamp is left alone: the next check reads the file again, which
                // also picks up anything another process wrote right after this.
                Ok((threads, outcome)) => {
                    self.threads = threads;
                    return outcome;
                }
                Err(error) => self.stop_saving_threads(error),
            }
        }
        change(&mut self.threads)
    }

    fn refresh_thread_anchors(&mut self) {
        if self.threads.is_empty() {
            return;
        }
        let file_diffs = std::mem::take(&mut self.file_diffs);
        self.change_threads(|threads| refresh_anchors(threads, &file_diffs));
        self.file_diffs = file_diffs;
    }

    pub fn thread_by_id(&self, id: &str) -> Option<&Thread> {
        self.threads.iter().find(|thread| thread.id == id)
    }

    /// The first open thread on the lines `target` covers. When none is open and
    /// `or_resolved` is set, the first resolved one.
    pub fn thread_at(
        &self,
        filename: &str,
        target: &AnnotationTarget,
        or_resolved: bool,
    ) -> Option<&Thread> {
        let first_thread = |resolved: bool| {
            self.threads
                .iter()
                .find(|thread| thread.overlaps(filename, target) && thread.resolved == resolved)
        };
        first_thread(false).or_else(|| first_thread(true).filter(|_| or_resolved))
    }

    /// Starts a thread with a queued comment. Returns the thread id and the comment id.
    pub fn add_thread(
        &mut self,
        filename: String,
        target: AnnotationTarget,
        body: String,
    ) -> (String, String) {
        let anchor_text = anchor_text_for(&self.file_diffs, &filename, &target);
        let comment = Comment::human(body, true);
        let thread = Thread::new(filename, target, anchor_text, comment);
        let ids = (thread.id.clone(), thread.comments[0].id.clone());
        self.change_threads(|threads| threads.push(thread.clone()));
        ids
    }

    /// Adds a queued reply to a thread, reopening it when it was resolved. Returns
    /// the comment id, or `None` when the thread no longer exists.
    pub fn add_reply(&mut self, thread_id: &str, body: String) -> Option<String> {
        let comment = Comment::human(body, true);
        let comment_id = comment.id.clone();
        self.change_threads(|threads| {
            let thread = threads.iter_mut().find(|thread| thread.id == thread_id)?;
            thread.comments.push(comment.clone());
            thread.resolved = false;
            Some(())
        })?;
        Some(comment_id)
    }

    pub fn update_comment(&mut self, thread_id: &str, comment_id: &str, body: String) {
        self.change_threads(|threads| {
            let comment = threads
                .iter_mut()
                .filter(|thread| thread.id == thread_id)
                .flat_map(|thread| thread.comments.iter_mut())
                .find(|comment| comment.id == comment_id);
            if let Some(comment) = comment {
                comment.body = body.clone();
            }
        });
    }

    /// Removes one comment, and its thread when it was the only one.
    pub fn remove_comment(&mut self, thread_id: &str, comment_id: &str) {
        self.change_threads(|threads| {
            if let Some(thread) = threads.iter_mut().find(|thread| thread.id == thread_id) {
                thread.comments.retain(|comment| comment.id != comment_id);
            }
            threads.retain(|thread| !thread.comments.is_empty());
        });
    }

    pub fn remove_thread(&mut self, thread_id: &str) {
        self.change_threads(|threads| threads.retain(|thread| thread.id != thread_id));
    }

    pub fn toggle_thread_resolved(&mut self, thread_id: &str) {
        self.change_threads(|threads| {
            if let Some(thread) = threads.iter_mut().find(|thread| thread.id == thread_id) {
                thread.resolved = !thread.resolved;
            }
        });
    }

    pub fn queued_count(&self) -> usize {
        self.threads.iter().map(Thread::queued_count).sum()
    }

    /// The queued comments as text for the agent, each under its thread and the code
    /// it is about: one comment of one thread, or all of them. Empty when there are none.
    pub fn queued_text_for_agent(&self, only: Option<(&str, &str)>) -> String {
        self.threads
            .iter()
            .filter_map(|thread| {
                let comments: Vec<&Comment> = thread
                    .comments
                    .iter()
                    .filter(|comment| {
                        comment.queued
                            && only.is_none_or(|(thread_id, comment_id)| {
                                thread.id == thread_id && comment.id == comment_id
                            })
                    })
                    .collect();
                (!comments.is_empty()).then(|| thread.describe(comments.into_iter()))
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Marks queued comments as sent: one comment of one thread, or all of them.
    /// Returns how many were sent.
    pub fn mark_sent(&mut self, only: Option<(&str, &str)>) -> usize {
        self.change_threads(|threads| {
            threads
                .iter_mut()
                .filter(|thread| only.is_none_or(|(thread_id, _)| thread.id == thread_id))
                .map(|thread| thread.mark_sent(only.map(|(_, comment_id)| comment_id)))
                .sum()
        })
    }

    /// Format threads for export (GitHub PR review comment style): the file, the
    /// line or line range and the side, then the comments. With `only_queued`,
    /// just the comments that were not sent yet and the threads that have some.
    pub fn format_threads_for_export(&self, only_queued: bool) -> String {
        let mut result = String::new();

        if let Some(ref reference) = self.diff_reference {
            result.push_str(&format!("# {}\n\n", reference));
        }

        let mut is_first = true;
        for thread in &self.threads {
            let comments: Vec<&Comment> = thread
                .comments
                .iter()
                .filter(|comment| comment.queued || !only_queued)
                .collect();
            if comments.is_empty() {
                continue;
            }
            if !is_first {
                result.push_str("---\n\n");
            }
            is_first = false;

            result.push_str(&format!("**{}**", thread.filename));
            if let Some(line_reference) = thread.line_reference() {
                result.push_str(&format!(" {}", line_reference));
            }
            result.push_str("\n\n");

            let names_authors = !only_queued && thread.comments.len() > 1;
            for comment in comments {
                if names_authors {
                    result.push_str(&format!("{}: ", comment.author_name));
                }
                result.push_str(&comment.body);
                result.push_str("\n\n");
            }
        }

        result.trim_end().to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::diff::types::FileStatus;

    fn make_file_diff(filename: &str) -> FileDiff {
        FileDiff {
            filename: filename.to_string(),
            old_content: String::new(),
            new_content: "content\n".to_string(),
            status: FileStatus::Added,
            is_binary: false,
        }
    }

    #[test]
    fn test_focus_selects_matching_file() {
        let diffs = vec![
            make_file_diff("src/main.rs"),
            make_file_diff("src/lib.rs"),
            make_file_diff("README.md"),
        ];

        let state = AppState::new(diffs, Some("src/lib.rs"));

        assert_eq!(state.file_diffs[state.current_file].filename, "src/lib.rs");
    }

    #[test]
    fn test_focus_none_selects_first_file_in_sidebar() {
        let diffs = vec![make_file_diff("bbb.rs"), make_file_diff("aaa.rs")];

        let state = AppState::new(diffs, None);

        // Sidebar sorts alphabetically, so aaa.rs (index 1) appears first
        assert_eq!(state.file_diffs[state.current_file].filename, "aaa.rs");
    }

    #[test]
    fn test_focus_not_found_falls_back_to_first_in_sidebar() {
        let diffs = vec![make_file_diff("bbb.rs"), make_file_diff("aaa.rs")];

        let state = AppState::new(diffs, Some("nonexistent.rs"));

        // Falls back to first file in sorted sidebar order
        assert_eq!(state.file_diffs[state.current_file].filename, "aaa.rs");
    }

    #[test]
    fn test_focus_updates_sidebar_selection() {
        let diffs = vec![
            make_file_diff("aaa.rs"),
            make_file_diff("bbb.rs"),
            make_file_diff("ccc.rs"),
        ];

        let state = AppState::new(diffs, Some("ccc.rs"));

        if let Some(SidebarItem::File { file_index, .. }) = state.sidebar_item_at_visible(state.sidebar_selected) {
            assert_eq!(*file_index, state.current_file);
        } else {
            panic!("sidebar_selected should point to a file");
        }
    }

    fn line_target(line: usize) -> AnnotationTarget {
        AnnotationTarget::LineRange {
            panel: DiffPanelFocus::New,
            start_line: line,
            end_line: line,
        }
    }

    #[test]
    fn test_new_comments_are_queued_until_they_are_marked_sent() {
        let mut state = AppState::new(vec![make_file_diff("a.rs")], None);
        let (thread_id, comment_id) =
            state.add_thread("a.rs".to_string(), line_target(1), "first".to_string());
        state.add_thread("a.rs".to_string(), AnnotationTarget::File, "second".to_string());

        assert_eq!(state.queued_count(), 2);
        assert!(!state.threads[0].awaiting_agent());
        assert_eq!(
            state.queued_text_for_agent(Some((&thread_id, &comment_id))),
            format!("## Thread {} · a.rs line 1 (RIGHT)\n\n```\ncontent\n```\n\nyou: first", thread_id)
        );
        assert_eq!(state.queued_text_for_agent(None).matches("## Thread ").count(), 2);

        assert_eq!(state.mark_sent(Some((&thread_id, &comment_id))), 1);
        assert_eq!(state.queued_count(), 1);
        assert!(state.threads[0].awaiting_agent());
        assert_eq!(state.format_threads_for_export(true), "**a.rs**\n\nsecond");

        assert_eq!(state.mark_sent(None), 1);
        assert_eq!(state.mark_sent(None), 0);
    }

    #[test]
    fn test_replying_to_a_resolved_thread_reopens_it() {
        let mut state = AppState::new(vec![make_file_diff("a.rs")], None);
        let (thread_id, _) =
            state.add_thread("a.rs".to_string(), line_target(1), "first".to_string());
        state.toggle_thread_resolved(&thread_id);
        assert!(state.threads[0].resolved);

        state.add_reply(&thread_id, "one more thing".to_string());

        assert!(!state.threads[0].resolved);
    }

    #[test]
    fn test_an_open_thread_is_found_before_an_older_resolved_one_on_the_same_lines() {
        let mut state = AppState::new(vec![make_file_diff("a.rs")], None);
        let (older_id, _) =
            state.add_thread("a.rs".to_string(), line_target(1), "first".to_string());
        state.toggle_thread_resolved(&older_id);
        let (newer_id, _) =
            state.add_thread("a.rs".to_string(), line_target(1), "second".to_string());
        let thread_to_toggle = |state: &AppState| {
            state
                .thread_at("a.rs", &line_target(1), true)
                .map(|thread| thread.id.clone())
        };

        assert_eq!(thread_to_toggle(&state), Some(newer_id.clone()));

        state.toggle_thread_resolved(&newer_id);
        assert_eq!(thread_to_toggle(&state), Some(older_id));
    }

    #[test]
    fn test_threads_persist_and_replies_written_by_another_process_are_picked_up() {
        let repository_root = crate::vcs::test_utils::make_temp_dir("state-thread-store");
        let open_state = || {
            let mut state = AppState::new(vec![make_file_diff("a.rs")], None);
            state.open_thread_store(ThreadStore::for_repository(&repository_root).unwrap());
            state
        };
        let mut state = open_state();
        let (thread_id, _) =
            state.add_thread("a.rs".to_string(), line_target(1), "why?".to_string());
        state.mark_sent(None);
        assert_eq!(state.threads[0].anchor_text.as_deref(), Some("content"));
        let bodies = |state: &AppState| -> Vec<String> {
            state.threads[0].comments.iter().map(|c| c.body.clone()).collect()
        };
        let reply_as_agent = |body: &str| {
            ThreadStore::for_repository(&repository_root)
                .unwrap()
                .update(|threads| {
                    threads[0].add_agent_reply(Comment::agent(body.to_string(), "pi".to_string()))
                })
                .unwrap();
        };

        // A reply shows up in the open review without the reviewer doing anything.
        reply_as_agent("because");
        assert!(state.reload_threads_if_changed());
        assert_eq!(bodies(&state), vec!["why?", "because"]);

        // A reply written just before the reviewer's own change is not overwritten.
        reply_as_agent("and also");
        state.add_reply(&thread_id, "ok".to_string());
        assert_eq!(bodies(&state), vec!["why?", "because", "and also", "ok"]);
        assert_eq!(bodies(&open_state()), vec!["why?", "because", "and also", "ok"]);
        let _ = std::fs::remove_dir_all(&repository_root);
    }

    #[test]
    fn test_reload_keeps_threads_of_files_that_left_the_diff_as_outdated() {
        let mut state = AppState::new(vec![make_file_diff("a.rs")], None);
        state.add_thread("a.rs".to_string(), line_target(1), "first".to_string());

        state.reload(vec![make_file_diff("b.rs")], None);

        assert_eq!(state.threads.len(), 1);
        assert!(state.threads[0].outdated);
    }

    #[test]
    fn test_focus_empty_diffs() {
        let diffs = vec![];

        let state = AppState::new(diffs, Some("any.rs"));

        assert_eq!(state.current_file, 0);
        assert!(state.file_diffs.is_empty());
    }
}
