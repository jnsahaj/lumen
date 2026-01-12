use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::command::diff::theme;

#[derive(Clone)]
pub struct KeyBind {
    pub key: &'static str,
    pub description: &'static str,
}

#[derive(Clone)]
pub struct KeyBindSection {
    pub title: &'static str,
    pub bindings: Vec<KeyBind>,
}

#[derive(Clone)]
pub struct FilePickerItem {
    pub name: String,
    pub file_index: usize,
    pub status: FileStatus,
    pub viewed: bool,
}

#[derive(Clone, Copy)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone)]
pub enum ModalContent {
    #[allow(dead_code)]
    Info { title: String, message: String },
    #[allow(dead_code)]
    Select {
        title: String,
        items: Vec<String>,
        selected: usize,
    },
    KeyBindings {
        title: String,
        sections: Vec<KeyBindSection>,
    },
    FilePicker {
        title: String,
        items: Vec<FilePickerItem>,
        filtered_indices: Vec<usize>,
        query: String,
        selected: usize,
    },
    CommitInput {
        title: String,
        message: String,
        cursor_pos: usize,
        files_to_commit: Vec<String>,
    },
}

pub struct Modal {
    pub content: ModalContent,
}

#[derive(Clone)]
pub enum ModalResult {
    Dismissed,
    #[allow(dead_code)]
    Selected(usize, String),
    FileSelected(usize),
    CommitConfirmed(String),
}

