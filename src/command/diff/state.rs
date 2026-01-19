use std::collections::{HashMap, HashSet};

use crate::command::diff::diff_algo::{compute_side_by_side, find_hunk_starts};

/// Maximum number of diff lines to include inline when exporting annotations.
/// Hunks with more lines than this will not include the diff content in the export
/// to keep the output concise.
const MAX_EXPORT_DIFF_LINES: usize = 5;
use crate::command::diff::search::SearchState;
use crate::command::diff::types::{
    build_file_tree, ChangeType, DiffFullscreen, DiffViewSettings, FileDiff, FocusedPanel,
    SidebarItem,
};
use crate::vcs::StackedCommitInfo;

#[derive(Default, Clone, Copy, PartialEq)]
pub enum PendingKey {
    #[default]
    None,
    G,
}

/// An annotation attached to a specific hunk in a file.
///
/// Annotations allow users to add notes to code changes during review.
/// Each annotation is uniquely identified by its file index and hunk index.
#[derive(Clone)]
pub struct HunkAnnotation {
    /// Index of the file in the file_diffs vector
    pub file_index: usize,
    /// Index of the hunk within the file (0-based)
    pub hunk_index: usize,
    /// The annotation text content (supports multi-line)
    pub content: String,
    /// Line range in the new file (start_line, end_line) for display purposes
    pub line_range: (usize, usize),
    /// The filename for display in export and UI
    pub filename: String,
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
    pub focused_hunk: Option<usize>,
    // Annotation fields
    pub annotations: Vec<HunkAnnotation>,
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
            stacked_mode: false,
            stacked_commits: Vec::new(),
            current_commit_index: 0,
            stacked_viewed_files: HashMap::new(),
            vcs_name: "git", // Default, will be set by caller
            diff_reference: None,
        }
    }

    /// Set the VCS backend name
    pub fn set_vcs_name(&mut self, name: &'static str) {
        self.vcs_name = name;
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

        // Clear annotations as they reference old file/hunk indices
        self.annotations.clear();

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

    /// Get annotation for a specific hunk in a file
    pub fn get_annotation(&self, file_index: usize, hunk_index: usize) -> Option<&HunkAnnotation> {
        self.annotations
            .iter()
            .find(|a| a.file_index == file_index && a.hunk_index == hunk_index)
    }

    /// Add or update an annotation
    pub fn set_annotation(&mut self, annotation: HunkAnnotation) {
        if let Some(existing) = self
            .annotations
            .iter_mut()
            .find(|a| a.file_index == annotation.file_index && a.hunk_index == annotation.hunk_index)
        {
            *existing = annotation;
        } else {
            self.annotations.push(annotation);
        }
    }

    /// Remove an annotation
    pub fn remove_annotation(&mut self, file_index: usize, hunk_index: usize) {
        self.annotations
            .retain(|a| !(a.file_index == file_index && a.hunk_index == hunk_index));
    }

    /// Format all annotations for export with full diff context
    pub fn format_annotations_for_export(&self) -> String {
        let mut result = String::new();

        // Add header with diff reference context
        if let Some(ref reference) = self.diff_reference {
            result.push_str(&format!("# Annotations for diff: {}\n\n", reference));
        }

        let annotations_text = self
            .annotations
            .iter()
            .map(|a| {
                // Try to get the diff content for this hunk
                let diff_content = self.get_hunk_diff_content(a.file_index, a.hunk_index);

                let mut output = format!("## {}", a.filename);

                // Add line info based on what we have
                if let Some((old_range, new_range, _)) = &diff_content {
                    // Format line ranges intelligently
                    match (old_range, new_range) {
                        (Some(_), Some((new_start, new_end))) => {
                            // Modified: show new file lines
                            if new_start == new_end {
                                output.push_str(&format!(":L{}", new_start));
                            } else {
                                output.push_str(&format!(":L{}-{}", new_start, new_end));
                            }
                        }
                        (Some((old_start, old_end)), None) => {
                            // Pure deletion: indicate where it was in the base
                            let base_ref = self
                                .diff_reference
                                .as_ref()
                                .and_then(|r| {
                                    // Check for three-dot range first, then two-dot, then single ref
                                    if let Some((base, _)) = r.split_once("...") {
                                        Some(base)
                                    } else if let Some((base, _)) = r.split_once("..") {
                                        Some(base)
                                    } else {
                                        Some(r.as_str())
                                    }
                                })
                                .unwrap_or("base");
                            if old_start == old_end {
                                output.push_str(&format!(" (deleted from {}:L{})", base_ref, old_start));
                            } else {
                                output.push_str(&format!(
                                    " (deleted from {}:L{}-{})",
                                    base_ref, old_start, old_end
                                ));
                            }
                        }
                        (None, Some((new_start, new_end))) => {
                            // Pure addition
                            if new_start == new_end {
                                output.push_str(&format!(":L{}", new_start));
                            } else {
                                output.push_str(&format!(":L{}-{}", new_start, new_end));
                            }
                        }
                        (None, None) => {
                            // Fallback to stored line_range
                            output.push_str(&format!(":L{}-{}", a.line_range.0, a.line_range.1));
                        }
                    }
                } else {
                    // Fallback if we can't compute diff
                    output.push_str(&format!(":L{}-{}", a.line_range.0, a.line_range.1));
                }

                output.push('\n');

                // Add diff content if available and small enough
                if let Some((_, _, lines)) = diff_content {
                    let line_count = lines.lines().count();
                    if line_count > 0 && line_count <= MAX_EXPORT_DIFF_LINES {
                        output.push_str("```diff\n");
                        output.push_str(&lines);
                        output.push_str("```\n");
                    }
                }

                // Add the annotation
                output.push_str(&format!("**Note:** {}\n", a.content));
                output
            })
            .collect::<Vec<_>>()
            .join("\n");

        result.push_str(&annotations_text);
        result
    }

    /// Get the diff content for a specific hunk
    /// Returns (old_line_range, new_line_range, diff_lines)
    fn get_hunk_diff_content(
        &self,
        file_index: usize,
        hunk_index: usize,
    ) -> Option<(Option<(usize, usize)>, Option<(usize, usize)>, String)> {
        let diff = self.file_diffs.get(file_index)?;
        let side_by_side =
            compute_side_by_side(&diff.old_content, &diff.new_content, self.settings.tab_width);
        let hunks = find_hunk_starts(&side_by_side);

        let hunk_start = *hunks.get(hunk_index)?;
        let next_hunk_start = hunks.get(hunk_index + 1).copied().unwrap_or(side_by_side.len());

        let mut diff_lines = String::new();
        let mut old_start: Option<usize> = None;
        let mut old_end: Option<usize> = None;
        let mut new_start: Option<usize> = None;
        let mut new_end: Option<usize> = None;

        for i in hunk_start..next_hunk_start {
            let dl = &side_by_side[i];
            if matches!(dl.change_type, ChangeType::Equal) {
                continue;
            }

            match dl.change_type {
                ChangeType::Delete => {
                    if let Some((num, text)) = &dl.old_line {
                        diff_lines.push_str(&format!("- {}\n", text));
                        if old_start.is_none() {
                            old_start = Some(*num);
                        }
                        old_end = Some(*num);
                    }
                }
                ChangeType::Insert => {
                    if let Some((num, text)) = &dl.new_line {
                        diff_lines.push_str(&format!("+ {}\n", text));
                        if new_start.is_none() {
                            new_start = Some(*num);
                        }
                        new_end = Some(*num);
                    }
                }
                ChangeType::Modified => {
                    if let Some((num, text)) = &dl.old_line {
                        diff_lines.push_str(&format!("- {}\n", text));
                        if old_start.is_none() {
                            old_start = Some(*num);
                        }
                        old_end = Some(*num);
                    }
                    if let Some((num, text)) = &dl.new_line {
                        diff_lines.push_str(&format!("+ {}\n", text));
                        if new_start.is_none() {
                            new_start = Some(*num);
                        }
                        new_end = Some(*num);
                    }
                }
                ChangeType::Equal => {}
            }
        }

        let old_range = old_start.zip(old_end);
        let new_range = new_start.zip(new_end);

        Some((old_range, new_range, diff_lines))
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
