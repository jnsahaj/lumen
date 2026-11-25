use std::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::{
    prelude::*,
    widgets::{
        Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
};

use crate::review::Thread;
use crate::ui::editor::AnnotationEditor;
use crate::ui::markdown::{render_code_snippet, render_comment_rows, truncate_with_ellipsis};
use crate::ui::theme;
use crate::ui::views::modal::ModalResult;

const MAX_ANCHOR_PREVIEW_LINES: usize = 8;

pub struct ThreadViewState {
    pub thread_id: String,
    pub tab_width: usize,
    pub return_to_list: bool,
    pub scroll: usize,
    pub pinned_to_bottom: bool,
    pub last_max_scroll: Cell<usize>,
    pub last_visible_height: Cell<usize>,
    pub last_editor_rect: Cell<Option<Rect>>,
}

impl ThreadViewState {
    pub fn new(thread_id: String, tab_width: usize, return_to_list: bool) -> Self {
        Self {
            thread_id,
            tab_width,
            return_to_list,
            scroll: 0,
            pinned_to_bottom: true,
            last_max_scroll: Cell::new(0),
            last_visible_height: Cell::new(0),
            last_editor_rect: Cell::new(None),
        }
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let max_scroll = self.last_max_scroll.get();
        let current = if self.pinned_to_bottom {
            max_scroll
        } else {
            self.scroll.min(max_scroll)
        };
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize).min(max_scroll)
        };
        self.scroll = next;
        self.pinned_to_bottom = next >= max_scroll;
    }

    pub fn handle_input(&mut self, key: KeyEvent) -> Option<ModalResult> {
        let max_scroll = self.last_max_scroll.get();
        let visible_height = self.last_visible_height.get().max(1);
        let half_page = (visible_height / 2).max(1) as isize;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Down | KeyCode::Char('k') => {
                self.scroll_by(1);
                None
            }
            KeyCode::Up | KeyCode::Char('j') => {
                self.scroll_by(-1);
                None
            }
            KeyCode::PageDown => {
                self.scroll_by(half_page);
                None
            }
            KeyCode::PageUp => {
                self.scroll_by(-half_page);
                None
            }
            KeyCode::Char('d') if ctrl => {
                self.scroll_by(half_page);
                None
            }
            KeyCode::Char('u') if ctrl => {
                self.scroll_by(-half_page);
                None
            }
            KeyCode::Home => {
                self.scroll = 0;
                self.pinned_to_bottom = max_scroll == 0;
                None
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.scroll = max_scroll;
                self.pinned_to_bottom = true;
                None
            }
            KeyCode::Char('i') => Some(ModalResult::ThreadReply {
                thread_id: self.thread_id.clone(),
            }),
            KeyCode::Char('x') => Some(ModalResult::ThreadToggleResolved {
                thread_id: self.thread_id.clone(),
            }),
            KeyCode::Char('g') => Some(ModalResult::AnnotationJump {
                annotation_id: self.thread_id.clone(),
            }),
            _ => None,
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                self.scroll_by(3);
                true
            }
            MouseEventKind::ScrollUp => {
                self.scroll_by(-3);
                true
            }
            _ => false,
        }
    }

    pub fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        thread: &Thread,
        editor: Option<&AnnotationEditor>,
    ) {
        let t = theme::get();
        let line_display = thread.line_range_display();
        let location = if line_display.is_empty() {
            thread.filename.clone()
        } else {
            format!("{}:{line_display}", thread.filename)
        };
        let comment_noun = if thread.comments.len() == 1 {
            "comment"
        } else {
            "comments"
        };
        let mut title = format!(
            "{location} [{}] · {} {comment_noun}",
            thread.target_label(),
            thread.comments.len(),
        );
        if thread.outdated {
            title.push_str(" · outdated");
        }
        if thread.resolved {
            title.push_str(" · resolved");
        }
        if thread.awaiting_agent() {
            title.push_str(" · awaiting agent");
        }
        let title = truncate_with_ellipsis(&title, area.width.saturating_sub(4) as usize);

        let block = Block::default()
            .title(format!(" {title} "))
            .title_style(Style::default().fg(t.ui.text_secondary).bold())
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(t.ui.border_focused))
            .style(Style::default().bg(t.ui.bg));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height == 0 || inner.width == 0 {
            self.last_max_scroll.set(0);
            self.last_visible_height.set(0);
            self.last_editor_rect.set(None);
            return;
        }

        let docked_editor = editor.filter(|e| e.thread_id.as_deref() == Some(thread.id.as_str()));

        let (comments_area, footer_area) = if let Some(ed) = docked_editor {
            let max_editor_h = (inner.height / 2)
                .max(4)
                .min(inner.height.saturating_sub(1));
            let editor_h = (ed.desired_height() as u16).min(max_editor_h);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(editor_h)])
                .split(inner);
            ed.render_inline(frame, chunks[1], t.ui.highlight, t.ui.bg, false);
            self.last_editor_rect.set(Some(chunks[1]));
            (chunks[0], None)
        } else {
            self.last_editor_rect.set(None);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(inner);
            (chunks[0], Some(chunks[1]))
        };

        let text_width = comments_area.width.saturating_sub(3).max(1) as usize;
        let mut lines: Vec<Line<'static>> = Vec::new();
        let pad_row = |spans: Vec<Span<'static>>| {
            let mut row = Vec::with_capacity(spans.len() + 1);
            row.push(Span::raw(" "));
            row.extend(spans);
            Line::from(row)
        };

        if let Some(anchor) = thread.anchor_text.as_deref().filter(|a| !a.is_empty()) {
            let anchor_lines: Vec<&str> = anchor.split('\n').collect();
            let shown_count = anchor_lines.len().min(MAX_ANCHOR_PREVIEW_LINES);
            let preview = anchor_lines[..shown_count].join("\n");
            for code_row in
                render_code_snippet(&preview, &thread.filename, text_width, self.tab_width, t)
            {
                lines.push(pad_row(code_row));
            }
            if anchor_lines.len() > shown_count {
                let omitted = anchor_lines.len() - shown_count;
                let noun = if omitted == 1 { "line" } else { "lines" };
                lines.push(pad_row(vec![Span::styled(
                    format!("⋯ {omitted} more anchored {noun}"),
                    Style::default().fg(t.ui.text_muted).italic(),
                )]));
            }
            lines.push(pad_row(vec![Span::styled(
                "─".repeat(text_width),
                Style::default().fg(t.ui.border_unfocused),
            )]));
            lines.push(Line::from(""));
        }

        let edited_comment_id = docked_editor.and_then(|e| e.comment_id.as_deref());
        let visible_comments = thread
            .comments
            .iter()
            .filter(|c| Some(c.id.as_str()) != edited_comment_id);
        for (idx, comment) in visible_comments.enumerate() {
            if idx > 0 {
                lines.push(Line::from(""));
            }
            for row in render_comment_rows(comment, &thread.filename, text_width, self.tab_width, t)
            {
                lines.push(pad_row(row));
            }
        }

        let visible_height = comments_area.height as usize;
        let total_rows = lines.len();
        let max_scroll = total_rows.saturating_sub(visible_height);
        self.last_max_scroll.set(max_scroll);
        self.last_visible_height.set(visible_height);

        let effective_scroll = if self.pinned_to_bottom {
            max_scroll
        } else {
            self.scroll.min(max_scroll)
        };
        let end = (effective_scroll + visible_height).min(total_rows);
        let visible_slice = lines[effective_scroll..end].to_vec();

        let content_area = Rect::new(
            comments_area.x,
            comments_area.y,
            comments_area.width.saturating_sub(1),
            comments_area.height,
        );
        frame.render_widget(
            Paragraph::new(visible_slice).style(Style::default().bg(t.ui.bg)),
            content_area,
        );

        if total_rows > visible_height {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .thumb_symbol("█");
            let mut scrollbar_state = ScrollbarState::new(max_scroll).position(effective_scroll);
            frame.render_stateful_widget(scrollbar, comments_area, &mut scrollbar_state);
        }

        if let Some(footer_area) = footer_area {
            let sep = Span::styled("│  ", Style::default().fg(t.ui.border_unfocused));
            let muted = Style::default().fg(t.ui.text_muted);
            let footer_text = Line::from(vec![
                Span::styled("j/k", muted),
                Span::styled(" scroll  ", muted),
                sep.clone(),
                Span::styled("i", muted),
                Span::styled(" reply  ", muted),
                sep.clone(),
                Span::styled("x", muted),
                Span::styled(
                    if thread.resolved {
                        " reopen  "
                    } else {
                        " resolve  "
                    },
                    muted,
                ),
                sep.clone(),
                Span::styled("g", muted),
                Span::styled(" jump  ", muted),
                sep,
                Span::styled("esc", muted),
                Span::styled(" close", muted),
            ]);
            frame.render_widget(
                Paragraph::new(footer_text).alignment(Alignment::Center),
                footer_area,
            );
        }
    }
}
