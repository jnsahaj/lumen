# Navigating Diffs

Lumiere presents a collapsible file tree on the left (`[1] Files`) and a side-by-side diff canvas (`[2] Old` / `New`) on the right, backed by a status footer.

## Hunks-only vs. full-file view

By default, Lumiere opens in **hunks-only** mode: each changed hunk is displayed with 3 lines of surrounding context, and untouched sections collapse into compact separator rows (`⋯ 63 unchanged lines`).

- Press <kbd>z</kbd> to toggle between **hunks-only** and **full-file** mode at any time. Lumiere anchors the focused hunk (or top visible line) in place so your eye never loses its spot.
- Lines anchored by a [review thread](review-threads.md) are **never folded away**, even when far from any changed hunk.
- Added files and deleted files automatically render in a clean single-panel layout (`[2] New File` / `[2] Deleted File`) in both modes.
- Set `"view": "full"` in `lumiere.config.json` or pass `--view full` if you prefer whole files by default.

## Panel layout and line wrapping

- <kbd>Tab</kbd> toggles the file tree sidebar on and off.
- <kbd>1</kbd> focuses the sidebar; <kbd>2</kbd> focuses the diff view.
- <kbd>0</kbd> toggles fullscreen **Old** panel; <kbd>-</kbd> toggles fullscreen **New** panel; <kbd>=</kbd> returns to side-by-side.
- Pass `--wrap` (or `"wrap": true` in config) to soft-wrap long lines with preserved leading indentation rather than scrolling horizontally (<kbd>h</kbd> / <kbd>l</kbd>).

## Moving through files and hunks

- **Jump between files:** <kbd>Alt+Down</kbd> / <kbd>Alt+Up</kbd>, <kbd>Alt+K</kbd> / <kbd>Alt+J</kbd>, or <kbd>Ctrl+K</kbd> / <kbd>Ctrl+J</kbd>.
- **Fuzzy file picker:** Press <kbd>Ctrl+P</kbd> to filter changed files by path.
- **Step between hunks:** Press <kbd>]</kbd> (next hunk) and <kbd>[</kbd> (previous hunk). A focused vertical bar (`▎`) highlights the active hunk.
- **Mark progress:**
  - Press <kbd>m</kbd> in the diff view to mark the focused hunk as viewed (dimming its diff tint while keeping its gutter colored) and automatically advance to the next hunk.
  - Press <kbd>Space</kbd> in the diff view to mark the current file viewed (`✓`) and jump directly to the next unviewed file.
  - In the sidebar, <kbd>Space</kbd> toggles viewed status on a file or recursively on an entire directory; <kbd>Enter</kbd> opens a file or collapses/expands a directory.

## Sticky scope context

When scrolling inside a function, class, impl block, or method in supported languages (Rust, Python, TypeScript, JavaScript, Go, C, C++, Java), Lumiere pins enclosing scope headers at the top of each panel (`12 ~ fn render_diff(...)`) using Tree-sitter AST queries.

> **Large files (> 1 MB):** Files over 1 MB skip whole-file Tree-sitter parsing and sticky context headers to keep navigation instantaneous. Lines are still syntax-highlighted on demand line by line, and the footer displays `large file: colours are approximate`.

## In-file search (`/`) and global fuzzy search (`Ctrl+F`)

| Mode | Trigger | Scope | Navigation |
|---|---|---|---|
| **In-file search** | <kbd>/</kbd> | Lines currently visible in the active file | <kbd>Enter</kbd> confirms, <kbd>n</kbd>/<kbd>N</kbd> or <kbd>↓</kbd>/<kbd>↑</kbd> step matches, <kbd>Esc</kbd> clears |
| **Global fuzzy search** | <kbd>Ctrl+F</kbd> | All lines across every changed file | Telescope-style 50/50 split with fuzzy matching (`nucleo-matcher`) and syntax-highlighted unified preview |

In **Global fuzzy search**:
- Type to fuzzy-filter across `filename:line  content`.
- Move with <kbd>↓</kbd>/<kbd>↑</kbd>, <kbd>Ctrl+N</kbd>/<kbd>Ctrl+P</kbd>, <kbd>PageUp</kbd>/<kbd>PageDown</kbd>, or mouse click/wheel. Hold <kbd>Shift</kbd> + scroll wheel to pan either pane horizontally.
- Press <kbd>Enter</kbd> to jump straight to that file and line. If the target line was folded in hunks-only view, Lumiere automatically switches to full-file view so the match lands cleanly on screen.

## Mouse selection, copying, and external editor

- **Character selection:** Click and drag anywhere in the code area of either panel.
- **Line selection:** Click and drag inside the line-number gutter.
- **Actions on selection:** Release the mouse to see the inline action bar (`i annotate  y copy  esc`). Press <kbd>y</kbd> to copy the selected text (or the current filename when nothing is selected) to the system clipboard.
- **Open in `$EDITOR`:** Press <kbd>e</kbd> to open the current file in `$EDITOR` (default `vim`), jumping to the focused hunk's line number (`+<line>`). `$EDITOR` supports shell-style arguments such as `code --wait`.
