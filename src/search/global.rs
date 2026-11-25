//! Global fuzzy search across all files in the diff using `nucleo-matcher`.

use std::cell::RefCell;
use std::collections::HashMap;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

use super::{erase_word_backward, MatchPanel};
use crate::diff::highlight::{too_large_to_parse, FileHighlighter};
use crate::diff::{compute_side_by_side, ChangeType, DiffLine, DiffViewSettings, FileDiff};

const MAX_RESULTS: usize = 500;

#[derive(Clone)]
pub struct GlobalSearchEntry {
    pub file_index: usize,
    pub sbs_line_index: usize,
    pub panel: MatchPanel,
    pub line_no: usize,
    pub change: LineChange,
    pub filename: String,
    pub haystack: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LineChange {
    Equal,
    Added,
    Removed,
    Modified,
}

#[derive(Clone)]
pub struct ScoredResult {
    pub entry_index: usize,
    pub score: u32,
    pub match_indices: Vec<u32>,
}

pub struct FileSnapshot {
    pub filename: String,
    pub old: String,
    pub new: String,
    pub tab_width: usize,
}

pub struct GlobalSearchState {
    pub query: String,
    pub entries: Vec<GlobalSearchEntry>,
    pub results: Vec<ScoredResult>,
    pub selected: usize,
    pub list_scroll: usize,
    pub list_scroll_x: usize,
    pub preview_scroll_y: i32,
    pub preview_scroll_x: usize,
    pub files: Vec<FileSnapshot>,
    highlighter_cache: RefCell<HashMap<(usize, bool), FileHighlighter>>,
    sbs_cache: RefCell<HashMap<usize, Vec<DiffLine>>>,
    matcher: Option<Matcher>,
}

impl GlobalSearchState {
    pub fn build(file_diffs: &[FileDiff], settings: &DiffViewSettings) -> Self {
        let mut entries = Vec::new();
        let mut files = Vec::with_capacity(file_diffs.len());
        let mut sbs_cache = HashMap::with_capacity(file_diffs.len());

        for (file_idx, file) in file_diffs.iter().enumerate() {
            files.push(FileSnapshot {
                filename: file.filename.clone(),
                old: file.old_content.clone(),
                new: file.new_content.clone(),
                tab_width: settings.tab_width,
            });

            if file.is_binary {
                continue;
            }
            let sbs =
                compute_side_by_side(&file.old_content, &file.new_content, settings.tab_width);

            for (sbs_idx, line) in sbs.iter().enumerate() {
                let change = match line.change_type {
                    ChangeType::Equal => LineChange::Equal,
                    ChangeType::Delete => LineChange::Removed,
                    ChangeType::Insert => LineChange::Added,
                    ChangeType::Modified => LineChange::Modified,
                };

                let (emit_old, emit_new) = match line.change_type {
                    ChangeType::Equal | ChangeType::Insert => (false, true),
                    ChangeType::Delete => (true, false),
                    ChangeType::Modified => (true, true),
                };

                let mut push = |panel: MatchPanel, ln: usize, text: &str| {
                    if text.trim().is_empty() {
                        return;
                    }
                    let haystack = format!("{}:{ln}  {text}", file.filename);
                    entries.push(GlobalSearchEntry {
                        file_index: file_idx,
                        sbs_line_index: sbs_idx,
                        panel,
                        line_no: ln,
                        change,
                        filename: file.filename.clone(),
                        haystack,
                    });
                };

                if emit_old {
                    if let Some((ln, text)) = &line.old_line {
                        push(MatchPanel::Old, *ln, text);
                    }
                }
                if emit_new {
                    if let Some((ln, text)) = &line.new_line {
                        push(MatchPanel::New, *ln, text);
                    }
                }
            }

            sbs_cache.insert(file_idx, sbs);
        }

        Self {
            query: String::new(),
            entries,
            results: Vec::new(),
            selected: 0,
            list_scroll: 0,
            list_scroll_x: 0,
            preview_scroll_y: 0,
            preview_scroll_x: 0,
            files,
            highlighter_cache: RefCell::new(HashMap::new()),
            sbs_cache: RefCell::new(sbs_cache),
            matcher: None,
        }
    }

    fn reset_preview_scroll(&mut self) {
        self.preview_scroll_y = 0;
        self.preview_scroll_x = 0;
    }

    pub fn scroll_preview_y(&mut self, delta: i32) {
        self.preview_scroll_y = self.preview_scroll_y.saturating_add(delta);
    }

    pub fn scroll_preview_x(&mut self, delta: i32) {
        self.preview_scroll_x =
            (self.preview_scroll_x as i32).saturating_add(delta).max(0) as usize;
    }

    pub fn scroll_list_y(&mut self, delta: i32, visible: usize) {
        let max = self.results.len().saturating_sub(visible.max(1));
        self.list_scroll = (self.list_scroll as i32 + delta).clamp(0, max as i32) as usize;
    }

    pub fn scroll_list_x(&mut self, delta: i32) {
        self.list_scroll_x = (self.list_scroll_x as i32 + delta).max(0) as usize;
    }

