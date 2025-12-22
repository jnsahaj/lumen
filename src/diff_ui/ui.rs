use std::collections::HashSet;

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::diff_ui::diff::compute_side_by_side;
use crate::diff_ui::highlight::highlight_line_spans;
use crate::diff_ui::sticky_lines::{compute_sticky_lines, StickyLine};
use crate::diff_ui::types::{ChangeType, DiffLine, DiffViewSettings, FileDiff, FocusedPanel, SidebarItem};

pub fn render_empty_state(frame: &mut Frame, watching: bool) {
    let watch_hint = if watching {
        " (watching for changes...)"
    } else {
        ""
    };
    let msg = Paragraph::new(format!("No changes detected.{}", watch_hint)).block(
        Block::default()
            .title(" Git Review ")
            .borders(Borders::ALL),
    );
    frame.render_widget(msg, frame.area());
}

fn truncate_middle(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    if max_len < 5 {
        return s.chars().take(max_len).collect();
    }
    let half = (max_len - 3) / 2;
    let start: String = s.chars().take(half).collect();
    let end: String = s.chars().skip(s.len() - half).collect();
    format!("{}...{}", start, end)
}

pub fn render_diff(
    frame: &mut Frame,
    diff: &FileDiff,
    file_diffs: &[FileDiff],
    sidebar_items: &[SidebarItem],
    current_file: usize,
    scroll: u16,
    h_scroll: u16,
    watching: bool,
    show_sidebar: bool,
    focused_panel: FocusedPanel,
    sidebar_selected: usize,
    sidebar_scroll: usize,
    viewed_files: &HashSet<usize>,
    settings: &DiffViewSettings,
    hunk_count: usize,
) {
    let area = frame.area();
    let side_by_side = compute_side_by_side(&diff.old_content, &diff.new_content);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let watch_indicator = if watching { " [watching]" } else { "" };
    let max_filename_len = 40;
    let truncated_filename = truncate_middle(&diff.filename, max_filename_len);
    let hunks_text = if hunk_count == 1 { "hunk" } else { "hunks" };
    let header = Paragraph::new(format!(
        " File {}/{}: {} ({} {}){}  |  [j/k] scroll  [C-j/k] files  [Space] viewed  [q] quit",
        current_file + 1,
        file_diffs.len(),
        truncated_filename,
        hunk_count,
        hunks_text,
        watch_indicator
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    frame.render_widget(header, chunks[0]);

    let main_area = if show_sidebar {
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(40), Constraint::Min(0)])
            .split(chunks[1]);

        render_sidebar(
            frame,
            main_chunks[0],
            sidebar_items,
            current_file,
            sidebar_selected,
            sidebar_scroll,
            viewed_files,
            focused_panel == FocusedPanel::Sidebar,
        );

        main_chunks[1]
    } else {
        chunks[1]
    };

    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_area);

    // Extract old and new content lines for sticky computation
    let old_content_lines: Vec<(usize, String)> = side_by_side
        .iter()
        .filter_map(|dl| dl.old_line.clone())
        .collect();
    let new_content_lines: Vec<(usize, String)> = side_by_side
        .iter()
        .filter_map(|dl| dl.new_line.clone())
        .collect();

    // Compute sticky lines for old and new panels
    let old_sticky = compute_sticky_lines(&old_content_lines, scroll as usize, &settings.sticky_lines);
    let new_sticky = compute_sticky_lines(&new_content_lines, scroll as usize, &settings.sticky_lines);
    let sticky_count = old_sticky.len().max(new_sticky.len());

    let visible_height = content_chunks[0].height.saturating_sub(2) as usize;
    let scroll_usize = scroll as usize;
    
    // Adjust visible lines to account for sticky lines
    let content_height = visible_height.saturating_sub(sticky_count);
    let visible_lines: Vec<&DiffLine> = side_by_side
        .iter()
        .skip(scroll_usize)
        .take(content_height)
        .collect();

    let mut old_lines: Vec<Line> = Vec::new();
    let mut new_lines: Vec<Line> = Vec::new();

    // Render sticky lines first (if enabled)
    if settings.sticky_lines.enabled && sticky_count > 0 {
        render_sticky_lines(&old_sticky, sticky_count, &mut old_lines, &diff.filename);
        render_sticky_lines(&new_sticky, sticky_count, &mut new_lines, &diff.filename);
    }

    for diff_line in &visible_lines {
        let (old_bg, new_bg) = match diff_line.change_type {
            ChangeType::Equal => (None, None),
            ChangeType::Delete => (Some(Color::Rgb(60, 30, 30)), None),
            ChangeType::Insert => (None, Some(Color::Rgb(30, 60, 30))),
        };

        let mut old_spans: Vec<Span> = Vec::new();
        match &diff_line.old_line {
            Some((num, text)) => {
                let prefix = format!("{:4} | ", num);
                old_spans.push(Span::styled(
                    prefix,
                    Style::default()
                        .fg(Color::DarkGray)
                        .bg(old_bg.unwrap_or(Color::Reset)),
                ));
                old_spans.extend(highlight_line_spans(text, &diff.filename, old_bg));
            }
            None => {
                old_spans.push(Span::styled("     |", Style::default().fg(Color::DarkGray)));
            }
        }

        let mut new_spans: Vec<Span> = Vec::new();
        match &diff_line.new_line {
            Some((num, text)) => {
                let prefix = format!("{:4} | ", num);
                new_spans.push(Span::styled(
                    prefix,
                    Style::default()
                        .fg(Color::DarkGray)
                        .bg(new_bg.unwrap_or(Color::Reset)),
                ));
                new_spans.extend(highlight_line_spans(text, &diff.filename, new_bg));
            }
            None => {
                new_spans.push(Span::styled("     |", Style::default().fg(Color::DarkGray)));
            }
        }

        old_lines.push(Line::from(old_spans));
        new_lines.push(Line::from(new_spans));
    }

    let diff_title_style = if focused_panel == FocusedPanel::DiffView {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let old_para = Paragraph::new(old_lines)
        .scroll((0, h_scroll))
        .block(
            Block::default()
                .title(" [2] Old ")
                .borders(Borders::ALL)
                .border_style(diff_title_style.patch(Style::default().fg(Color::Red))),
        );
    let new_para = Paragraph::new(new_lines)
        .scroll((0, h_scroll))
        .block(
            Block::default()
                .title(" New ")
                .borders(Borders::ALL)
                .border_style(diff_title_style.patch(Style::default().fg(Color::Green))),
        );

    frame.render_widget(old_para, content_chunks[0]);
    frame.render_widget(new_para, content_chunks[1]);
}

