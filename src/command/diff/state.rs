use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use crate::command::diff::diff_algo::{compute_side_by_side, count_added_removed, find_hunk_starts};
use crate::command::diff::highlight::FileHighlighter;
use crate::command::diff::outline::{compute_outline, row_plan, DiffRow, OutlineEntry};

use crate::command::diff::search::SearchState;
use crate::command::diff::types::{
    build_file_tree, ChangeType, CursorPosition, DiffFullscreen, DiffLine, DiffPanelFocus,
    DiffViewSettings, FileDiff, FocusedPanel, Selection, SelectionMode, SidebarItem,
};
use crate::vcs::StackedCommitInfo;

#[derive(Default, Clone, Copy, PartialEq)]
pub enum PendingKey {
    #[default]
    None,
    G,
}

/// Which content view the diff panel is showing. Shift+O cycles through them.
#[derive(Default, Clone, Copy, PartialEq)]
pub enum ViewMode {
    /// Normal side-by-side diff (everything).
    #[default]
    Diff,
    /// Folded outline: declaration signatures only; changed bodies become
    /// `⋯ +N`/`-N` fold markers.
    Outline,
    /// Like `Outline`, but the actual changed lines are expanded inline while
    /// unchanged bodies stay folded away.
    OutlineExpanded,
}

impl ViewMode {
    /// True for either folded view (not the full diff).
    pub fn is_outline(self) -> bool {
        matches!(self, ViewMode::Outline | ViewMode::OutlineExpanded)
    }

    /// True when changed lines should be shown inline rather than counted.
    pub fn expands(self) -> bool {
        matches!(self, ViewMode::OutlineExpanded)
    }

    /// Next mode in the Shift+O cycle.
    pub fn next(self) -> ViewMode {
        match self {
            ViewMode::Diff => ViewMode::Outline,
            ViewMode::Outline => ViewMode::OutlineExpanded,
            ViewMode::OutlineExpanded => ViewMode::Diff,
        }
    }
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
    /// Hunks marked as viewed, keyed by filename (stable across reloads).
    /// Values are hunk indices within that file's `find_hunk_starts` result.
    pub viewed_hunks: HashMap<String, HashSet<usize>>,
    pub show_sidebar: bool,
    pub settings: DiffViewSettings,
    pub diff_fullscreen: DiffFullscreen,
    /// Whether the diff panel shows the normal diff or the folded outline.
    pub view_mode: ViewMode,
    /// When true, the folded outline shows only declarations touched by the diff.
    pub outline_changed_only: bool,
    pub search_state: SearchState,
    pub pending_key: PendingKey,
    pub needs_reload: bool,
    pub watching: bool,
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
    /// Cached total line count for current file (avoids recomputing diff for max_scroll)
    cached_total_lines: Option<(usize, usize)>,
    /// Cached syntax highlighters for current file (avoids re-parsing with tree-sitter every frame)
    cached_highlighters: Option<(usize, FileHighlighter, FileHighlighter)>,
    /// Cached structural outline for current file (invalidated on file change)
    cached_outline: Option<(usize, Vec<OutlineEntry>)>,
    /// Cached row plan for current file, keyed by (file, changed_only, view_mode).
    cached_plan: Option<(usize, bool, ViewMode, Vec<DiffRow>)>,
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
    pub annotation_rects: Vec<(u64, ratatui::layout::Rect)>,
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
            view_mode: ViewMode::default(),
            outline_changed_only: false,
            search_state: SearchState::default(),
            pending_key: PendingKey::default(),
            needs_reload: false,
            watching: false,
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
            cached_total_lines: None,
            cached_highlighters: None,
            cached_outline: None,
            cached_plan: None,
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

    /// Ensure the structural outline for the current file is computed and cached.
    /// Marks entries as `changed` when their line span intersects a changed hunk.
    /// Call before using `outline_ref()`.
    pub fn ensure_outline(&mut self) {
        if self.file_diffs.is_empty() {
            return;
        }
        let current = self.current_file;
        let needs_recompute = match &self.cached_outline {
            Some((cached_file, _)) => *cached_file != current,
            None => true,
        };
        if !needs_recompute {
            return;
        }

        // Side-by-side is needed to mark which declarations changed.
        self.ensure_cache();

        let diff = &self.file_diffs[current];
        // For deleted files the new side is empty, so outline the old content.
        let is_deleted = !diff.old_content.is_empty() && diff.new_content.is_empty();
        let source = if is_deleted {
            &diff.old_content
        } else {
            &diff.new_content
        };
        let mut entries = compute_outline(source, &diff.filename);

        // Collect changed source line numbers on the side we're outlining.
        let mut changed_lines: Vec<usize> = Vec::new();
        if let Some((_, sbs)) = &self.cached_side_by_side {
            for dl in sbs {
                if matches!(dl.change_type, ChangeType::Equal) {
                    continue;
                }
                let ln = if is_deleted {
                    dl.old_line.as_ref().map(|(n, _)| *n)
                } else {
                    dl.new_line.as_ref().map(|(n, _)| *n)
                };
                if let Some(n) = ln {
                    changed_lines.push(n);
                }
            }
        }
        for entry in &mut entries {
            let lo = entry.start_row + 1;
            let hi = entry.end_row + 1;
            entry.changed = changed_lines.iter().any(|&n| n >= lo && n <= hi);
        }

        self.cached_outline = Some((current, entries));
    }

