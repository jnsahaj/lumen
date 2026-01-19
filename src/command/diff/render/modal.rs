use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::command::diff::state::HunkAnnotation;
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
    Annotations {
        title: String,
        items: Vec<String>,
        annotations: Vec<HunkAnnotation>,
        selected: usize,
        export_input: Option<String>,
        /// Error message to display (e.g., for failed export)
        error_message: Option<String>,
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
    AnnotationJump { file_index: usize, hunk_index: usize },
    AnnotationEdit { file_index: usize, hunk_index: usize },
    AnnotationDelete { file_index: usize, hunk_index: usize },
    AnnotationCopyAll,
    AnnotationExport(String),
    /// Export failed with an error message - keeps modal open to retry
    AnnotationExportError(String),
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

    pub fn annotations(
        title: impl Into<String>,
        items: Vec<String>,
        annotations: Vec<HunkAnnotation>,
    ) -> Self {
        Self {
            content: ModalContent::Annotations {
                title: title.into(),
                items,
                annotations,
                selected: 0,
                export_input: None,
                error_message: None,
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
            ModalContent::Annotations {
                items, export_input, ..
            } => {
                let width = 70.min(area.width.saturating_sub(4));
                let items_count = items.len().min(12) as u16;
                // Compact height
                let extra = if export_input.is_some() { 4 } else { 2 };
                let height = (items_count + extra + 2).min(area.height * 80 / 100).max(8);
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
            ModalContent::Annotations {
                title,
                items,
                selected,
                export_input,
                error_message,
                ..
            } => {
                self.render_annotations(frame, modal_area, title, items, *selected, export_input.as_deref(), error_message.as_deref());
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

    fn render_annotations(
        &self,
        frame: &mut Frame,
        area: Rect,
        title: &str,
        items: &[String],
        selected: usize,
        export_input: Option<&str>,
        error_message: Option<&str>,
    ) {
        let t = theme::get();

        // Compact title with count
        let title_text = format!(" {} ({}) ", title, items.len());

        let block = Block::default()
            .title(title_text)
            .title_style(Style::default().fg(t.ui.text_secondary))
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(t.ui.border_focused));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        use ratatui::layout::{Constraint, Direction, Layout};

        // Different layout based on export input state
        let (list_area, footer_area) = if export_input.is_some() {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(1),
                    Constraint::Length(3),
                    Constraint::Length(1),
                ])
                .split(inner);
            (chunks[0], chunks[2])
        } else {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(inner);
            (chunks[0], chunks[1])
        };

        // Render annotations list
        let visible_count = list_area.height as usize;
        let scroll_offset = if selected >= visible_count {
            selected - visible_count + 1
        } else {
            0
        };

        let list_items: Vec<ListItem> = items
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_count)
            .map(|(i, item)| {
                let is_selected = i == selected;

                // Parse the item to extract filename and preview
                // Format is: "filename:L#-# | preview..."
                let (location, preview) = item.split_once(" | ").unwrap_or((item, ""));

                let spans = if is_selected {
                    vec![
                        Span::styled(
                            format!(" {} ", location),
                            Style::default().fg(t.ui.selection_fg).bg(t.ui.selection_bg),
                        ),
                        Span::styled(
                            format!(" {}", preview),
                            Style::default()
                                .fg(t.ui.selection_fg)
                                .bg(t.ui.selection_bg)
                                .italic(),
                        ),
                    ]
                } else {
                    vec![
                        Span::styled(
                            format!(" {} ", location),
                            Style::default().fg(t.ui.text_secondary),
                        ),
                        Span::styled(
                            format!(" {}", preview),
                            Style::default().fg(t.ui.text_muted).italic(),
                        ),
                    ]
                };

                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(list_items);
        frame.render_widget(list, list_area);

        // Render export input if active
        if let Some(input) = export_input {
            let input_area = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(1),
                    Constraint::Length(3),
                    Constraint::Length(1),
                ])
                .split(inner)[1];

            let input_block = Block::default()
                .title(" Export path ")
                .title_style(Style::default().fg(t.ui.text_muted))
                .borders(Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(Style::default().fg(t.ui.border_unfocused));

            let input_inner = input_block.inner(input_area);
            frame.render_widget(input_block, input_area);

            let input_line = Line::from(vec![
                Span::styled(input, Style::default().fg(t.ui.text_primary)),
                Span::styled("_", Style::default().fg(t.ui.text_muted)),
            ]);
            frame.render_widget(Paragraph::new(input_line), input_inner);
        }

        // Display error message if present
        if let Some(error) = error_message {
            // Show error above footer
            let error_line = Line::from(vec![
                Span::styled("Error: ", Style::default().fg(t.ui.status_deleted).bold()),
                Span::styled(error, Style::default().fg(t.ui.status_deleted)),
            ]);
            let error_para = Paragraph::new(error_line).alignment(ratatui::prelude::Alignment::Center);
            // Render error in the list area's last line
            let error_area = Rect::new(list_area.x, list_area.y + list_area.height.saturating_sub(1), list_area.width, 1);
            frame.render_widget(error_para, error_area);
        }

        // Compact footer
        let footer_text = if export_input.is_some() {
            Line::from(vec![
                Span::styled("enter", Style::default().fg(t.ui.text_muted)),
                Span::styled(" confirm  ", Style::default().fg(t.ui.text_muted)),
                Span::styled("│  ", Style::default().fg(t.ui.border_unfocused)),
                Span::styled("esc", Style::default().fg(t.ui.text_muted)),
                Span::styled(" cancel", Style::default().fg(t.ui.text_muted)),
            ])
        } else {
            Line::from(vec![
                Span::styled("enter", Style::default().fg(t.ui.text_muted)),
                Span::styled(" jump  ", Style::default().fg(t.ui.text_muted)),
                Span::styled("│  ", Style::default().fg(t.ui.border_unfocused)),
                Span::styled("e", Style::default().fg(t.ui.text_muted)),
                Span::styled(" edit  ", Style::default().fg(t.ui.text_muted)),
                Span::styled("│  ", Style::default().fg(t.ui.border_unfocused)),
                Span::styled("d", Style::default().fg(t.ui.text_muted)),
                Span::styled(" del  ", Style::default().fg(t.ui.text_muted)),
                Span::styled("│  ", Style::default().fg(t.ui.border_unfocused)),
                Span::styled("y", Style::default().fg(t.ui.text_muted)),
                Span::styled(" copy  ", Style::default().fg(t.ui.text_muted)),
                Span::styled("│  ", Style::default().fg(t.ui.border_unfocused)),
                Span::styled("o", Style::default().fg(t.ui.text_muted)),
                Span::styled(" export", Style::default().fg(t.ui.text_muted)),
            ])
        };
        let footer = Paragraph::new(footer_text).alignment(ratatui::prelude::Alignment::Center);
        frame.render_widget(footer, footer_area);
    }

    /// Handle keyboard input for the modal.
    /// Returns Some(ModalResult) if the modal should close.
    pub fn handle_input(&mut self, key: KeyEvent) -> Option<ModalResult> {
        // FilePicker and Annotations handle their own dismiss logic
        if !matches!(
            self.content,
            ModalContent::FilePicker { .. } | ModalContent::Annotations { .. }
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
            ModalContent::Annotations {
                items,
                annotations,
                selected,
                export_input,
                error_message,
                ..
            } => {
                // Export input mode
                if let Some(ref mut input) = export_input {
                    match key.code {
                        KeyCode::Esc => {
                            *export_input = None;
                            *error_message = None;
                            None
                        }
                        KeyCode::Enter => {
                            let filename = input.trim();
                            // Basic path validation
                            if filename.is_empty() {
                                *error_message = Some("Path cannot be empty".to_string());
                                return None;
                            }
                            if filename.contains("..") {
                                *error_message = Some("Path cannot contain '..'".to_string());
                                return None;
                            }
                            // Clear any previous error and proceed
                            *error_message = None;
                            Some(ModalResult::AnnotationExport(filename.to_string()))
                        }
                        KeyCode::Backspace => {
                            input.pop();
                            *error_message = None; // Clear error on edit
                            None
                        }
                        KeyCode::Char(c) => {
                            input.push(c);
                            *error_message = None; // Clear error on edit
                            None
                        }
                        _ => None,
                    }
                } else {
                    // Normal mode
                    match key.code {
                        KeyCode::Esc
                        | KeyCode::Char('q')
                        | KeyCode::Char('c')
                            if key.code == KeyCode::Esc
                                || key.code == KeyCode::Char('q')
                                || key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            Some(ModalResult::Dismissed)
                        }
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
                        KeyCode::Enter => annotations.get(*selected).map(|ann| {
                            ModalResult::AnnotationJump {
                                file_index: ann.file_index,
                                hunk_index: ann.hunk_index,
                            }
                        }),
                        KeyCode::Char('e') => annotations.get(*selected).map(|ann| {
                            ModalResult::AnnotationEdit {
                                file_index: ann.file_index,
                                hunk_index: ann.hunk_index,
                            }
                        }),
                        KeyCode::Char('d') => annotations.get(*selected).map(|ann| {
                            ModalResult::AnnotationDelete {
                                file_index: ann.file_index,
                                hunk_index: ann.hunk_index,
                            }
                        }),
                        KeyCode::Char('y') => Some(ModalResult::AnnotationCopyAll),
                        KeyCode::Char('o') => {
                            *export_input = Some(String::from("annotations.txt"));
                            None
                        }
                        _ => None,
                    }
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
