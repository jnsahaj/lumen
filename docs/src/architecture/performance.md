# Performance & Benchmarks

Lumiere is engineered to feel instantaneous on everyday commits while staying responsive on 300-file branches, 360,000-line generated files, and repositories with thousands of gitignored directories (such as `.venv/` or `node_modules/`).

## Benchmark suite

The repository includes an end-to-end PTY benchmark harness in [`bench/bench.py`](https://github.com/aduverger/lumiere/blob/main/bench/bench.py). Rather than benchmarking isolated functions, it launches the compiled release binary inside a 140×40 pseudo-terminal, feeds real terminal escape replies and keystrokes (`Alt+k`, `z`, `q`), and measures wall-clock latency, peak RSS memory, and idle CPU usage across four synthetic Git repositories:

| Repository fixture | Contents |
|---|---|
| **`small`** | 3 modified Python files (120 lines each) |
| **`many`** | 300 modified Python files across 15 directories (240 lines each) |
| **`large`** | 1 small Python file + 1 modified 360,000-line Python file |
| **`ignored`** | `small` plus 8,000 gitignored `.venv/` subdirectories (48,000 files) |

### Running benchmarks locally

```bash
cargo build --release
python3 bench/bench.py run baseline --bin target/release/lumiere --runs 5
# ... make changes and rebuild ...
python3 bench/bench.py run candidate --bin target/release/lumiere --runs 5
python3 bench/bench.py compare baseline candidate
```

## Measured results

Median of 5 runs on a 2-vCPU Linux host comparing the pre-refactor release binary (`after`) against the refactored release binary (`refactored-final`):

| Metric | Before | After | Change |
|---|---:|---:|---:|
| Start (`small` repo) | 45.6 ms | **21.7 ms** | **-52%** |
| Start (`small` repo, `--watch`) | 50.8 ms | **22.4 ms** | **-56%** |
| File change to screen (`--watch`) | 522.3 ms | **504.0 ms** | **-3%** |
| Idle CPU (`--watch`) | 0.5% | **0.0%** | **-100%** |
| Peak memory (`small` repo) | 26.4 MB | **10.2 MB** | **-61%** |
| Start (`many` repo, 300 files) | 79.5 ms | **65.7 ms** | **-17%** |
| First key answered | 199.8 ms | **6.1 ms** | **-97%** |
| Next file (`Alt+k`) | 4.7 ms | **4.4 ms** | **-6%** |
| Start (`many` repo, `--watch`) | 74.1 ms | **68.6 ms** | **-7%** |
| Start (`large` repo, 360k lines) | 231.4 ms | **195.9 ms** | **-15%** |
| Open the 360k-line file | 250.4 ms | **206.8 ms** | **-17%** |
| 360k-line file, toggle full-file view (`z`) | 237.5 ms | **184.2 ms** | **-22%** |
| Peak memory (`large` file) | 142.8 MB | **130.1 MB** | **-9%** |
| Start (`large` repo, `--watch`) | 242.5 ms | **194.7 ms** | **-20%** |
| Start (`ignored` repo, 8k `.venv` dirs) | 46.5 ms | **21.7 ms** | **-53%** |
| Start (`ignored` repo, `--watch`) | 44.5 ms | **26.0 ms** | **-42%** |

> [!NOTE]
> `file change to screen, watch` is intentionally bounded below by the 500 ms `GATHERING_TIME` debounce window in `diff::watcher` so editors that write temp files and rename them settle into a single reload.

## Key architectural optimizations

### 1. Non-blocking background `AgentWorker` (-97% first-key latency)

Previously, Rex pane discovery ran synchronously on the UI thread immediately after the first frame (and again every 3 seconds for status refreshes and whenever opening the `A` picker). Any `rex` CLI invocation stalled the event loop for ~200 ms.

Now `review::agent::AgentWorker` runs all Rex queries on a dedicated background thread over `std::sync::mpsc` channels (`Attach`, `Refresh`, `FindCandidates`). The UI thread remains immediately responsive (**6.1 ms** first-key latency) and reconciles responses atomically on wakeup.

### 2. Event-driven `dirty` redraws (0.0% idle CPU)

Rather than redrawing the entire terminal unconditionally every poll interval, `App` tracks a `dirty: bool` flag and only calls `terminal.draw(...)` when input arrives, the terminal resizes, a watcher event fires, `threads.json` changes on disk, an `AgentResponse` updates visible state, or a status notice expires. While idle in `--watch` mode, Lumiere consumes **0.0% CPU**.

### 3. Lazy per-language Tree-sitter compilation (-61% baseline RSS, -52% startup)

Both `diff::highlight::config` and `diff::context` compile Tree-sitter `HighlightConfiguration` and scope `Query` objects lazily per language via `OnceLock`, instead of compiling all 16+ grammars at startup. Opening a Python repository only compiles the Python grammar, cutting small-repo peak RSS from **26.4 MB to 10.2 MB** and startup time from **45.6 ms to 21.7 ms**.

### 4. Cached `ComputedDiff` and branch name

`ui::state::AppState` caches the active file's `ComputedDiff` (paired side-by-side rows, hunk indices, `LineStats`, and `max_old_width` / `max_new_width`) and the repository's current branch name. Per-frame rendering, footer statistics, and horizontal scroll clamping are O(1) instead of rescanning hundreds of thousands of lines or invoking VCS lookups on every draw.

### 5. Batched VCS tree reads (-17% on 300-file repos)

`VcsBackend::get_files_content_at_ref` resolves the Git revision tree once per batch rather than parsing the revision and peeling to a commit/tree for every single changed file. On a 300-file diff, this avoids 299 redundant `revparse_single` lookups.

### 6. Large-file threshold (>1 MB) and zero-alloc hot paths

- Files exceeding 1 MB skip full-file Tree-sitter parsing and scope extraction (`AppState::current_file_is_too_large_to_parse`), falling back to fast single-line highlighting of visible rows only while showing a `large file: colours are approximate` notice.
- `diff::context::extract_line` scans bytes directly to the target line in O(line length) instead of collecting every line of a 360k-line file into a `Vec<&str>`.
- `ui::views::sidebar` checks directory path prefixes without heap-allocating `format!("{dir}/")` strings on every render pass.
- `ui::input` coalesces consecutive buffered mouse wheel events before scrolling and redrawing once.

### 7. Gitignore-aware watch startup on Linux (-42% to -56% `--watch` startup)

On Linux, `diff::watcher` installs non-recursive inotify watches only on non-gitignored directories (plus `.git/` metadata for commits/checkouts) instead of walking `.venv/` or `node_modules/`. Adding 8,000 gitignored directories adds just ~4 ms to `--watch` startup.

### 8. Release build profile

`Cargo.toml` configures `[profile.release]` with `lto = "thin"`, `codegen-units = 1`, and `strip = "symbols"`, and trims unused `git2` features (`https` / `vendored-openssl`, which Lumiere never calls because remote operations go through `gh`).