    /// Immutable reference to the cached outline. Call `ensure_outline()` first.
    pub fn outline_ref(&self) -> &[OutlineEntry] {
        match &self.cached_outline {
            Some((_, data)) => data,
            None => &[],
        }
    }

    /// Ensure the row plan for the current file/mode is computed and cached.
    /// The plan is the single list the renderer and all primitives iterate; a
    /// view mode only changes which rows it contains. Call before `plan_ref()`.
    pub fn ensure_plan(&mut self) {
        if self.file_diffs.is_empty() {
            return;
        }
        let current = self.current_file;
        let changed_only = self.outline_changed_only;
        let view_mode = self.view_mode;
        let needs = match &self.cached_plan {
            Some((f, co, vm, _)) => *f != current || *co != changed_only || *vm != view_mode,
            None => true,
        };
        if !needs {
            return;
        }
        self.ensure_cache();
        self.ensure_outline();
        let is_deleted = {
            let diff = &self.file_diffs[current];
            !diff.old_content.is_empty() && diff.new_content.is_empty()
        };
        let sbs = self
            .cached_side_by_side
            .as_ref()
            .map(|(_, d)| d.as_slice())
            .unwrap_or(&[]);
        let entries = self.outline_ref();
        let rows = row_plan(view_mode, sbs, entries, is_deleted, changed_only);
        self.cached_plan = Some((current, changed_only, view_mode, rows));
    }

    /// Immutable reference to the cached row plan. Call `ensure_plan()` first.
    pub fn plan_ref(&self) -> &[DiffRow] {
        match &self.cached_plan {
            Some((_, _, _, data)) => data,
            None => &[],
        }
    }

    /// The side-by-side index a plan row renders, if it's a code row (folds
    /// return `None`). Used to map a click to a diff line for starting selection.
    pub fn sbs_index_for_plan_row(&self, plan_idx: usize) -> Option<usize> {
        match self.plan_ref().get(plan_idx)? {
            DiffRow::Code(i) => Some(*i),
            DiffRow::Fold { .. } => None,
        }
    }

    /// Side-by-side index for a plan row, clamped into range and falling back to
    /// a fold's anchor line. Used while dragging a selection (always needs a line).
    pub fn sbs_index_for_plan_row_clamped(&self, plan_idx: usize) -> usize {
        let plan = self.plan_ref();
        if plan.is_empty() {
            return 0;
        }
        plan[plan_idx.min(plan.len() - 1)].sbs_index()
    }

    /// Map a 1-based source line number to its side-by-side index (exact match
    /// on either side, else the closest preceding line).
    pub fn sbs_index_for_line(&self, line: usize) -> Option<usize> {
        let sbs = self.side_by_side_ref();
        let mut best = None;
        for (i, dl) in sbs.iter().enumerate() {
            for side in [dl.new_line.as_ref(), dl.old_line.as_ref()].into_iter().flatten() {
                if side.0 == line {
                    return Some(i);
                }
                if side.0 < line {
                    best = Some(i);
                }
            }
        }
        best
    }

    /// The source line (new side preferred) of the currently focused hunk, used
    /// as a stable anchor when switching view modes. `None` if no hunk is focused.
    pub fn focused_hunk_line(&self) -> Option<usize> {
        let fh = self.focused_hunk?;
        let idx = *self.hunks_ref().get(fh)?;
        let dl = self.side_by_side_ref().get(idx)?;
        dl.new_line
            .as_ref()
            .map(|(n, _)| *n)
            .or_else(|| dl.old_line.as_ref().map(|(n, _)| *n))
    }

    /// Plan-row index for a side-by-side index. In full mode the plan is 1:1, so
    /// this is the identity; in outline modes it finds the row (code or fold)
    /// that represents the line.
    pub fn active_row_for_sbs(&mut self, sbs_idx: usize) -> usize {
        if !self.view_mode.is_outline() {
            return sbs_idx;
        }
        self.ensure_plan();
        let mut best = 0;
        for (ri, row) in self.plan_ref().iter().enumerate() {
            if row.sbs_index() <= sbs_idx {
                best = ri;
            } else {
                break;
            }
        }
        best
    }

