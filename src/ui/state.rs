use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::RangeInclusive;

use tree_sitter::{Parser, Tree};

use crate::diff::context::get_language_context;
use crate::diff::highlight::{too_large_to_parse, FileHighlighter};
use crate::diff::{
    count_added_removed, get_current_branch, ComputedDiff, DiffFullscreen, DiffLine, DiffOptions,
    DiffPanelFocus, DiffViewSettings, FileDiff, FileStatus, LineStats,
};
use crate::review::store::{StoreStamp, ThreadStore};
use crate::review::{anchor_text_for, refresh_anchors, AnnotationTarget, Comment, Thread};
use crate::search::SearchState;
use crate::ui::coordinates::{CursorPosition, Selection, SelectionMode, ThreadHitRect};
use crate::vcs::{StackedCommitInfo, VcsBackend};

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum FocusedPanel {
    Sidebar,
    #[default]
    DiffView,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum PendingKey {
    #[default]
    None,
    G,
}

#[derive(Clone)]
pub enum SidebarItem {
    Directory {
        name: String,
        path: String,
        depth: usize,
    },
    File {
        name: String,
        path: String,
        file_index: usize,
        depth: usize,
        status: FileStatus,
    },
}

impl SidebarItem {
    pub fn path(&self) -> &str {
        match self {
            Self::Directory { path, .. } | Self::File { path, .. } => path,
        }
    }
}

pub fn is_child_path(path: &str, parent: &str) -> bool {
    !parent.is_empty()
        && path
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub fn build_file_tree(file_diffs: &[FileDiff]) -> Vec<SidebarItem> {
    if file_diffs.is_empty() {
        return Vec::new();
    }

    let mut file_paths: Vec<(String, usize, FileStatus)> = file_diffs
        .iter()
        .enumerate()
        .map(|(idx, diff)| (diff.filename.clone(), idx, diff.status))
        .collect();
    file_paths.sort_by(|a, b| a.0.cmp(&b.0));

    let mut dir_children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    dir_children.insert(String::new(), BTreeSet::new());

    for (path, _, _) in &file_paths {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() > 1 {
            let parent_path = parts[..parts.len() - 1].join("/");
            dir_children
                .entry(parent_path)
                .or_default()
                .insert(path.clone());
        } else {
            dir_children.get_mut("").unwrap().insert(path.clone());
        }

        for i in 0..parts.len().saturating_sub(1) {
            let dir_path = parts[..=i].join("/");
            let parent_path = if i == 0 {
                String::new()
            } else {
                parts[..i].join("/")
            };
            dir_children.entry(dir_path.clone()).or_default();
            dir_children
                .entry(parent_path)
                .or_default()
                .insert(dir_path);
        }
    }

    fn collapse_single_child_dirs(
        dir_path: &str,
        dir_children: &BTreeMap<String, BTreeSet<String>>,
        file_paths_set: &BTreeSet<String>,
    ) -> String {
        let mut current = dir_path.to_string();
        while let Some(children) = dir_children.get(&current) {
            if children.len() == 1 {
                let child = children.iter().next().unwrap();
                if !file_paths_set.contains(child) {
                    current = child.clone();
                    continue;
                }
            }
            break;
        }
        current
    }

    let file_paths_set: BTreeSet<String> = file_paths.iter().map(|(p, _, _)| p.clone()).collect();
    let mut items = Vec::new();
    let mut added_dirs = BTreeSet::new();
    let mut path_to_collapsed: BTreeMap<String, String> = BTreeMap::new();
    let mut collapsed_depth: BTreeMap<String, usize> = BTreeMap::new();

    for (path, file_idx, status) in &file_paths {
        let parts: Vec<&str> = path.split('/').collect();
        let file_name = (*parts.last().unwrap_or(&"")).to_string();

        let mut i = 0;
        while i < parts.len().saturating_sub(1) {
            let dir_path = parts[..=i].join("/");
            let collapsed_path =
                collapse_single_child_dirs(&dir_path, &dir_children, &file_paths_set);
            let collapsed_len = collapsed_path.split('/').count();

            if added_dirs.insert(collapsed_path.clone()) {
                let collapsed_parts: Vec<&str> = collapsed_path.split('/').collect();
                for j in 0..collapsed_parts.len() {
                    let intermediate = collapsed_parts[..=j].join("/");
                    path_to_collapsed
                        .entry(intermediate)
                        .or_insert_with(|| collapsed_path.clone());
                }

                let depth = if i == 0 {
                    0
                } else {
                    let parent_dir = parts[..i].join("/");
                    path_to_collapsed
                        .get(&parent_dir)
                        .and_then(|parent_collapsed| collapsed_depth.get(parent_collapsed))
                        .map_or(0, |d| d + 1)
                };
                collapsed_depth.insert(collapsed_path.clone(), depth);

                let display_name = if i == 0 {
                    collapsed_path.clone()
                } else {
                    let parent_dir = parts[..i].join("/");
                    path_to_collapsed
                        .get(&parent_dir)
                        .and_then(|parent_collapsed| {
                            collapsed_path
                                .strip_prefix(parent_collapsed.as_str())
                                .and_then(|rest| rest.strip_prefix('/'))
                        })
                        .map_or_else(|| collapsed_path.clone(), str::to_string)
                };

                items.push(SidebarItem::Directory {
                    name: display_name,
                    path: collapsed_path,
                    depth,
                });
            }

            i = collapsed_len;
        }

        let file_depth = if parts.len() > 1 {
            let parent_dir = parts[..parts.len() - 1].join("/");
            path_to_collapsed
                .get(&parent_dir)
                .and_then(|parent_collapsed| collapsed_depth.get(parent_collapsed))
                .map_or(0, |d| d + 1)
        } else {
            0
        };

        items.push(SidebarItem::File {
            name: file_name,
            path: path.clone(),
            file_index: *file_idx,
            depth: file_depth,
            status: *status,
        });
    }

    items
}

fn build_sidebar_visible_indices(
    items: &[SidebarItem],
    collapsed_dirs: &HashSet<String>,
) -> Vec<usize> {
    let mut visible = Vec::new();
    let mut collapsed_stack: Vec<&str> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        let path = item.path();
        while let Some(&last) = collapsed_stack.last() {
            if is_child_path(path, last) {
                break;
            }
            collapsed_stack.pop();
        }
        if collapsed_stack
            .last()
            .is_some_and(|&last| is_child_path(path, last))
        {
            continue;
        }
        visible.push(idx);
        if let SidebarItem::Directory { path, .. } = item {
            if collapsed_dirs.contains(path) {
                collapsed_stack.push(path.as_str());
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
    pub viewed_hunks: HashMap<String, HashSet<usize>>,
    pub show_sidebar: bool,
    pub settings: DiffViewSettings,
    pub diff_fullscreen: DiffFullscreen,
    pub search_state: SearchState,
    pub pending_key: PendingKey,
    pub needs_reload: bool,
    pub watching: bool,
    pub focused_hunk: Option<usize>,
    pub threads: Vec<Thread>,
    thread_store: Option<ThreadStore>,
    thread_store_stamp: Option<StoreStamp>,
    pub thread_store_error: Option<String>,
    pub stacked_mode: bool,
    pub stacked_commits: Vec<StackedCommitInfo>,
    pub current_commit_index: usize,
    stacked_viewed_files: HashMap<String, HashSet<String>>,
    pub vcs_name: &'static str,
    pub current_branch: String,
    pub diff_reference: Option<String>,
    pub diff_panel_focus: DiffPanelFocus,
    pub selection: Selection,
    pub is_dragging: bool,
    pub show_selection_tooltip: bool,
    cached_diff: Option<ComputedDiff>,
    cached_highlighters: Option<(usize, FileHighlighter, FileHighlighter)>,
    pub cached_trees: TreeCache,
    search_dirty: bool,
    pub content_row_offset: usize,
    pub annotation_overlay_gaps: Vec<(usize, usize)>,
    pub annotation_rects: Vec<ThreadHitRect>,
    pub editor_rect: Option<ratatui::layout::Rect>,
    pub total_added: usize,
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
        Self::new_with_settings(file_diffs, focus_file, DiffViewSettings::default())
    }

    pub fn new_with_settings(
        file_diffs: Vec<FileDiff>,
        focus_file: Option<&str>,
        settings: DiffViewSettings,
    ) -> Self {
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
                    "\x1b[93mwarning:\x1b[0m --focus file '{focus_path}' not found in diff, using first file"
                );
                Self::find_first_file(&sidebar_items, &sidebar_visible)
            }
        } else {
            Self::find_first_file(&sidebar_items, &sidebar_visible)
        };
        let (total_added, total_removed) = compute_total_line_stats(&file_diffs);

        let mut state = Self {
            file_diffs,
            sidebar_items,
            sidebar_visible,
            collapsed_dirs,
            current_file,
            sidebar_selected,
            sidebar_scroll: 0,
            sidebar_h_scroll: 0,
            scroll: 0,
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
            focused_hunk: None,
            threads: Vec::new(),
            thread_store: None,
            thread_store_stamp: None,
            thread_store_error: None,
            stacked_mode: false,
            stacked_commits: Vec::new(),
            current_commit_index: 0,
            stacked_viewed_files: HashMap::new(),
            vcs_name: "git",
            current_branch: "unknown".to_string(),
            diff_reference: None,
            diff_panel_focus: DiffPanelFocus::default(),
            selection: Selection::default(),
            is_dragging: false,
            show_selection_tooltip: false,
            cached_diff: None,
            cached_highlighters: None,
            cached_trees: TreeCache::default(),
            search_dirty: true,
            content_row_offset: 0,
            annotation_overlay_gaps: Vec::new(),
            annotation_rects: Vec::new(),
            editor_rect: None,
            total_added,
            total_removed,
        };
        if current_file < state.file_diffs.len() {
            state.select_file(current_file);
        }
        state
    }

    pub fn for_repo(
        options: &DiffOptions,
        file_diffs: Vec<FileDiff>,
        backend: &dyn VcsBackend,
        focus_file: Option<&str>,
    ) -> Self {
        let settings = DiffViewSettings {
            wrap: options.wrap,
            hunks_only: options.hunks_only,
            ..DiffViewSettings::default()
        };
        let mut state = Self::new_with_settings(file_diffs, focus_file, settings);
        state.set_vcs_name(backend.name());
        state.refresh_branch(backend);
        state.set_diff_reference(options.reference.as_ref().map(ToString::to_string));
        state
    }

    pub fn attach_thread_store(&mut self, backend: &dyn VcsBackend) {
        match ThreadStore::for_repository(backend.working_tree_root()) {
            Ok(store) => self.open_thread_store(store),
            Err(error) => self.thread_store_error = Some(error),
        }
    }

    pub fn refresh_branch(&mut self, backend: &dyn VcsBackend) {
        self.current_branch = get_current_branch(backend);
    }

    fn find_first_file(sidebar_items: &[SidebarItem], sidebar_visible: &[usize]) -> (usize, usize) {
        for (visible_idx, &item_idx) in sidebar_visible.iter().enumerate() {
            if let SidebarItem::File { file_index, .. } = &sidebar_items[item_idx] {
                return (visible_idx, *file_index);
            }
        }
        (0, 0)
    }

    pub fn get_hunks(&mut self) -> &[usize] {
        self.ensure_cache();
        self.hunks_ref()
    }

    pub fn ensure_cache(&mut self) {
        if self.file_diffs.is_empty() {
            return;
        }
        let cached_for_current = self
            .cached_diff
            .as_ref()
            .is_some_and(|c| c.file_index == self.current_file);
        if !cached_for_current {
            self.build_rows();
            self.parse_syntax_trees();
        } else if self
            .cached_diff
            .as_ref()
            .is_some_and(|c| c.pinned_ranges != self.thread_line_ranges())
        {
            self.rebuild_rows_keeping_in_place(self.scroll as usize);
        }
    }

    fn build_rows(&mut self) {
        let current = self.current_file;
        let diff = &self.file_diffs[current];
        let pinned_ranges = self.thread_line_ranges();
        self.cached_diff = Some(ComputedDiff::build(
            current,
            diff,
            &self.settings,
            pinned_ranges,
        ));
    }

    fn thread_line_ranges(&self) -> Vec<(DiffPanelFocus, RangeInclusive<usize>)> {
        let Some(diff) = self.file_diffs.get(self.current_file) else {
            return Vec::new();
        };
        if !self.settings.hunks_only {
            return Vec::new();
        }
        self.threads
            .iter()
            .filter(|thread| thread.filename == diff.filename)
            .filter_map(|thread| match &thread.target {
                AnnotationTarget::LineRange {
                    panel,
                    start_line,
                    end_line,
                } => Some((*panel, *start_line..=*end_line)),
                AnnotationTarget::File => None,
            })
            .collect()
    }

    fn rebuild_rows_keeping_in_place(&mut self, anchor_row: usize) {
        let line_of = |row: &DiffLine| {
            let line_on = |panel: DiffPanelFocus| row.line_number(panel).map(|line| (panel, line));
            line_on(DiffPanelFocus::New).or_else(|| line_on(DiffPanelFocus::Old))
        };
        let scroll = self.scroll as usize;
        let previous_rows = self.side_by_side_ref();
        let previous_row_count = previous_rows.len();
        let anchor = previous_rows
            .iter()
            .enumerate()
            .skip(anchor_row)
            .find_map(|(row, shown)| Some((row.saturating_sub(scroll), line_of(shown)?)));

        self.build_rows();
        self.search_dirty = true;
        let rows = self.side_by_side_ref();
        if rows.len() == previous_row_count {
            return;
        }
        let scroll_keeping_anchor = anchor.map(|(rows_below_top, (panel, line))| {
            rows.iter()
                .position(|row| row.line_number(panel).is_some_and(|shown| shown >= line))
                .unwrap_or(rows.len().saturating_sub(1))
                .saturating_sub(rows_below_top)
        });
        if let Some(scroll) = scroll_keeping_anchor {
            self.scroll = scroll.min(u16::MAX as usize) as u16;
        }
        self.clear_selection();
    }

    pub fn set_hunks_only(&mut self, hunks_only: bool, visible_rows: usize) {
        if self.settings.hunks_only == hunks_only {
            return;
        }
        self.settings.hunks_only = hunks_only;
        if self.cached_diff.is_none() {
            return;
        }
        let scroll = self.scroll as usize;
        let focused_hunk_row = self
            .focused_hunk
            .and_then(|hunk| self.hunks_ref().get(hunk).copied())
            .filter(|row| (scroll..scroll + visible_rows).contains(row));
        self.rebuild_rows_keeping_in_place(focused_hunk_row.unwrap_or(scroll));
    }

    fn parse_syntax_trees(&mut self) {
        let diff = &self.file_diffs[self.current_file];
        if self.current_file_is_too_large_to_parse() {
            self.cached_trees.old_file_trees.remove(&diff.filename);
            self.cached_trees.new_file_trees.remove(&diff.filename);
            return;
        }
        let Some(lang_ctx) = get_language_context(&diff.filename) else {
            return;
        };
        let mut parser = Parser::new();
        if parser.set_language(&lang_ctx.language).is_err() {
            return;
        }
        let filename = diff.filename.clone();
        if let Some(tree) = parser.parse(&diff.old_content, None) {
            self.cached_trees
                .old_file_trees
                .insert(filename.clone(), tree);
        }
        if let Some(tree) = parser.parse(&diff.new_content, None) {
            self.cached_trees.new_file_trees.insert(filename, tree);
        }
    }

    pub fn side_by_side_ref(&self) -> &[DiffLine] {
        self.cached_diff.as_ref().map_or(&[], |c| c.rows.as_slice())
    }

    pub fn hunks_ref(&self) -> &[usize] {
        self.cached_diff
            .as_ref()
            .map_or(&[], |c| c.hunks.as_slice())
    }

    pub fn line_stats(&self) -> LineStats {
        self.cached_diff
            .as_ref()
            .map_or_else(LineStats::default, |c| c.stats)
    }

    pub fn max_line_widths(&self) -> (usize, usize) {
        self.cached_diff
            .as_ref()
            .map_or((0, 0), |c| (c.max_old_width, c.max_new_width))
    }

    pub fn update_search_matches(&mut self) {
        self.ensure_cache();
        if self.search_dirty {
            let sbs = self
                .cached_diff
                .as_ref()
                .map_or(&[][..], |c| c.rows.as_slice());
            self.search_state.update_matches(sbs, self.diff_fullscreen);
            self.search_dirty = false;
        }
    }

    pub fn mark_search_dirty(&mut self) {
        self.search_dirty = true;
    }

    pub fn total_lines(&mut self) -> usize {
        self.ensure_cache();
        self.cached_diff.as_ref().map_or(0, |c| c.rows.len())
    }

    pub fn get_highlighters(&mut self) -> (&FileHighlighter, &FileHighlighter) {
        let current = self.current_file;
        let needs_recompute =
            !matches!(&self.cached_highlighters, Some((f, _, _)) if *f == current);
        if needs_recompute {
            let diff = &self.file_diffs[current];
            let (old_hl, new_hl) = if self.current_file_is_too_large_to_parse() {
                (FileHighlighter::default(), FileHighlighter::default())
            } else {
                (
                    FileHighlighter::new(&diff.old_content, &diff.filename),
                    FileHighlighter::new(&diff.new_content, &diff.filename),
                )
            };
            self.cached_highlighters = Some((current, old_hl, new_hl));
        }
        let (_, old_hl, new_hl) = self.cached_highlighters.as_ref().unwrap();
        (old_hl, new_hl)
    }

    pub fn current_file_is_too_large_to_parse(&self) -> bool {
        self.file_diffs
            .get(self.current_file)
            .is_some_and(|diff| too_large_to_parse(&diff.old_content, &diff.new_content))
    }

    pub fn highlighters_ref(&self) -> Option<(&FileHighlighter, &FileHighlighter)> {
        self.cached_highlighters
            .as_ref()
            .map(|(_, old_hl, new_hl)| (old_hl, new_hl))
    }

    pub fn invalidate_cache(&mut self) {
        self.cached_diff = None;
        self.cached_highlighters = None;
        self.search_dirty = true;
        self.content_row_offset = 0;
        self.annotation_overlay_gaps.clear();
        self.annotation_rects.clear();
        self.editor_rect = None;
    }

    pub fn adjust_for_overlay_gaps(&self, content_y: usize) -> Option<usize> {
        let mut cumulative = 0;
        for &(after_line, gap_height) in &self.annotation_overlay_gaps {
            let gap_screen_start = after_line + 1 + cumulative;
            let gap_screen_end = gap_screen_start + gap_height;
            if content_y < gap_screen_start {
                break;
            }
            if content_y < gap_screen_end {
                return None;
            }
            cumulative += gap_height;
        }
        Some(content_y - cumulative)
    }

    pub fn adjust_for_overlay_gaps_clamped(&self, content_y: usize) -> usize {
        let mut cumulative = 0;
        for &(after_line, gap_height) in &self.annotation_overlay_gaps {
            let gap_screen_start = after_line + 1 + cumulative;
            let gap_screen_end = gap_screen_start + gap_height;
            if content_y < gap_screen_start {
                break;
            }
            if content_y < gap_screen_end {
                return after_line;
            }
            cumulative += gap_height;
        }
        content_y - cumulative
    }

    pub fn clear_selection(&mut self) {
        self.diff_panel_focus = DiffPanelFocus::None;
        self.selection = Selection::default();
        self.is_dragging = false;
        self.show_selection_tooltip = false;
    }

    pub fn start_selection(
        &mut self,
        panel: DiffPanelFocus,
        pos: CursorPosition,
        mode: SelectionMode,
    ) {
        self.diff_panel_focus = panel;
        self.selection = Selection {
            panel,
            anchor: pos,
            head: pos,
            mode,
        };
        self.is_dragging = true;
    }

    pub fn extend_selection(&mut self, pos: CursorPosition) {
        if self.is_dragging {
            self.selection.head = pos;
        }
    }

    pub fn end_drag(&mut self) {
        self.is_dragging = false;
        if self.selection.is_active() && self.selection.anchor != self.selection.head {
            self.show_selection_tooltip = true;
        }
    }

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
        let existing_dirs: HashSet<&str> = self
            .sidebar_items
            .iter()
            .filter_map(|item| match item {
                SidebarItem::Directory { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect();
        self.collapsed_dirs
            .retain(|path| existing_dirs.contains(path.as_str()));
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
                if is_child_path(item.path(), dir_path) {
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

    pub fn set_diff_reference(&mut self, reference: Option<String>) {
        self.diff_reference = reference;
    }

    pub fn init_stacked_mode(&mut self, commits: Vec<StackedCommitInfo>) {
        self.stacked_mode = true;
        self.stacked_commits = commits;
        self.current_commit_index = 0;
    }

    pub fn current_commit(&self) -> Option<&StackedCommitInfo> {
        self.stacked_mode
            .then(|| self.stacked_commits.get(self.current_commit_index))
            .flatten()
    }

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

    pub fn reload(&mut self, file_diffs: Vec<FileDiff>, changed_files: Option<&HashSet<String>>) {
        let old_filename = self
            .file_diffs
            .get(self.current_file)
            .map(|f| f.filename.clone());
        let old_scroll = self.scroll;
        let old_h_scroll = self.h_scroll;

        let mut viewed_filenames: HashSet<String> = self
            .viewed_files
            .iter()
            .filter_map(|&idx| self.file_diffs.get(idx).map(|f| f.filename.clone()))
            .collect();

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

        let filenames: HashSet<&str> = self
            .file_diffs
            .iter()
            .map(|f| f.filename.as_str())
            .collect();
        self.viewed_hunks
            .retain(|fname, _| filenames.contains(fname.as_str()));

        self.viewed_files = self
            .file_diffs
            .iter()
            .enumerate()
            .filter(|(_, f)| viewed_filenames.contains(&f.filename))
            .map(|(i, _)| i)
            .collect();

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
        self.cached_trees.old_file_trees.clear();
        self.cached_trees.new_file_trees.clear();
        self.invalidate_cache();

        if !self.file_diffs.is_empty() {
            let total = self.total_lines();
            let max_scroll = total.saturating_sub(10).min(u16::MAX as usize) as u16;
            self.scroll = old_scroll.min(max_scroll);
            self.h_scroll = old_h_scroll;
            let hunk_count = self.hunks_ref().len();
            self.focused_hunk = if hunk_count == 0 {
                None
            } else {
                self.focused_hunk.map(|idx| idx.min(hunk_count - 1))
            };
        } else {
            self.focused_hunk = None;
        }

        self.refresh_thread_anchors();
    }

    pub fn select_file(&mut self, file_index: usize) {
        self.current_file = file_index;
        self.diff_fullscreen = DiffFullscreen::None;
        self.clear_selection();
        self.invalidate_cache();

        let hunks = self.get_hunks().to_vec();
        self.scroll = hunks
            .first()
            .map(|&h| (h as u16).saturating_sub(5))
            .unwrap_or(0);
        self.h_scroll = 0;
        self.focused_hunk = (!hunks.is_empty()).then_some(0);
    }

    pub fn open_thread_store(&mut self, store: ThreadStore) {
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

    fn change_threads<T>(&mut self, change: impl Fn(&mut Vec<Thread>) -> T) -> T {
        if let Some(store) = &self.thread_store {
            match store.update(&change) {
                Ok((threads, outcome)) => {
                    self.threads = threads;
                    self.thread_store_stamp = store.stamp();
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

    pub fn mark_sent(&mut self, only: Option<(&str, &str)>) -> usize {
        self.change_threads(|threads| {
            threads
                .iter_mut()
                .filter(|thread| only.is_none_or(|(thread_id, _)| thread.id == thread_id))
                .map(|thread| thread.mark_sent(only.map(|(_, comment_id)| comment_id)))
                .sum()
        })
    }

    pub fn format_threads_for_export(&self, only_queued: bool) -> String {
        let mut result = String::new();

        if let Some(ref reference) = self.diff_reference {
            result.push_str(&format!("# {reference}\n\n"));
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
                result.push_str(&format!(" {line_reference}"));
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
    let max_scroll = max_scroll.min(u16::MAX as usize) as u16;

    let new_scroll = if line < scroll_usize + margin {
        line.saturating_sub(margin).min(u16::MAX as usize) as u16
    } else if line >= scroll_usize + content_height.saturating_sub(margin) {
        line.saturating_sub(content_height.saturating_sub(margin).saturating_sub(1))
            .min(u16::MAX as usize) as u16
    } else {
        scroll
    };
    new_scroll.min(max_scroll)
}

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
    let max_scroll = max_scroll.min(u16::MAX as usize) as u16;

    if hunk_line < scroll_usize + top_margin {
        return (hunk_line.saturating_sub(top_margin).min(u16::MAX as usize) as u16)
            .min(max_scroll);
    }

    if hunk_line >= scroll_usize + content_height.saturating_sub(bottom_margin) {
        return (hunk_line
            .saturating_sub(
                content_height
                    .saturating_sub(bottom_margin)
                    .saturating_sub(1),
            )
            .min(u16::MAX as usize) as u16)
            .min(max_scroll);
    }

    scroll
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn make_file_diff(filename: &str) -> FileDiff {
        FileDiff {
            filename: filename.to_string(),
            old_content: String::new(),
            new_content: "content\n".to_string(),
            status: FileStatus::Added,
            is_binary: false,
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
    fn test_focus_selection_and_sidebar_ordering() {
        let diffs = vec![
            make_file_diff("src/main.rs"),
            make_file_diff("src/lib.rs"),
            make_file_diff("README.md"),
        ];
        let focused = AppState::new(diffs, Some("src/lib.rs"));
        assert_eq!(
            focused.file_diffs[focused.current_file].filename,
            "src/lib.rs"
        );
        assert!(matches!(
            focused.sidebar_item_at_visible(focused.sidebar_selected),
            Some(SidebarItem::File { file_index, .. }) if *file_index == focused.current_file
        ));

        let sorted = AppState::new(
            vec![make_file_diff("bbb.rs"), make_file_diff("aaa.rs")],
            None,
        );
        assert_eq!(sorted.file_diffs[sorted.current_file].filename, "aaa.rs");

        let fallback = AppState::new(
            vec![make_file_diff("bbb.rs"), make_file_diff("aaa.rs")],
            Some("nonexistent.rs"),
        );
        assert_eq!(
            fallback.file_diffs[fallback.current_file].filename,
            "aaa.rs"
        );

        let empty = AppState::new(vec![], Some("any.rs"));
        assert_eq!(empty.current_file, 0);
        assert!(empty.file_diffs.is_empty());
    }

    #[test]
    fn test_new_comments_are_queued_until_marked_sent() {
        let mut state = AppState::new(vec![make_file_diff("a.rs")], None);
        let (thread_id, comment_id) =
            state.add_thread("a.rs".to_string(), line_target(1), "first".to_string());
        state.add_thread(
            "a.rs".to_string(),
            AnnotationTarget::File,
            "second".to_string(),
        );

        assert_eq!(state.queued_count(), 2);
        assert!(!state.threads[0].awaiting_agent());
        assert_eq!(
            state.queued_text_for_agent(Some((&thread_id, &comment_id))),
            format!(
                "## Thread {thread_id} · a.rs line 1 (RIGHT)\n\n```\ncontent\n```\n\nyou: first"
            )
        );
        assert_eq!(
            state
                .queued_text_for_agent(None)
                .matches("## Thread ")
                .count(),
            2
        );

        assert_eq!(state.mark_sent(Some((&thread_id, &comment_id))), 1);
        assert_eq!(state.queued_count(), 1);

        let follow_up_id = state
            .add_reply(&thread_id, "and this?".to_string())
            .expect("reply");
        assert_eq!(
            state.queued_text_for_agent(Some((&thread_id, &follow_up_id))),
            format!(
                "## Thread {thread_id} · a.rs line 1 (RIGHT) · awaiting your reply · earlier comments: 1\n\nyou: and this?"
            )
        );
        assert_eq!(state.mark_sent(Some((&thread_id, &follow_up_id))), 1);
        assert!(state.threads[0].awaiting_agent());
        assert_eq!(state.format_threads_for_export(true), "**a.rs**\n\nsecond");

        assert_eq!(state.mark_sent(None), 1);
        assert_eq!(state.mark_sent(None), 0);
    }

    #[test]
    fn test_hunks_only_view_keeps_thread_lines_and_top_line_when_toggled() {
        let numbered = |changed: usize| -> String {
            (1..=100)
                .map(|line| {
                    if line == changed {
                        format!("changed {line}\n")
                    } else {
                        format!("line {line}\n")
                    }
                })
                .collect()
        };
        let mut state = AppState::new(
            vec![FileDiff {
                filename: "a.rs".to_string(),
                old_content: numbered(0),
                new_content: numbered(50),
                status: FileStatus::Modified,
                is_binary: false,
            }],
            None,
        );
        let top_line = |state: &AppState| {
            state.side_by_side_ref()[state.scroll as usize].line_number(DiffPanelFocus::New)
        };

        assert_eq!(state.total_lines(), 9);
        assert_eq!(
            state.line_stats(),
            LineStats {
                added: 1,
                removed: 1
            }
        );

        state.set_hunks_only(false, 20);
        assert_eq!((state.total_lines(), top_line(&state)), (100, Some(46)));
        state.set_hunks_only(true, 20);
        assert_eq!((state.total_lines(), state.scroll), (9, 0));

        let (thread_id, _) =
            state.add_thread("a.rs".to_string(), line_target(90), "why?".to_string());
        assert_eq!(state.total_lines(), 11);
        state.scroll = 9;
        assert_eq!(top_line(&state), Some(90));

        state.set_hunks_only(false, 0);
        assert_eq!(state.total_lines(), 100);
        assert_eq!(top_line(&state), Some(90));
        assert_eq!(state.get_hunks().len(), 1);

        state.set_hunks_only(true, 0);
        assert_eq!(top_line(&state), Some(90));
        assert_eq!(state.get_hunks().len(), 1);

        state.remove_thread(&thread_id);
        assert_eq!(state.total_lines(), 9);
    }

    #[test]
    fn test_replying_reopens_thread_and_open_thread_wins_over_resolved() {
        let mut state = AppState::new(vec![make_file_diff("a.rs")], None);
        let (older_id, _) =
            state.add_thread("a.rs".to_string(), line_target(1), "first".to_string());
        state.toggle_thread_resolved(&older_id);
        assert!(state.threads[0].resolved);

        let (newer_id, _) =
            state.add_thread("a.rs".to_string(), line_target(1), "second".to_string());
        let found = |state: &AppState| {
            state
                .thread_at("a.rs", &line_target(1), true)
                .map(|t| t.id.clone())
        };
        assert_eq!(found(&state), Some(newer_id.clone()));

        state.toggle_thread_resolved(&newer_id);
        assert_eq!(found(&state), Some(older_id.clone()));

        state.add_reply(&older_id, "one more thing".to_string());
        assert!(!state.threads[0].resolved);
    }

    #[test]
    fn test_threads_persist_and_pick_up_external_replies() {
        let dir = TempDir::new().unwrap();
        let open_state = || {
            let mut state = AppState::new(vec![make_file_diff("a.rs")], None);
            state.open_thread_store(ThreadStore::for_repository(dir.path()).unwrap());
            state
        };
        let mut state = open_state();
        let (thread_id, _) =
            state.add_thread("a.rs".to_string(), line_target(1), "why?".to_string());
        state.mark_sent(None);
        assert_eq!(state.threads[0].anchor_text.as_deref(), Some("content"));
        let bodies = |state: &AppState| -> Vec<String> {
            state.threads[0]
                .comments
                .iter()
                .map(|c| c.body.clone())
                .collect()
        };
        let reply_as_agent = |body: &str| {
            ThreadStore::for_repository(dir.path())
                .unwrap()
                .update(|threads| {
                    threads[0].add_agent_reply(Comment::agent(body.to_string(), "pi".to_string()))
                })
                .unwrap();
        };

        reply_as_agent("because");
        assert!(state.reload_threads_if_changed());
        assert_eq!(bodies(&state), vec!["why?", "because"]);

        reply_as_agent("and also");
        state.add_reply(&thread_id, "ok".to_string());
        assert_eq!(bodies(&state), vec!["why?", "because", "and also", "ok"]);
        assert_eq!(
            bodies(&open_state()),
            vec!["why?", "because", "and also", "ok"]
        );
    }

    #[test]
    fn test_reload_marks_removed_file_threads_outdated() {
        let mut state = AppState::new(vec![make_file_diff("a.rs")], None);
        state.add_thread("a.rs".to_string(), line_target(1), "first".to_string());
        state.reload(vec![make_file_diff("b.rs")], None);
        assert_eq!(state.threads.len(), 1);
        assert!(state.threads[0].outdated);
    }

    fn python_file_diff(filename: &str, old_content: String) -> FileDiff {
        FileDiff {
            filename: filename.to_string(),
            new_content: format!("{old_content}added = True\n"),
            old_content,
            status: FileStatus::Modified,
            is_binary: false,
        }
    }

    fn small_python_source() -> String {
        "def f():\n    return 1\n".to_string()
    }

    fn large_python_source() -> String {
        (0..60_000).map(|n| format!("value_{n} = {n}\n")).collect()
    }

    #[test]
    fn test_large_files_skip_whole_file_parsing_and_drop_stale_trees() {
        let mut state = AppState::new(
            vec![
                python_file_diff("a_small.py", small_python_source()),
                python_file_diff("b_large.py", large_python_source()),
            ],
            None,
        );

        assert!(!state.current_file_is_too_large_to_parse());
        assert!(!state.get_highlighters().1.is_empty());
        assert!(state.cached_trees.new_file_trees.contains_key("a_small.py"));

        state.select_file(1);
        assert!(state.current_file_is_too_large_to_parse());
        let (old_hl, new_hl) = state.get_highlighters();
        assert!(old_hl.is_empty() && new_hl.is_empty());
        assert!(!state.cached_trees.old_file_trees.contains_key("b_large.py"));
        assert!(!state.cached_trees.new_file_trees.contains_key("b_large.py"));

        let mut growing =
            AppState::new(vec![python_file_diff("a.py", small_python_source())], None);
        assert!(growing.cached_trees.new_file_trees.contains_key("a.py"));
        growing.reload(vec![python_file_diff("a.py", large_python_source())], None);
        growing.ensure_cache();
        assert!(!growing.cached_trees.old_file_trees.contains_key("a.py"));
        assert!(!growing.cached_trees.new_file_trees.contains_key("a.py"));
    }

    #[test]
    fn test_reload_clamps_focused_hunk_when_hunks_shrink() {
        let two_hunks = FileDiff {
            filename: "a.rs".to_string(),
            old_content: "a1\n".to_string() + &"same\n".repeat(12) + "b1\n",
            new_content: "a2\n".to_string() + &"same\n".repeat(12) + "b2\n",
            status: FileStatus::Modified,
            is_binary: false,
        };
        let one_hunk = FileDiff {
            filename: "a.rs".to_string(),
            old_content: "a1\n".to_string() + &"same\n".repeat(12) + "b1\n",
            new_content: "a2\n".to_string() + &"same\n".repeat(12) + "b1\n",
            status: FileStatus::Modified,
            is_binary: false,
        };

        let mut state = AppState::new(vec![two_hunks], None);
        assert_eq!(state.get_hunks().len(), 2);
        state.focused_hunk = Some(1);

        state.reload(vec![one_hunk], None);
        assert_eq!(state.get_hunks().len(), 1);
        assert_eq!(state.focused_hunk, Some(0));

        let no_hunks = FileDiff {
            filename: "a.rs".to_string(),
            old_content: "same\n".to_string(),
            new_content: "same\n".to_string(),
            status: FileStatus::Modified,
            is_binary: false,
        };
        state.reload(vec![no_hunks], None);
        assert_eq!(state.focused_hunk, None);
    }
}
