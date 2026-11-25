use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Clear, Paragraph},
};
use tui_textarea::TextArea;

use super::theme;
use crate::review::{AnnotationTarget, Comment, Thread};

pub enum AnnotationEditorResult {
    Continue,
    Send,
    Queue,
    Cancel,
    Delete,
}

const MIN_INNER_LINES: usize = 3;

/// Inline comment editor rendered either in the diff stream below the anchored lines
/// (or top of file for file-level threads) or docked at the bottom of `ThreadView`.
pub struct AnnotationEditor<'a> {
    textarea: TextArea<'a>,
    pub filename: String,
    pub target: AnnotationTarget,
    pub thread_id: Option<String>,
    pub comment_id: Option<String>,
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
            thread_id: None,
            comment_id: None,
        }
    }

    fn reply_to(thread: &Thread) -> Self {
        let mut editor = Self::new(thread.filename.clone(), thread.target.clone());
        editor.thread_id = Some(thread.id.clone());
        editor
    }

    pub fn for_thread(thread: &Thread) -> Self {
        match thread.draft() {
            Some(draft) => Self::edit_comment(thread, draft),
            None => Self::reply_to(thread),
        }
    }

    fn edit_comment(thread: &Thread, comment: &Comment) -> Self {
        let mut editor = Self::reply_to(thread);
        editor.comment_id = Some(comment.id.clone());
        editor.textarea = TextArea::new(comment.body.lines().map(String::from).collect());

        let t = theme::get();
        editor.textarea.set_cursor_line_style(Style::default());
        editor
            .textarea
            .set_cursor_style(Style::default().bg(t.ui.text_primary).fg(t.ui.bg));

        editor
            .textarea
            .move_cursor(tui_textarea::CursorMove::Bottom);
        editor.textarea.move_cursor(tui_textarea::CursorMove::End);

        editor
    }

    pub fn content(&self) -> String {
        self.textarea.lines().join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.content().trim().is_empty()
    }

    fn finish(&self, saved: AnnotationEditorResult) -> AnnotationEditorResult {
        if !self.is_empty() {
            saved
        } else if self.comment_id.is_some() {
            AnnotationEditorResult::Delete
        } else {
            AnnotationEditorResult::Cancel
        }
    }

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

            // Ctrl+S queues too, for terminals that do not report Alt+Enter.
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.finish(AnnotationEditorResult::Queue)
            }

            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::ALT) {
                    self.finish(AnnotationEditorResult::Queue)
                } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.textarea.insert_char('\n');
                    AnnotationEditorResult::Continue
                } else if key.modifiers.contains(KeyModifiers::CONTROL) {
                    // Unbound so Ctrl+Enter cannot accidentally send a comment.
                    AnnotationEditorResult::Continue
                } else {
                    self.finish(AnnotationEditorResult::Send)
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
        let row_bg = Style::default().bg(bg);

        frame.render_widget(Clear, area);

        let box_height = area.height - 1;
        let border_width = area.width.saturating_sub(3) as usize;
        let gutter_bar = if has_gutter { "▍" } else { " " };

        let horizontal_rule = |left: char, right: char| {
            Line::from(vec![
                Span::styled(gutter_bar, border_style),
                Span::styled(
                    format!("{left}{}{right}", "─".repeat(border_width)),
                    border_style,
                ),
            ])
        };

        frame.render_widget(
            Paragraph::new(horizontal_rule('┌', '┐')).style(row_bg),
            Rect::new(area.x, area.y, area.width, 1),
        );

        let middle_rows = box_height.saturating_sub(2);
        for row in 0..middle_rows {
            let y = area.y + 1 + row;
            let prefix = Line::from(vec![
                Span::styled(gutter_bar, border_style),
                Span::styled("│ ", border_style),
            ]);
            frame.render_widget(
                Paragraph::new(prefix).style(row_bg),
                Rect::new(area.x, y, 3, 1),
            );
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled("│", border_style))).style(row_bg),
                Rect::new(area.x + area.width - 1, y, 1, 1),
            );
        }

        let inner_w = area.width.saturating_sub(4);
        if inner_w > 0 && middle_rows > 0 {
            let inner = Rect::new(area.x + 3, area.y + 1, inner_w, middle_rows);
            frame.render_widget(Clear, inner);
            frame.render_widget(Paragraph::new("").style(row_bg), inner);
            frame.render_widget(&self.textarea, inner);
        }

        frame.render_widget(
            Paragraph::new(horizontal_rule('└', '┘')).style(row_bg),
            Rect::new(area.x, area.y + box_height - 1, area.width, 1),
        );

        let key_style = Style::default().fg(t.ui.text_secondary);
        let label_style = Style::default().fg(t.ui.border_unfocused);
        let hint = Line::from(vec![
            Span::styled("Enter", key_style),
            Span::styled(" send", label_style),
            Span::styled(" . ", label_style),
            Span::styled("Alt+Enter", key_style),
            Span::styled(" queue", label_style),
            Span::styled(" . ", label_style),
            Span::styled("Esc", key_style),
            Span::styled(" cancel", label_style),
            Span::styled(" ", row_bg),
        ])
        .alignment(Alignment::Right);
        frame.render_widget(
            Paragraph::new(hint).style(row_bg),
            Rect::new(area.x, area.y + area.height - 1, area.width, 1),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor_with(text: &str) -> AnnotationEditor<'static> {
        let mut editor = AnnotationEditor::new("a.rs".to_string(), AnnotationTarget::File);
        for character in text.chars() {
            editor.handle_input(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        editor
    }

    fn press(
        editor: &mut AnnotationEditor,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> &'static str {
        match editor.handle_input(KeyEvent::new(code, modifiers)) {
            AnnotationEditorResult::Continue => "continue",
            AnnotationEditorResult::Send => "send",
            AnnotationEditorResult::Queue => "queue",
            AnnotationEditorResult::Cancel => "cancel",
            AnnotationEditorResult::Delete => "delete",
        }
    }

    #[test]
    fn test_enter_sends_and_alt_enter_or_ctrl_s_queue_unless_the_comment_is_empty() {
        let mut editor = editor_with("note");

        assert_eq!(
            press(&mut editor, KeyCode::Enter, KeyModifiers::NONE),
            "send"
        );
        assert_eq!(
            press(&mut editor, KeyCode::Enter, KeyModifiers::ALT),
            "queue"
        );
        assert_eq!(
            press(&mut editor, KeyCode::Char('s'), KeyModifiers::CONTROL),
            "queue"
        );

        let mut empty_editor = editor_with("  ");

        assert_eq!(
            press(&mut empty_editor, KeyCode::Enter, KeyModifiers::NONE),
            "cancel"
        );
        assert_eq!(
            press(&mut empty_editor, KeyCode::Enter, KeyModifiers::ALT),
            "cancel"
        );
    }

    #[test]
    fn test_shift_enter_and_ctrl_j_insert_a_newline_and_ctrl_enter_does_nothing() {
        let mut editor = editor_with("a");

        assert_eq!(
            press(&mut editor, KeyCode::Enter, KeyModifiers::SHIFT),
            "continue"
        );
        assert_eq!(
            press(&mut editor, KeyCode::Char('j'), KeyModifiers::CONTROL),
            "continue"
        );
        assert_eq!(editor.content(), "a\n\n");

        assert_eq!(
            press(&mut editor, KeyCode::Enter, KeyModifiers::CONTROL),
            "continue"
        );
        assert_eq!(editor.content(), "a\n\n");
    }
}
