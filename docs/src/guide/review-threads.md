# Review Threads

Review threads are the heart of Lumiere. Instead of copying line numbers into a chat window and losing context, you attach threaded discussions directly to the diff and let your coding agent reply right back inside Lumiere.

## Anchoring targets

When you press <kbd>i</kbd> to comment, Lumiere picks the target automatically using a clear precedence order:

1. **Active mouse selection** — attaches to the selected line range on the selected panel (`Old` or `New`).
2. **Focused hunk** — if no lines are selected, attaches to the changed lines of the focused hunk (`New` side by default, `Old` side for pure deletions or when viewing only the Old panel via <kbd>0</kbd>).
3. **Whole file** — if neither lines nor a hunk are focused, attaches a file-level thread pinned at the top of the diff.

If an open thread already overlaps the target lines, pressing <kbd>i</kbd> continues that thread (editing your unsent draft if one exists, or starting a reply if everything was already sent). Clicking an inline thread box also opens the reply/draft editor directly.

## Writing, sending, and queueing comments

Inside the inline comment editor:

| Key | Action |
|---|---|
| <kbd>Enter</kbd> | Save the comment and **send it immediately** to the attached agent |
| <kbd>Alt+Enter</kbd> or <kbd>Ctrl+S</kbd> | Save the comment as **queued** (not sent yet) |
| <kbd>Shift+Enter</kbd> or <kbd>Ctrl+J</kbd> | Insert a newline |
| <kbd>Ctrl+U</kbd> / <kbd>Cmd+Backspace</kbd> | Delete back to start of line |
| <kbd>Esc</kbd> or <kbd>Ctrl+C</kbd> | Cancel without saving |

- Clicking outside the inline editor saves a non-empty draft as queued (or deletes an emptied existing draft).
- While a comment is queued, it is marked `· queued` in the thread box and counted in the footer (`· 3 queued`). Queued comments stay private from the agent until you send them with <kbd>s</kbd>.
- If no agent is attached, <kbd>Enter</kbd> queues the comment instead, and pressing <kbd>s</kbd> prompts to exit Lumiere and write all queued comments to `stdout` in GitHub-style Markdown (for piping or the non-Rex `pi` integration).
- Pressing <kbd>q</kbd> with queued comments still unsent prompts you to send/export them, exit keeping them queued for later, or cancel.

## Reading long threads and Markdown formatting

Inline thread boxes stay compact so they never overwhelm the diff:

- Older comments fold into `⋯ N earlier comments (t to view)`.
- Very long single comments fold trailing lines into `⋯ N more lines (t to view)`.
- Press <kbd>t</kbd> on a thread (or click any `⋯` indicator row) to open the **focused Thread View modal**:
  - Displays up to 8 lines of syntax-highlighted anchored code at the top.
  - Renders full CommonMark comments including headings, **bold**, *italic*, ~~strikethrough~~, bullet/numbered/task lists, blockquotes, links, inline code, and fenced code blocks highlighted with Tree-sitter (falling back to the reviewed file's language when a fence is untagged).
  - Opens pinned to the latest reply; scroll with <kbd>j</kbd>/<kbd>k</kbd>, arrows, <kbd>PageUp</kbd>/<kbd>PageDown</kbd>, <kbd>Home</kbd>/<kbd>End</kbd>/<kbd>G</kbd>, or the mouse wheel.
  - Press <kbd>i</kbd> inside the modal to dock the reply editor at the bottom while keeping the conversation readable above.

## Resolving and browsing threads

- Press <kbd>x</kbd> on a thread (in the diff view, the thread modal, or the threads list) to **resolve** or **reopen** it. Resolved threads collapse to a single muted line (`✓ resolved · author: ...`). Replying to a resolved thread automatically reopens it.
- Press <kbd>I</kbd> to open the **Threads list** across the current repository:
  - By default, resolved threads are hidden (`Threads (2) · 3 resolved hidden`). Press <kbd>r</kbd> to toggle showing resolved threads.
  - Press <kbd>Enter</kbd> to open the focused Thread View (dismissing returns to the list), <kbd>g</kbd> to jump straight to the thread's lines in the diff, <kbd>e</kbd> to jump and open the inline editor, <kbd>x</kbd> to resolve/reopen, or <kbd>d</kbd> to delete.

## Persistence and outdated tracking

Threads are persisted outside your Git working tree in `~/.local/state/lumiere/<repo-hash>/threads.json` (respecting `XDG_STATE_HOME` on Linux and `~/Library/Application Support/lumiere/` on macOS):

- Threads are shared across all diff ranges reviewed in the same repository and survive restarts.
- External replies written by `lumiere thread reply` are picked up automatically by the running TUI without manual refresh.
- Whenever the diff reloads, Lumiere re-anchors every line-range thread:
  - If the exact anchored snippet still matches the same line range, nothing changes.
  - If lines shifted up or down, Lumiere relocates the thread if the snippet matches at **exactly one** location on the same panel side.
  - If the snippet was edited, deleted, or matches ambiguously, the thread is marked `outdated` while preserving its conversation history.
