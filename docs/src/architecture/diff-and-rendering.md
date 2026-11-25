# Diff Engine, Syntax & Rendering

This page explains how Lumiere transforms raw file versions into a syntax-highlighted, word-emphasized, thread-annotated side-by-side frame in milliseconds.

## 1. Loading diffs (`src/diff/mod.rs` & `src/diff/github.rs`)

`DiffRefs::from_options` resolves any `CommitReference` variant into one of four canonical strategies:

- `WorkingTree` (`parent -> disk`)
- `Single(sha)` (`sha^ -> sha`)
- `Range { from, to }` (`from -> to`, with `A...B` resolved to `merge-base(A, B) -> B`)
- `RangeToWorkingTree { from }` (`from -> disk`, with `A...-` resolved to `merge-base(A, parent) -> disk`)

For GitHub PRs (`diff::github`), Lumiere fetches metadata via parameterized GraphQL queries (`gh api graphql -f ... -F ...`) deserialized into typed `serde` structs — avoiding string interpolation and fragile manual JSON slicing. File contents for base and head revisions are pulled by a bounded pool of 8 worker threads (`PR_FETCH_CONCURRENCY = 8`).

## 2. Side-by-side pairing and word diff (`src/diff/algo.rs`)

`compute_side_by_side` diffs `old_content` against `new_content` using `similar::TextDiff::from_lines`:

1. Consecutive `Delete` and `Insert` runs within a change group are paired into `ChangeType::Modified` rows up to `min(deletes, inserts)`; leftover lines become standalone `Delete` or `Insert` rows with diagonal stripe placeholders on the opposite side.
2. Each `Modified` pair runs `similar::TextDiff::from_words` to emit `InlineSegment { text, emphasized }` slices for intra-line word highlighting.
3. Tabs are expanded consistently according to `settings.tab_width` (default 4).

### Hunks-only folding

When `settings.hunks_only` is enabled, `hide_unchanged_rows` scans the paired rows and keeps:

- Every non-`Equal` row plus `HUNK_CONTEXT_ROWS` (`3`) lines above and below each change.
- Any row pinned by a review thread (`thread.target` covers that row's line number on the thread's panel).

Contiguous hidden runs collapse into a single `DiffLine::separator(hidden_lines)` row (`⋯ N unchanged lines`).

## 3. Caching in `ComputedDiff`

`AppState` wraps the active file's computed artifacts in a single `ComputedDiff` struct:

```rust
pub struct ComputedDiff {
    pub file_index: usize,
    pub rows: Vec<DiffLine>,
    pub hunks: Vec<usize>,
    pub stats: LineStats,
    pub max_old_width: usize,
    pub max_new_width: usize,
    pub pinned_ranges: Vec<(DiffPanelFocus, RangeInclusive<usize>)>,
}
```

- Recomputing occurs **only** when switching files, reloading, toggling hunks-only/full-file view, or when a thread is added/removed on the active file.
- Per-frame footer stats (`+added -removed`) and horizontal scroll bounds (`max_h_scroll`) are O(1) field reads instead of O(N) full-file scans.
- Switching between `hunks_only` and `full` anchors either the focused hunk (when visible) or the top visible line number so the viewport stays rock-steady.

## 4. Lazy Tree-sitter highlighting & sticky scope context

### Syntax highlighting (`src/diff/highlight/`)

Lumiere bundles 18 Tree-sitter grammars (Rust, TypeScript/TSX, JavaScript/JSX, Python, Go, C, C++, C#, Java, Zig, Ruby, Elixir, Bash, JSON, TOML, HTML, CSS, Markdown):

- **Lazy per-language initialization:** Each language's `HighlightConfiguration` is compiled once on first use via `std::sync::OnceLock`, instead of compiling all 18 grammars at startup.
- **Whole-file `FileHighlighter`:** Files up to 1 MB (`MAX_WHOLE_FILE_BYTES = 1_048_576`) are highlighted once into per-line byte-range token spans using a scope stack (so nested captures like string interpolations restore the enclosing color on `HighlightEvent::HighlightEnd`).
- **Large-file fallback:** Files exceeding 1 MB skip whole-file parsing and highlight only the ~40 visible rows on demand using a thread-local `tree_sitter_highlight::Highlighter` (`SNIPPET_HIGHLIGHTER`), avoiding per-line parser allocations.

### Sticky scope headers (`src/diff/context.rs`)

- Scope queries for the 8 context-enabled languages are lazily compiled per language via `OnceLock`.
- Given the top visible line number of each panel, `compute_context_lines` walks the cached Tree-sitter AST to find enclosing scopes (`fn`, `impl`, `class`, `method`, etc.) and slices each header line directly via O(line_len) byte scanning (`line_at_byte`) without collecting all file lines into a `Vec`.

## 5. Rendering pipeline (`src/ui/views/`)

`render_diff(frame, &DiffRenderInput)` produces a `DiffRenderOutput` containing layout feedback for mouse hit-testing (`content_row_offset`, `overlay_gaps`, `annotation_rects`, `editor_rect`):

1. **Unified panel paths:** Added and deleted files share one `render_single_panel` implementation; side-by-side Old and New panels share one `render_panel_row` builder.
2. **Single-pass line styling (`highlight_diff_line`):** Combines base syntax tokens, word-level diff emphasis (with automatic luminance/saturation boosting for low-contrast tokens), search match highlights, and blended mouse-selection tint in one UTF-8-safe pass.
3. **Inline overlays (`overlays.rs`):** File-level threads reserve placeholder rows at the top; line-range threads inject synchronized placeholder gaps into both panels after `end_line`, then draw bordered Markdown thread boxes (or the inline `AnnotationEditor`) over those slots and extend the shared center-border marker (`▎`) for New-panel threads.
4. **Event-driven redraws:** The event loop tracks `app.dirty: bool` and only invokes `terminal.draw` when user input, a resize, a file-watcher reload, a thread file change, an agent status transition, or notice expiration occurred.
