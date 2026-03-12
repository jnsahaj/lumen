use std::time::SystemTime;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, Paragraph},
};
use tui_textarea::TextArea;

use super::state::AnnotationTarget;
use super::theme;
use crate::command::diff::types::DiffPanelFocus;

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

/// A modal editor for creating/editing annotations
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
        textarea.set_block(Block::default());

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
        self.textarea.set_block(Block::default());

        self.textarea.move_cursor(tui_textarea::CursorMove::Bottom);
        self.textarea.move_cursor(tui_textarea::CursorMove::End);

        self
    }

    /// Get the annotation content
    pub fn content(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Get the creation time (original if editing, now if new)
    pub fn created_at(&self) -> SystemTime {
        self.original_created_at.unwrap_or_else(SystemTime::now)
    }

    /// Handle a key event. Returns the result of the input handling.
    pub fn handle_input(&mut self, key: KeyEvent) -> AnnotationEditorResult {
        match key.code {
            KeyCode::Esc => AnnotationEditorResult::Cancel,

            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                AnnotationEditorResult::Cancel
            }

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

            KeyCode::Enter => {
                if key.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL) {
                    self.textarea.insert_char('\n');
                    AnnotationEditorResult::Continue
                } else {
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

    /// Render the annotation editor as a centered modal
    pub fn render(&self, frame: &mut Frame) {
        let t = theme::get();
        let area = frame.area();

        let width = 60.min(area.width.saturating_sub(4));
        let height = 10.min(area.height.saturating_sub(4));
        let x = (area.width.saturating_sub(width)) / 2;
        let y = (area.height.saturating_sub(height)) / 2;
        let modal_area = Rect::new(x, y, width, height);

        frame.render_widget(Clear, modal_area);

        let short_filename = self
            .filename
            .rsplit('/')
            .next()
            .unwrap_or(&self.filename);

        let title = match &self.target {
            AnnotationTarget::File => format!(" {} [file] ", short_filename),
            AnnotationTarget::LineRange { panel, start_line, end_line, .. } => {
                let panel_label = match panel {
                    DiffPanelFocus::Old => "old",
                    DiffPanelFocus::New | DiffPanelFocus::None => "new",
                };
                if start_line == end_line {
                    format!(" {} · L{} [{}] ", short_filename, start_line, panel_label)
                } else {
                    format!(" {} · L{}-{} [{}] ", short_filename, start_line, end_line, panel_label)
                }
            }
        };

        let block = Block::default()
            .title(title)
            .title_style(Style::default().fg(t.ui.text_secondary))
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(t.ui.border_focused))
            .style(Style::default().bg(t.ui.bg));

        let inner = block.inner(modal_area);
        frame.render_widget(block, modal_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        frame.render_widget(&self.textarea, chunks[0]);

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
}
