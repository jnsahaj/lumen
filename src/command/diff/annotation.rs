use std::time::SystemTime;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, Paragraph},
};
use tui_textarea::TextArea;

use super::state::HunkAnnotation;
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

/// A modal editor for creating/editing hunk annotations
pub struct AnnotationEditor<'a> {
    textarea: TextArea<'a>,
    pub file_index: usize,
    pub hunk_index: usize,
    pub filename: String,
    pub line_range: (usize, usize),
    is_edit: bool,
    /// Original creation time (preserved when editing)
    original_created_at: Option<SystemTime>,
}

impl<'a> AnnotationEditor<'a> {
    pub fn new(
        file_index: usize,
        hunk_index: usize,
        filename: String,
        line_range: (usize, usize),
    ) -> Self {
        let mut textarea = TextArea::default();
        let t = theme::get();

        // Style the textarea
        textarea.set_cursor_line_style(Style::default());
        textarea.set_cursor_style(Style::default().bg(t.ui.text_primary).fg(t.ui.bg));
        textarea.set_block(Block::default()); // We'll draw our own block

        Self {
            textarea,
            file_index,
            hunk_index,
            filename,
            line_range,
            is_edit: false,
            original_created_at: None,
        }
    }

    pub fn with_content(mut self, content: &str, created_at: SystemTime) -> Self {
        self.textarea = TextArea::new(content.lines().map(String::from).collect());
        self.is_edit = true;
        self.original_created_at = Some(created_at);

        let t = theme::get();
        self.textarea.set_cursor_line_style(Style::default());
        self.textarea
            .set_cursor_style(Style::default().bg(t.ui.text_primary).fg(t.ui.bg));
        self.textarea.set_block(Block::default());

        // Move cursor to end
        self.textarea.move_cursor(tui_textarea::CursorMove::Bottom);
        self.textarea.move_cursor(tui_textarea::CursorMove::End);

        self
    }

    /// Handle a key event. Returns the result of the input handling.
    pub fn handle_input(&mut self, key: KeyEvent) -> AnnotationEditorResult {
        match key.code {
            // Escape: cancel
            KeyCode::Esc => AnnotationEditorResult::Cancel,

            // Ctrl+C: cancel
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                AnnotationEditorResult::Cancel
            }

            // Ctrl+S: save
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let content = self.textarea.lines().join("\n");
                if content.trim().is_empty() {
                    if self.is_edit {
                        AnnotationEditorResult::Delete
                    } else {
                        AnnotationEditorResult::Cancel
                    }
                } else {
                    AnnotationEditorResult::Save
                }
            }

            // Enter handling: with modifiers = newline, without = save
            KeyCode::Enter => {
                // Shift+Enter, Alt+Enter, or Ctrl+Enter = newline
                if key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL)
                {
                    self.textarea.insert_char('\n');
                    AnnotationEditorResult::Continue
                } else {
                    // Plain Enter = save
                    let content = self.textarea.lines().join("\n");
                    if content.trim().is_empty() {
                        if self.is_edit {
                            AnnotationEditorResult::Delete
                        } else {
                            AnnotationEditorResult::Cancel
                        }
                    } else {
                        AnnotationEditorResult::Save
                    }
                }
            }

            // Ctrl+J: alternative for newline (works in all terminals)
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.textarea.insert_char('\n');
                AnnotationEditorResult::Continue
            }

            // Cmd+Backspace (macOS): delete from cursor to beginning of line
            KeyCode::Backspace if key.modifiers.contains(KeyModifiers::SUPER) => {
                self.textarea.delete_line_by_head();
                AnnotationEditorResult::Continue
            }

            // Ctrl+U: delete from cursor to beginning of line (Unix standard, works in all terminals)
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.textarea.delete_line_by_head();
                AnnotationEditorResult::Continue
            }

            // Let tui-textarea handle everything else (but not Enter which we handle above)
            _ => {
                self.textarea.input(key);
                AnnotationEditorResult::Continue
            }
        }
    }

    /// Render the annotation editor as a centered modal
    pub fn render(&self, frame: &mut Frame) {
        let t = theme::get();
        let area = frame.area();

        // Calculate modal size - compact but comfortable
        let width = 60.min(area.width.saturating_sub(4));
        let height = 10.min(area.height.saturating_sub(4));
        let x = (area.width.saturating_sub(width)) / 2;
        let y = (area.height.saturating_sub(height)) / 2;
        let modal_area = Rect::new(x, y, width, height);

        // Clear the area
        frame.render_widget(Clear, modal_area);

        // Extract just the filename without path for cleaner display
        let short_filename = self.filename.rsplit('/').next().unwrap_or(&self.filename);

        // Compact title
        let title = format!(
            " {} · L{}-{} ",
            short_filename, self.line_range.0, self.line_range.1
        );

        let block = Block::default()
            .title(title)
            .title_style(Style::default().fg(t.ui.text_secondary))
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(t.ui.border_focused))
            .style(Style::default().bg(t.ui.bg));

        let inner = block.inner(modal_area);
        frame.render_widget(block, modal_area);

        // Split inner into textarea and footer
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        // Render textarea
        frame.render_widget(&self.textarea, chunks[0]);

        // Compact footer
        let footer_text = Line::from(vec![
            Span::styled("enter", Style::default().fg(t.ui.text_muted)),
            Span::styled(" save  ", Style::default().fg(t.ui.text_muted)),
            Span::styled("│  ", Style::default().fg(t.ui.border_unfocused)),
            Span::styled("esc", Style::default().fg(t.ui.text_muted)),
            Span::styled(" cancel  ", Style::default().fg(t.ui.text_muted)),
            Span::styled("│  ", Style::default().fg(t.ui.border_unfocused)),
            Span::styled("shift+enter", Style::default().fg(t.ui.text_muted)),
            Span::styled(" newline", Style::default().fg(t.ui.text_muted)),
        ]);

        let footer = Paragraph::new(footer_text)
            .style(Style::default().bg(t.ui.bg))
            .alignment(Alignment::Center);
        frame.render_widget(footer, chunks[1]);
    }

    /// Create a HunkAnnotation from the current editor state
    pub fn to_annotation(&self) -> HunkAnnotation {
        HunkAnnotation {
            file_index: self.file_index,
            hunk_index: self.hunk_index,
            content: self.textarea.lines().join("\n"),
            line_range: self.line_range,
            filename: self.filename.clone(),
            created_at: self.original_created_at.unwrap_or_else(SystemTime::now),
        }
    }
}
