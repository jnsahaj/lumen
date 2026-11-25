mod global_search;
pub mod thread;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::{
    prelude::*,
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState,
    },
};

use crate::diff::FileStatus;
use crate::review::Thread;
use crate::search::global::GlobalSearchState;
use crate::search::{erase_word_backward, MatchPanel};
use crate::ui::editor::AnnotationEditor;
use crate::ui::keybindings::KeyBindSection;
use crate::ui::markdown::truncate_with_ellipsis;
use crate::ui::theme;

pub use thread::ThreadViewState;

#[derive(Clone)]
pub struct FilePickerItem {
    pub name: String,
    pub file_index: usize,
    pub status: FileStatus,
    pub viewed: bool,
}

pub enum ModalContent {
    Info {
        title: String,
        message: String,
    },
    Confirm {
        title: String,
        message: String,
    },
    Select {
        title: String,
        items: Vec<String>,
        selected: usize,
    },
    KeyBindings {
        title: String,
        sections: Vec<KeyBindSection>,
        scroll: u16,
        content_height: u16,
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
        annotations: Vec<Thread>,
        selected: usize,
    },
    ThreadView(ThreadViewState),
    GlobalSearch {
        title: String,
        state: Box<GlobalSearchState>,
    },
}

pub struct Modal {
    pub content: ModalContent,
}

#[derive(Clone)]
pub enum ModalResult {
    Dismissed,
    Confirmed,
    Selected(usize, String),
    FileSelected(usize),
    JumpToLine {
        file_index: usize,
        line_no: usize,
        panel: MatchPanel,
    },
    AnnotationOpen {
        annotation_id: String,
    },
    AnnotationJump {
        annotation_id: String,
    },
    AnnotationEdit {
        annotation_id: String,
    },
    AnnotationDelete {
        annotation_id: String,
    },
    AnnotationToggleResolved {
        annotation_id: String,
    },
    AnnotationsToggleResolvedShown {
        selected_id: Option<String>,
    },
    ThreadReply {
        thread_id: String,
    },
    ThreadToggleResolved {
        thread_id: String,
    },
}

const KEYBINDINGS_GAP: &str = "   ";

fn keybindings_key_width(sections: &[KeyBindSection]) -> usize {
    sections
        .iter()
        .flat_map(|section| section.bindings.iter())
        .map(|bind| bind.key.len())
        .max()
        .unwrap_or(0)
}

fn rounded_block(title: &str, focused_border: bool) -> Block<'static> {
    let t = theme::get();
    let (title_style, border_color) = if focused_border {
        (
            Style::default().fg(t.ui.text_secondary),
            t.ui.border_focused,
        )
    } else {
        (
            Style::default().fg(t.ui.border_focused).bold(),
            t.ui.border_unfocused,
        )
    };
    Block::default()
        .title(format!(" {title} "))
        .title_style(title_style)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
}

