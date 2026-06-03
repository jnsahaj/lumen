//! Telescope-style global search across all files in the diff.
//!
//! Indexes every side-by-side line in every file once when the modal opens,
//! then re-filters with a nucleo-matcher fuzzy score on each keystroke.

use std::cell::RefCell;
use std::collections::HashMap;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};

use super::diff_algo::compute_side_by_side;
use super::highlight::FileHighlighter;
use super::search::MatchPanel;
use super::types::{ChangeType, DiffLine, DiffViewSettings, FileDiff};

/// Maximum number of results shown in the list. Above this, the user should narrow the query.
const MAX_RESULTS: usize = 500;

/// One searchable line in the global index.
#[derive(Clone)]
pub struct GlobalSearchEntry {
    pub file_index: usize,
    /// Index into the file's side-by-side rows.
    pub sbs_line_index: usize,
    pub panel: MatchPanel,
    /// 1-based line number from the original file (for display).
    pub line_no: usize,
    /// The line text itself, used both for matching and for the preview snippet.
    #[allow(dead_code)]
    pub text: String,
    /// What kind of change this line represents (drives the gutter color).
    pub change: LineChange,
    /// Short filename (display only).
    pub filename: String,
    /// Combined haystack: "filename:lineno  text". Built once at index time.
    /// Public for the renderer so it can highlight matched chars by index.
    pub haystack: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum LineChange {
    Equal,
    Added,
    Removed,
    Modified,
}

/// A filtered result with its match score and which characters of the haystack matched
/// (for highlighting in the result list).
#[derive(Clone)]
pub struct ScoredResult {
    pub entry_index: usize,
    pub score: u32,
    /// Char indices into `haystack` that matched, sorted ascending. Used to bold the
    /// matched chars in the result row.
    pub match_indices: Vec<u32>,
}

/// Per-file content kept around so we can lazily build a `FileHighlighter`
/// (and re-derive the side-by-side rows) for the preview pane. Cheap clones
/// happen once at modal-open time.
pub struct FileSnapshot {
    pub filename: String,
    pub old: String,
    pub new: String,
    /// Tab width to use when re-computing side-by-side for the preview.
    pub tab_width: usize,
}

pub struct GlobalSearchState {
    pub query: String,
    /// Full corpus, built once on modal open.
    pub entries: Vec<GlobalSearchEntry>,
    /// Filtered + scored results for the current query. Capped at MAX_RESULTS.
    pub results: Vec<ScoredResult>,
    /// Currently highlighted row in `results`. Set by arrow-key navigation
    /// only — mouse wheel scrolls the list view without touching this.
    pub selected: usize,
    /// Vertical scroll offset into the results list. Adjusted by mouse wheel
    /// directly, and auto-adjusted by selection moves to keep the cursor
    /// visible. The selected row may scroll off-screen if the user wheels past it.
    pub list_scroll: usize,
    /// Horizontal scroll into result rows. The selector (`❯ `) stays pinned;
    /// only the path/text body slides left as this grows.
    pub list_scroll_x: usize,
    /// Extra vertical scroll applied to the preview pane on top of cursor
    /// centering. Resets to 0 when the selected result changes.
    pub preview_scroll_y: i32,
    /// Horizontal scroll into the preview body (chars from the left edge of
    /// the body content, after the line-number gutter and change symbol).
    pub preview_scroll_x: usize,
    /// File contents, indexed by file_index, used to build highlighters on demand.
    pub files: Vec<FileSnapshot>,
    /// Tree-sitter highlighter cache, keyed by (file_index, is_new_panel).
    /// `RefCell` lets us populate on demand inside the immutable preview renderer.
    highlighter_cache: RefCell<HashMap<(usize, bool), FileHighlighter>>,
    /// Per-file side-by-side rows. Pre-populated during `build()` (since we
    /// already compute SBS there to index entries) so the first preview render
    /// is cache-hot. `RefCell` covers the rare miss path for binary files.
    sbs_cache: RefCell<HashMap<usize, Vec<DiffLine>>>,
    /// Reusable nucleo `Matcher`. Held across keystrokes so we don't pay
    /// `Matcher::new` (allocates internal scratch slabs) on every refilter.
    /// `take()`-and-restore pattern dodges the borrow-checker issue of holding
    /// `&mut matcher` and `&mut self.results` simultaneously.
    matcher: Option<Matcher>,
}

impl GlobalSearchState {
    /// Build the full index from all file diffs. This is the expensive step;
    /// call once when the modal opens.
    pub fn build(file_diffs: &[FileDiff], settings: &DiffViewSettings) -> Self {
        let mut entries = Vec::new();
        let mut files = Vec::with_capacity(file_diffs.len());
        // Pre-seed the sbs cache with what we compute below so the first
        // preview render is cache-hot instead of triggering a recompute.
        let mut sbs_cache: HashMap<usize, Vec<DiffLine>> = HashMap::with_capacity(file_diffs.len());

        for (file_idx, file) in file_diffs.iter().enumerate() {
            // Always push a snapshot so `files[file_idx]` lines up with file_diffs
            // indexing. Binary files get an empty snapshot — they're skipped below.
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

            // Full relative path from project root — git already emits diff
            // filenames as project-relative, so no further work needed.
            let filename_short = file.filename.clone();

            for (sbs_idx, line) in sbs.iter().enumerate() {
                let change = match line.change_type {
                    ChangeType::Equal => LineChange::Equal,
                    ChangeType::Delete => LineChange::Removed,
                    ChangeType::Insert => LineChange::Added,
                    ChangeType::Modified => LineChange::Modified,
                };

                // Decide which panels are worth indexing for this row:
                //   Equal     → new only (old and new are identical text — no point indexing twice)
                //   Delete    → old only (only side that has text)
                //   Insert    → new only (only side that has text)
                //   Modified  → both    (old and new are genuinely different text)
                let (emit_old, emit_new) = match line.change_type {
                    ChangeType::Equal => (false, true),
                    ChangeType::Delete => (true, false),
                    ChangeType::Insert => (false, true),
                    ChangeType::Modified => (true, true),
                };

                let mut push = |panel: MatchPanel, ln: usize, text: &str| {
                    if text.trim().is_empty() {
                        return;
                    }
                    let haystack = format!("{}:{}  {}", filename_short, ln, text);
                    entries.push(GlobalSearchEntry {
                        file_index: file_idx,
                        sbs_line_index: sbs_idx,
                        panel,
                        line_no: ln,
                        text: text.to_string(),
                        change,
                        filename: filename_short.clone(),
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

            // Stash the freshly-computed SBS so the first preview render for
            // this file doesn't have to recompute it.
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

    /// Scroll the preview pane vertically by `delta` rows (negative = up).
    /// Triggered by mouse wheel over the preview area.
    pub fn scroll_preview_y(&mut self, delta: i32) {
        self.preview_scroll_y = self.preview_scroll_y.saturating_add(delta);
    }

    /// Scroll the preview body horizontally by `delta` columns. Clamped to 0.
    pub fn scroll_preview_x(&mut self, delta: i32) {
        let new = (self.preview_scroll_x as i32).saturating_add(delta);
        self.preview_scroll_x = new.max(0) as usize;
    }

    /// Mouse wheel on the list pane scrolls the view by `delta` rows. Unlike
    /// arrow-key navigation, this does NOT change the selected row — the user
    /// can scroll past the selection. `visible` is the pane's row count,
    /// computed by the caller from the terminal size.
    pub fn scroll_list_y(&mut self, delta: i32, visible: usize) {
        let max = self.results.len().saturating_sub(visible.max(1));
        let new = (self.list_scroll as i32 + delta).clamp(0, max as i32);
        self.list_scroll = new as usize;
    }

    /// Horizontal mouse scroll on the list pane shifts result rows left/right
    /// so long paths / matched text can be read in full.
    pub fn scroll_list_x(&mut self, delta: i32) {
        let new = (self.list_scroll_x as i32 + delta).max(0);
        self.list_scroll_x = new as usize;
    }

    /// Adjust `list_scroll` so the selected row is on-screen. Called from the
    /// selection-move methods only — mouse-wheel scrolling intentionally
    /// bypasses this so the user can scroll past the cursor.
    fn ensure_selection_visible(&mut self, visible: usize) {
        let visible = visible.max(1);
        if self.selected < self.list_scroll {
            self.list_scroll = self.selected;
        } else if self.selected >= self.list_scroll + visible {
            self.list_scroll = self.selected + 1 - visible;
        }
    }

    /// Borrow the cached side-by-side rows for `file_index`, computing them
    /// on first access. The closure receives a slice — handing the renderer
    /// raw `Ref` ergonomics would force lifetime gymnastics for callers.
    pub fn with_sbs<R>(&self, file_index: usize, f: impl FnOnce(&[DiffLine]) -> R) -> R {
        // Populate the cache on miss. Split into its own scope so the mutable
        // borrow drops before we take the immutable borrow below.
        if !self.sbs_cache.borrow().contains_key(&file_index) {
            if let Some(file) = self.files.get(file_index) {
                let sbs = compute_side_by_side(&file.old, &file.new, file.tab_width);
                self.sbs_cache.borrow_mut().insert(file_index, sbs);
            }
        }
        let cache = self.sbs_cache.borrow();
        f(cache.get(&file_index).map(Vec::as_slice).unwrap_or(&[]))
    }

    /// Render-side helper: get pre-highlighted spans for a specific line.
    ///
    /// Lazily builds (and caches) a `FileHighlighter` for the requested
    /// `(file_index, panel)` pair. Panels are tracked separately because old and
    /// new content are independent files from tree-sitter's perspective —
    /// constructs straddling line boundaries differ between the two.
    pub fn highlighted_line_spans<'a>(
        &self,
        file_index: usize,
        panel: MatchPanel,
        line_no: usize,
        bg: Option<ratatui::style::Color>,
    ) -> Vec<ratatui::text::Span<'a>> {
        let is_new = matches!(panel, MatchPanel::New);
        let key = (file_index, is_new);

        // Fast path: cache hit.
        if let Some(h) = self.highlighter_cache.borrow().get(&key) {
            return h.get_line_spans(line_no, bg);
        }

        // Miss: build the highlighter from the corresponding file content.
        let Some(file) = self.files.get(file_index) else {
            return Vec::new();
        };
        let content = if is_new { &file.new } else { &file.old };
        let highlighter = FileHighlighter::new(content, &file.filename);
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

    /// Drop trailing whitespace and then the last word from the query.
    /// Used by both `ctrl+w` and `opt+backspace`. Re-filters once after
    /// the edit, instead of paying for one refilter per removed char.
    pub fn erase_query_word(&mut self) {
        // Walk back from the end: skip trailing whitespace, then keep dropping
        // chars until the next char is whitespace (or we run out).
        while matches!(self.query.chars().last(), Some(c) if c.is_whitespace()) {
            self.query.pop();
        }
        while matches!(self.query.chars().last(), Some(c) if !c.is_whitespace()) {
            self.query.pop();
        }
        self.refilter();
    }

    /// Move the selected row to `target` (clamped) and update list scroll +
    /// preview scroll. Single chokepoint for every selection-changing key.
    /// `step_moves` skips the work when the position didn't change (so a
    /// no-op `move_down` at end-of-list doesn't reset preview scroll);
    /// jump-style moves bypass that guard.
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
        let step = visible.max(1);
        self.set_selection(self.selected.saturating_add(step), visible, true);
    }

    pub fn page_up(&mut self, visible: usize) {
        let step = visible.max(1);
        self.set_selection(self.selected.saturating_sub(step), visible, true);
    }

    pub fn jump_top(&mut self, visible: usize) {
        self.set_selection(0, visible, false);
    }

    pub fn jump_bottom(&mut self, visible: usize) {
        self.set_selection(usize::MAX, visible, false);
    }

    /// Mouse-click selection: jump straight to `target` (clamped) and reset
    /// preview scroll. Treated as a jump (not a step) so clicking the already-
    /// selected row still resets preview scroll.
    pub fn select(&mut self, target: usize, visible: usize) {
        self.set_selection(target, visible, false);
    }

    /// Get the currently selected entry, if any.
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
            // Empty query → empty results. The list and preview stay blank until
            // the user types something. This avoids the "what is all this stuff?"
            // feeling of seeing the first N file lines on modal open.
            return;
        }

        let pattern = Pattern::parse(&self.query, CaseMatching::Smart, Normalization::Smart);
        // Reuse the matcher across keystrokes — `Matcher::new` allocates
        // internal scratch slabs we'd rather not re-pay every refilter. Take
        // it out so we can hold `&mut matcher` while also writing self.results
        // below, then put it back at the end.
        let mut matcher = self
            .matcher
            .take()
            .unwrap_or_else(|| Matcher::new(Config::DEFAULT));

        // Score every entry. nucleo-matcher returns Option<u32>; None = no match.
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

        // Highest score first, then by entry-index for tie stability.
        let cmp = |a: &ScoredResult, b: &ScoredResult| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.entry_index.cmp(&b.entry_index))
        };
        // Partition first (O(N)) to find the top MAX_RESULTS unsorted, then
        // sort just those K (O(K log K)). For typical big diffs this is ~10x
        // faster than sorting the full match list before truncating.
        if scored.len() > MAX_RESULTS {
            scored.select_nth_unstable_by(MAX_RESULTS, cmp);
            scored.truncate(MAX_RESULTS);
        }
        scored.sort_by(cmp);
        self.results = scored;
        self.matcher = Some(matcher);
    }
}

