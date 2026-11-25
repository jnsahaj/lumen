use ratatui::layout::Rect;

use crate::diff::{DiffFullscreen, DiffLine, DiffPanelFocus};

/// Screen rect of a rendered inline thread overlay, plus the screen `y` rows of
/// any `⋯` truncation indicators inside it (clicking those opens the thread modal).
#[derive(Clone, Debug)]
pub struct ThreadHitRect {
    pub thread_id: String,
    pub area: Rect,
    pub expand_rows: Vec<u16>,
}

pub fn rect_contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct CursorPosition {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum SelectionMode {
    #[default]
    None,
    Character,
    Line,
}

#[derive(Clone, PartialEq, Eq, Default, Debug)]
pub struct Selection {
    pub panel: DiffPanelFocus,
    pub anchor: CursorPosition,
    pub head: CursorPosition,
    pub mode: SelectionMode,
}

impl Selection {
    pub fn normalized_range(&self) -> (CursorPosition, CursorPosition) {
        if self.anchor.line < self.head.line
            || (self.anchor.line == self.head.line && self.anchor.column <= self.head.column)
        {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub fn is_active(&self) -> bool {
        self.mode != SelectionMode::None && self.panel != DiffPanelFocus::None
    }
}

#[derive(Debug, Clone)]
pub struct PanelLayout {
    pub old_panel_x: u16,
    pub old_panel_width: u16,
    pub new_panel_x: u16,
    pub new_panel_width: u16,
    pub focus_indicator_width: u16,
    pub gutter_width: u16,
    pub diff_fullscreen: DiffFullscreen,
}

impl PanelLayout {
    pub fn calculate(
        term_width: u16,
        sidebar_width: u16,
        show_sidebar: bool,
        diff_fullscreen: DiffFullscreen,
    ) -> Self {
        let focus_indicator_width = 1u16;
        let gutter_width = 5u16;
        let border_width = 1u16;

        let diff_area_start = if show_sidebar { sidebar_width } else { 0 };
        let diff_area_width = term_width.saturating_sub(diff_area_start);

        let (old_panel_x, old_panel_width, new_panel_x, new_panel_width) = match diff_fullscreen {
            DiffFullscreen::OldOnly => {
                let panel_x = diff_area_start + border_width;
                let panel_width = diff_area_width.saturating_sub(border_width * 2);
                (panel_x, panel_width, 0, 0)
            }
            DiffFullscreen::NewOnly => {
                let panel_x = diff_area_start + border_width;
                let panel_width = diff_area_width.saturating_sub(border_width * 2);
                (0, 0, panel_x, panel_width)
            }
            DiffFullscreen::None => {
                let half_width = diff_area_width / 2;
                let old_x = diff_area_start + border_width;
                let old_width = half_width.saturating_sub(border_width);
                let new_x = diff_area_start + half_width;
                let new_width = diff_area_width
                    .saturating_sub(half_width)
                    .saturating_sub(border_width);
                (old_x, old_width, new_x, new_width)
            }
        };

        Self {
            old_panel_x,
            old_panel_width,
            new_panel_x,
            new_panel_width,
            focus_indicator_width,
            gutter_width,
            diff_fullscreen,
        }
    }

    pub fn panel_at_x(&self, x: u16) -> Option<DiffPanelFocus> {
        let in_old = self.old_panel_width > 0
            && x >= self.old_panel_x
            && x < self.old_panel_x + self.old_panel_width;
        let in_new = self.new_panel_width > 0
            && x >= self.new_panel_x
            && x < self.new_panel_x + self.new_panel_width;

        match self.diff_fullscreen {
            DiffFullscreen::OldOnly => in_old.then_some(DiffPanelFocus::Old),
            DiffFullscreen::NewOnly => in_new.then_some(DiffPanelFocus::New),
            DiffFullscreen::None => {
                if in_old {
                    Some(DiffPanelFocus::Old)
                } else if in_new {
                    Some(DiffPanelFocus::New)
                } else {
                    None
                }
            }
        }
    }

    pub fn is_in_gutter(&self, x: u16, panel: DiffPanelFocus) -> bool {
        let (panel_x, panel_width) = match panel {
            DiffPanelFocus::Old => (self.old_panel_x, self.old_panel_width),
            DiffPanelFocus::New => (self.new_panel_x, self.new_panel_width),
            DiffPanelFocus::None => return false,
        };
        if panel_width == 0 {
            return false;
        }
        let rel_x = x.saturating_sub(panel_x);
        rel_x < self.content_x_offset(panel)
    }

    pub fn content_x_offset(&self, panel: DiffPanelFocus) -> u16 {
        match panel {
            DiffPanelFocus::Old => self.focus_indicator_width + self.gutter_width,
            DiffPanelFocus::New => {
                if self.diff_fullscreen == DiffFullscreen::NewOnly {
                    self.focus_indicator_width + self.gutter_width
                } else {
                    self.gutter_width
                }
            }
            DiffPanelFocus::None => 0,
        }
    }

    pub fn panel_x(&self, panel: DiffPanelFocus) -> u16 {
        match panel {
            DiffPanelFocus::Old => self.old_panel_x,
            DiffPanelFocus::New => self.new_panel_x,
            DiffPanelFocus::None => 0,
        }
    }
}

/// Slices `text` by 0-based Unicode scalar column offsets (matching how diff selection
/// columns are counted in characters), returning an empty slice when out of bounds.
fn slice_by_char_columns(text: &str, start_col: usize, end_col: usize) -> &str {
    if start_col >= end_col {
        return "";
    }
    let start_byte = text
        .char_indices()
        .nth(start_col)
        .map_or(text.len(), |(idx, _)| idx);
    let end_byte = if end_col == usize::MAX {
        text.len()
    } else {
        text.char_indices()
            .nth(end_col)
            .map_or(text.len(), |(idx, _)| idx)
    };
    &text[start_byte..end_byte.max(start_byte)]
}

pub fn extract_selected_text(selection: &Selection, side_by_side: &[DiffLine]) -> Option<String> {
    if !selection.is_active() {
        return None;
    }

    let (start, end) = selection.normalized_range();
    let mut result = String::new();

    for (line_idx, diff_line) in side_by_side
        .iter()
        .enumerate()
        .take(end.line + 1)
        .skip(start.line)
    {
        let line_content = match selection.panel {
            DiffPanelFocus::Old => diff_line.old_line.as_ref().map(|(_, text)| text.as_str()),
            DiffPanelFocus::New => diff_line.new_line.as_ref().map(|(_, text)| text.as_str()),
            DiffPanelFocus::None => None,
        };

        let Some(text) = line_content else {
            continue;
        };

        match selection.mode {
            SelectionMode::Line => {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str(text);
            }
            SelectionMode::Character => {
                if !result.is_empty() {
                    result.push('\n');
                }
                let slice = if start.line == end.line {
                    slice_by_char_columns(text, start.column, end.column)
                } else if line_idx == start.line {
                    slice_by_char_columns(text, start.column, usize::MAX)
                } else if line_idx == end.line {
                    slice_by_char_columns(text, 0, end.column)
                } else {
                    text
                };
                result.push_str(slice);
            }
            SelectionMode::None => {}
        }
    }

    (!result.is_empty()).then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::compute_side_by_side;

    #[test]
    fn test_extract_selected_text_handles_multibyte_utf8_char_columns() {
        let rows = compute_side_by_side("", "café → 世界\nsecond résumé line\n", 4);

        let single_line = Selection {
            panel: DiffPanelFocus::New,
            anchor: CursorPosition { line: 0, column: 3 },
            head: CursorPosition { line: 0, column: 9 },
            mode: SelectionMode::Character,
        };
        assert_eq!(
            extract_selected_text(&single_line, &rows).as_deref(),
            Some("é → 世界")
        );

        let reversed_multiline = Selection {
            panel: DiffPanelFocus::New,
            anchor: CursorPosition {
                line: 1,
                column: 13,
            },
            head: CursorPosition { line: 0, column: 5 },
            mode: SelectionMode::Character,
        };
        assert_eq!(
            extract_selected_text(&reversed_multiline, &rows).as_deref(),
            Some("→ 世界\nsecond résumé")
        );

        let line_mode = Selection {
            panel: DiffPanelFocus::New,
            anchor: CursorPosition { line: 0, column: 0 },
            head: CursorPosition { line: 1, column: 0 },
            mode: SelectionMode::Line,
        };
        assert_eq!(
            extract_selected_text(&line_mode, &rows).as_deref(),
            Some("café → 世界\nsecond résumé line")
        );
    }
}
