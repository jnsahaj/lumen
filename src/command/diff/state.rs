use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use crate::command::diff::diff_algo::{compute_side_by_side, find_hunk_starts};

use crate::command::diff::search::SearchState;
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

/// The target of an annotation: either a whole file or a specific line range on one panel.
#[derive(Clone)]
pub enum AnnotationTarget {
    /// Annotation applies to the whole file
    File,
    /// Annotation applies to a range of lines on a specific panel
    LineRange {
        panel: DiffPanelFocus,
        start_line: usize,
        end_line: usize,
    },
}

/// An annotation attached to a file or line range.
///
/// Annotations allow users to add notes to code changes during review.
/// Each annotation is uniquely identified by its `id`.
#[derive(Clone)]
pub struct Annotation {
    pub id: u64,
    pub filename: String,
    pub target: AnnotationTarget,
    pub content: String,
    pub created_at: SystemTime,
}

impl Annotation {
    /// Format the creation time as HH:MM in local time
    #[cfg(feature = "jj")]
    pub fn format_time(&self) -> String {
        use chrono::{DateTime, Local};
        let datetime: DateTime<Local> = self.created_at.into();
        datetime.format("%H:%M").to_string()
    }

    /// Format the creation time as HH:MM (UTC fallback when chrono unavailable)
    #[cfg(not(feature = "jj"))]
    pub fn format_time(&self) -> String {
        use std::time::UNIX_EPOCH;
        let duration = self.created_at.duration_since(UNIX_EPOCH).unwrap_or_default();
        let secs = duration.as_secs();
        let hours = (secs / 3600) % 24;
        let minutes = (secs / 60) % 60;
        format!("{:02}:{:02}", hours, minutes)
    }

    /// Get a display label for the target type
    pub fn target_label(&self) -> &'static str {
        match &self.target {
            AnnotationTarget::File => "file",
            AnnotationTarget::LineRange { panel, .. } => match panel {
                DiffPanelFocus::Old => "old",
                DiffPanelFocus::New => "new",
                DiffPanelFocus::None => "new",
            },
        }
    }

    /// Get the line range display string
    pub fn line_range_display(&self) -> String {
        match &self.target {
            AnnotationTarget::File => String::new(),
            AnnotationTarget::LineRange { start_line, end_line, .. } => {
                if start_line == end_line {
                    format!("L{}", start_line)
                } else {
                    format!("L{}-{}", start_line, end_line)
                }
            }
        }
    }
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
    // Annotation fields
    pub annotations: Vec<Annotation>,
    annotation_next_id: u64,
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
    /// Number of non-content rows at the top of the rendered diff (context lines + file annotations).
    /// Set by render_diff each frame, used by mouse handlers for coordinate mapping.
    pub content_row_offset: usize,
    /// Annotation overlay gaps within the content area. Each entry is
    /// `(content_line_after, gap_height)` where `content_line_after` is the 0-based
    /// visible content line index after which an overlay gap of `gap_height` rows appears.
    /// Used by mouse handlers to correctly map screen rows to side_by_side indices.
    pub annotation_overlay_gaps: Vec<(usize, usize)>,
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
            annotations: Vec::new(),
            annotation_next_id: 0,
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
            content_row_offset: 0,
            annotation_overlay_gaps: Vec::new(),
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
            self.cached_side_by_side = Some((current, side_by_side));
            self.cached_hunks = Some((current, hunks));
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
            self.cached_side_by_side = Some((current, sbs));
            self.cached_hunks = Some((current, hnks));
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
        let sbs = match &self.cached_side_by_side {
            Some((_, data)) => data.as_slice(),
            None => &[],
        };
        self.search_state.update_matches(sbs, self.diff_fullscreen);
    }

    /// Invalidate the cache (call when file changes)
    pub fn invalidate_cache(&mut self) {
        self.cached_side_by_side = None;
        self.cached_hunks = None;
        self.content_row_offset = 0;
        self.annotation_overlay_gaps.clear();
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
            }
        }

        self.file_diffs = file_diffs;
        self.sidebar_items = build_file_tree(&self.file_diffs);

        // Retain annotations whose file still exists
        let filenames: HashSet<&str> = self.file_diffs.iter().map(|f| f.filename.as_str()).collect();
        self.annotations.retain(|ann| filenames.contains(ann.filename.as_str()));

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
        self.invalidate_cache(); // Clear cache after reload
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

    /// Get annotation by id
    pub fn get_annotation_by_id(&self, id: u64) -> Option<&Annotation> {
        self.annotations.iter().find(|a| a.id == id)
    }

    /// Get all annotations for a file
    #[allow(dead_code)]
    pub fn get_annotations_for_file(&self, filename: &str) -> Vec<&Annotation> {
        self.annotations
            .iter()
            .filter(|a| a.filename == filename)
            .collect()
    }

    /// Add a new annotation, returns its id
    pub fn add_annotation(&mut self, filename: String, target: AnnotationTarget, content: String, created_at: SystemTime) -> u64 {
        let id = self.annotation_next_id;
        self.annotation_next_id += 1;
        self.annotations.push(Annotation {
            id,
            filename,
            target,
            content,
            created_at,
        });
        id
    }

    /// Update an existing annotation's content
    pub fn update_annotation(&mut self, id: u64, content: String) {
        if let Some(ann) = self.annotations.iter_mut().find(|a| a.id == id) {
            ann.content = content;
        }
    }

    /// Remove an annotation by id
    pub fn remove_annotation(&mut self, id: u64) {
        self.annotations.retain(|a| a.id != id);
    }

    /// Format all annotations for export (GitHub PR review comment style).
    ///
    /// Uses `path`, `line`/`start_line`, and `side` references instead of
    /// embedding full source code — matching the shape of the GitHub Pull
    /// Request review comment API.
    pub fn format_annotations_for_export(&self) -> String {
        let mut result = String::new();

        if let Some(ref reference) = self.diff_reference {
            result.push_str(&format!("# {}\n\n", reference));
        }

        for (i, ann) in self.annotations.iter().enumerate() {
            if i > 0 {
                result.push_str("---\n\n");
            }

            match &ann.target {
                AnnotationTarget::File => {
                    result.push_str(&format!("**{}**\n\n", ann.filename));
                }
                AnnotationTarget::LineRange { panel, start_line, end_line, .. } => {
                    let side = match panel {
                        DiffPanelFocus::Old => "LEFT",
                        _ => "RIGHT",
                    };
                    if start_line == end_line {
                        result.push_str(&format!(
                            "**{}** line {} ({})\n\n",
                            ann.filename, start_line, side,
                        ));
                    } else {
                        result.push_str(&format!(
                            "**{}** lines {}-{} ({})\n\n",
                            ann.filename, start_line, end_line, side,
                        ));
                    }
                }
            }

            // Comment body
            result.push_str(&ann.content);
            result.push_str("\n\n");
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

    #[test]
    fn test_focus_empty_diffs() {
        let diffs = vec![];

        let state = AppState::new(diffs, Some("any.rs"));

        assert_eq!(state.current_file, 0);
        assert!(state.file_diffs.is_empty());
    }
}
