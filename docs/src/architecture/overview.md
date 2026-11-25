# Architecture & Core Primitives

Lumiere is organized as a library crate (`src/lib.rs`) paired with a minimal CLI entrypoint (`src/main.rs`). Rather than sprawling feature folders with circular dependencies, the codebase is structured around **five orthogonal primitives** with strict one-way dependencies.

## The five primitives

```text
                       ┌─────────────────────┐
                       │   main.rs / lib.rs  │   CLI dispatch & error boundary
                       │  cli.rs · config.rs │
                       └──────────┬──────────┘
                                  │
                                  ▼
                       ┌─────────────────────┐
                       │         ui/         │   Model-Update-View Ratatui TUI
                       │  App · State · View │   TerminalGuard · Event Loop
                       └──┬───────┬───────┬──┘
                          │       │       │
            ┌─────────────┘       │       └─────────────┐
            ▼                     ▼                     ▼
  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐
  │     search/      │  │     review/      │  │      diff/       │
  │ SearchState      │  │ Thread · Store   │  │ FileDiff         │
  │ GlobalSearch     │  │ AgentLink/Worker │  │ ComputedDiff     │
  └────────┬─────────┘  │ Workspace · CLI  │  │ Highlight · PR   │
           │            └────────┬─────────┘  │ Context · Watch  │
           └──────────────┐      │            └────────┬─────────┘
                          ▼      ▼                     │
                       ┌─────────────────────┐         │
                       │        diff/        │◄────────┘
                       └──────────┬──────────┘
                                  │
                                  ▼
                       ┌─────────────────────┐
                       │        vcs/         │   GitBackend (libgit2)
                       │  VcsBackend trait   │   JjBackend (jj-lib)
                       │  CommitReference    │   Repository detection
                       └─────────────────────┘
```

### 1. `vcs` — Version control source of truth (`src/vcs/`)

Defines the `VcsBackend` trait and `CommitReference` parser, abstracting Git (`libgit2`) and Jujutsu (`jj-lib`, feature-gated behind `jj`). All repository queries — changed file lists, file contents at any revision, merge bases, parent resolution, and commit ranges for `--stacked` — flow through this boundary. No module above `vcs` touches `git2` or `jj-lib` directly.

### 2. `diff` — Pure diff computation & enrichment (`src/diff/`)

Turns VCS revisions (or GitHub PRs via `diff::github`) into `FileDiff` values and `ComputedDiff` views:

- `FileDiff` holds raw old/new text, `FileStatus`, and binary detection.
- `ComputedDiff` caches side-by-side paired `DiffLine` rows, hunk start offsets, `LineStats` (`+added`/`-removed`), maximum line widths, and thread-pinned ranges so rendering and scrolling stay O(1) even on 360k-line files.
- `diff::algo` implements side-by-side pairing, word-level emphasis via `similar`, hunk boundary detection, and hunks-only folding.
- `diff::highlight` and `diff::context` provide lazy per-language Tree-sitter syntax highlighting (mapping capture indices directly against the active `ui::theme` palette) and enclosing scope headers.
- `diff::watcher` debounces filesystem and VCS events (skipping gitignored directories on Linux).

### 3. `review` — Threaded conversations, persistence & agent bridge (`src/review/`)

Owns all review-workflow state independent of any terminal UI:

- `Thread`, `Comment`, `Author`, and `AnnotationTarget` model line-range and file-level conversations plus anchor-text re-matching.
- `ThreadStore` reads and writes versioned `threads.json` files safely across concurrent processes (`lumiere diff` and `lumiere thread reply`) using advisory file locks and atomic temp-file rename.
- `review::cli` implements `lumiere thread {list, show, reply}` directly on top of `ThreadStore` and `Workspace`.
- `review::agent` implements `Rex` pane discovery, safety checks, bracketed-paste submission, and the non-blocking background `AgentWorker` thread.
- `review::workspace` discovers sibling repositories under a workspace directory without importing any UI types.

### 4. `search` — In-file & global fuzzy search (`src/search/`)

- `SearchState` performs case-insensitive UTF-8-safe match scanning over visible diff rows (with an ASCII fast path and full Unicode char-boundary mapping for expanding case folds).
- `GlobalSearchState` indexes every non-blank line across all diff files into `nucleo-matcher` and caches syntax highlighters lazily for the Telescope-style preview pane.

### 5. `ui` — Model-Update-View Ratatui frontend (`src/ui/`)

Clean separation of lifecycle, state, input transitions, and pure rendering:

- **`ui/mod.rs`:** `TerminalGuard` (RAII Drop + panic hook that restores raw mode, alternate screen, mouse capture, and kitty keyboard enhancement flags on error or unwind), `App` runtime container, non-blocking `AgentWorker` polling, and event-driven `dirty` redraw loop.
- **`ui/state.rs` & `ui/workspace.rs`:** `AppState` (owns `ComputedDiff`, cached branch name, sidebar tree, selection, and thread store connection) and `WorkspaceSession` (parks/swaps `AppState` per repository).
- **`ui/input.rs`:** Pure keyboard and mouse handlers returning `LoopAction` (`Continue`, `Quit`, `OpenExternalEditor`), decoupled from terminal I/O so entire user flows can be tested headlessly.
- **`ui/views/`:** Stateless Ratatui rendering modules (`diff`, `diff_lines`, `overlays`, `sidebar`, `footer`, `header`, `modal/{mod, thread, global_search}`) driven by explicit input structs (`DiffRenderInput` -> `DiffRenderOutput`).

## Key design invariants

- **Zero `unsafe` code:** Enforced crate-wide via `[lints.rust] unsafe_code = "deny"`.
- **Zero `process::exit` in library code:** All failure modes bubble up as `LumiereError` to `src/main.rs`.
- **UTF-8 safety everywhere:** Every text truncation, path abbreviation, search scan, selection extraction, and span split operates on char/grapheme boundaries — never raw byte slicing.
- **No external I/O on the render thread:** Rex discovery, 3-second agent status refreshes, and `A` picker scans execute on the background `AgentWorker` thread over `mpsc` channels.