impl Modal {
    pub fn info(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            content: ModalContent::Info {
                title: title.into(),
                message: message.into(),
            },
        }
    }

    pub fn confirm(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            content: ModalContent::Confirm {
                title: title.into(),
                message: message.into(),
            },
        }
    }

    pub fn select(title: impl Into<String>, items: Vec<String>, selected: usize) -> Self {
        Self {
            content: ModalContent::Select {
                title: title.into(),
                items,
                selected,
            },
        }
    }

    pub fn keybindings(title: impl Into<String>, sections: Vec<KeyBindSection>) -> Self {
        let content_height: u16 = sections.iter().map(|s| s.bindings.len() as u16 + 2).sum();
        Self {
            content: ModalContent::KeyBindings {
                title: title.into(),
                sections,
                scroll: 0,
                content_height,
            },
        }
    }

    pub fn file_picker(title: impl Into<String>, items: Vec<FilePickerItem>) -> Self {
        let filtered_indices = (0..items.len()).collect();
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

    pub fn global_search(title: impl Into<String>, state: GlobalSearchState) -> Self {
        Self {
            content: ModalContent::GlobalSearch {
                title: title.into(),
                state: Box::new(state),
            },
        }
    }

    pub fn annotations(
        title: impl Into<String>,
        items: Vec<String>,
        annotations: Vec<Thread>,
        selected: usize,
    ) -> Self {
        Self {
            content: ModalContent::Annotations {
                title: title.into(),
                items,
                annotations,
                selected,
            },
        }
    }

    pub fn thread_view(thread_id: String, tab_width: usize, return_to_list: bool) -> Self {
        Self {
            content: ModalContent::ThreadView(ThreadViewState::new(
                thread_id,
                tab_width,
                return_to_list,
            )),
        }
    }

    pub fn is_thread_view(&self) -> bool {
        matches!(self.content, ModalContent::ThreadView(_))
    }

    pub fn pin_thread_to_bottom(&mut self) {
        if let ModalContent::ThreadView(view) = &mut self.content {
            view.pinned_to_bottom = true;
        }
    }

    pub fn editor_rect(&self) -> Option<Rect> {
        match &self.content {
            ModalContent::ThreadView(view) => view.last_editor_rect.get(),
            _ => None,
        }
    }

    pub fn render(&self, frame: &mut Frame, threads: &[Thread], editor: Option<&AnnotationEditor>) {
        let area = frame.area();

        if let ModalContent::GlobalSearch { title, state } = &self.content {
            frame.render_widget(Clear, area);
            global_search::render_global_search(frame, area, title, state);
            return;
        }

        let (modal_width, modal_height) = match &self.content {
            ModalContent::Info { message, .. } => {
                let width = 80.min(area.width.saturating_sub(4));
                let lines = message.lines().count() as u16;
                let height = (lines + 4).min(area.height * 80 / 100).max(5);
                (width, height)
            }
            ModalContent::Confirm { message, .. } => {
                let width = 80.min(area.width.saturating_sub(4));
                let lines = message.lines().count() as u16;
                let height = (lines + 5).min(area.height * 80 / 100).max(6);
                (width, height)
            }
            ModalContent::Select { items, .. } => {
                let width = 80.min(area.width.saturating_sub(4));
                let height = (items.len() as u16 + 4).min(area.height * 80 / 100).max(5);
                (width, height)
            }
            ModalContent::KeyBindings { sections, .. } => {
                let widest_row = sections
                    .iter()
                    .flat_map(|section| section.bindings.iter())
                    .map(|bind| bind.description.chars().count())
                    .max()
                    .unwrap_or(0)
                    + keybindings_key_width(sections)
                    + KEYBINDINGS_GAP.len();
                let width = (widest_row as u16 + 4).min(area.width.saturating_sub(4));
                let total_lines: usize = sections.iter().map(|s| s.bindings.len() + 2).sum();
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
            ModalContent::Annotations { items, .. } => {
                let width = 100.min(area.width.saturating_sub(4));
                let items_count = items.len().min(12) as u16;
                let height = (items_count + 4).min(area.height * 80 / 100).max(8);
                (width, height)
            }
            ModalContent::ThreadView(_) => {
                let width = (area.width * 85 / 100)
                    .max(50)
                    .min(area.width.saturating_sub(4));
                let height = (area.height * 85 / 100)
                    .max(12)
                    .min(area.height.saturating_sub(2));
                (width, height)
            }
            ModalContent::GlobalSearch { .. } => unreachable!(),
        };

        let modal_x = area.width.saturating_sub(modal_width) / 2;
        let modal_y = area.height.saturating_sub(modal_height) / 2;
        let modal_area = Rect::new(modal_x, modal_y, modal_width, modal_height);

        frame.render_widget(Clear, modal_area);

        match &self.content {
            ModalContent::Info { title, message } => {
                render_info(frame, modal_area, title, message);
            }
            ModalContent::Confirm { title, message } => {
                render_confirm(frame, modal_area, title, message);
            }
            ModalContent::Select {
                title,
                items,
                selected,
            } => {
                render_select(frame, modal_area, title, items, *selected);
            }
            ModalContent::KeyBindings {
                title,
                sections,
                scroll,
                content_height,
            } => {
                render_keybindings(frame, modal_area, title, sections, *scroll, *content_height);
            }
            ModalContent::FilePicker {
                title,
                items,
                filtered_indices,
                query,
                selected,
            } => {
                render_file_picker(
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
                ..
            } => {
                render_annotations(frame, modal_area, title, items, *selected);
            }
            ModalContent::ThreadView(view) => {
                if let Some(thread) = threads.iter().find(|t| t.id == view.thread_id) {
                    view.render(frame, modal_area, thread, editor);
                } else {
                    view.last_max_scroll.set(0);
                    view.last_visible_height.set(0);
                    view.last_editor_rect.set(None);
                }
            }
            ModalContent::GlobalSearch { .. } => unreachable!(),
        }
    }

    pub fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        terminal_width: u16,
        terminal_height: u16,
    ) -> bool {
        match &mut self.content {
            ModalContent::KeyBindings {
                scroll,
                content_height,
                ..
            } => {
                let visible_height =
                    calculate_keybindings_visible_height(terminal_height, *content_height);
                let max_scroll = content_height.saturating_sub(visible_height);
                match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        *scroll = (*scroll + 3).min(max_scroll);
                        true
                    }
                    MouseEventKind::ScrollUp => {
                        *scroll = scroll.saturating_sub(3);
                        true
                    }
                    _ => false,
                }
            }
            ModalContent::ThreadView(view) => view.handle_mouse(mouse),
            ModalContent::GlobalSearch { state, .. } => global_search::handle_global_search_mouse(
                state,
                mouse,
                terminal_width,
                terminal_height,
            ),
            _ => false,
        }
    }

    pub fn handle_input(&mut self, key: KeyEvent, terminal_height: u16) -> Option<ModalResult> {
        if !matches!(
            self.content,
            ModalContent::FilePicker { .. } | ModalContent::GlobalSearch { .. }
        ) && (key.code == KeyCode::Esc
            || key.code == KeyCode::Char('q')
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)))
        {
            return Some(ModalResult::Dismissed);
        }

        match &mut self.content {
            ModalContent::Info { .. } => {
                (key.code == KeyCode::Enter).then_some(ModalResult::Dismissed)
            }
            ModalContent::Confirm { .. } => {
                (key.code == KeyCode::Enter).then_some(ModalResult::Confirmed)
            }
            ModalContent::Select {
                items, selected, ..
            } => match key.code {
                KeyCode::Down | KeyCode::Char('k') => {
                    if *selected < items.len().saturating_sub(1) {
                        *selected += 1;
                    }
                    None
                }
                KeyCode::Up | KeyCode::Char('j') => {
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
            ModalContent::KeyBindings {
                scroll,
                content_height,
                ..
            } => {
                let visible_height =
                    calculate_keybindings_visible_height(terminal_height, *content_height);
                let max_scroll = content_height.saturating_sub(visible_height);

                match key.code {
                    KeyCode::Enter => Some(ModalResult::Dismissed),
                    KeyCode::Down | KeyCode::Char('k') => {
                        *scroll = (*scroll + 1).min(max_scroll);
                        None
                    }
                    KeyCode::Up | KeyCode::Char('j') => {
                        *scroll = scroll.saturating_sub(1);
                        None
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        *scroll = (*scroll + visible_height / 2).min(max_scroll);
                        None
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        *scroll = scroll.saturating_sub(visible_height / 2);
                        None
                    }
                    KeyCode::Char('g') => {
                        *scroll = 0;
                        None
                    }
                    KeyCode::Char('G') => {
                        *scroll = max_scroll;
                        None
                    }
                    _ => None,
                }
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
                KeyCode::Down | KeyCode::Char('k')
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        || key.code == KeyCode::Down =>
                {
                    if *selected < filtered_indices.len().saturating_sub(1) {
                        *selected += 1;
                    }
                    None
                }
                KeyCode::Up | KeyCode::Char('j')
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
                KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
                    erase_word_backward(query);
                    update_filtered_indices(items, query, filtered_indices, selected);
                    None
                }
                KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    erase_word_backward(query);
                    update_filtered_indices(items, query, filtered_indices, selected);
                    None
                }
                KeyCode::Backspace => {
                    query.pop();
                    update_filtered_indices(items, query, filtered_indices, selected);
                    None
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::ALT)
                        && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    query.push(c);
                    update_filtered_indices(items, query, filtered_indices, selected);
                    None
                }
                _ => None,
            },
            ModalContent::Annotations {
                items,
                annotations,
                selected,
                ..
            } => {
                match key.code {
                    KeyCode::Down | KeyCode::Char('k') => {
                        if *selected < items.len().saturating_sub(1) {
                            *selected += 1;
                        }
                        None
                    }
                    KeyCode::Up | KeyCode::Char('j') => {
                        *selected = selected.saturating_sub(1);
                        None
                    }
                    KeyCode::Enter => {
                        annotations
                            .get(*selected)
                            .map(|ann| ModalResult::AnnotationOpen {
                                annotation_id: ann.id.clone(),
                            })
                    }
                    KeyCode::Char('g') => {
                        annotations
                            .get(*selected)
                            .map(|ann| ModalResult::AnnotationJump {
                                annotation_id: ann.id.clone(),
                            })
                    }
                    KeyCode::Char('e') => {
                        annotations
                            .get(*selected)
                            .map(|ann| ModalResult::AnnotationEdit {
                                annotation_id: ann.id.clone(),
                            })
                    }
                    KeyCode::Char('d') => {
                        annotations
                            .get(*selected)
                            .map(|ann| ModalResult::AnnotationDelete {
                                annotation_id: ann.id.clone(),
                            })
                    }
                    KeyCode::Char('x') => annotations.get(*selected).map(|ann| {
                        ModalResult::AnnotationToggleResolved {
                            annotation_id: ann.id.clone(),
                        }
                    }),
                    KeyCode::Char('r') => Some(ModalResult::AnnotationsToggleResolvedShown {
                        selected_id: annotations.get(*selected).map(|ann| ann.id.clone()),
                    }),
                    _ => None,
                }
            }
            ModalContent::ThreadView(view) => view.handle_input(key),
            ModalContent::GlobalSearch { state, .. } => {
                global_search::handle_global_search_input(state, key, terminal_height)
            }
        }
    }
}

