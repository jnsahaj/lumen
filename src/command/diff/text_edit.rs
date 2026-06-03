//! Text-editing helpers shared across every single-line input in the diff UI:
//! the global-search query, the inline `/` search, the file-picker query, and
//! the annotation export filename. The multi-line annotation editor already
//! gets the same behavior from `tui-textarea::delete_word` — this module
//! mirrors its semantics so the entire app handles `opt+backspace` and `^W`
//! the same way.

/// 3-class character kind used to find word boundaries. Matches the
/// `tui-textarea` classifier so single-line and multi-line inputs agree.
#[derive(PartialEq, Eq, Clone, Copy)]
enum CharKind {
    Space,
    Punct,
    Other,
}

impl CharKind {
    fn classify(c: char) -> Self {
        if c.is_whitespace() {
            Self::Space
        } else if c.is_ascii_punctuation() {
            Self::Punct
        } else {
            Self::Other
        }
    }
}

/// Erase the trailing "word" from `s`, macOS `opt+backspace` semantics.
///
/// Word boundaries sit at transitions between three character classes:
/// whitespace, ASCII punctuation, and "other" (letters, digits, underscore,
/// non-ASCII). A trailing whitespace run is absorbed into the word being
/// deleted instead of being deleted alone.
///
/// Examples (each line shows one `opt+backspace`):
/// ```text
/// "something/another" → "something/"
/// "something/"        → "something"
/// "abc.def"           → "abc."
/// "abc..def"          → "abc.."
/// "foo bar"           → "foo "
/// "foo "              → ""
/// "  "                → ""
/// ""                  → ""
/// ```
pub fn erase_word_backward(s: &mut String) {
    let mut iter = s.char_indices().rev();
    let Some((_, last)) = iter.next() else {
        return;
    };
    let mut cur = CharKind::classify(last);
    for (byte_idx, c) in iter {
        let next = CharKind::classify(c);
        // Boundary: the run we're currently inside (`cur`) is non-whitespace
        // and the next char back is a different class. Truncate so everything
        // from `cur`'s run is gone but `c` is kept.
        if cur != CharKind::Space && next != cur {
            s.truncate(byte_idx + c.len_utf8());
            return;
        }
        cur = next;
    }
    // No boundary found — the whole remaining buffer is one run (possibly
    // preceded by trailing whitespace that got absorbed). Wipe it.
    s.clear();
}

#[cfg(test)]
mod tests {
    use super::erase_word_backward;

    fn erase(s: &str) -> String {
        let mut s = s.to_string();
        erase_word_backward(&mut s);
        s
    }

    #[test]
    fn respects_punctuation_boundary() {
        assert_eq!(erase("something/another"), "something/");
        assert_eq!(erase("abc.def"), "abc.");
        assert_eq!(erase("foo-bar-baz"), "foo-bar-");
    }

    #[test]
    fn deletes_punctuation_run_when_at_end() {
        assert_eq!(erase("something/"), "something");
        assert_eq!(erase("abc.."), "abc");
        assert_eq!(erase("a---"), "a");
    }

    #[test]
    fn absorbs_trailing_whitespace_into_word() {
        assert_eq!(erase("foo bar"), "foo ");
        assert_eq!(erase("foo "), "");
        assert_eq!(erase("foo   "), "");
    }

    #[test]
    fn handles_edges() {
        assert_eq!(erase(""), "");
        assert_eq!(erase("a"), "");
        assert_eq!(erase(" "), "");
        assert_eq!(erase("abc"), "");
    }

    #[test]
    fn handles_unicode() {
        // Multi-byte chars classified as Other; boundary at the punctuation.
        assert_eq!(erase("naïve/résumé"), "naïve/");
        assert_eq!(erase("café"), "");
    }
}
