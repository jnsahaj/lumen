use std::collections::HashSet;

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::command::diff::theme::{self, Theme};
use crate::command::diff::types::{FileDiff, FileStatus, SidebarItem};
use crate::grouped_summary::DiffGroup;

/// Greedy word-wrap at `width` columns. Words longer than `width` are left
/// unbroken (overflow rather than mid-word split).
pub(super) fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Render a single file row (icon/viewed-marker/status-color/name), shared
/// by the directory-tree sidebar and the Guide sidebar's per-group file
/// list so both stay visually consistent.
#[allow(clippy::too_many_arguments)]
fn file_row_line(
    theme: &Theme,
    indent_depth: usize,
    name: &str,
    status: FileStatus,
    viewed: bool,
    is_current: bool,
    is_selected: bool,
    is_focused: bool,
) -> Line<'static> {
    let indent = "  ".repeat(indent_depth);
    let marker = if viewed { "✓ " } else { "  " };
    let status_color = match status {
        FileStatus::Modified => Some(theme.ui.status_modified),
        FileStatus::Added => Some(theme.ui.status_added),
        FileStatus::Deleted => Some(theme.ui.status_deleted),
    };
    let status_symbol = status.symbol().to_string();
    let prefix = format!("{}{}", indent, marker);
    let name = format!(" {}", name);

    let base_style = if is_selected {
        Style::default()
            .fg(theme.ui.selection_fg)
            .bg(if is_focused {
                theme.ui.selection_bg
            } else {
                theme.ui.border_unfocused
            })
    } else if is_current {
        Style::default().fg(theme.ui.highlight)
    } else if viewed {
        Style::default().fg(theme.ui.viewed)
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
}