fn render_info(frame: &mut Frame, area: Rect, title: &str, message: &str) {
    let t = theme::get();
    let block = rounded_block(title, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = message
        .lines()
        .map(|line| Line::from(Span::styled(line, Style::default().fg(t.ui.text_primary))))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_confirm(frame: &mut Frame, area: Rect, title: &str, message: &str) {
    let t = theme::get();
    let block = rounded_block(title, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = message
        .lines()
        .map(|line| Line::from(Span::styled(line, Style::default().fg(t.ui.text_primary))))
        .collect();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("enter", Style::default().fg(t.ui.border_focused).bold()),
        Span::styled(" confirm  ", Style::default().fg(t.ui.text_muted)),
        Span::styled("esc", Style::default().fg(t.ui.border_focused).bold()),
        Span::styled(" cancel", Style::default().fg(t.ui.text_muted)),
    ]));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_select(frame: &mut Frame, area: Rect, title: &str, items: &[String], selected: usize) {
    let t = theme::get();
    let block = rounded_block(title, false);
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
            ListItem::new(format!("  {item} ")).style(style)
        })
        .collect();
    frame.render_widget(List::new(list_items), inner);
}

fn render_keybindings(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    sections: &[KeyBindSection],
    scroll: u16,
    content_height: u16,
) {
    let t = theme::get();
    let block = rounded_block(title, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let key_width = keybindings_key_width(sections);
    let mut lines: Vec<Line> = Vec::new();
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        let section_label = format!("[{}]", section.title);
        lines.push(Line::from(Span::styled(
            format!("{section_label:>key_width$}"),
            Style::default().fg(t.ui.highlight).bold(),
        )));
        for bind in &section.bindings {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:>key_width$}", bind.key),
                    Style::default().fg(t.ui.status_added),
                ),
                Span::styled(KEYBINDINGS_GAP, Style::default()),
                Span::styled(bind.description, Style::default().fg(t.ui.text_primary)),
            ]));
        }
    }

    let content_area = Rect::new(
        inner.x,
        inner.y,
        inner.width.saturating_sub(1),
        inner.height,
    );
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), content_area);

    let visible_height = inner.height;
    if content_height > visible_height {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        let mut scrollbar_state =
            ScrollbarState::new(content_height.saturating_sub(visible_height) as usize)
                .position(scroll as usize);
        frame.render_stateful_widget(scrollbar, inner, &mut scrollbar_state);
    }
}