    /// Row index of `line` within the active view's row list. Used to keep the
    /// focused hunk pinned at the same screen height across a mode switch.
    pub fn active_row_for_line(&mut self, line: usize) -> usize {
        let target = self.sbs_index_for_line(line).unwrap_or(0);
        self.active_row_for_sbs(target)
    }

    /// Number of rows in the active view (the plan length). Used to clamp scroll.
    pub fn active_line_count(&mut self) -> usize {
        self.ensure_plan();
        self.plan_ref().len()
    }

    /// Invalidate the cache (call when file changes)
    pub fn invalidate_cache(&mut self) {
        self.cached_side_by_side = None;
        self.cached_hunks = None;
        self.cached_total_lines = None;
        self.cached_highlighters = None;
        self.cached_outline = None;
        self.cached_plan = None;
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

        // Retain annotations whose file still exists
        let filenames: HashSet<&str> = self.file_diffs.iter().map(|f| f.filename.as_str()).collect();
        self.annotations.retain(|ann| filenames.contains(ann.filename.as_str()));
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
    }

    pub fn select_file(&mut self, file_index: usize) {
        self.current_file = file_index;
        self.diff_fullscreen = DiffFullscreen::None;
        self.clear_selection(); // Clear selection when changing files
        self.invalidate_cache(); // Clear cache for new file

        let first_hunk = self.get_hunks().first().copied();
        self.focused_hunk = if first_hunk.is_some() { Some(0) } else { None };
        self.h_scroll = 0;
        // Scroll to the first hunk in the active view's coordinate space: plan
        // rows in outline modes, side-by-side indices in full diff. Without this
        // mapping, outline mode would scroll past the (much shorter) plan and
        // show an empty panel.
        self.scroll = match first_hunk {
            Some(h) => (self.active_row_for_sbs(h) as u16).saturating_sub(5),
            None => 0,
        };
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

    #[test]
    fn test_outline_marks_changed_declarations_and_folds() {
        use crate::command::diff::outline::DiffRow;
        let old_content = "fn unchanged() {\n    let x = 1;\n}\n\nfn changed() {\n    let y = 2;\n}\n";
        let new_content = "fn unchanged() {\n    let x = 1;\n}\n\nfn changed() {\n    let y = 99;\n}\n";
        let diffs = vec![FileDiff {
            filename: "lib.rs".to_string(),
            old_content: old_content.to_string(),
            new_content: new_content.to_string(),
            status: FileStatus::Modified,
            is_binary: false,
        }];

        let mut state = AppState::new(diffs, None);
        state.ensure_outline();

        // `fn unchanged` starts on line 1 (row 0), `fn changed` on line 5 (row 4).
        let changed_flag = state
            .outline_ref()
            .iter()
            .find(|e| e.line_number == 5)
            .expect("changed fn should appear in outline")
            .changed;
        let unchanged_flag = state
            .outline_ref()
            .iter()
            .find(|e| e.line_number == 1)
            .expect("unchanged fn should appear in outline")
            .changed;

        assert!(changed_flag, "modified function should be marked changed");
        assert!(
            !unchanged_flag,
            "untouched function should not be marked changed"
        );

        // Outline plan keeps both fn signatures as Code rows and collapses the
        // bodies into Fold rows; the changed body's fold reports an addition.
        state.view_mode = ViewMode::Outline;
        state.ensure_plan();
        let code_rows = state
            .plan_ref()
            .iter()
            .filter(|r| matches!(r, DiffRow::Code(_)))
            .count();
        assert!(code_rows >= 2, "both fn signatures should remain visible");
        let has_changed_fold = state
            .plan_ref()
            .iter()
            .any(|r| matches!(r, DiffRow::Fold { added, removed, .. } if *added > 0 || *removed > 0));
        assert!(has_changed_fold, "a fold should report the hidden change");

        // changed-only narrows the visible declarations.
        let full_code = code_rows;
        state.outline_changed_only = true;
        state.ensure_plan();
        let changed_code = state
            .plan_ref()
            .iter()
            .filter(|r| matches!(r, DiffRow::Code(_)))
            .count();
        assert!(changed_code < full_code, "changed-only should hide the untouched fn");

        // Full-diff plan is 1:1 with the side-by-side lines.
        state.view_mode = ViewMode::Diff;
        state.outline_changed_only = false;
        state.ensure_plan();
        assert!(state.plan_ref().iter().all(|r| matches!(r, DiffRow::Code(_))));
        assert_eq!(state.plan_ref().len(), state.total_lines());
    }
}