fn render_sticky_lines(
    sticky: &[StickyLine],
    total_count: usize,
    lines: &mut Vec<Line>,
    filename: &str,
) {
    let sticky_bg = Color::Rgb(40, 40, 50);
    
    for i in 0..total_count {
        if let Some(sl) = sticky.get(i) {
            let prefix = format!("{:4} ~ ", sl.line_number);
            let mut spans: Vec<Span> = vec![Span::styled(
                prefix,
                Style::default().fg(Color::DarkGray).bg(sticky_bg),
            )];
            spans.extend(highlight_line_spans(&sl.content, filename, Some(sticky_bg)));
            lines.push(Line::from(spans));
        } else {
            // Empty sticky line placeholder (when other panel has more sticky lines)
            lines.push(Line::from(vec![Span::styled(
                "     ~".to_string(),
                Style::default().fg(Color::DarkGray).bg(sticky_bg),
            )]));
        }
    }
}

fn render_sidebar(
    frame: &mut Frame,
    area: Rect,
    sidebar_items: &[SidebarItem],
    current_file: usize,
    sidebar_selected: usize,
    sidebar_scroll: usize,
    viewed_files: &HashSet<usize>,
    is_focused: bool,
) {
    let visible_height = area.height.saturating_sub(2) as usize;
    let items: Vec<ListItem> = sidebar_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let (display, is_current_file, is_viewed) = match item {
                SidebarItem::Directory { name, path, depth, .. } => {
                    let indent = "  ".repeat(*depth);
                    let all_children_viewed = sidebar_items.iter().all(|child| {
                        if let SidebarItem::File {
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
                    let has_children = sidebar_items.iter().any(|child| {
                        if let SidebarItem::File { path: file_path, .. } = child {
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
                    (format!("{}{}▸ {}", marker, indent, name), false, all_children_viewed && has_children)
                }
                SidebarItem::File {
                    name,
                    file_index,
                    depth,
                    ..
                } => {
                    let indent = "  ".repeat(*depth);
                    let viewed = viewed_files.contains(file_index);
                    let marker = if viewed { "✓ " } else { "  " };
                    (
                        format!("{}{}  {}", marker, indent, name),
                        *file_index == current_file,
                        viewed,
                    )
                }
            };

            let style = if i == sidebar_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(if is_focused {
                        Color::Cyan
                    } else {
                        Color::DarkGray
                    })
            } else if is_current_file {
                Style::default().fg(Color::Yellow)
            } else if is_viewed {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };

            ListItem::new(display).style(style)
        })
        .collect();

    let border_color = if is_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let visible_items: Vec<ListItem> = items
        .into_iter()
        .skip(sidebar_scroll)
        .take(visible_height)
        .collect();

    let list = List::new(visible_items).block(
        Block::default()
            .title(" [1] Files ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color)),
    );

    frame.render_widget(list, area);
}
