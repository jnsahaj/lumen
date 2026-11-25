# Keybindings

Press <kbd>?</kbd> anywhere in Lumiere to open the interactive, scrollable keybinding sheet. On Linux and Windows, `Opt` corresponds to `Alt`.

## Global

| Key | Action |
|---|---|
| <kbd>q</kbd> / <kbd>Ctrl+C</kbd> | Quit (prompts if queued comments remain) |
| <kbd>Tab</kbd> | Toggle sidebar visibility |
| <kbd>1</kbd> / <kbd>2</kbd> | Focus sidebar / diff view |
| <kbd>Opt+↓</kbd> / <kbd>Opt+K</kbd> / <kbd>Ctrl+K</kbd> | Next file |
| <kbd>Opt+↑</kbd> / <kbd>Opt+J</kbd> / <kbd>Ctrl+J</kbd> | Previous file |
| <kbd>Ctrl+D</kbd> / <kbd>Ctrl+U</kbd> | Scroll half page down / up |
| <kbd>Ctrl+P</kbd> | Open fuzzy file picker |
| <kbd>r</kbd> | Refresh diff / reload PR |
| <kbd>R</kbd> | Switch repository (in a workspace folder) |
| <kbd>A</kbd> | Choose coding agent pane (or detach) |
| <kbd>y</kbd> | Copy active text selection, or current filename |
| <kbd>e</kbd> | Open current file in `$EDITOR` (at focused hunk line) |
| <kbd>o</kbd> | Open current file on GitHub (PR mode) |
| <kbd>Ctrl+L</kbd> / <kbd>Ctrl+H</kbd> | Next / previous commit (`--stacked` mode) |
| <kbd>?</kbd> | Show keybindings modal |

## Sidebar

| Key | Action |
|---|---|
| <kbd>j</kbd> / <kbd>↑</kbd> | Move up |
| <kbd>k</kbd> / <kbd>↓</kbd> | Move down |
| <kbd>h</kbd> / <kbd>l</kbd> or <kbd>←</kbd> / <kbd>→</kbd> | Scroll horizontally |
| <kbd>Enter</kbd> | Open file in diff view, or collapse/expand directory |
| <kbd>Space</kbd> | Toggle viewed status for file or entire directory |

## Diff view

| Key | Action |
|---|---|
| <kbd>j</kbd> / <kbd>↑</kbd> | Scroll up (or previous search match with arrows when search is active) |
| <kbd>k</kbd> / <kbd>↓</kbd> | Scroll down (or next search match with arrows when search is active) |
| <kbd>h</kbd> / <kbd>l</kbd> or <kbd>←</kbd> / <kbd>→</kbd> | Scroll horizontally (when `--wrap` is off) |
| <kbd>PageUp</kbd> / <kbd>PageDown</kbd> | Scroll 20 lines up / down |
| <kbd>gg</kbd> / <kbd>G</kbd> | Jump to top / bottom of file |
| <kbd>[</kbd> / <kbd>]</kbd> | Focus previous / next hunk |
| <kbd>m</kbd> | Toggle focused hunk viewed and advance to next hunk |
| <kbd>Space</kbd> | Toggle current file viewed and jump to next unviewed file |
| <kbd>z</kbd> | Toggle hunks-only / full-file view |
| <kbd>w</kbd> | Toggle file watch mode |
| <kbd>0</kbd> | Toggle fullscreen Old panel |
| <kbd>-</kbd> | Toggle fullscreen New panel |
| <kbd>=</kbd> | Reset to side-by-side panels |
| <kbd>Esc</kbd> | Clear active selection, or clear active search highlight |

## Search

| Key | Action |
|---|---|
| <kbd>/</kbd> | Start in-file incremental search (<kbd>Opt+Backspace</kbd> / <kbd>Ctrl+W</kbd> erases word) |
| <kbd>Ctrl+F</kbd> | Open global fuzzy search across all files with live preview |
| <kbd>n</kbd> / <kbd>N</kbd> | Jump to next / previous in-file match |
| <kbd>Esc</kbd> / <kbd>Ctrl+C</kbd> | Cancel search |

## Review threads

| Key | Action |
|---|---|
| <kbd>i</kbd> | Comment on selection / hunk / file, or reply / edit unsent draft |
| <kbd>Enter</kbd> *(in editor)* | Save comment and send immediately to the attached agent |
| <kbd>Alt+Enter</kbd> / <kbd>Ctrl+S</kbd> *(in editor)* | Save comment as queued |
| <kbd>Shift+Enter</kbd> / <kbd>Ctrl+J</kbd> *(in editor)* | Insert newline |
| <kbd>t</kbd> | Open thread at cursor in focused scrollable Thread View modal |
| <kbd>x</kbd> | Resolve or reopen thread at cursor |
| <kbd>I</kbd> | Open Threads list (<kbd>Enter</kbd> open, <kbd>g</kbd> jump, <kbd>e</kbd> edit/reply, <kbd>x</kbd> resolve, <kbd>r</kbd> show/hide resolved, <kbd>d</kbd> delete) |
| <kbd>s</kbd> | Send all queued comments (or export to stdout when no agent is attached) |