#[allow(clippy::too_many_arguments)]
pub fn render_sidebar(
    frame: &mut Frame,
    area: Rect,
    sidebar_items: &[SidebarItem],
    sidebar_visible: &[usize],
    collapsed_dirs: &HashSet<String>,
    current_file: usize,
    sidebar_selected: usize,
    sidebar_scroll: usize,
    sidebar_h_scroll: u16,
    viewed_files: &HashSet<usize>,
    is_focused: bool,
    total_files: usize,
    total_added: usize,
    total_removed: usize,
) {
    let t = theme::get();
    let bg = t.ui.bg;
    let visible_height = area.height.saturating_sub(2) as usize;
    let lines: Vec<Line> = sidebar_visible
        .iter()
        .enumerate()
        .map(|(i, item_idx)| {
            let item = &sidebar_items[*item_idx];
            let is_selected = i == sidebar_selected;
            match item {
                SidebarItem::Directory {
                    name, path, depth, ..
                } => {
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
                        if let SidebarItem::File {
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
                    let is_viewed = all_children_viewed && has_children;

                    let prefix = format!("{}{}", indent, marker);
                    let name = format!(" {}", name);
                    let base_style = if is_selected {
                        Style::default().fg(t.ui.selection_fg).bg(if is_focused {
                            t.ui.selection_bg
                        } else {
                            t.ui.border_unfocused
                        })
                    } else if is_viewed {
                        Style::default().fg(t.ui.viewed)
                    } else {
                        Style::default()
                    };

                    Line::from(vec![
                        Span::styled(prefix, base_style),
                        Span::styled(status_symbol.to_string(), base_style),
                        Span::styled(name, base_style),
                    ])
                }
                SidebarItem::File {
                    name,
                    file_index,
                    depth,
                    status,
                    ..
                } => file_row_line(
                    t,
                    *depth,
                    name,
                    *status,
                    viewed_files.contains(file_index),
                    *file_index == current_file,
                    is_selected,
                    is_focused,
                ),
            }
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
        Span::styled(format!("· {} ", total_files), muted_style),
        Span::styled(
            format!("+{}", total_added),
            Style::default().fg(t.ui.stats_added),
        ),
        Span::raw(" "),
        Span::styled(
            format!("-{}", total_removed),
            Style::default().fg(t.ui.stats_removed),
        ),
        Span::raw(" "),
    ]);

    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(sidebar_scroll)
        .take(visible_height)
        .collect();

    // Drop the right border so the sidebar shares a single vertical line with
    // the adjacent diff panel. The parent renderer fixes up the corner cells
    // at the boundary so the joined borders use `┬` / `┴` junctions.
    let borders = Borders::TOP | Borders::LEFT | Borders::BOTTOM;

    let para = Paragraph::new(visible_lines)
        .style(Style::default().bg(bg))
        .scroll((0, sidebar_h_scroll))
        .block(
            Block::default()
                .title(title)
                .borders(borders)
                .border_style(border_style)
                .style(Style::default().bg(bg)),
        );

    frame.render_widget(para, area);
}

/// Availability of the current diff's grouped-summary data — drives what
/// `render_guide_sidebar` shows in place of the group/file list.
#[derive(Clone)]
pub enum GuideStatus {
    Ready,
    Pending,
    Error(String),
    Empty,
    Disabled,
}

/// Render the Guide sidebar: the AI-generated grouping of the current diff,
/// one group at a time, with its file list underneath. Replaces
/// `render_sidebar` in the same panel slot when `SidebarMode::Guide` is
/// active. No scrollbar in v1 — content clips on overflow.
#[allow(clippy::too_many_arguments)]
pub fn render_guide_sidebar(
    frame: &mut Frame,
    area: Rect,
    groups: &[DiffGroup],
    group_selected: usize,
    guide_file_selected: usize,
    file_diffs: &[FileDiff],
    viewed_files: &HashSet<usize>,
    is_focused: bool,
    status: GuideStatus,
) {
    let t = theme::get();
    let bg = t.ui.bg;
    let title_style = if is_focused {
        Style::default().fg(t.ui.border_focused)
    } else {
        Style::default().fg(t.ui.border_unfocused)
    };
    let border_style = Style::default().fg(t.ui.border_unfocused);
    let muted_style = Style::default().fg(t.ui.text_muted);

    let title = Line::from(vec![Span::styled(" [1] Guide ", title_style)]);
    // Same border convention as `render_sidebar`: no right border, since the
    // adjacent diff panel's left border stands in for it.
    let borders = Borders::TOP | Borders::LEFT | Borders::BOTTOM;
    let block = Block::default()
        .title(title)
        .borders(borders)
        .border_style(border_style)
        .style(Style::default().bg(bg));
    let inner_width = block.inner(area).width.saturating_sub(1).max(1) as usize;

    let lines: Vec<Line> = match status {
        GuideStatus::Pending => vec![Line::from(Span::styled("Generating guide…", muted_style))],
        GuideStatus::Error(e) => {
            vec![Line::from(Span::styled(
                format!("Guide failed: {e}"),
                muted_style,
            ))]
        }
        GuideStatus::Empty => vec![Line::from(Span::styled("No groups", muted_style))],
        GuideStatus::Disabled => vec![Line::from(Span::styled("Guide disabled", muted_style))],
        GuideStatus::Ready if groups.is_empty() => {
            vec![Line::from(Span::styled("No groups", muted_style))]
        }
        GuideStatus::Ready => {
            let group_selected = group_selected.min(groups.len() - 1);
            let group = &groups[group_selected];
            let mut lines = Vec::new();

            lines.push(Line::from(Span::styled(
                format!("{:02} / {:02}", group_selected + 1, groups.len()),
                muted_style,
            )));
            lines.push(Line::from(""));
            for wrapped in wrap_plain(&group.title, inner_width) {
                lines.push(Line::from(Span::styled(
                    wrapped,
                    Style::default().fg(t.ui.text_primary).bold(),
                )));
            }
            lines.push(Line::from(""));
            for wrapped in wrap_plain(&group.summary, inner_width) {
                lines.push(Line::from(Span::styled(
                    wrapped,
                    Style::default().fg(t.ui.text_primary),
                )));
            }
            lines.push(Line::from(""));
            for (idx, filename) in group.files.iter().enumerate() {
                let is_selected = idx == guide_file_selected;
                match file_diffs.iter().position(|f| &f.filename == filename) {
                    Some(file_index) => {
                        lines.push(file_row_line(
                            t,
                            0,
                            filename,
                            file_diffs[file_index].status,
                            viewed_files.contains(&file_index),
                            false,
                            is_selected,
                            is_focused,
                        ));
                    }
                    // File named in the group no longer matches any current
                    // file_diffs entry (e.g. reconciliation left a stale
                    // name) — show it plainly rather than dropping it.
                    None => {
                        lines.push(Line::from(Span::styled(
                            format!("  {}", filename),
                            muted_style,
                        )));
                    }
                }
            }
            lines
        }
    };

    let para = Paragraph::new(lines)
        .style(Style::default().bg(bg))
        .block(block);

    frame.render_widget(para, area);
}
