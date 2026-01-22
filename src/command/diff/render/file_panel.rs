use std::collections::HashSet;

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::command::diff::theme;
use crate::command::diff::types::{FilePanelItem, FileStatus};

#[allow(clippy::too_many_arguments)]
pub fn render_file_panel(
    frame: &mut Frame,
    area: Rect,
    file_panel_items: &[FilePanelItem],
    file_panel_visible: &[usize],
    collapsed_dirs: &HashSet<String>,
    current_file: usize,
    file_panel_selected: usize,
    file_panel_scroll: usize,
    file_panel_h_scroll: u16,
    viewed_files: &HashSet<usize>,
    is_focused: bool,
) {
    let t = theme::get();
    let bg = t.ui.bg;
    let visible_height = area.height.saturating_sub(2) as usize;
    let lines: Vec<Line> = file_panel_visible
        .iter()
        .enumerate()
        .map(|(i, item_idx)| {
            let item = &file_panel_items[*item_idx];
            let (prefix, status_symbol, status_color, name, is_current_file, is_viewed) = match item
            {
                FilePanelItem::Directory {
                    name, path, depth, ..
                } => {
                    let indent = "  ".repeat(*depth);
                    let all_children_viewed = file_panel_items.iter().all(|child| {
                        if let FilePanelItem::File {
                            path: file_path,
                            file_index,
                            ..
                        } = child
                        {
                            if file_path.starts_with(&format!("{}/", path)) {
                                return viewed_files.contains(file_index);
                            }
                        }
                        true
                    });
                    let has_children = file_panel_items.iter().any(|child| {
                        if let FilePanelItem::File {
                            path: file_path, ..
                        } = child
                        {
                            file_path.starts_with(&format!("{}/", path))
                        } else {
                            false
                        }
                    });
                    let marker = if has_children && all_children_viewed {
                        "✓ "
                    } else {
                        "  "
                    };
                    let status_symbol = if has_children {
                        if collapsed_dirs.contains(path) {
                            "▶"
                        } else {
                            "▼"
                        }
                    } else {
                        " "
                    };
                    (
                        format!("{}{}", indent, marker),
                        status_symbol.to_string(),
                        None,
                        format!(" {}", name),
                        false,
                        all_children_viewed && has_children,
                    )
                }
                FilePanelItem::File {
                    name,
                    file_index,
                    depth,
                    status,
                    ..
                } => {
                    let indent = "  ".repeat(*depth);
                    let viewed = viewed_files.contains(file_index);
                    let marker = if viewed { "✓ " } else { "  " };
                    let status_color = match status {
                        FileStatus::Modified => Some(t.ui.status_modified),
                        FileStatus::Added => Some(t.ui.status_added),
                        FileStatus::Deleted => Some(t.ui.status_deleted),
                    };
                    let status_symbol = status.symbol().to_string();
                    (
                        format!("{}{}", indent, marker),
                        status_symbol,
                        status_color,
                        format!(" {}", name),
                        *file_index == current_file,
                        viewed,
                    )
                }
            };

            let is_selected = i == file_panel_selected;
            let base_style = if is_selected {
                Style::default().fg(t.ui.selection_fg).bg(if is_focused {
                    t.ui.selection_bg
                } else {
                    t.ui.border_unfocused
                })
            } else if is_current_file {
                Style::default().fg(t.ui.highlight)
            } else if is_viewed {
                Style::default().fg(t.ui.viewed)
            } else {
                Style::default()
            };

            let status_style = if is_selected {
                base_style
            } else if let Some(color) = status_color {
                Style::default().fg(color)
            } else {
                base_style
            };

            Line::from(vec![
                Span::styled(prefix, base_style),
                Span::styled(status_symbol, status_style),
                Span::styled(name, base_style),
            ])
        })
        .collect();

    let title_style = if is_focused {
        Style::default().fg(t.ui.border_focused)
    } else {
        Style::default().fg(t.ui.border_unfocused)
    };
    let border_style = Style::default().fg(t.ui.border_unfocused);

    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(file_panel_scroll)
        .take(visible_height)
        .collect();

    let para = Paragraph::new(visible_lines)
        .style(Style::default().bg(bg))
        .scroll((0, file_panel_h_scroll))
        .block(
            Block::default()
                .title(Line::styled(" [1] Files ", title_style))
                .borders(Borders::ALL)
                .border_style(border_style)
                .style(Style::default().bg(bg)),
        );

    frame.render_widget(para, area);
}
