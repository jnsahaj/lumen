use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyModifiers};

/// Actions that can be rebound by the user via `lumen.config.json`.
///
/// Only a curated subset of the TUI key handlers are exposed here.
/// Everything else (search mode, annotation editor, modals, etc.)
/// remains hardcoded in the main event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyAction {
    FocusSidebar,
    FocusDiff,
    ToggleSidebar,
    HScrollLeft,
    HScrollRight,
    NextFile,
    PrevFile,
    HalfPageDown,
    HalfPageUp,
    NextHunk,
    PrevHunk,
    Quit,
}

/// Maps physical key combinations to logical `KeyAction`s.
///
/// Built once at startup from default bindings merged with user overrides.
/// The `resolve` method is called on every key press (before the fallback
/// match block) and returns `Some(action)` when a rebindable action matches.
pub struct KeyBindings {
    map: HashMap<(KeyCode, KeyModifiers), KeyAction>,
}

impl KeyBindings {
    /// Construct the binding map by applying user overrides on top of defaults.
    ///
    /// # Arguments
    ///
    /// * `overrides` - Optional map from action name (e.g. `"focus_sidebar"`) to
    ///   key string (e.g. `"Left"`, `"Shift+Right"`, `"Ctrl+j"`).
    pub fn new(overrides: Option<&HashMap<String, String>>) -> Self {
        let mut map: HashMap<(KeyCode, KeyModifiers), KeyAction> = HashMap::new();

        // Defaults: each action can be bound to multiple keys.
        let defaults: Vec<(KeyAction, Vec<(KeyCode, KeyModifiers)>)> = vec![
            (
                KeyAction::FocusSidebar,
                vec![(KeyCode::Char('1'), KeyModifiers::NONE)],
            ),
            (
                KeyAction::FocusDiff,
                vec![(KeyCode::Char('2'), KeyModifiers::NONE)],
            ),
            (
                KeyAction::ToggleSidebar,
                vec![(KeyCode::Tab, KeyModifiers::NONE)],
            ),
            (
                KeyAction::HScrollLeft,
                vec![
                    (KeyCode::Char('h'), KeyModifiers::NONE),
                    (KeyCode::Left, KeyModifiers::NONE),
                ],
            ),
            (
                KeyAction::HScrollRight,
                vec![
                    (KeyCode::Char('l'), KeyModifiers::NONE),
                    (KeyCode::Right, KeyModifiers::NONE),
                ],
            ),
            (
                KeyAction::NextFile,
                vec![(KeyCode::Char('j'), KeyModifiers::CONTROL)],
            ),
            (
                KeyAction::PrevFile,
                vec![(KeyCode::Char('k'), KeyModifiers::CONTROL)],
            ),
            (
                KeyAction::HalfPageDown,
                vec![(KeyCode::Char('d'), KeyModifiers::CONTROL)],
            ),
            (
                KeyAction::HalfPageUp,
                vec![(KeyCode::Char('u'), KeyModifiers::CONTROL)],
            ),
            (
                KeyAction::NextHunk,
                vec![(KeyCode::Char('}'), KeyModifiers::NONE)],
            ),
            (
                KeyAction::PrevHunk,
                vec![(KeyCode::Char('{'), KeyModifiers::NONE)],
            ),
            (
                KeyAction::Quit,
                vec![
                    (KeyCode::Char('q'), KeyModifiers::NONE),
                    (KeyCode::Esc, KeyModifiers::NONE),
                ],
            ),
        ];

        // Collect overridden actions so we can skip their defaults.
        let overridden_actions: HashMap<KeyAction, Vec<(KeyCode, KeyModifiers)>> =
            if let Some(user_map) = overrides {
                let mut result: HashMap<KeyAction, Vec<(KeyCode, KeyModifiers)>> = HashMap::new();
                for (action_str, key_str) in user_map {
                    if let Some(action) = parse_action_name(action_str) {
                        if let Some(combo) = parse_key_string(key_str) {
                            result.entry(action).or_default().push(combo);
                        }
                    }
                }
                result
            } else {
                HashMap::new()
            };

        // Insert defaults, skipping any action that has user overrides.
        for (action, combos) in &defaults {
            if !overridden_actions.contains_key(action) {
                for &combo in combos {
                    map.insert(combo, *action);
                }
            }
        }

        // Insert user overrides (possibly replacing default combos from other actions).
        for (action, combos) in &overridden_actions {
            for &combo in combos {
                map.insert(combo, *action);
            }
        }

        Self { map }
    }