fn render_file_picker(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    items: &[FilePickerItem],
    filtered_indices: &[usize],
    query: &str,
    selected: usize,
) {
    let t = theme::get();
    let block = rounded_block(title, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

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
            let status_color = match item.status {
                FileStatus::Added => t.ui.status_added,
                FileStatus::Modified => t.ui.status_modified,
                FileStatus::Deleted => t.ui.status_deleted,
            };
            let viewed_char = if item.viewed { "✓" } else { " " };
            let spans = if is_selected {
                let selected_style = Style::default().fg(t.ui.selection_fg).bg(t.ui.selection_bg);
                vec![
                    Span::styled(format!(" {viewed_char} "), selected_style),
                    Span::styled(format!("{} ", item.status.symbol()), selected_style),
                    Span::styled(item.name.clone(), selected_style),
                ]
            } else {
                vec![
                    Span::styled(format!(" {viewed_char} "), Style::default().fg(t.ui.viewed)),
                    Span::styled(
                        format!("{} ", item.status.symbol()),
                        Style::default().fg(status_color),
                    ),
                    Span::styled(item.name.clone(), Style::default().fg(t.ui.text_primary)),
                ]
            };
            ListItem::new(Line::from(spans))
        })
        .collect();

    frame.render_widget(List::new(list_items), chunks[2]);
}

