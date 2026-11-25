# Threads, Storage & Agent Protocol

Lumiere's review loop is designed so the interactive TUI (`lumiere diff`) and external agent commands (`lumiere thread ...`) can safely read and mutate the same conversation concurrently.

## Data model (`src/review/thread.rs`)

A repository's review state is a list of `Thread` records:

- **`id`:** 8-hex-character random identifier (e.g. `1a2b3c4d`).
- **`filename`:** Repository-relative path.
- **`target`:** Either `AnnotationTarget::File` or `AnnotationTarget::LineRange { panel: Old | New, start_line, end_line }` (1-indexed inclusive).
- **`anchor_text`:** Exact text of the anchored lines captured when the thread was created, used for relocating the thread across edits and showing context to agents.
- **`outdated` / `resolved`:** Lifecycle flags.
- **`comments`:** Ordered list of `Comment { id, author: Human | Agent, author_name, body, created_at, queued }`.

### Visibility and turn-taking semantics

- **Queued drafts (`queued: true`):** Newly created reviewer comments start queued until sent via `Enter` or `s`. All `lumiere thread` subcommands ignore queued comments, and a thread containing only queued comments is invisible to agents.
- **Awaiting agent (`thread.awaiting_agent()`):** True when a thread is open (`!resolved`) and its most recent non-queued comment was written by `Author::Human`.
- **Outdated anchor refresh (`refresh_anchors`):** On every diff reload:
  - File-level threads are outdated only if the file left the diff.
  - Line-range threads check if the lines at `start_line..=end_line` still match `anchor_text`. If not, Lumiere searches the file on the same panel side for `anchor_text`: a **unique** match updates `start_line..=end_line` seamlessly; zero or multiple matches flag `outdated = true`.

## Concurrent persistence (`src/review/store.rs`)

Threads are stored outside the repository working tree so Git status stays completely clean:

```text
<state_dir>/lumiere/<sha256(canonical_repo_root)[..16]>/threads.json
```

Where `<state_dir>` is `$XDG_STATE_HOME` (default `~/.local/state`) on Linux and `~/Library/Application Support` on macOS.

### Schema (v1)

```json
{
  "version": 1,
  "threads": [
    {
      "id": "1a2b3c4d",
      "filename": "src/ui/mod.rs",
      "target": {
        "kind": "line_range",
        "panel": "new",
        "start_line": 42,
        "end_line": 48
      },
      "anchor_text": "...",
      "created_at": 1764000000,
      "resolved": false,
      "outdated": false,
      "comments": [
        {
          "id": "9f8e7d6c",
          "author": "human",
          "author_name": "you",
          "body": "Should this restore flags on panic?",
          "created_at": 1764000000,
          "queued": false
        }
      ]
    }
  ]
}
```

### Lock-and-rename concurrency protocol

Every write goes through `ThreadStore::update`:

1. Acquire an exclusive OS advisory lock (`File::lock`, Rust 1.89+) on a sibling `threads.lock` file.
2. Re-read the latest `threads.json` from disk inside the lock (so concurrent replies written by an agent seconds earlier are never clobbered).
3. Reject files whose `version != 1` without overwriting them (forward-compatibility safety).
4. Apply the mutation closure.
5. Write serialized JSON to a unique sibling temp file (`threads.<pid>.<rand>.tmp`) and atomically `fs::rename` it over `threads.json`.

### Live TUI pickup without filesystem watches

On every loop tick, `AppState::reload_threads_if_changed()` compares `StoreStamp` (`(len, modified_time)` from a single `stat()` call) against the last loaded stamp. Whenever an agent runs `lumiere thread reply`, the stamp changes, Lumiere reloads `threads.json`, marks the frame `dirty`, and renders the reply live — even when `--watch` is off.

## Rex agent protocol (`src/review/agent.rs`)

### Non-blocking `AgentWorker`

Shelling out to `rex` takes tens to hundreds of milliseconds. To keep startup and typing completely stall-free, `AgentWorker::spawn` runs a dedicated background thread communicating over `std::sync::mpsc` channels:

- **`AgentRequest::Attach`** — dispatched immediately on startup before the first frame; updates the footer as soon as Rex responds.
- **`AgentRequest::Refresh`** — dispatched every 3 seconds while an agent pane is attached (never overlapping an in-flight refresh; stale responses for a previously attached pane are ignored by `(session, block)` comparison).
- **`AgentRequest::FindCandidates`** — dispatched when the user presses <kbd>A</kbd>, opening the picker immediately with cached candidates and refreshing it live when discovery finishes.

### Discovery & delivery

- Discovery executes a single Lua script via `rex do -s <session> --json` to read each terminal block's `process` (foreground PID and CWD) and `program_status` (OSC 7501 records).
- Only panes where a recognized coding agent's status `owner.pid == foreground.pid` and whose CWD is inside one of the `review_roots` qualify.
- Delivery (`send_comments`) performs a live pane refresh first, aborts if the agent exited or its state is `"blocked"`, strips non-newline control characters, wraps the prompt in bracketed paste (`\x1b[200~...\x1b[201~`), and submits with `rex send-key ... enter`.