    /// Look up the action bound to the given key code and modifiers.
    ///
    /// Returns `None` if no rebindable action is mapped to this combination,
    /// meaning the caller should fall through to the hardcoded match block.
    pub fn resolve(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<KeyAction> {
        self.map.get(&(code, modifiers)).copied()
    }
}

/// Parse a user-facing action name string into the corresponding enum variant.
///
/// Returns `None` for unrecognized names (silently ignored).
fn parse_action_name(name: &str) -> Option<KeyAction> {
    match name {
        "focus_sidebar" => Some(KeyAction::FocusSidebar),
        "focus_diff" => Some(KeyAction::FocusDiff),
        "toggle_sidebar" => Some(KeyAction::ToggleSidebar),
        "h_scroll_left" => Some(KeyAction::HScrollLeft),
        "h_scroll_right" => Some(KeyAction::HScrollRight),
        "next_file" => Some(KeyAction::NextFile),
        "prev_file" => Some(KeyAction::PrevFile),
        "half_page_down" => Some(KeyAction::HalfPageDown),
        "half_page_up" => Some(KeyAction::HalfPageUp),
        "next_hunk" => Some(KeyAction::NextHunk),
        "prev_hunk" => Some(KeyAction::PrevHunk),
        "quit" => Some(KeyAction::Quit),
        _ => None,
    }
}

/// Parse a key string like `"Ctrl+j"`, `"Shift+Left"`, `"Tab"`, `"q"`, `"}"` into
/// a `(KeyCode, KeyModifiers)` tuple.
///
/// Supported modifier prefixes (case-insensitive, combinable with `+`):
/// - `Ctrl`
/// - `Shift`
/// - `Alt`
///
/// Supported key names (case-insensitive for named keys):
/// - `Left`, `Right`, `Up`, `Down`
/// - `Tab`, `Enter`, `Esc`, `Space`
/// - `PageUp`, `PageDown`, `Home`, `End`
/// - `Backspace`, `Delete`
/// - Single characters: `a`-`z`, `0`-`9`, punctuation
///
/// # Returns
///
/// `None` if the string cannot be parsed.
fn parse_key_string(s: &str) -> Option<(KeyCode, KeyModifiers)> {
    let parts: Vec<&str> = s.split('+').collect();
    let mut modifiers = KeyModifiers::NONE;

    // All parts except the last are modifiers.
    for &part in &parts[..parts.len() - 1] {
        match part.to_lowercase().as_str() {
            "ctrl" => modifiers |= KeyModifiers::CONTROL,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            "alt" => modifiers |= KeyModifiers::ALT,
            _ => return None,
        }
    }

    let key_part = parts.last()?;
    let code = parse_key_code(key_part)?;

    Some((code, modifiers))
}

/// Parse the key name portion (after modifiers) into a `KeyCode`.
fn parse_key_code(s: &str) -> Option<KeyCode> {
    // Named keys (case-insensitive).
    match s.to_lowercase().as_str() {
        "left" => return Some(KeyCode::Left),
        "right" => return Some(KeyCode::Right),
        "up" => return Some(KeyCode::Up),
        "down" => return Some(KeyCode::Down),
        "tab" => return Some(KeyCode::Tab),
        "enter" => return Some(KeyCode::Enter),
        "esc" | "escape" => return Some(KeyCode::Esc),
        "space" => return Some(KeyCode::Char(' ')),
        "pageup" => return Some(KeyCode::PageUp),
        "pagedown" => return Some(KeyCode::PageDown),
        "home" => return Some(KeyCode::Home),
        "end" => return Some(KeyCode::End),
        "backspace" => return Some(KeyCode::Backspace),
        "delete" => return Some(KeyCode::Delete),
        _ => {}
    }

    // Single character (preserves case for char matching).
    let chars: Vec<char> = s.chars().collect();
    if chars.len() == 1 {
        return Some(KeyCode::Char(chars[0]));
    }

    // Function keys: F1-F12.
    if s.len() >= 2 && (s.starts_with('F') || s.starts_with('f')) {
        if let Ok(n) = s[1..].parse::<u8>() {
            if (1..=12).contains(&n) {
                return Some(KeyCode::F(n));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_string_simple_char() {
        assert_eq!(
            parse_key_string("q"),
            Some((KeyCode::Char('q'), KeyModifiers::NONE))
        );
    }

    #[test]
    fn test_parse_key_string_ctrl_modifier() {
        assert_eq!(
            parse_key_string("Ctrl+j"),
            Some((KeyCode::Char('j'), KeyModifiers::CONTROL))
        );
    }

    #[test]
    fn test_parse_key_string_shift_modifier() {
        assert_eq!(
            parse_key_string("Shift+Left"),
            Some((KeyCode::Left, KeyModifiers::SHIFT))
        );
    }

    #[test]
    fn test_parse_key_string_named_key() {
        assert_eq!(
            parse_key_string("Tab"),
            Some((KeyCode::Tab, KeyModifiers::NONE))
        );
        assert_eq!(
            parse_key_string("Esc"),
            Some((KeyCode::Esc, KeyModifiers::NONE))
        );
        assert_eq!(
            parse_key_string("Left"),
            Some((KeyCode::Left, KeyModifiers::NONE))
        );
    }

    #[test]
    fn test_parse_key_string_punctuation() {
        assert_eq!(
            parse_key_string("}"),
            Some((KeyCode::Char('}'), KeyModifiers::NONE))
        );
        assert_eq!(
            parse_key_string("{"),
            Some((KeyCode::Char('{'), KeyModifiers::NONE))
        );
    }

    #[test]
    fn test_parse_key_string_multiple_modifiers() {
        assert_eq!(
            parse_key_string("Ctrl+Shift+Left"),
            Some((KeyCode::Left, KeyModifiers::CONTROL | KeyModifiers::SHIFT))
        );
    }

    #[test]
    fn test_parse_action_name_valid() {
        assert_eq!(parse_action_name("focus_sidebar"), Some(KeyAction::FocusSidebar));
        assert_eq!(parse_action_name("quit"), Some(KeyAction::Quit));
    }

    #[test]
    fn test_parse_action_name_invalid() {
        assert_eq!(parse_action_name("nonexistent"), None);
    }

    #[test]
    fn test_keybindings_defaults() {
        let kb = KeyBindings::new(None);
        assert_eq!(
            kb.resolve(KeyCode::Char('1'), KeyModifiers::NONE),
            Some(KeyAction::FocusSidebar)
        );
        assert_eq!(
            kb.resolve(KeyCode::Char('q'), KeyModifiers::NONE),
            Some(KeyAction::Quit)
        );
        assert_eq!(
            kb.resolve(KeyCode::Esc, KeyModifiers::NONE),
            Some(KeyAction::Quit)
        );
    }

    #[test]
    fn test_keybindings_override_removes_old_default() {
        // Override focus_sidebar from '1' to 'Left'
        let mut overrides = HashMap::new();
        overrides.insert("focus_sidebar".to_string(), "Left".to_string());
        let kb = KeyBindings::new(Some(&overrides));

        // New binding works
        assert_eq!(
            kb.resolve(KeyCode::Left, KeyModifiers::NONE),
            Some(KeyAction::FocusSidebar)
        );
        // Old default '1' is no longer bound to FocusSidebar
        assert_eq!(kb.resolve(KeyCode::Char('1'), KeyModifiers::NONE), None);
    }

    #[test]
    fn test_keybindings_override_replaces_conflicting_default() {
        // Override focus_sidebar to 'Left', which was previously h_scroll_left
        let mut overrides = HashMap::new();
        overrides.insert("focus_sidebar".to_string(), "Left".to_string());
        let kb = KeyBindings::new(Some(&overrides));

        // Left is now focus_sidebar, not h_scroll_left
        assert_eq!(
            kb.resolve(KeyCode::Left, KeyModifiers::NONE),
            Some(KeyAction::FocusSidebar)
        );
        // 'h' still maps to h_scroll_left (unaffected)
        assert_eq!(
            kb.resolve(KeyCode::Char('h'), KeyModifiers::NONE),
            Some(KeyAction::HScrollLeft)
        );
    }

    #[test]
    fn test_parse_key_string_space() {
        assert_eq!(
            parse_key_string("Space"),
            Some((KeyCode::Char(' '), KeyModifiers::NONE))
        );
    }

    #[test]
    fn test_parse_key_string_digit() {
        assert_eq!(
            parse_key_string("1"),
            Some((KeyCode::Char('1'), KeyModifiers::NONE))
        );
    }
}