fn render_annotations(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    items: &[String],
    selected: usize,
) {
    let t = theme::get();
    let block = rounded_block(title, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let (list_area, footer_area) = (chunks[0], chunks[1]);

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
            let parts: Vec<&str> = item.splitn(3, " | ").collect();
            let location = parts.first().copied().unwrap_or("");
            let preview = parts.get(1).copied().unwrap_or("");
            let time = parts.get(2).copied().unwrap_or("");

            let available_width = list_area.width as usize;
            let time_width = Span::raw(time).width() + 2;
            let content_width = available_width.saturating_sub(time_width + 4);
            let location_max = (content_width * 45 / 100)
                .max(20)
                .min(content_width.saturating_sub(20));
            let preview_max = content_width.saturating_sub(location_max);

            let truncated_location = if location_max > 3 {
                truncate_with_ellipsis(location, location_max)
            } else {
                location.to_string()
            };
            let truncated_preview = if preview_max > 3 {
                truncate_with_ellipsis(preview, preview_max)
            } else {
                preview.to_string()
            };

            let used_width = Span::raw(&truncated_location).width()
                + 2
                + Span::raw(&truncated_preview).width()
                + 1
                + time_width;
            let padding = available_width.saturating_sub(used_width);

            let spans = if is_selected {
                let sel = Style::default().fg(t.ui.selection_fg).bg(t.ui.selection_bg);
                vec![
                    Span::styled(format!(" {truncated_location} "), sel),
                    Span::styled(format!(" {truncated_preview}"), sel.italic()),
                    Span::styled(" ".repeat(padding), Style::default().bg(t.ui.selection_bg)),
                    Span::styled(format!(" {time} "), sel),
                ]
            } else {
                vec![
                    Span::styled(
                        format!(" {truncated_location} "),
                        Style::default().fg(t.ui.text_secondary),
                    ),
                    Span::styled(
                        format!(" {truncated_preview}"),
                        Style::default().fg(t.ui.text_muted).italic(),
                    ),
                    Span::raw(" ".repeat(padding)),
                    Span::styled(format!(" {time} "), Style::default().fg(t.ui.text_muted)),
                ]
            };
            ListItem::new(Line::from(spans))
        })
        .collect();

    frame.render_widget(List::new(list_items), list_area);

    let muted = Style::default().fg(t.ui.text_muted);
    let sep = Span::styled("│  ", Style::default().fg(t.ui.border_unfocused));
    let footer_text = Line::from(vec![
        Span::styled("enter", muted),
        Span::styled(" open  ", muted),
        sep.clone(),
        Span::styled("g", muted),
        Span::styled(" jump  ", muted),
        sep.clone(),
        Span::styled("e", muted),
        Span::styled(" edit/reply  ", muted),
        sep.clone(),
        Span::styled("x", muted),
        Span::styled(" resolve  ", muted),
        sep.clone(),
        Span::styled("r", muted),
        Span::styled(" resolved  ", muted),
        sep,
        Span::styled("d", muted),
        Span::styled(" del", muted),
    ]);
    frame.render_widget(
        Paragraph::new(footer_text).alignment(Alignment::Center),
        footer_area,
    );
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

