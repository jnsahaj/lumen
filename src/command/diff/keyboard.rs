use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Return the character actually typed in the active layout, without control shortcuts.
pub(super) fn typed_char(key: KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
        {
            Some(c)
        }
        _ => None,
    }
}

/// Convert the protocol's physical PC-101 key to Lumen's shifted shortcut glyph.
/// The active keyboard layout is never used for this mapping.
pub(super) fn shifted_pc101_key(base: char) -> Option<char> {
    if base.is_ascii_lowercase() {
        return Some(base.to_ascii_uppercase());
    }
    Some(match base {
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '`' => '~',
        _ => return None,
    })
}
