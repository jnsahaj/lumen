pub mod global;

use crate::diff::{DiffFullscreen, DiffLine};

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

/// Erases the trailing word using `tui-textarea` / macOS `Opt+Backspace` word boundaries.
pub fn erase_word_backward(s: &mut String) {
    let mut iter = s.char_indices().rev();
    let Some((_, last)) = iter.next() else {
        return;
    };
    let mut cur = CharKind::classify(last);
    for (byte_idx, c) in iter {
        let next = CharKind::classify(c);
        if cur != CharKind::Space && next != cur {
            s.truncate(byte_idx + c.len_utf8());
            return;
        }
        cur = next;
    }
    s.clear();
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    #[default]
    Inactive,
    InputForward,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub line_index: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub panel: MatchPanel,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatchPanel {
    Old,
    New,
}

#[derive(Default, Clone)]
pub struct SearchState {
    pub mode: SearchMode,
    pub query: String,
    pub matches: Vec<SearchMatch>,
    pub current_match: Option<usize>,
}

/// Finds all case-insensitive occurrences of `query_lower` in `text`, returning valid
/// UTF-8 byte ranges into the original `text`.
fn find_case_insensitive_byte_ranges(text: &str, query_lower: &str) -> Vec<(usize, usize)> {
    if query_lower.is_empty() || text.is_empty() {
        return Vec::new();
    }

    if text.is_ascii() && query_lower.is_ascii() {
        let text_lower = text.to_ascii_lowercase();
        let query_len = query_lower.len();
        let mut ranges = Vec::new();
        let mut start = 0;
        while let Some(pos) = text_lower[start..].find(query_lower) {
            let abs_pos = start + pos;
            ranges.push((abs_pos, abs_pos + query_len));
            start = abs_pos + 1;
        }
        return ranges;
    }

    // Build lowered text while mapping each lowered byte index back to its original
    // char's (start_byte, end_byte) in `text` so multi-byte and expanding case folds
    // (like 'İ' -> "i\u{307}") always produce valid UTF-8 slice boundaries.
    let mut text_lower = String::with_capacity(text.len());
    let mut orig_bounds: Vec<(usize, usize)> = Vec::with_capacity(text.len());
    for (orig_start, ch) in text.char_indices() {
        let orig_end = orig_start + ch.len_utf8();
        for lower_ch in ch.to_lowercase() {
            let start_len = text_lower.len();
            text_lower.push(lower_ch);
            for _ in start_len..text_lower.len() {
                orig_bounds.push((orig_start, orig_end));
            }
        }
    }

    let query_len = query_lower.len();
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < text_lower.len() {
        let Some(pos) = text_lower[start..].find(query_lower) else {
            break;
        };
        let abs_pos = start + pos;
        let abs_end = abs_pos + query_len;
        if let (Some(&(orig_start, _)), Some(&(_, orig_end))) = (
            orig_bounds.get(abs_pos),
            orig_bounds.get(abs_end.saturating_sub(1)),
        ) {
            if orig_start < orig_end {
                ranges.push((orig_start, orig_end));
            }
        }
        let step = text_lower[abs_pos..]
            .chars()
            .next()
            .map_or(1, char::len_utf8);
        start = abs_pos + step;
    }
    ranges
}

impl SearchState {
    pub fn start_forward(&mut self) {
        self.mode = SearchMode::InputForward;
        self.query.clear();
        self.matches.clear();
        self.current_match = None;
    }

    pub fn cancel(&mut self) {
        self.mode = SearchMode::Inactive;
        self.clear();
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.matches.clear();
        self.current_match = None;
    }

    pub fn confirm(&mut self) {
        self.mode = SearchMode::Inactive;
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
    }

    pub fn erase_word(&mut self) {
        erase_word_backward(&mut self.query);
    }

    pub fn is_active(&self) -> bool {
        self.mode != SearchMode::Inactive
    }

    pub fn has_query(&self) -> bool {
        !self.query.is_empty()
    }

    pub fn update_matches(&mut self, lines: &[DiffLine], fullscreen: DiffFullscreen) {
        if self.query.is_empty() {
            self.matches.clear();
            self.current_match = None;
            return;
        }

        let prev_match = self
            .current_match
            .and_then(|idx| self.matches.get(idx))
            .map(|m| (m.line_index, m.start_col, m.end_col, m.panel));

        self.matches.clear();
        let query_lower = self.query.to_lowercase();

        for (i, line) in lines.iter().enumerate() {
            if !matches!(fullscreen, DiffFullscreen::NewOnly) {
                if let Some((_, text)) = &line.old_line {
                    for (start_col, end_col) in
                        find_case_insensitive_byte_ranges(text, &query_lower)
                    {
                        self.matches.push(SearchMatch {
                            line_index: i,
                            start_col,
                            end_col,
                            panel: MatchPanel::Old,
                        });
                    }
                }
            }

            if !matches!(fullscreen, DiffFullscreen::OldOnly) {
                if let Some((_, text)) = &line.new_line {
                    for (start_col, end_col) in
                        find_case_insensitive_byte_ranges(text, &query_lower)
                    {
                        self.matches.push(SearchMatch {
                            line_index: i,
                            start_col,
                            end_col,
                            panel: MatchPanel::New,
                        });
                    }
                }
            }
        }

        if let Some((line_idx, start, end, panel)) = prev_match {
            self.current_match = self.matches.iter().position(|m| {
                m.line_index == line_idx
                    && m.start_col == start
                    && m.end_col == end
                    && m.panel == panel
            });

            if self.current_match.is_none() && !self.matches.is_empty() {
                self.current_match = self
                    .matches
                    .iter()
                    .position(|m| m.line_index >= line_idx)
                    .or(Some(0));
            }
        }
    }

    pub fn find_next(&mut self) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        let next = (self.current_match.unwrap_or(0) + 1) % self.matches.len();
        self.current_match = Some(next);
        Some(self.matches[next].line_index)
    }

    pub fn find_prev(&mut self) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        let current = self.current_match.unwrap_or(0);
        let prev = if current == 0 {
            self.matches.len() - 1
        } else {
            current - 1
        };
        self.current_match = Some(prev);
        Some(self.matches[prev].line_index)
    }

    pub fn jump_to_first_match(&mut self, current_scroll: usize) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        let idx = self
            .matches
            .iter()
            .position(|m| m.line_index >= current_scroll)
            .unwrap_or(0);
        self.current_match = Some(idx);
        Some(self.matches[idx].line_index)
    }

    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    pub fn current_match_index(&self) -> Option<usize> {
        self.current_match
    }

    pub fn get_matches_for_line(
        &self,
        line_index: usize,
        panel: MatchPanel,
    ) -> Vec<(usize, usize, bool)> {
        self.matches
            .iter()
            .enumerate()
            .filter(|(_, m)| m.line_index == line_index && m.panel == panel)
            .map(|(idx, m)| (m.start_col, m.end_col, self.current_match == Some(idx)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::compute_side_by_side;

    #[test]
    fn test_erase_word_backward() {
        let erase = |s: &str| {
            let mut buf = s.to_string();
            erase_word_backward(&mut buf);
            buf
        };
        assert_eq!(erase("something/another"), "something/");
        assert_eq!(erase("something/"), "something");
        assert_eq!(erase("abc.."), "abc");
        assert_eq!(erase("foo bar"), "foo ");
        assert_eq!(erase("foo   "), "");
        assert_eq!(erase("naïve/résumé"), "naïve/");
        assert_eq!(erase(""), "");
    }

    #[test]
    fn test_search_handles_multibyte_utf8_and_expanding_case_folds_without_panicking() {
        let lines = compute_side_by_side("", "café CAFÉ İstanbul ⚠️ éé\n", 4);
        let mut search = SearchState {
            query: "é".to_string(),
            ..SearchState::default()
        };
        search.update_matches(&lines, DiffFullscreen::NewOnly);
        assert_eq!(search.match_count(), 4);
        let text = &lines[0].new_line.as_ref().unwrap().1;
        for m in &search.matches {
            assert!(text.is_char_boundary(m.start_col));
            assert!(text.is_char_boundary(m.end_col));
            assert_eq!(text[m.start_col..m.end_col].to_lowercase(), "é");
        }

        // 'İ' expands to 2 code points on .to_lowercase()
        search.query = "istanbul".to_string();
        search.update_matches(&lines, DiffFullscreen::NewOnly);
        for m in &search.matches {
            assert!(text.is_char_boundary(m.start_col));
            assert!(text.is_char_boundary(m.end_col));
        }
    }
}