impl Modal {
    #[allow(dead_code)]
    pub fn info(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            content: ModalContent::Info {
                title: title.into(),
                message: message.into(),
            },
        }
    }

    #[allow(dead_code)]
    pub fn select(title: impl Into<String>, items: Vec<String>) -> Self {
        Self {
            content: ModalContent::Select {
                title: title.into(),
                items,
                selected: 0,
            },
        }
    }

    pub fn keybindings(title: impl Into<String>, sections: Vec<KeyBindSection>) -> Self {
        Self {
            content: ModalContent::KeyBindings {
                title: title.into(),
                sections,
            },
        }
    }

    pub fn file_picker(title: impl Into<String>, items: Vec<FilePickerItem>) -> Self {
        let filtered_indices: Vec<usize> = (0..items.len()).collect();
        Self {
            content: ModalContent::FilePicker {
                title: title.into(),
                items,
                filtered_indices,
                query: String::new(),
                selected: 0,
            },
        }
    }

    pub fn commit_input(title: impl Into<String>, files_to_commit: Vec<String>) -> Self {
        Self {
            content: ModalContent::CommitInput {
                title: title.into(),
                message: String::new(),
                cursor_pos: 0,
                files_to_commit,
            },
        }
    }

    pub fn render(&self, frame: &mut Frame) {
        let area = frame.area();

        let (modal_width, modal_height) = match &self.content {
            ModalContent::Info { message, .. } => {
                let width = 80.min(area.width.saturating_sub(4));
                let lines = message.lines().count() as u16;
                let height = (lines + 4).min(area.height * 80 / 100).max(5);
                (width, height)
            }
            ModalContent::Select { items, .. } => {
                let width = 80.min(area.width.saturating_sub(4));
                let items_count = items.len() as u16;
                let height = (items_count + 4).min(area.height * 80 / 100).max(5);
                (width, height)
            }
            ModalContent::KeyBindings { sections, .. } => {
                let width = 60.min(area.width.saturating_sub(4));
                let total_lines: usize = sections
                    .iter()
                    .map(|s| s.bindings.len() + 2) // +2 for section title and spacing
                    .sum();
                let height = (total_lines as u16 + 4).min(area.height * 80 / 100).max(5);
                (width, height)
            }
            ModalContent::FilePicker {
                filtered_indices, ..
            } => {
                let width = 80.min(area.width.saturating_sub(4));
                let items_count = filtered_indices.len().min(15) as u16;
                let height = (items_count + 5).min(area.height * 80 / 100).max(8);
                (width, height)
            }
            ModalContent::CommitInput {
                files_to_commit, ..
            } => {
                let width = 70.min(area.width.saturating_sub(4));
                // Height: 1 for input, 1 for separator, files list (max 8), 1 for hint, 2 for padding
                let files_count = files_to_commit.len().min(8) as u16;
                let height = (files_count + 6).min(area.height * 80 / 100).max(8);
                (width, height)
            }
        };

        let modal_x = (area.width.saturating_sub(modal_width)) / 2;
        let modal_y = (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = Rect::new(modal_x, modal_y, modal_width, modal_height);

        frame.render_widget(Clear, modal_area);

        match &self.content {
            ModalContent::Info { title, message } => {
                self.render_info(frame, modal_area, title, message);
            }
            ModalContent::Select {
                title,
                items,
                selected,
            } => {
                self.render_select(frame, modal_area, title, items, *selected);
            }
            ModalContent::KeyBindings { title, sections } => {
                self.render_keybindings(frame, modal_area, title, sections);
            }
            ModalContent::FilePicker {
                title,
                items,
                filtered_indices,
                query,
                selected,
            } => {
                self.render_file_picker(
                    frame,
                    modal_area,
                    title,
                    items,
                    filtered_indices,
                    query,
                    *selected,
                );
            }
            ModalContent::CommitInput {
                title,
                message,
                cursor_pos,
                files_to_commit,
            } => {
                self.render_commit_input(
                    frame,
                    modal_area,
                    title,
                    message,
                    *cursor_pos,
                    files_to_commit,
                );
            }
        }
    }

    fn render_info(&self, frame: &mut Frame, area: Rect, title: &str, message: &str) {
        let t = theme::get();
        let block = Block::default()
            .title(format!(" {} ", title))
            .title_style(Style::default().fg(t.ui.border_focused).bold())
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(t.ui.border_unfocused));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let lines: Vec<Line> = message
            .lines()
            .map(|line| Line::from(Span::styled(line, Style::default().fg(t.ui.text_primary))))
            .collect();

        let para = Paragraph::new(lines);
        frame.render_widget(para, inner);
    }

    fn render_select(
        &self,
        frame: &mut Frame,
        area: Rect,
        title: &str,
        items: &[String],
        selected: usize,
    ) {
        let t = theme::get();
        let block = Block::default()
            .title(format!(" {} ", title))
            .title_style(Style::default().fg(t.ui.border_focused).bold())
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(t.ui.border_unfocused));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let list_items: Vec<ListItem> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let style = if i == selected {
                    Style::default().fg(t.ui.selection_fg).bg(t.ui.selection_bg)
                } else {
                    Style::default().fg(t.ui.text_primary)
                };
                ListItem::new(format!("  {} ", item)).style(style)
            })
            .collect();

        let list = List::new(list_items);
        frame.render_widget(list, inner);
    }

    fn render_keybindings(
        &self,
        frame: &mut Frame,
        area: Rect,
        title: &str,
        sections: &[KeyBindSection],
    ) {
        let t = theme::get();
        let block = Block::default()
            .title(format!(" {} ", title))
            .title_style(Style::default().fg(t.ui.border_focused).bold())
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(t.ui.border_unfocused));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let key_width = sections
            .iter()
            .flat_map(|s| s.bindings.iter())
            .map(|b| b.key.len())
            .max()
            .unwrap_or(0);

        let mut lines: Vec<Line> = Vec::new();
        for (i, section) in sections.iter().enumerate() {
            if i > 0 {
                lines.push(Line::from(""));
            }
            let section_label = format!("[{}]", section.title);
            lines.push(Line::from(Span::styled(
                format!("{:>width$}", section_label, width = key_width),
                Style::default().fg(t.ui.highlight).bold(),
            )));
            for bind in &section.bindings {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{:>width$}", bind.key, width = key_width),
                        Style::default().fg(t.ui.status_added),
                    ),
                    Span::styled("   ", Style::default()),
                    Span::styled(bind.description, Style::default().fg(t.ui.text_primary)),
                ]));
            }
        }

        let para = Paragraph::new(lines);
        frame.render_widget(para, inner);
    }

    #[allow(clippy::too_many_arguments)]
    fn render_file_picker(
        &self,
        frame: &mut Frame,
        area: Rect,
        title: &str,
        items: &[FilePickerItem],
        filtered_indices: &[usize],
        query: &str,
        selected: usize,
    ) {
        let t = theme::get();
        let block = Block::default()
            .title(format!(" {} ", title))
            .title_style(Style::default().fg(t.ui.border_focused).bold())
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(t.ui.border_unfocused));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        use ratatui::layout::{Constraint, Direction, Layout};
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(inner);

        let input_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(t.ui.status_added)),
            Span::styled(query, Style::default().fg(t.ui.text_primary)),
            Span::styled("_", Style::default().fg(t.ui.text_muted)),
        ]);
        frame.render_widget(Paragraph::new(input_line), chunks[0]);

        let visible_count = chunks[2].height as usize;
        let scroll_offset = if selected >= visible_count {
            selected - visible_count + 1
        } else {
            0
        };

        let list_items: Vec<ListItem> = filtered_indices
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_count)
            .map(|(i, &idx)| {
                let item = &items[idx];
                let is_selected = i == selected;

                let (status_char, status_color) = match item.status {
                    FileStatus::Added => ("A", t.ui.status_added),
                    FileStatus::Modified => ("M", t.ui.status_modified),
                    FileStatus::Deleted => ("D", t.ui.status_deleted),
                };

                let viewed_char = if item.viewed { "✓" } else { " " };

                let spans = if is_selected {
                    let selected_style =
                        Style::default().fg(t.ui.selection_fg).bg(t.ui.selection_bg);
                    vec![
                        Span::styled(format!(" {} ", viewed_char), selected_style),
                        Span::styled(format!("{} ", status_char), selected_style),
                        Span::styled(item.name.clone(), selected_style),
                    ]
                } else {
                    vec![
                        Span::styled(
                            format!(" {} ", viewed_char),
                            Style::default().fg(t.ui.viewed),
                        ),
                        Span::styled(
                            format!("{} ", status_char),
                            Style::default().fg(status_color),
                        ),
                        Span::styled(item.name.clone(), Style::default().fg(t.ui.text_primary)),
                    ]
                };

                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(list_items);
        frame.render_widget(list, chunks[2]);
    }

    fn render_commit_input(
        &self,
        frame: &mut Frame,
        area: Rect,
        title: &str,
        message: &str,
        cursor_pos: usize,
        files_to_commit: &[String],
    ) {
        let t = theme::get();
        let block = Block::default()
            .title(format!(" {} ", title))
            .title_style(Style::default().fg(t.ui.border_focused).bold())
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(t.ui.border_unfocused));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        use ratatui::layout::{Constraint, Direction, Layout};
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Input line
                Constraint::Length(1), // Separator
                Constraint::Min(1),    // Files list
                Constraint::Length(1), // Hint line
            ])
            .split(inner);

        // Render input line with cursor
        let (before_cursor, after_cursor) = message.split_at(cursor_pos.min(message.len()));
        let cursor_char = after_cursor.chars().next().unwrap_or(' ');
        let after_cursor_rest = if after_cursor.len() > 1 {
            &after_cursor[cursor_char.len_utf8()..]
        } else {
            ""
        };

        let input_line = Line::from(vec![
            Span::styled(before_cursor, Style::default().fg(t.ui.text_primary)),
            Span::styled(
                cursor_char.to_string(),
                Style::default().fg(t.ui.text_primary).bg(t.ui.selection_bg),
            ),
            Span::styled(after_cursor_rest, Style::default().fg(t.ui.text_primary)),
        ]);
        frame.render_widget(Paragraph::new(input_line), chunks[0]);

        // Separator
        let separator = Line::from(Span::styled(
            "─".repeat(chunks[1].width as usize),
            Style::default().fg(t.ui.border_unfocused),
        ));
        frame.render_widget(Paragraph::new(separator), chunks[1]);

        // Files list
        let visible_count = chunks[2].height as usize;
        let list_items: Vec<ListItem> = files_to_commit
            .iter()
            .take(visible_count)
            .map(|file| {
                ListItem::new(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(file.clone(), Style::default().fg(t.ui.text_muted)),
                ]))
            })
            .collect();

        if files_to_commit.len() > visible_count {
            // Show indicator that there are more files
            let remaining = files_to_commit.len() - visible_count;
            let mut items = list_items;
            if let Some(last) = items.last_mut() {
                *last = ListItem::new(Line::from(Span::styled(
                    format!("  ... and {} more files", remaining + 1),
                    Style::default().fg(t.ui.text_muted),
                )));
            }
            frame.render_widget(List::new(items), chunks[2]);
        } else {
            frame.render_widget(List::new(list_items), chunks[2]);
        }

        // Hint line
        let hint = Line::from(vec![
            Span::styled("Enter", Style::default().fg(t.ui.status_added)),
            Span::styled(": commit  ", Style::default().fg(t.ui.text_muted)),
            Span::styled("Esc", Style::default().fg(t.ui.status_deleted)),
            Span::styled(": cancel", Style::default().fg(t.ui.text_muted)),
        ]);
        frame.render_widget(Paragraph::new(hint), chunks[3]);
    }

    /// Handle keyboard input for the modal.
    /// Returns Some(ModalResult) if the modal should close.
    pub fn handle_input(&mut self, key: KeyEvent) -> Option<ModalResult> {
        // FilePicker and CommitInput handle their own dismiss logic (need to allow typing 'q')
        if !matches!(
            self.content,
            ModalContent::FilePicker { .. } | ModalContent::CommitInput { .. }
        ) {
            // Close on Esc, q, or Ctrl+C
            if key.code == KeyCode::Esc
                || key.code == KeyCode::Char('q')
                || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
            {
                return Some(ModalResult::Dismissed);
            }
        }

        match &mut self.content {
            ModalContent::Info { .. } => {
                // Any key closes info modal
                if key.code == KeyCode::Enter {
                    return Some(ModalResult::Dismissed);
                }
                None
            }
            ModalContent::Select {
                items, selected, ..
            } => match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    if *selected < items.len().saturating_sub(1) {
                        *selected += 1;
                    }
                    None
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    *selected = selected.saturating_sub(1);
                    None
                }
                KeyCode::Enter => {
                    let idx = *selected;
                    let value = items.get(idx).cloned().unwrap_or_default();
                    Some(ModalResult::Selected(idx, value))
                }
                _ => None,
            },
            ModalContent::KeyBindings { .. } => {
                if key.code == KeyCode::Enter {
                    return Some(ModalResult::Dismissed);
                }
                None
            }
            ModalContent::FilePicker {
                items,
                filtered_indices,
                query,
                selected,
                ..
            } => match key.code {
                KeyCode::Esc => Some(ModalResult::Dismissed),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(ModalResult::Dismissed)
                }
                KeyCode::Down | KeyCode::Char('j')
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        || key.code == KeyCode::Down =>
                {
                    if *selected < filtered_indices.len().saturating_sub(1) {
                        *selected += 1;
                    }
                    None
                }
                KeyCode::Up | KeyCode::Char('k')
                    if key.modifiers.contains(KeyModifiers::CONTROL) || key.code == KeyCode::Up =>
                {
                    *selected = selected.saturating_sub(1);
                    None
                }
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if *selected < filtered_indices.len().saturating_sub(1) {
                        *selected += 1;
                    }
                    None
                }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    *selected = selected.saturating_sub(1);
                    None
                }
                KeyCode::Enter => {
                    if let Some(&file_idx) = filtered_indices.get(*selected) {
                        Some(ModalResult::FileSelected(items[file_idx].file_index))
                    } else {
                        Some(ModalResult::Dismissed)
                    }
                }
                KeyCode::Backspace => {
                    query.pop();
                    Self::update_filtered_indices(items, query, filtered_indices, selected);
                    None
                }
                KeyCode::Char(c) => {
                    query.push(c);
                    Self::update_filtered_indices(items, query, filtered_indices, selected);
                    None
                }
                _ => None,
            },
            ModalContent::CommitInput {
                message,
                cursor_pos,
                ..
            } => {
                // macOS-style keybinds for text editing
                match key.code {
                    KeyCode::Esc => Some(ModalResult::Dismissed),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        Some(ModalResult::Dismissed)
                    }
                    KeyCode::Enter => {
                        if message.trim().is_empty() {
                            // Don't allow empty commit messages
                            None
                        } else {
                            Some(ModalResult::CommitConfirmed(message.clone()))
                        }
                    }
                    // Cmd+Backspace (or Ctrl+U): delete to beginning of line
                    KeyCode::Backspace if key.modifiers.contains(KeyModifiers::SUPER) => {
                        message.drain(..*cursor_pos);
                        *cursor_pos = 0;
                        None
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        message.drain(..*cursor_pos);
                        *cursor_pos = 0;
                        None
                    }
                    // Option+Backspace (Alt+Backspace): delete word backwards
                    KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
                        if *cursor_pos > 0 {
                            let before = &message[..*cursor_pos];
                            // Find start of previous word
                            let word_start = before
                                .trim_end()
                                .rfind(|c: char| c.is_whitespace())
                                .map(|i| i + 1)
                                .unwrap_or(0);
                            message.drain(word_start..*cursor_pos);
                            *cursor_pos = word_start;
                        }
                        None
                    }
                    // Ctrl+W: delete word backwards (unix style)
                    KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if *cursor_pos > 0 {
                            let before = &message[..*cursor_pos];
                            let word_start = before
                                .trim_end()
                                .rfind(|c: char| c.is_whitespace())
                                .map(|i| i + 1)
                                .unwrap_or(0);
                            message.drain(word_start..*cursor_pos);
                            *cursor_pos = word_start;
                        }
                        None
                    }
                    // Regular backspace
                    KeyCode::Backspace => {
                        if *cursor_pos > 0 {
                            // Find the byte index of the previous character
                            let prev_char_boundary = message[..*cursor_pos]
                                .char_indices()
                                .last()
                                .map(|(i, _)| i)
                                .unwrap_or(0);
                            message.drain(prev_char_boundary..*cursor_pos);
                            *cursor_pos = prev_char_boundary;
                        }
                        None
                    }
                    // Delete forward
                    KeyCode::Delete => {
                        if *cursor_pos < message.len() {
                            let next_char_len = message[*cursor_pos..]
                                .chars()
                                .next()
                                .map(|c| c.len_utf8())
                                .unwrap_or(0);
                            message.drain(*cursor_pos..*cursor_pos + next_char_len);
                        }
                        None
                    }
                    // Cmd+Left (or Home): move to beginning
                    KeyCode::Left if key.modifiers.contains(KeyModifiers::SUPER) => {
                        *cursor_pos = 0;
                        None
                    }
                    KeyCode::Home => {
                        *cursor_pos = 0;
                        None
                    }
                    KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        *cursor_pos = 0;
                        None
                    }
                    // Cmd+Right (or End): move to end
                    KeyCode::Right if key.modifiers.contains(KeyModifiers::SUPER) => {
                        *cursor_pos = message.len();
                        None
                    }
                    KeyCode::End => {
                        *cursor_pos = message.len();
                        None
                    }
                    KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        *cursor_pos = message.len();
                        None
                    }
                    // Option+Left (Alt+Left): move word backwards
                    KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
                        if *cursor_pos > 0 {
                            let before = &message[..*cursor_pos];
                            *cursor_pos = before
                                .trim_end()
                                .rfind(|c: char| c.is_whitespace())
                                .map(|i| i + 1)
                                .unwrap_or(0);
                        }
                        None
                    }
                    KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
                        if *cursor_pos > 0 {
                            let before = &message[..*cursor_pos];
                            *cursor_pos = before
                                .trim_end()
                                .rfind(|c: char| c.is_whitespace())
                                .map(|i| i + 1)
                                .unwrap_or(0);
                        }
                        None
                    }
                    // Option+Right (Alt+Right): move word forwards
                    KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => {
                        if *cursor_pos < message.len() {
                            let after = &message[*cursor_pos..];
                            let word_end = after
                                .trim_start()
                                .find(|c: char| c.is_whitespace())
                                .map(|i| *cursor_pos + after.len() - after.trim_start().len() + i)
                                .unwrap_or(message.len());
                            *cursor_pos = word_end;
                        }
                        None
                    }
                    KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
                        if *cursor_pos < message.len() {
                            let after = &message[*cursor_pos..];
                            let word_end = after
                                .trim_start()
                                .find(|c: char| c.is_whitespace())
                                .map(|i| *cursor_pos + after.len() - after.trim_start().len() + i)
                                .unwrap_or(message.len());
                            *cursor_pos = word_end;
                        }
                        None
                    }
                    // Regular Left: move one character back
                    KeyCode::Left => {
                        if *cursor_pos > 0 {
                            *cursor_pos = message[..*cursor_pos]
                                .char_indices()
                                .last()
                                .map(|(i, _)| i)
                                .unwrap_or(0);
                        }
                        None
                    }
                    // Regular Right: move one character forward
                    KeyCode::Right => {
                        if *cursor_pos < message.len() {
                            let next_char_len = message[*cursor_pos..]
                                .chars()
                                .next()
                                .map(|c| c.len_utf8())
                                .unwrap_or(0);
                            *cursor_pos += next_char_len;
                        }
                        None
                    }
                    // Type character
                    KeyCode::Char(c) => {
                        message.insert(*cursor_pos, c);
                        *cursor_pos += c.len_utf8();
                        None
                    }
                    _ => None,
                }
            }
        }
    }

    fn update_filtered_indices(
        items: &[FilePickerItem],
        query: &str,
        filtered_indices: &mut Vec<usize>,
        selected: &mut usize,
    ) {
        let query_lower = query.to_lowercase();
        *filtered_indices = items
            .iter()
            .enumerate()
            .filter(|(_, item)| fuzzy_match(&item.name.to_lowercase(), &query_lower))
            .map(|(i, _)| i)
            .collect();
        if *selected >= filtered_indices.len() {
            *selected = filtered_indices.len().saturating_sub(1);
        }
    }
}

fn fuzzy_match(text: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    let mut pattern_chars = pattern.chars().peekable();
    for c in text.chars() {
        if pattern_chars.peek() == Some(&c) {
            pattern_chars.next();
        }
        if pattern_chars.peek().is_none() {
            return true;
        }
    }
    pattern_chars.peek().is_none()
}
