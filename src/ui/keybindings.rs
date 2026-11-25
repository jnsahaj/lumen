#[derive(Clone)]
pub struct KeyBind {
    pub key: &'static str,
    pub description: &'static str,
}

#[derive(Clone)]
pub struct KeyBindSection {
    pub title: &'static str,
    pub bindings: Vec<KeyBind>,
}

const fn bind(key: &'static str, description: &'static str) -> KeyBind {
    KeyBind { key, description }
}

pub fn help_sections() -> Vec<KeyBindSection> {
    vec![
        KeyBindSection {
            title: "Global",
            bindings: vec![
                bind("q", "Quit"),
                bind("tab", "Toggle sidebar"),
                bind("R", "Switch repository (workspace folder)"),
                bind("A", "Choose the agent that answers comments"),
                bind("1 / 2", "Focus sidebar / diff"),
                bind("opt+down/k, opt+up/j", "Next / previous file"),
                bind("ctrl+d / ctrl+u", "Scroll half page down / up"),
                bind("ctrl+p", "Open file picker"),
                bind("r", "Refresh diff / PR"),
                bind("y", "Copy current filename"),
                bind("e", "Edit file (at hunk line if focused)"),
                bind("o", "Open file in browser (PR mode)"),
                bind("ctrl+l / ctrl+h", "Next / prev commit (stacked)"),
                bind("?", "Show keybindings"),
            ],
        },
        KeyBindSection {
            title: "Sidebar",
            bindings: vec![
                bind("j/k or up/down", "Navigate files"),
                bind("h/l or left/right", "Scroll horizontally"),
                bind("enter", "Open file in diff view / toggle directory"),
                bind("space", "Toggle file as viewed"),
            ],
        },
        KeyBindSection {
            title: "Diff View",
            bindings: vec![
                bind("j/k or up/down", "Scroll vertically"),
                bind("h/l or left/right", "Scroll horizontally"),
                bind("w", "Toggle watch mode"),
                bind("z", "Hunks only / full file"),
                bind("gg / G", "Scroll to top / bottom"),
                bind("[ / ]", "Focus prev / next hunk"),
                bind("pageup / pagedown", "Scroll by page"),
                bind("space", "Mark viewed & next file"),
                bind("m", "Mark hunk viewed & next hunk"),
                bind("0", "Toggle old panel"),
                bind("-", "Toggle new panel"),
                bind("=", "Reset fullscreen to side-by-side"),
            ],
        },
        KeyBindSection {
            title: "Search",
            bindings: vec![
                bind("/", "In-file incremental search"),
                bind("ctrl+f", "Global fuzzy search (all files, with preview)"),
                bind("n or down", "Next match (in-file)"),
                bind("N or up", "Previous match (in-file)"),
                bind("ctrl+c or esc", "Cancel search"),
            ],
        },
        KeyBindSection {
            title: "Threads",
            bindings: vec![
                bind("y", "Copy selection (or filename)"),
                bind("i", "Comment on selection / hunk / file, or reply"),
                bind("enter", "In a comment: save and send to the agent"),
                bind("alt+enter / ctrl+s", "In a comment: save and queue"),
                bind("shift+enter / ctrl+j", "In a comment: new line"),
                bind("t", "Open thread in focused view"),
                bind("x", "Resolve / reopen the thread here"),
                bind(
                    "I",
                    "List threads (enter opens, g jumps, x resolves, r shows resolved)",
                ),
                bind("s", "Send queued comments (exits if no agent)"),
            ],
        },
    ]
}
