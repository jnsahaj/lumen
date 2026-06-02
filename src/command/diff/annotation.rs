use std::time::SystemTime;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Clear, Paragraph},
};
use tui_textarea::TextArea;

use super::state::AnnotationTarget;
use super::theme;

/// Result of handling input in the annotation editor
pub enum AnnotationEditorResult {
    /// Continue editing
    Continue,
    /// Save the annotation
    Save,
    /// Cancel editing
    Cancel,
    /// Delete the annotation (when editing an existing annotation and content is emptied)
    Delete,
}

const MIN_INNER_LINES: usize = 3;

/// Inline annotation editor: a bordered textbox rendered in the same slot
/// where the saved annotation overlay would live (below a line range, or
/// at the top of the file for file-level annotations).
pub struct AnnotationEditor<'a> {
    textarea: TextArea<'a>,
    pub filename: String,
    pub target: AnnotationTarget,
    /// If editing an existing annotation, its id
    pub id: Option<u64>,
    is_edit: bool,
    /// Original creation time (preserved when editing)
    original_created_at: Option<SystemTime>,
}

impl<'a> AnnotationEditor<'a> {
    pub fn new(filename: String, target: AnnotationTarget) -> Self {
        let mut textarea = TextArea::default();
        let t = theme::get();

        textarea.set_cursor_line_style(Style::default());
        textarea.set_cursor_style(Style::default().bg(t.ui.text_primary).fg(t.ui.bg));

        Self {
            textarea,
            filename,
            target,
            id: None,
            is_edit: false,
            original_created_at: None,
        }
    }

    pub fn with_existing(mut self, id: u64, content: &str, created_at: SystemTime) -> Self {
        self.textarea = TextArea::new(content.lines().map(String::from).collect());
        self.id = Some(id);
        self.is_edit = true;
        self.original_created_at = Some(created_at);

        let t = theme::get();
        self.textarea.set_cursor_line_style(Style::default());
        self.textarea.set_cursor_style(Style::default().bg(t.ui.text_primary).fg(t.ui.bg));

        self.textarea.move_cursor(tui_textarea::CursorMove::Bottom);
        self.textarea.move_cursor(tui_textarea::CursorMove::End);

        self
    }

    pub fn content(&self) -> String {
        self.textarea.lines().join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.content().trim().is_empty()
    }

    pub fn created_at(&self) -> SystemTime {
        self.original_created_at.unwrap_or_else(SystemTime::now)
    }

    /// Rows the editor wants to occupy: top + bottom borders, content padded to
    /// `MIN_INNER_LINES`, plus a dim hint row floating below the box.
    pub fn desired_height(&self) -> usize {
        let content_lines = self.textarea.lines().len().max(1);
        content_lines.max(MIN_INNER_LINES) + 2 + 1
    }

    pub fn handle_input(&mut self, key: KeyEvent) -> AnnotationEditorResult {
        match key.code {
            KeyCode::Esc => AnnotationEditorResult::Cancel,

            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                AnnotationEditorResult::Cancel
            }

            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.is_empty() {
                    if self.is_edit {
                        AnnotationEditorResult::Delete
                    } else {
                        AnnotationEditorResult::Cancel
                    }
                } else {
                    AnnotationEditorResult::Save
                }
            }

