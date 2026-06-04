use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

use crate::command::diff::global_search::{GlobalSearchState, LineChange};
use crate::command::diff::render::diff_view::expand_tabs_in_spans;
use crate::command::diff::search::MatchPanel;
use crate::command::diff::state::Annotation;
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

pub enum ModalContent {
    #[allow(dead_code)]
    Info { title: String, message: String },
    Confirm { title: String, message: String },
    #[allow(dead_code)]
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
        annotations: Vec<Annotation>,
        selected: usize,
        export_input: Option<String>,
        /// Error message to display (e.g., for failed export)
        error_message: Option<String>,
    },
    /// Telescope-style fuzzy search across all files, with a preview pane.
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
    #[allow(dead_code)]
    Selected(usize, String),
    FileSelected(usize),
    /// User picked a result in the global search; jump to that file + line
    /// and pin the line to the top of the content area.
    JumpToLine {
        file_index: usize,
        sbs_line_index: usize,
        panel: MatchPanel,
    },
    AnnotationJump { annotation_id: u64 },
    AnnotationEdit { annotation_id: u64 },
    AnnotationDelete { annotation_id: u64 },
    AnnotationCopyAll,
    AnnotationExport(String),
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

    pub fn confirm(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            content: ModalContent::Confirm {
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
        let content_height: u16 = sections
            .iter()
            .map(|s| s.bindings.len() as u16 + 2) // +2 for section title and spacing
            .sum();
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

    pub fn global_search(title: impl Into<String>, state: GlobalSearchState) -> Self {
        // No pre-population — list + preview are blank until the user types.
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
        annotations: Vec<Annotation>,
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

        // GlobalSearch takes the full screen with its own two-pane layout.
        if let ModalContent::GlobalSearch { title, state } = &self.content {
            frame.render_widget(Clear, area);
            self.render_global_search(frame, area, title, state);
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
                // +1 for the hint line ("Enter to confirm, Esc to cancel")
                let height = (lines + 5).min(area.height * 80 / 100).max(6);
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
                let width = 100.min(area.width.saturating_sub(4));
                let items_count = items.len().min(12) as u16;
                // Compact height
                let extra = if export_input.is_some() { 4 } else { 2 };
                let height = (items_count + extra + 2).min(area.height * 80 / 100).max(8);
                (width, height)
            }
            // Handled above with its own near-fullscreen layout.
            ModalContent::GlobalSearch { .. } => unreachable!(),
        };

        let modal_x = (area.width.saturating_sub(modal_width)) / 2;
        let modal_y = (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = Rect::new(modal_x, modal_y, modal_width, modal_height);

        frame.render_widget(Clear, modal_area);

        match &self.content {
            ModalContent::Info { title, message } => {
                self.render_info(frame, modal_area, title, message);
            }
            ModalContent::Confirm { title, message } => {
                self.render_confirm(frame, modal_area, title, message);
            }
            ModalContent::Select {
                title,
                items,
                selected,
            } => {
                self.render_select(frame, modal_area, title, items, *selected);
            }
            ModalContent::KeyBindings { title, sections, scroll, content_height } => {
                self.render_keybindings(frame, modal_area, title, sections, *scroll, *content_height);
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
            ModalContent::GlobalSearch { .. } => unreachable!(),
        }
    }

    fn render_global_search(
        &self,
        frame: &mut Frame,
        area: Rect,
        title: &str,
        state: &GlobalSearchState,
    ) {
        let t = theme::get();

        use ratatui::layout::{Constraint, Direction, Layout};
        use ratatui::widgets::BorderType;

        // Telescope-style: two separate rounded-bordered panes side-by-side.
        // No outer wrapping border and no hint footer — the keybindings are
        // self-explanatory enough that the footer was just noise.
        let panes_area = area;

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(panes_area);

        // ── Left: results pane (prompt + list) ────────────────────────────────
        let count = state.result_count();
        let total = state.total_indexed();
        let results_title = if state.query.is_empty() {
            format!(" {} · {} lines ", title, total)
        } else {
            format!(" {} · {}/{} ", title, count, total)
        };
        let results_block = Block::default()
            .title(results_title)
            .title_style(Style::default().fg(t.ui.text_secondary))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(t.ui.border_focused));
        let results_inner = results_block.inner(cols[0]);
        frame.render_widget(results_block, cols[0]);

        let left_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // prompt
                Constraint::Length(1), // separator
                Constraint::Min(1),    // list
            ])
            .split(results_inner);

        // Prompt line — `❯ query_`
        let prompt_line = Line::from(vec![
            Span::styled("❯ ", Style::default().fg(t.ui.border_focused).bold()),
            Span::styled(&state.query, Style::default().fg(t.ui.text_primary)),
            Span::styled("█", Style::default().fg(t.ui.border_focused)),
        ]);
        frame.render_widget(Paragraph::new(prompt_line), left_rows[0]);

        // Thin separator under the prompt
        let sep = "─".repeat(left_rows[1].width as usize);
        frame.render_widget(
            Paragraph::new(Span::styled(sep, Style::default().fg(t.ui.border_unfocused))),
            left_rows[1],
        );

        // Result list. The state owns its scroll position now — render just
        // reads it. Mouse-wheel changes list_scroll directly (no auto-correct);
        // arrow keys also auto-scroll to keep the selection visible.
        let list_area = left_rows[2];
        let visible = list_area.height as usize;
        // SAFETY of mutation here: we mutate a Box<GlobalSearchState> inside a
        // &self render path. ratatui's Frame doesn't let us hold &mut state at
        // this point, so we work around by going through interior-mutability —
        // but `last_visible_rows` is plain field. Skip the write here (we'll
        // capture it via the mouse path or via re-clamping in scroll_list_y).
        // Instead, clamp list_scroll on read so an out-of-range value (e.g.
        // after a refilter) lands cleanly.
        let scroll = state
            .list_scroll
            .min(state.results.len().saturating_sub(visible.max(1)));

        let list_items: Vec<ListItem> = state
            .results
            .iter()
            .enumerate()
            .skip(scroll)
            .take(visible)
            .filter_map(|(i, r)| {
                let entry = state.entries.get(r.entry_index)?;
                let is_selected = (scroll + i) == state.selected;
                Some(build_result_row(
                    entry,
                    r,
                    is_selected,
                    t,
                    list_area.width,
                    state.list_scroll_x,
                ))
            })
            .collect();
        frame.render_widget(List::new(list_items), list_area);

        // ── Right: preview pane (border title shows file:line) ───────────────
        let preview_title = state
            .current_entry()
            .map(|e| format!(" {}:{} ", e.filename, e.line_no))
            .unwrap_or_else(|| String::from(" preview "));
        // .style() fills the block's interior with the diff view's page bg
        // (`t.ui.bg`) — same color the main diff view's empty area uses.
        // `t.diff.context_bg` would be wrong here: that's the slight tint used
        // for context rows and the sticky header, not the page background.
        let preview_block = Block::default()
            .title(preview_title)
            .title_style(Style::default().fg(t.ui.text_secondary))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(t.ui.border_unfocused))
            .style(Style::default().bg(t.ui.bg));
        let preview_inner = preview_block.inner(cols[1]);
        frame.render_widget(preview_block, cols[1]);

        render_preview_pane(frame, preview_inner, state, t);
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

    fn render_confirm(&self, frame: &mut Frame, area: Rect, title: &str, message: &str) {
        let t = theme::get();
        let block = Block::default()
            .title(format!(" {} ", title))
            .title_style(Style::default().fg(t.ui.border_focused).bold())
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(t.ui.border_unfocused));

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
        scroll: u16,
        content_height: u16,
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

        // Reserve space for scrollbar on the right
        let content_area = Rect::new(inner.x, inner.y, inner.width.saturating_sub(1), inner.height);

        let para = Paragraph::new(lines).scroll((scroll, 0));
        frame.render_widget(para, content_area);

        // Render scrollbar if content exceeds visible area
        let visible_height = inner.height;
        if content_height > visible_height {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .thumb_symbol("█");

            let mut scrollbar_state = ScrollbarState::new(content_height.saturating_sub(visible_height) as usize)
                .position(scroll as usize);

            frame.render_stateful_widget(scrollbar, inner, &mut scrollbar_state);
        }
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

                // Parse the item to extract filename, preview, and time
                // Format is: "filename:L#-# | preview... | HH:MM"
                let parts: Vec<&str> = item.splitn(3, " | ").collect();
                let location = parts.first().unwrap_or(&"");
                let preview = parts.get(1).unwrap_or(&"");
                let time = parts.get(2).unwrap_or(&"");

                let available_width = list_area.width as usize;
                let time_width = time.len() + 2; // " time "

                // Reserve space for time and calculate remaining space for location + preview
                let content_width = available_width.saturating_sub(time_width + 4); // 4 for padding/separators

                // Allocate: 45% for location, 55% for preview (minimum 20 chars each if space allows)
                let location_max = (content_width * 45 / 100).max(20).min(content_width.saturating_sub(20));
                let preview_max = content_width.saturating_sub(location_max);

                // Truncate location if needed (using char count for proper UTF-8 handling)
                let truncated_location = if location.chars().count() > location_max && location_max > 3 {
                    let truncate_at = location_max - 1;
                    let truncated: String = location.chars().take(truncate_at).collect();
                    format!("{}…", truncated)
                } else {
                    location.to_string()
                };

                // Truncate preview if needed (using char count for proper UTF-8 handling)
                let truncated_preview = if preview.chars().count() > preview_max && preview_max > 3 {
                    let truncate_at = preview_max - 1;
                    let truncated: String = preview.chars().take(truncate_at).collect();
                    format!("{}…", truncated)
                } else {
                    preview.to_string()
                };

                // Calculate padding to right-align time (using char count for proper width calculation)
                let location_len = truncated_location.chars().count() + 2; // " location "
                let preview_len = truncated_preview.chars().count() + 1;   // " preview"
                let used_width = location_len + preview_len + time_width;
                let padding = available_width.saturating_sub(used_width);

                let spans = if is_selected {
                    vec![
                        Span::styled(
                            format!(" {} ", truncated_location),
                            Style::default().fg(t.ui.selection_fg).bg(t.ui.selection_bg),
                        ),
                        Span::styled(
                            format!(" {}", truncated_preview),
                            Style::default()
                                .fg(t.ui.selection_fg)
                                .bg(t.ui.selection_bg)
                                .italic(),
                        ),
                        Span::styled(
                            format!("{:>width$}", "", width = padding),
                            Style::default().bg(t.ui.selection_bg),
                        ),
                        Span::styled(
                            format!(" {} ", time),
                            Style::default().fg(t.ui.selection_fg).bg(t.ui.selection_bg),
                        ),
                    ]
                } else {
                    vec![
                        Span::styled(
                            format!(" {} ", truncated_location),
                            Style::default().fg(t.ui.text_secondary),
                        ),
                        Span::styled(
                            format!(" {}", truncated_preview),
                            Style::default().fg(t.ui.text_muted).italic(),
                        ),
                        Span::styled(
                            format!("{:>width$}", "", width = padding),
                            Style::default(),
                        ),
                        Span::styled(
                            format!(" {} ", time),
                            Style::default().fg(t.ui.text_muted),
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

    /// Handle mouse scroll for the modal.
    /// Returns true if the scroll was handled.
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
            ModalContent::GlobalSearch { state, .. } => {
                // Modal is full-screen, split 50/50 between list and preview.
                // Compute the column boundary so we know which pane the wheel
                // event happened over.
                let left_pane_w = (terminal_width as u32 * 50 / 100) as u16;
                let pane_boundary = left_pane_w;
                let in_left = mouse.column < pane_boundary;
                // Chrome above the list rows inside the left pane:
                //   border top (1) + prompt (1) + separator (1) = 3
                // (No footer anymore, so the bottom subtractor is just the border).
                let list_y_start: u16 = 3;
                let visible_rows = (terminal_height as usize).saturating_sub(4);

                // Shift modifier on a wheel event flips it into a horizontal
                // scroll on whichever pane the cursor is over.
                let horizontal = mouse.modifiers.contains(KeyModifiers::SHIFT);

                match mouse.kind {
                    MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                        // Click in the list pane → select that row. The
                        // preview pane already mirrors the selection, so the
                        // user can verify before pressing Enter to jump.
                        // Clicks outside the list rows (border / prompt /
                        // separator / right pane) do nothing.
                        if in_left
                            && mouse.row >= list_y_start
                            && mouse.row + 1 < terminal_height
                        {
                            let row_in_list = (mouse.row - list_y_start) as usize;
                            let target = state.list_scroll + row_in_list;
                            if target < state.results.len() {
                                state.select(target, visible_rows);
                            }
                        }
                        true
                    }
                    MouseEventKind::ScrollDown => {
                        if in_left {
                            if horizontal {
                                state.scroll_list_x(4);
                            } else {
                                // Scroll the list view; selection stays put
                                // (and may scroll off-screen — by design).
                                state.scroll_list_y(3, visible_rows);
                            }
                        } else if horizontal {
                            state.scroll_preview_x(4);
                        } else {
                            state.scroll_preview_y(3);
                        }
                        true
                    }
                    MouseEventKind::ScrollUp => {
                        if in_left {
                            if horizontal {
                                state.scroll_list_x(-4);
                            } else {
                                state.scroll_list_y(-3, visible_rows);
                            }
                        } else if horizontal {
                            state.scroll_preview_x(-4);
                        } else {
                            state.scroll_preview_y(-3);
                        }
                        true
                    }
                    MouseEventKind::ScrollLeft => {
                        if in_left {
                            state.scroll_list_x(-4);
                        } else {
                            state.scroll_preview_x(-4);
                        }
                        true
                    }
                    MouseEventKind::ScrollRight => {
                        if in_left {
                            state.scroll_list_x(4);
                        } else {
                            state.scroll_preview_x(4);
                        }
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Handle keyboard input for the modal.
    /// Returns Some(ModalResult) if the modal should close.
    pub fn handle_input(&mut self, key: KeyEvent, terminal_height: u16) -> Option<ModalResult> {
        // FilePicker, Annotations, and GlobalSearch handle their own dismiss logic
        // (they consume printable chars for query input).
        if !matches!(
            self.content,
            ModalContent::FilePicker { .. }
                | ModalContent::Annotations { .. }
                | ModalContent::GlobalSearch { .. }
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
            ModalContent::Confirm { .. } => {
                if key.code == KeyCode::Enter {
                    return Some(ModalResult::Confirmed);
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
            ModalContent::KeyBindings { scroll, content_height, .. } => {
                let visible_height = calculate_keybindings_visible_height(terminal_height, *content_height);
                let max_scroll = content_height.saturating_sub(visible_height);

                match key.code {
                    KeyCode::Enter => Some(ModalResult::Dismissed),
                    KeyCode::Down | KeyCode::Char('j') => {
                        *scroll = (*scroll + 1).min(max_scroll);
                        None
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        *scroll = scroll.saturating_sub(1);
                        None
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Half-page down
                        *scroll = (*scroll + visible_height / 2).min(max_scroll);
                        None
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Half-page up
                        *scroll = scroll.saturating_sub(visible_height / 2);
                        None
                    }
                    KeyCode::Char('g') => {
                        // Go to top
                        *scroll = 0;
                        None
                    }
                    KeyCode::Char('G') => {
                        // Go to bottom
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
                // Word-erase: opt+backspace (macOS) / ctrl+w (readline). Must
                // sit BEFORE the plain Backspace arm so the modified variant
                // matches first.
                KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
                    crate::command::diff::text_edit::erase_word_backward(query);
                    Self::update_filtered_indices(items, query, filtered_indices, selected);
                    None
                }
                KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    crate::command::diff::text_edit::erase_word_backward(query);
                    Self::update_filtered_indices(items, query, filtered_indices, selected);
                    None
                }
                KeyCode::Backspace => {
                    query.pop();
                    Self::update_filtered_indices(items, query, filtered_indices, selected);
                    None
                }
                // Skip modified char keys so combos like opt+letter don't
                // insert the letter into the query.
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::ALT)
                        && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
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
                        // Word-erase: opt+backspace (macOS) / ctrl+w
                        // (readline). Goes BEFORE plain Backspace so the
                        // modified variant matches first.
                        KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
                            crate::command::diff::text_edit::erase_word_backward(input);
                            *error_message = None;
                            None
                        }
                        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            crate::command::diff::text_edit::erase_word_backward(input);
                            *error_message = None;
                            None
                        }
                        KeyCode::Backspace => {
                            input.pop();
                            *error_message = None; // Clear error on edit
                            None
                        }
                        // Skip modified char keys so combos like opt+letter
                        // don't insert the letter into the path.
                        KeyCode::Char(c)
                            if !key.modifiers.contains(KeyModifiers::ALT)
                                && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
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
                                annotation_id: ann.id,
                            }
                        }),
                        KeyCode::Char('e') => annotations.get(*selected).map(|ann| {
                            ModalResult::AnnotationEdit {
                                annotation_id: ann.id,
                            }
                        }),
                        KeyCode::Char('d') => annotations.get(*selected).map(|ann| {
                            ModalResult::AnnotationDelete {
                                annotation_id: ann.id,
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
            ModalContent::GlobalSearch { state, .. } => {
                // List pane occupies the full terminal height minus the
                // bordered pane chrome (2) and the prompt + separator (2).
                // That's the visible-rows count `ensure_*` methods need to
                // keep the selection on-screen.
                let visible_rows = (terminal_height as usize).saturating_sub(4);
                let page_step = (visible_rows / 2).max(5);
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let alt = key.modifiers.contains(KeyModifiers::ALT);
                let cmd = key.modifiers.contains(KeyModifiers::SUPER)
                    || key.modifiers.contains(KeyModifiers::META);

                // "Next/prev result" — ↓ / Ctrl+n / Ctrl+j (all telescope-standard).
                let is_down = matches!(key.code, KeyCode::Down)
                    || (ctrl && matches!(key.code, KeyCode::Char('n') | KeyCode::Char('j')));
                // "Prev result" — ↑ / Ctrl+p / Ctrl+k.
                let is_up = matches!(key.code, KeyCode::Up)
                    || (ctrl && matches!(key.code, KeyCode::Char('p') | KeyCode::Char('k')));
                // Half-page down — PageDown / Ctrl+d. (Ctrl+u is taken by clear-query.)
                let is_page_down =
                    matches!(key.code, KeyCode::PageDown) || (ctrl && matches!(key.code, KeyCode::Char('d')));

                if is_down {
                    state.move_down(visible_rows);
                    return None;
                }
                if is_up {
                    state.move_up(visible_rows);
                    return None;
                }
                if is_page_down {
                    state.page_down(page_step);
                    return None;
                }

                match key.code {
                    KeyCode::Esc => Some(ModalResult::Dismissed),
                    KeyCode::Char('c') if ctrl => Some(ModalResult::Dismissed),
                    KeyCode::Enter => state.current_entry().map(|e| ModalResult::JumpToLine {
                        file_index: e.file_index,
                        sbs_line_index: e.sbs_line_index,
                        panel: e.panel,
                    }),
                    KeyCode::PageUp => {
                        state.page_up(page_step);
                        None
                    }
                    KeyCode::Home => {
                        state.jump_top(visible_rows);
                        None
                    }
                    KeyCode::End => {
                        state.jump_bottom(visible_rows);
                        None
                    }
                    // Clear-entire-query: cmd+backspace on macOS terminals.
                    //   - Terminal.app / iTerm2 / most macOS defaults translate
                    //     ⌘⌫ into ^U (readline "unix-line-discard"), so we bind
                    //     ctrl+u here. This costs the previous half-page-up
                    //     binding, but PgUp/PgDn still work for paging.
                    //   - Terminals with kitty keyboard protocol forward ⌘⌫ as
                    //     Backspace + SUPER (or META). Bind both for safety.
                    KeyCode::Char('u') if ctrl => {
                        state.clear_query();
                        None
                    }
                    KeyCode::Backspace if cmd => {
                        state.clear_query();
                        None
                    }
                    // Word-erase: ctrl+w (readline-style) OR opt+backspace (macOS-style).
                    // Both should drop trailing whitespace then the last word.
                    KeyCode::Backspace if alt => {
                        state.erase_query_word();
                        None
                    }
                    KeyCode::Char('w') if ctrl => {
                        state.erase_query_word();
                        None
                    }
                    // Plain backspace: delete one char. Listed AFTER the
                    // modified variants above so they match first.
                    KeyCode::Backspace => {
                        state.pop_char();
                        None
                    }
                    // Catch printable characters — but explicitly exclude Ctrl- and
                    // Alt-prefixed sequences so combos like opt+letter don't insert
                    // the letter into the query.
                    KeyCode::Char(c) if !ctrl && !alt => {
                        state.push_char(c);
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

/// Build a single row in the global-search results list, telescope-style.
///
///   ❯ + src/foo/bar.rs:123  matched body text…       (selected row)
///     - src/foo/bar.rs:124  another match…           (other row)
///
/// - Leading `❯ ` (focused color) marks the selected row.
/// - A small `+`/`-`/`~`/` ` change symbol in its color sits between the
///   selector and the path — conveys what kind of line this is without
///   tinting the line number.
/// - Filename and line number both in a dim/muted color (neutral, not green).
/// - Body text in primary color, matched chars in search-match fg + bold.
/// - Horizontal scroll (`scroll_x`) shifts everything after the selector left;
///   the selector + change symbol stay pinned.
/// - Selected row gets a full-width selection bg.
fn build_result_row(
    entry: &crate::command::diff::global_search::GlobalSearchEntry,
    result: &crate::command::diff::global_search::ScoredResult,
    is_selected: bool,
    t: &crate::command::diff::theme::Theme,
    width: u16,
    scroll_x: usize,
) -> ListItem<'static> {
    let (sym_char, sym_color) = match entry.change {
        LineChange::Added => ('+', t.ui.stats_added),
        LineChange::Removed => ('-', t.ui.stats_removed),
        LineChange::Modified => ('~', t.ui.status_modified),
        LineChange::Equal => (' ', t.ui.text_muted),
    };

    // Background everything will sit on (None = terminal default).
    let bg = if is_selected {
        Some(t.ui.selection_bg)
    } else {
        None
    };
    let apply_bg = |mut s: Style| -> Style {
        if let Some(bg) = bg {
            s = s.bg(bg);
        }
        s
    };

    let body_color = if is_selected {
        t.ui.selection_fg
    } else {
        t.ui.text_primary
    };
    let meta_color = if is_selected {
        t.ui.selection_fg
    } else {
        t.ui.text_muted
    };

    // Selector + change symbol — fixed prefix that doesn't horizontally scroll.
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(16);
    let selector = if is_selected {
        Span::styled("❯ ", apply_bg(Style::default().fg(t.ui.border_focused).bold()))
    } else {
        Span::styled("  ", apply_bg(Style::default()))
    };
    spans.push(selector);
    spans.push(Span::styled(
        format!("{} ", sym_char),
        apply_bg(Style::default().fg(sym_color).bold()),
    ));

    // Walk the haystack ("filename:lineno  text") char by char so we can
    // overlay match-highlight on individual chars. Indices into the haystack:
    //   [0, filename_len)             → filename (meta color)
    //   filename_len                  → ':'      (meta color)
    //   [filename_len+1, prefix_len)  → lineno   (meta color — neutral)
    //   [prefix_len, end)             → body text (body color)
    let filename_len = entry.filename.chars().count();
    let lineno_str_len = entry.line_no.to_string().chars().count();
    let prefix_len = filename_len + 1 + lineno_str_len + 2;

    // Walk haystack chars, coalescing runs of same style into a single Span.
    // Previously emitted one `Span::styled(ch.to_string(), …)` per char — for
    // ~50 visible rows × ~80 chars that's thousands of tiny heap allocs per
    // frame. Style only changes at the filename→lineno→body boundary or when
    // crossing a match-index, so runs are typically long.
    let mut to_skip = scroll_x;
    let mut match_iter = result.match_indices.iter().peekable();
    let mut emitted_chars = 0;
    let mut run = String::new();
    let mut run_style: Option<Style> = None;
    for (i, ch) in entry.haystack.chars().enumerate() {
        // Advance match cursor even while skipping so highlights line up.
        let is_match = match match_iter.peek() {
            Some(&&idx) if idx as usize == i => {
                match_iter.next();
                true
            }
            _ => false,
        };
        if to_skip > 0 {
            to_skip -= 1;
            continue;
        }
        let fg = if is_match {
            t.ui.search_match_fg
        } else if i < prefix_len {
            meta_color
        } else {
            body_color
        };
        let mut style = Style::default().fg(fg);
        if is_match {
            style = style.bold();
        }
        let style = apply_bg(style);
        if run_style != Some(style) {
            if let Some(prev) = run_style.take() {
                if !run.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut run), prev));
                }
            }
            run_style = Some(style);
        }
        run.push(ch);
        emitted_chars += 1;
    }
    if let Some(prev) = run_style {
        if !run.is_empty() {
            spans.push(Span::styled(run, prev));
        }
    }

    // Pad the line to full width when selected so the highlight bar runs across.
    let used = 2 + 2 + emitted_chars; // "❯ " + "X " + visible haystack
    if is_selected {
        let pad = (width as usize).saturating_sub(used);
        if pad > 0 {
            spans.push(Span::styled(
                " ".repeat(pad),
                Style::default().bg(t.ui.selection_bg),
            ));
        }
    }
    ListItem::new(Line::from(spans))
}

/// Render the preview pane body as a stacked (unified) mini-diff around the
/// matched line. Telescope-style: no in-body header (the bordered pane already
/// shows `filename:lineno` in its title), no arrow gutter — the cursor row is
/// indicated by a thin left-edge accent and a brighter body color.
fn render_preview_pane(
    frame: &mut Frame,
    area: Rect,
    state: &GlobalSearchState,
    t: &crate::command::diff::theme::Theme,
) {
    use crate::command::diff::types::ChangeType;

    if area.height == 0 || area.width == 0 {
        return;
    }

    let Some(entry) = state.current_entry() else {
        // Two empty states: pre-query (just opened the modal) and post-query
        // (typed but nothing matched). The block's bg already fills the area;
        // we only render a hint for the post-query case so the pre-query view
        // is fully blank.
        if !state.query.is_empty() {
            let msg = Line::from(Span::styled(
                "  no matches",
                Style::default()
                    .fg(t.ui.text_muted)
                    .bg(t.ui.bg)
                    .italic(),
            ));
            frame.render_widget(Paragraph::new(msg), area);
        }
        return;
    };

    let body_area = area;
    let body_width = body_area.width as usize;
    let body_height = body_area.height as usize;
    // Tab width for the file we're previewing — used to expand tabs in the
    // tree-sitter span output so it lines up with the diff view.
    let tab_width = state
        .files
        .get(entry.file_index)
        .map(|f| f.tab_width)
        .unwrap_or(4);

    // ── Two-pass strategy ──────────────────────────────────────────────────
    // Pass 1: walk the full SBS and build a cheap Vec of `PreviewRowMeta`
    //   (just enum + a few usize/bool) in stacked-diff display order. No
    //   tree-sitter calls, no tab expansion, no horizontal scroll work — so
    //   this is ~free even for huge files.
    // Pass 2: clamp the cursor-centered scroll window to a [drop, drop+h)
    //   range and only call `make_preview_row` (the expensive path with
    //   highlighter + tab expand + scroll) for that visible window.
    //
    // Before this split, a 1000-line file paid for 1000 highlighter calls per
    // frame even though only ~30 rows are visible. Now it's ~30 per frame.
    #[derive(Clone, Copy)]
    enum Side {
        Old,
        New,
    }
    #[derive(Clone, Copy)]
    struct PreviewRowMeta {
        sbs_idx: usize,
        side: Side,
        is_cursor: bool,
    }

    state.with_sbs(entry.file_index, |sbs| {
        // Pass 1: build display-order metas.
        let mut metas: Vec<PreviewRowMeta> = Vec::with_capacity(sbs.len() + sbs.len() / 4);
        let mut pending: Vec<PreviewRowMeta> = Vec::new();
        let mut cursor_idx: Option<usize> = None;

        // Within a change block, all '-' rows render before all '+' rows
        // (stacked diff). We emit '-' rows immediately and queue '+' rows;
        // the queue is flushed at every Equal row and at the end of the file.
        let flush = |metas: &mut Vec<PreviewRowMeta>,
                     pending: &mut Vec<PreviewRowMeta>,
                     cursor_idx: &mut Option<usize>| {
            for p in pending.drain(..) {
                if p.is_cursor {
                    *cursor_idx = Some(metas.len());
                }
                metas.push(p);
            }
        };

        for (sbs_idx, sbs_line) in sbs.iter().enumerate() {
            let is_match_row = sbs_idx == entry.sbs_line_index;
            match sbs_line.change_type {
                ChangeType::Equal => {
                    flush(&mut metas, &mut pending, &mut cursor_idx);
                    if sbs_line.new_line.is_some() {
                        let is_cursor = is_match_row && entry.panel == MatchPanel::New;
                        if is_cursor {
                            cursor_idx = Some(metas.len());
                        }
                        metas.push(PreviewRowMeta { sbs_idx, side: Side::New, is_cursor });
                    }
                }
                ChangeType::Insert => {
                    if sbs_line.new_line.is_some() {
                        pending.push(PreviewRowMeta {
                            sbs_idx,
                            side: Side::New,
                            is_cursor: is_match_row && entry.panel == MatchPanel::New,
                        });
                    }
                }
                ChangeType::Delete => {
                    if sbs_line.old_line.is_some() {
                        let is_cursor = is_match_row && entry.panel == MatchPanel::Old;
                        if is_cursor {
                            cursor_idx = Some(metas.len());
                        }
                        metas.push(PreviewRowMeta { sbs_idx, side: Side::Old, is_cursor });
                    }
                }
                ChangeType::Modified => {
                    if sbs_line.old_line.is_some() {
                        let is_cursor = is_match_row && entry.panel == MatchPanel::Old;
                        if is_cursor {
                            cursor_idx = Some(metas.len());
                        }
                        metas.push(PreviewRowMeta { sbs_idx, side: Side::Old, is_cursor });
                    }
                    if sbs_line.new_line.is_some() {
                        pending.push(PreviewRowMeta {
                            sbs_idx,
                            side: Side::New,
                            is_cursor: is_match_row && entry.panel == MatchPanel::New,
                        });
                    }
                }
            }
        }
        flush(&mut metas, &mut pending, &mut cursor_idx);

        // Cursor-centered scroll window, with user preview_scroll_y stacked on top.
        let total_rows = metas.len();
        let base_drop = cursor_idx
            .map(|i| i.saturating_sub(body_height / 2))
            .unwrap_or(0);
        let max_drop = total_rows.saturating_sub(body_height);
        let drop = ((base_drop as i32 + state.preview_scroll_y).max(0) as usize).min(max_drop);
        let end = (drop + body_height).min(total_rows);

        // Pass 2: materialize ONLY the visible rows. This is where the
        // expensive per-row work (highlighter lookup, tab expansion,
        // horizontal scroll, span allocation) actually happens.
        let lines: Vec<Line> = metas[drop..end]
            .iter()
            .filter_map(|m| {
                let sbs_line = &sbs[m.sbs_idx];
                let (line_no, text, sym, row_bg, gutter_bg, panel) = match m.side {
                    Side::Old => {
                        let (ln, text) = sbs_line.old_line.as_ref()?;
                        (
                            *ln,
                            text.as_str(),
                            '-',
                            t.diff.deleted_bg,
                            t.diff.deleted_gutter_bg,
                            MatchPanel::Old,
                        )
                    }
                    Side::New => {
                        let (ln, text) = sbs_line.new_line.as_ref()?;
                        // Equal rows use `t.ui.bg` (the diff view's page bg)
                        // for both body and gutter — same as the main diff
                        // view's context rendering. `t.diff.context_bg` is
                        // only for sticky-header context, NOT body context.
                        let (sym, row_bg, gutter_bg) = match sbs_line.change_type {
                            ChangeType::Equal => (' ', t.ui.bg, t.ui.bg),
                            ChangeType::Insert | ChangeType::Modified => {
                                ('+', t.diff.added_bg, t.diff.added_gutter_bg)
                            }
                            // Side::New is never paired with Delete in pass 1.
                            ChangeType::Delete => return None,
                        };
                        (*ln, text.as_str(), sym, row_bg, gutter_bg, MatchPanel::New)
                    }
                };
                Some(make_preview_row(
                    state,
                    entry.file_index,
                    panel,
                    line_no,
                    text,
                    sym,
                    row_bg,
                    gutter_bg,
                    m.is_cursor,
                    t,
                    body_width,
                    state.preview_scroll_x,
                    tab_width,
                ))
            })
            .collect();

        frame.render_widget(Paragraph::new(lines), body_area);
    });
}

/// Build one row in the preview pane, telescope-style.
///
///   ▌ 1234 │ + tree-sitter-highlighted body…       (cursor row)
///     1235 │ - …                                    (other rows)
///
/// Layout:
///   - ▌ (focused color) on the cursor row only, blank otherwise — a thin
///     left-edge accent, not a heavy arrow.
///   - Line number, right-padded to 4 cols.
///   - A subtle `│` divider between gutter and content (uses border color).
///   - Change symbol (+/-/space) in its color.
///   - Tree-sitter highlighted body, with diff bg.
///
/// `row_bg` colors the body. `gutter_bg` sits behind the line-number column.
#[allow(clippy::too_many_arguments)]
fn make_preview_row<'a>(
    state: &GlobalSearchState,
    file_index: usize,
    panel: MatchPanel,
    line_no: usize,
    text: &str,
    sym: char,
    row_bg: ratatui::style::Color,
    gutter_bg: ratatui::style::Color,
    is_cursor: bool,
    t: &crate::command::diff::theme::Theme,
    width: usize,
    scroll_x: usize,
    tab_width: usize,
) -> Line<'a> {
    // Cursor row needs a visibly different bg from the row above/below.
    // For RGB bgs (added/deleted/explicit page bg), brighten in place so the
    // green-on-green / red-on-red palette is preserved. For non-RGB bgs
    // (Color::Reset on themes that defer to the terminal default), brighten
    // returns the color unchanged — fall back to a subtle theme-mode-aware
    // gray rather than `selection_bg`, which is a saturated accent (cyan /
    // blue) and clashes with the syntax-highlighted body.
    let (row_bg, gutter_bg) = if is_cursor {
        let fallback = cursor_row_fallback_bg(t);
        let row = brighten_bg(row_bg, 18).unwrap_or(fallback);
        let gut = brighten_bg(gutter_bg, 18).unwrap_or(fallback);
        (row, gut)
    } else {
        (row_bg, gutter_bg)
    };

    // Left-edge cursor accent: a single thin vertical block on the matched row,
    // a blank cell otherwise. Sits flush against the pane's left edge.
    let accent = if is_cursor {
        Span::styled(
            "▌",
            Style::default().fg(t.ui.border_focused).bg(gutter_bg),
        )
    } else {
        Span::styled(" ", Style::default().bg(gutter_bg))
    };

    // Line number gutter. For add/del rows the main diff view uses a colored
    // gutter_fg (added_gutter_fg / deleted_gutter_fg) instead of the neutral
    // line_number color — match that so the preview looks like the main view.
    // We use `sym` only as a discriminator here; the visible `+`/`-` glyph is
    // not rendered (the row bg already conveys add vs delete vs context).
    let gutter_fg = match sym {
        '+' => t.diff.added_gutter_fg,
        '-' => t.diff.deleted_gutter_fg,
        _ => t.ui.line_number,
    };
    let gutter_text = format!("{:>4} ", line_no);
    let gutter_style = Style::default().fg(gutter_fg).bg(gutter_bg);

    // Tree-sitter highlighted body, cached per (file, panel) inside state.
    // Keep token colors on EVERY row — telescope-style. The cursor row is
    // distinguished by its brighter bg and the accent, not by dimming the rest.
    //
    // The highlighter returns spans built from the *raw* file content, so we
    // expand tabs into spaces here for visual consistency with the main diff
    // view (which does the same).
    let highlighted = state.highlighted_line_spans(file_index, panel, line_no, Some(row_bg));
    let body_spans: Vec<Span<'static>> = if highlighted.is_empty() {
        vec![Span::styled(
            text.to_string(),
            Style::default().fg(t.ui.text_primary).bg(row_bg),
        )]
    } else {
        expand_tabs_in_spans(highlighted, tab_width)
    };

    // Chrome width: accent(1) + gutter(5) = 6. No visible +/- glyph — the row
    // bg color already says whether this is an added/removed/context line.
    let chrome = 1 + 5;
    let avail = width.saturating_sub(chrome);

    let mut row: Vec<Span> = Vec::with_capacity(body_spans.len() + 3);
    row.push(accent);
    row.push(Span::styled(gutter_text, gutter_style));

    // Apply horizontal scroll by skipping `scroll_x` chars from the body before
    // taking up to `avail` more chars. The chrome (gutter + sym + accent) never
    // scrolls — only the body content does.
    let mut to_skip = scroll_x;
    let mut remaining = avail;
    let mut used_chars = 0;
    for sp in body_spans {
        if remaining == 0 {
            break;
        }
        let count = sp.content.chars().count();
        // Skip phase: this whole span is hidden by horizontal scroll.
        if to_skip >= count {
            to_skip -= count;
            continue;
        }
        // Partial skip: drop `to_skip` chars from the front of this span.
        let visible_text: String = if to_skip > 0 {
            let s: String = sp.content.chars().skip(to_skip).collect();
            to_skip = 0;
            s
        } else {
            sp.content.to_string()
        };
        let visible_count = visible_text.chars().count();
        if visible_count <= remaining {
            used_chars += visible_count;
            remaining -= visible_count;
            row.push(Span::styled(visible_text, sp.style));
        } else {
            let truncated: String = visible_text.chars().take(remaining).collect();
            used_chars += remaining;
            remaining = 0;
            row.push(Span::styled(truncated, sp.style));
        }
    }

    let pad = avail.saturating_sub(used_chars);
    if pad > 0 {
        row.push(Span::styled(" ".repeat(pad), Style::default().bg(row_bg)));
    }

    Line::from(row)
}

/// Calculate visible height for keybindings modal based on terminal size.
/// Lighten an RGB color by `amount` per channel (clamped). Used to give the
/// preview's cursor row a brighter shade of whatever bg it would normally have
/// (page, added, deleted) so the highlight respects the diff palette.
///
/// Returns `None` for non-RGB colors (e.g. `Color::Reset`, named colors) —
/// callers fall back to an explicit highlight color since "brighten" has no
/// meaning when we don't know the underlying RGB.
fn brighten_bg(c: ratatui::style::Color, amount: u8) -> Option<ratatui::style::Color> {
    use ratatui::style::Color;
    match c {
        Color::Rgb(r, g, b) => Some(Color::Rgb(
            r.saturating_add(amount),
            g.saturating_add(amount),
            b.saturating_add(amount),
        )),
        _ => None,
    }
}

/// Subtle neutral gray used to highlight the cursor row when the underlying
/// row bg is the terminal default (Color::Reset). Picked to read as "slightly
/// raised" without competing with the syntax-highlighted body.
fn cursor_row_fallback_bg(t: &crate::command::diff::theme::Theme) -> ratatui::style::Color {
    use crate::command::diff::theme::ThemeMode;
    use ratatui::style::Color;
    match t.mode {
        ThemeMode::Dark => Color::Rgb(55, 58, 68),
        ThemeMode::Light => Color::Rgb(225, 228, 232),
    }
}

fn calculate_keybindings_visible_height(terminal_height: u16, content_height: u16) -> u16 {
    // Modal height calculation from render: (total_lines + 4).min(height * 80 / 100).max(5)
    let modal_height = (content_height + 4).min(terminal_height * 80 / 100).max(5);
    // Subtract 2 for top/bottom borders
    modal_height.saturating_sub(2)
}
