use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Match commands against the PC-101 key position when the terminal reports it.
/// Keep the original event for text fields, where the active layout matters.
pub(super) fn shortcut_key(mut key: KeyEvent) -> KeyEvent {
    let Some(KeyCode::Char(base)) = key.base_layout_code else {
        return key;
    };

    let modifiers = key.base_layout_modifiers.unwrap_or(key.modifiers);
    key.code = KeyCode::Char(if modifiers.contains(KeyModifiers::SHIFT) {
        shifted_us_key(base).unwrap_or(base)
    } else {
        base
    });
    key.modifiers = modifiers;
    if modifiers.contains(KeyModifiers::SHIFT) && shifted_us_key(base).is_some() {
        key.modifiers.remove(KeyModifiers::SHIFT);
    }
    key
}

fn shifted_us_key(base: char) -> Option<char> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_key_works_across_layouts_and_preserves_shift() {
        let cyrillic_l = KeyEvent::new(KeyCode::Char('д'), KeyModifiers::CONTROL)
            .with_base_layout_code(Some(KeyCode::Char('l')))
            .with_base_layout_modifiers(Some(KeyModifiers::CONTROL));
        let shortcut = shortcut_key(cyrillic_l);
        assert_eq!(shortcut.code, KeyCode::Char('l'));
        assert_eq!(shortcut.modifiers, KeyModifiers::CONTROL);
        assert_eq!(cyrillic_l.code, KeyCode::Char('д'));

        let shifted = KeyEvent::new(KeyCode::Char('Д'), KeyModifiers::CONTROL)
            .with_base_layout_code(Some(KeyCode::Char('l')))
            .with_base_layout_modifiers(Some(KeyModifiers::CONTROL | KeyModifiers::SHIFT));
        assert_eq!(shortcut_key(shifted).code, KeyCode::Char('L'));

        let question = KeyEvent::new(KeyCode::Char(','), KeyModifiers::NONE)
            .with_base_layout_code(Some(KeyCode::Char('/')))
            .with_base_layout_modifiers(Some(KeyModifiers::SHIFT));
        assert_eq!(shortcut_key(question).code, KeyCode::Char('?'));

        // A Latin layout can also put another character on the same position.
        let azerty_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)
            .with_base_layout_code(Some(KeyCode::Char('q')))
            .with_base_layout_modifiers(Some(KeyModifiers::NONE));
        assert_eq!(shortcut_key(azerty_a).code, KeyCode::Char('q'));
    }

    #[test]
    fn legacy_events_keep_their_reported_code() {
        let key = KeyEvent::new(KeyCode::Char('д'), KeyModifiers::CONTROL);
        assert_eq!(shortcut_key(key), key);
    }
}
