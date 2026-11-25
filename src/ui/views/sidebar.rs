use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::diff::FileStatus;
use crate::ui::state::{is_child_path, AppState, FocusedPanel, SidebarItem};
use crate::ui::theme;

pub fn render_sidebar(frame: &mut Frame, area: Rect, state: &AppState) {
    let t = theme::get();
    let bg = t.ui.bg;
    let is_focused = state.focused_panel == FocusedPanel::Sidebar;
    let visible_height = area.height.saturating_sub(2) as usize;

    let lines: Vec<Line> = state
        .sidebar_visible
        .iter()
        .enumerate()
        .skip(state.sidebar_scroll)
        .take(visible_height)
        .map(|(i, item_idx)| {
            let item = &state.sidebar_items[*item_idx];
            let (prefix, status_symbol, status_color, name, is_current_file, is_viewed) = match item
            {
                SidebarItem::Directory { name, path, depth } => {
                    let indent = "  ".repeat(*depth);
                    let mut has_children = false;
                    let mut all_children_viewed = true;
                    for child in &state.sidebar_items {
                        if let SidebarItem::File {
                            path: file_path,
                            file_index,
                            ..
                        } = child
                        {
                            if is_child_path(file_path, path) {
                                has_children = true;
                                if !state.viewed_files.contains(file_index) {
                                    all_children_viewed = false;
                                }
                            }
                        }
                    }
                    let marker = if has_children && all_children_viewed {
                        "✓ "
                    } else {
                        "  "
                    };
                    let status_symbol = if has_children {
                        if state.collapsed_dirs.contains(path) {
                            "▶"
                        } else {
                            "▼"
                        }
                    } else {
                        " "
                    };
                    (
                        format!("{indent}{marker}"),
                        status_symbol,
                        None,
                        format!(" {name}"),
                        false,
                        has_children && all_children_viewed,
                    )
                }
                SidebarItem::File {
                    name,
                    file_index,
                    depth,
                    status,
                    ..
                } => {
                    let indent = "  ".repeat(*depth);
                    let viewed = state.viewed_files.contains(file_index);
                    let marker = if viewed { "✓ " } else { "  " };
                    let status_color = match status {
                        FileStatus::Modified => Some(t.ui.status_modified),
                        FileStatus::Added => Some(t.ui.status_added),
                        FileStatus::Deleted => Some(t.ui.status_deleted),
                    };
                    (
                        format!("{indent}{marker}"),
                        status.symbol(),
                        status_color,
                        format!(" {name}"),
                        *file_index == state.current_file,
                        viewed,
                    )
                }
            };

            let is_selected = i == state.sidebar_selected;
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
    let muted_style = Style::default().fg(t.ui.text_muted);

    let title = Line::from(vec![
        Span::styled(" [1] Files ", title_style),
        Span::styled(format!("· {} ", state.file_diffs.len()), muted_style),
        Span::styled(
            format!("+{}", state.total_added),
            Style::default().fg(t.ui.stats_added),
        ),
        Span::raw(" "),
        Span::styled(
            format!("-{}", state.total_removed),
            Style::default().fg(t.ui.stats_removed),
        ),
        Span::raw(" "),
    ]);

    let para = Paragraph::new(lines)
        .style(Style::default().bg(bg))
        .scroll((0, state.sidebar_h_scroll))
        .block(
            Block::default()
                .title(title)
                .borders(Borders::TOP | Borders::LEFT | Borders::BOTTOM)
                .border_style(border_style)
                .style(Style::default().bg(bg)),
        );

    frame.render_widget(para, area);
}