fn calculate_keybindings_visible_height(terminal_height: u16, content_height: u16) -> u16 {
    let modal_height = (content_height + 4).min(terminal_height * 80 / 100).max(5);
    modal_height.saturating_sub(2)
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use crate::diff::DiffPanelFocus;
    use crate::review::{AnnotationTarget, Comment};
    use crate::ui::keybindings::KeyBind;

    fn drawn_with_editor(
        modal: &Modal,
        threads: &[Thread],
        editor: Option<&AnnotationEditor>,
        columns: u16,
        rows: u16,
    ) -> String {
        let mut terminal = Terminal::new(TestBackend::new(columns, rows)).expect("terminal");
        terminal
            .draw(|frame| modal.render(frame, threads, editor))
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn test_the_help_shows_its_longest_row_in_full_when_the_terminal_has_room() {
        let description = "Comment on the selection, the hunk or the file, or reply to a thread";
        let sections = vec![KeyBindSection {
            title: "Threads",
            bindings: vec![
                KeyBind {
                    key: "opt+down/k, opt+up/j",
                    description,
                },
                KeyBind {
                    key: "q",
                    description: "Quit",
                },
            ],
        }];
        let help = Modal::keybindings("Keybindings", sections);
        assert!(drawn_with_editor(&help, &[], None, 120, 20).contains(description));
    }

    #[test]
    fn test_thread_view_opens_at_bottom_scrolls_and_docks_editor() {
        let anchor = (1..=10)
            .map(|n| format!("let v{n} = {n};"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut thread = Thread::new(
            "src/app.rs".to_string(),
            AnnotationTarget::LineRange {
                panel: DiffPanelFocus::New,
                start_line: 10,
                end_line: 19,
            },
            Some(anchor),
            Comment::human("comment-01 opening".to_string(), false),
        );
        for i in 2..=8 {
            thread.comments.push(Comment::agent(
                format!("comment-{i:02} body line\nsecond line {i}"),
                "pi".to_string(),
            ));
        }

        let mut modal = Modal::thread_view(thread.id.clone(), 4, false);

        let initial = drawn_with_editor(&modal, std::slice::from_ref(&thread), None, 90, 20);
        assert!(initial.contains("src/app.rs:L10-19 [new] · 8 comments"));
        assert!(initial.contains("comment-08 body line"));
        assert!(!initial.contains("comment-01 opening"));
        assert!(modal.editor_rect().is_none());

        modal.handle_input(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), 20);
        let at_top = drawn_with_editor(&modal, std::slice::from_ref(&thread), None, 90, 20);
        assert!(at_top.contains("let v1 = 1;"));
        assert!(at_top.contains("let v8 = 8;"));
        assert!(!at_top.contains("let v9 = 9;"));
        assert!(at_top.contains("⋯ 2 more anchored lines"));

        modal.handle_input(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE), 20);
        let back_at_bottom = drawn_with_editor(&modal, std::slice::from_ref(&thread), None, 90, 20);
        assert!(back_at_bottom.contains("comment-08 body line"));

        let wheel_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 45,
            row: 10,
            modifiers: KeyModifiers::NONE,
        };
        assert!(modal.handle_mouse(wheel_up, 90, 20));
        let scrolled_up = drawn_with_editor(&modal, std::slice::from_ref(&thread), None, 90, 20);
        assert!(!scrolled_up.contains("second line 8"));

        assert!(matches!(
            modal.handle_input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), 20),
            Some(ModalResult::ThreadReply { ref thread_id }) if thread_id == &thread.id
        ));
        assert!(matches!(
            modal.handle_input(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), 20),
            Some(ModalResult::ThreadToggleResolved { ref thread_id }) if thread_id == &thread.id
        ));
        assert!(matches!(
            modal.handle_input(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE), 20),
            Some(ModalResult::AnnotationJump { ref annotation_id }) if annotation_id == &thread.id
        ));
        assert!(matches!(
            modal.handle_input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 20),
            Some(ModalResult::Dismissed)
        ));

        thread
            .comments
            .push(Comment::human("unsent draft text".to_string(), true));
        modal.pin_thread_to_bottom();
        let live_reply = drawn_with_editor(&modal, std::slice::from_ref(&thread), None, 90, 24);
        assert!(live_reply.contains("unsent draft text"));

        let editor = AnnotationEditor::for_thread(&thread);
        let with_editor =
            drawn_with_editor(&modal, std::slice::from_ref(&thread), Some(&editor), 90, 24);
        assert!(modal.editor_rect().is_some());
        assert!(with_editor.contains("Enter send"));
        assert!(with_editor.contains("comment-08 body line"));
        assert_eq!(with_editor.matches("unsent draft text").count(), 1);
    }
}