    fn ensure_selection_visible(&mut self, visible: usize) {
        let visible = visible.max(1);
        if self.selected < self.list_scroll {
            self.list_scroll = self.selected;
        } else if self.selected >= self.list_scroll + visible {
            self.list_scroll = self.selected + 1 - visible;
        }
    }

    pub fn with_sbs<R>(&self, file_index: usize, f: impl FnOnce(&[DiffLine]) -> R) -> R {
        if !self.sbs_cache.borrow().contains_key(&file_index) {
            if let Some(file) = self.files.get(file_index) {
                let sbs = compute_side_by_side(&file.old, &file.new, file.tab_width);
                self.sbs_cache.borrow_mut().insert(file_index, sbs);
            }
        }
        let cache = self.sbs_cache.borrow();
        f(cache.get(&file_index).map(Vec::as_slice).unwrap_or(&[]))
    }

    pub fn highlighted_line_spans<'a>(
        &self,
        file_index: usize,
        panel: MatchPanel,
        line_no: usize,
        bg: Option<ratatui::style::Color>,
    ) -> Vec<ratatui::text::Span<'a>> {
        let is_new = matches!(panel, MatchPanel::New);
        let key = (file_index, is_new);

        if let Some(h) = self.highlighter_cache.borrow().get(&key) {
            return h.get_line_spans(line_no, bg);
        }

        let Some(file) = self.files.get(file_index) else {
            return Vec::new();
        };
        let content = if is_new { &file.new } else { &file.old };
        let highlighter = if too_large_to_parse(&file.old, &file.new) {
            FileHighlighter::default()
        } else {
            FileHighlighter::new(content, &file.filename)
        };
        let spans = highlighter.get_line_spans(line_no, bg);
        self.highlighter_cache.borrow_mut().insert(key, highlighter);
        spans
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
        self.refilter();
    }

    pub fn clear_query(&mut self) {
        self.query.clear();
        self.refilter();
    }

    pub fn erase_query_word(&mut self) {
        erase_word_backward(&mut self.query);
        self.refilter();
    }

    fn set_selection(&mut self, target: usize, visible: usize, step_move: bool) {
        let max = self.results.len().saturating_sub(1);
        let new = target.min(max);
        if step_move && new == self.selected {
            return;
        }
        self.selected = new;
        self.ensure_selection_visible(visible);
        self.reset_preview_scroll();
    }

    pub fn move_down(&mut self, visible: usize) {
        self.set_selection(self.selected.saturating_add(1), visible, true);
    }

    pub fn move_up(&mut self, visible: usize) {
        self.set_selection(self.selected.saturating_sub(1), visible, true);
    }

    pub fn page_down(&mut self, visible: usize) {
        self.set_selection(self.selected.saturating_add(visible.max(1)), visible, true);
    }

    pub fn page_up(&mut self, visible: usize) {
        self.set_selection(self.selected.saturating_sub(visible.max(1)), visible, true);
    }

    pub fn jump_top(&mut self, visible: usize) {
        self.set_selection(0, visible, false);
    }

    pub fn jump_bottom(&mut self, visible: usize) {
        self.set_selection(usize::MAX, visible, false);
    }

    pub fn select(&mut self, target: usize, visible: usize) {
        self.set_selection(target, visible, false);
    }

    pub fn current_entry(&self) -> Option<&GlobalSearchEntry> {
        self.results
            .get(self.selected)
            .and_then(|r| self.entries.get(r.entry_index))
    }

    pub fn result_count(&self) -> usize {
        self.results.len()
    }

    pub fn total_indexed(&self) -> usize {
        self.entries.len()
    }

    fn refilter(&mut self) {
        self.selected = 0;
        self.list_scroll = 0;
        self.reset_preview_scroll();
        self.results.clear();

        if self.query.is_empty() {
            return;
        }

        let pattern = Pattern::parse(&self.query, CaseMatching::Smart, Normalization::Smart);
        let mut matcher = self
            .matcher
            .take()
            .unwrap_or_else(|| Matcher::new(Config::DEFAULT));

        let mut scored: Vec<ScoredResult> = Vec::with_capacity(self.entries.len() / 4);
        let mut indices_buf: Vec<u32> = Vec::new();
        let mut haystack_buf = Vec::new();

        for (i, entry) in self.entries.iter().enumerate() {
            haystack_buf.clear();
            let haystack = nucleo_matcher::Utf32Str::new(&entry.haystack, &mut haystack_buf);
            indices_buf.clear();
            if let Some(score) = pattern.indices(haystack, &mut matcher, &mut indices_buf) {
                scored.push(ScoredResult {
                    entry_index: i,
                    score,
                    match_indices: indices_buf.clone(),
                });
            }
        }

        let cmp = |a: &ScoredResult, b: &ScoredResult| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.entry_index.cmp(&b.entry_index))
        };
        if scored.len() > MAX_RESULTS {
            scored.select_nth_unstable_by(MAX_RESULTS, cmp);
            scored.truncate(MAX_RESULTS);
        }
        scored.sort_by(cmp);
        self.results = scored;
        self.matcher = Some(matcher);
    }
}
