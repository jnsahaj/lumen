# Quick Start

Run `lumiere diff` from any directory inside a Git or Jujutsu repository (or from a parent workspace folder containing several repositories).

## Common starting commands

```bash
# Uncommitted working-tree changes
lumiere diff

# Everything added on your feature branch, including uncommitted edits
lumiere diff origin/main...-

# Reload automatically whenever you or your agent edit files
lumiere diff origin/main...- --watch

# Review a range of commits one commit at a time
lumiere diff main..feature --stacked

# Review a GitHub pull request (requires `gh`)
lumiere diff --pr 123
lumiere diff --detect-pr
```

## A 60-second review workflow

1. **Move between files and hunks.**
   - Press <kbd>Alt+Down</kbd> / <kbd>Alt+Up</kbd> (or <kbd>Opt+k</kbd> / <kbd>Opt+j</kbd> on macOS) to step between files, or <kbd>Ctrl+P</kbd> for the fuzzy file picker.
   - Press <kbd>]</kbd> and <kbd>[</kbd> to jump between hunks. Press <kbd>m</kbd> to dim a hunk as reviewed and advance to the next one.
   - Press <kbd>z</kbd> at any time to toggle between **hunks-only** view and the **full file**.

2. **Search across the diff.**
   - Press <kbd>/</kbd> to search incrementally in the current file (<kbd>n</kbd>/<kbd>N</kbd> to step through matches).
   - Press <kbd>Ctrl+F</kbd> for Telescope-style fuzzy search across every file in the diff with a live syntax-highlighted preview pane.

3. **Start a review thread.**
   - Focus a hunk with <kbd>]</kbd> / <kbd>[</kbd>, or click-and-drag with the mouse to select specific lines or characters.
   - Press <kbd>i</kbd> to open the inline comment editor.
   - Press <kbd>Enter</kbd> to save and send immediately to your attached coding agent, or <kbd>Alt+Enter</kbd> (<kbd>Ctrl+S</kbd>) to queue the comment for later batch sending.

4. **Review responses live.**
   - When an agent runs `lumiere thread reply <id> --body '...'`, Lumiere detects the file update and renders the reply live in your diff view.
   - Press <kbd>t</kbd> on any thread (or click a `⋯` truncation row) to open the full scrollable thread modal with Markdown formatting and syntax-highlighted code snippets.
   - Press <kbd>x</kbd> to resolve a thread once satisfied.

5. **Mark files viewed and finish.**
   - Press <kbd>Space</kbd> in the diff view to mark the current file as viewed and jump to the next unviewed file.
   - Press <kbd>s</kbd> to send all queued comments as a single message (or export them to stdout when no agent is attached).
   - Press <kbd>q</kbd> to quit, or <kbd>?</kbd> at any time to inspect the full keybinding sheet.