            KeyCode::Enter => {
                if key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL)
                {
                    self.textarea.insert_char('\n');
                    AnnotationEditorResult::Continue
                } else if self.is_empty() {
                    if self.is_edit {
                        AnnotationEditorResult::Delete
                    } else {
                        AnnotationEditorResult::Cancel
                    }
                } else {
                    AnnotationEditorResult::Save
                }
            }

            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.textarea.insert_char('\n');
                AnnotationEditorResult::Continue
            }

            KeyCode::Backspace if key.modifiers.contains(KeyModifiers::SUPER) => {
                self.textarea.delete_line_by_head();
                AnnotationEditorResult::Continue
            }

            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.textarea.delete_line_by_head();
                AnnotationEditorResult::Continue
            }

            _ => {
                self.textarea.input(key);
                AnnotationEditorResult::Continue
            }
        }
    }

    /// Render the editor as an overlay at `area`. `accent` is the border color,
    /// `bg` the panel background. When `has_gutter` is true a `▍` indicator is
    /// painted on the leftmost column (matches saved line-range overlays).
    ///
    /// The box itself takes `area.height - 1` rows; the final row is a dim
    /// `enter save · esc cancel` hint floating below the box.
    pub fn render_inline(
        &self,
        frame: &mut Frame,
        area: Rect,
        accent: Color,
        bg: Color,
        has_gutter: bool,
    ) {
        if area.height < 4 || area.width < 5 {
            return;
        }

        let t = theme::get();
        let border_style = Style::default().fg(accent);
        let indicator_style = Style::default().fg(accent);
        let row_bg = Style::default().bg(bg);

        frame.render_widget(Clear, area);

        // The bordered box occupies all rows except the last (which is the hint).
        let box_height = area.height - 1;
        let border_width = area.width.saturating_sub(3) as usize;

        // Top border
        let top = if has_gutter {
            Line::from(vec![
                Span::styled("▍", indicator_style),
                Span::styled(format!("┌{}┐", "─".repeat(border_width)), border_style),
            ])
        } else {
            Line::from(vec![Span::styled(
                format!(" ┌{}┐", "─".repeat(border_width)),
                border_style,
            )])
        };
        frame.render_widget(
            Paragraph::new(top).style(row_bg),
            Rect::new(area.x, area.y, area.width, 1),
        );

        // Middle rows: gutter + left bar + (textarea) + right bar
        let middle_rows = box_height.saturating_sub(2);
        for row in 0..middle_rows {
            let y = area.y + 1 + row;
            let prefix = if has_gutter {
                Line::from(vec![
                    Span::styled("▍", indicator_style),
                    Span::styled("│ ", border_style),
                ])
            } else {
                Line::from(vec![Span::styled(" │ ", border_style)])
            };
            frame.render_widget(
                Paragraph::new(prefix).style(row_bg),
                Rect::new(area.x, y, 3, 1),
            );
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled("│", border_style))).style(row_bg),
                Rect::new(area.x + area.width - 1, y, 1, 1),
            );
        }

        // Textarea inside the inner area (after gutter + left bar + space)
        let inner_w = area.width.saturating_sub(4);
        let inner_h = middle_rows;
        if inner_w > 0 && inner_h > 0 {
            let inner = Rect::new(area.x + 3, area.y + 1, inner_w, inner_h);
            frame.render_widget(Clear, inner);
            frame.render_widget(Paragraph::new("").style(row_bg), inner);
            frame.render_widget(&self.textarea, inner);
        }

        // Clean bottom border
        let bottom = if has_gutter {
            Line::from(vec![
                Span::styled("▍", indicator_style),
                Span::styled(format!("└{}┘", "─".repeat(border_width)), border_style),
            ])
        } else {
            Line::from(vec![Span::styled(
                format!(" └{}┘", "─".repeat(border_width)),
                border_style,
            )])
        };
        frame.render_widget(
            Paragraph::new(bottom).style(row_bg),
            Rect::new(area.x, area.y + box_height - 1, area.width, 1),
        );

        // Hint row floating below the box, right-aligned with a soft tone.
        let key_style = Style::default().fg(t.ui.text_secondary);
        let label_style = Style::default().fg(t.ui.border_unfocused);
        let sep_style = Style::default().fg(t.ui.border_unfocused);
        let hint_spans = vec![
            Span::styled("Enter", key_style),
            Span::styled(" save", label_style),
            Span::styled(" . ", sep_style),
            Span::styled("Esc", key_style),
            Span::styled(" cancel", label_style),
            Span::styled(" ", row_bg),
        ];
        let hint = Line::from(hint_spans).alignment(Alignment::Right);
        frame.render_widget(
            Paragraph::new(hint).style(row_bg),
            Rect::new(area.x, area.y + area.height - 1, area.width, 1),
        );
    }
}
