use std::collections::HashSet;

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::diff_ui::context::{compute_context_lines, ContextLine};
use crate::diff_ui::diff::compute_side_by_side;
use crate::diff_ui::git::get_current_branch;
use crate::diff_ui::highlight::highlight_line_spans;
use crate::diff_ui::search::{MatchPanel, SearchState};
use crate::diff_ui::theme;
use crate::diff_ui::types::{ChangeType, DiffFullscreen, DiffLine, DiffViewSettings, FileDiff, FocusedPanel, SidebarItem};

pub struct LineStats {
    pub added: usize,
    pub removed: usize,
}

/// Apply search highlighting to text. match_ranges contains (start_col, end_col, is_current_match)
fn apply_search_highlight<'a>(
    text: &str,
    filename: &str,
    bg: Option<Color>,
    match_ranges: &[(usize, usize, bool)],
) -> Vec<Span<'a>> {
    let t = theme::get();
    
    if match_ranges.is_empty() {
        return highlight_line_spans(text, filename, bg);
    }
    
    // Get base highlighted spans
    let base_spans = highlight_line_spans(text, filename, bg);
    
    // Now we need to split spans at match boundaries and apply search highlight
    let mut result: Vec<Span<'a>> = Vec::new();
    let mut char_pos = 0;
    
    for span in base_spans {
        let span_text = span.content.to_string();
        let span_len = span_text.len();
        let span_end = char_pos + span_len;
        
        // Check if any match overlaps with this span
        let mut current_pos = 0;
        let mut remaining = span_text.as_str();
        
        for &(match_start, match_end, is_current) in match_ranges {
            if match_end <= char_pos || match_start >= span_end {
                // No overlap
                continue;
            }
            
            // Calculate overlap within this span
            let rel_start = match_start.saturating_sub(char_pos);
            let rel_end = (match_end - char_pos).min(span_len);
            
            // Add text before match (if any)
            if rel_start > current_pos {
                let before = &remaining[..(rel_start - current_pos)];
                if !before.is_empty() {
                    result.push(Span::styled(before.to_string(), span.style));
                }
            }
            
            // Add highlighted match portion
            let match_portion_start = rel_start.max(current_pos) - current_pos;
            let match_portion_end = rel_end - current_pos;
            if match_portion_end > match_portion_start {
                let match_text = &remaining[match_portion_start..match_portion_end];
                if !match_text.is_empty() {
                    let (fg, bg) = if is_current {
                        (t.ui.search_current_fg, t.ui.search_current_bg)
                    } else {
                        (t.ui.search_match_fg, t.ui.search_match_bg)
                    };
                    result.push(Span::styled(
                        match_text.to_string(),
                        Style::default().fg(fg).bg(bg).bold(),
                    ));
                }
            }
            
            remaining = &remaining[(rel_end - current_pos).min(remaining.len())..];
            current_pos = rel_end;
        }
        
        // Add any remaining text after matches
        if !remaining.is_empty() {
            result.push(Span::styled(remaining.to_string(), span.style));
        }
        
        char_pos = span_end;
    }
    
    result
}

pub fn compute_line_stats(side_by_side: &[DiffLine]) -> LineStats {
    let mut added = 0;
    let mut removed = 0;
    for line in side_by_side {
        match line.change_type {
            ChangeType::Insert => added += 1,
            ChangeType::Delete => removed += 1,
            ChangeType::Equal => {}
        }
    }
    LineStats { added, removed }
}

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
    _file_diffs: &[FileDiff],
    sidebar_items: &[SidebarItem],
    current_file: usize,
    scroll: u16,
    h_scroll: u16,
    watching: bool,
    show_sidebar: bool,
    focused_panel: FocusedPanel,
    sidebar_selected: usize,
    sidebar_scroll: usize,
    sidebar_h_scroll: u16,
    viewed_files: &HashSet<usize>,
    settings: &DiffViewSettings,
    hunk_count: usize,
    diff_fullscreen: DiffFullscreen,
    search_state: &SearchState,
) {
    let area = frame.area();
    let side_by_side = compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
    let line_stats = compute_line_stats(&side_by_side);
    let branch = get_current_branch();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let main_area = if show_sidebar {
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(45), Constraint::Min(0)])
            .split(chunks[0]);

        render_sidebar(
            frame,
            main_chunks[0],
            sidebar_items,
            current_file,
            sidebar_selected,
            sidebar_scroll,
            sidebar_h_scroll,
            viewed_files,
            focused_panel == FocusedPanel::Sidebar,
        );

        main_chunks[1]
    } else {
        chunks[0]
    };

    // Determine if this is a new file (no old content) or deleted file (no new content)
    let is_new_file = diff.old_content.is_empty() && !diff.new_content.is_empty();
    let is_deleted_file = !diff.old_content.is_empty() && diff.new_content.is_empty();

    let t = theme::get();
    let diff_title_style = if focused_panel == FocusedPanel::DiffView {
        Style::default().fg(t.ui.border_focused)
    } else {
        Style::default().fg(t.ui.border_unfocused)
    };

    if is_new_file {
        // Show only the new file panel
        let visible_height = main_area.height.saturating_sub(2) as usize;
        let new_context = compute_context_lines(&diff.new_content, &diff.filename, scroll as usize, &settings.context, settings.tab_width);
        let context_count = new_context.len();
        let content_height = visible_height.saturating_sub(context_count);

        let visible_lines: Vec<&DiffLine> = side_by_side
            .iter()
            .skip(scroll as usize)
            .take(content_height)
            .collect();

        let mut new_lines: Vec<Line> = Vec::new();
        if settings.context.enabled && context_count > 0 {
            render_context_lines(&new_context, context_count, &mut new_lines, &diff.filename);
        }

        for (i, diff_line) in visible_lines.iter().enumerate() {
            let line_idx = scroll as usize + i;
            if let Some((num, text)) = &diff_line.new_line {
                let prefix = format!("{:4} | ", num);
                let mut spans: Vec<Span> = vec![Span::styled(
                    prefix,
                    Style::default()
                        .fg(t.ui.line_number)
                        .bg(t.diff.added_bg),
                )];
                let matches = search_state.get_matches_for_line(line_idx, MatchPanel::New);
                spans.extend(apply_search_highlight(text, &diff.filename, Some(t.diff.added_bg), &matches));
                new_lines.push(Line::from(spans));
            }
        }

        let new_para = Paragraph::new(new_lines)
            .scroll((0, h_scroll))
            .block(
                Block::default()
                    .title(" [2] New File ")
                    .borders(Borders::ALL)
                    .border_style(diff_title_style.patch(Style::default().fg(t.ui.status_added))),
            );
        frame.render_widget(new_para, main_area);
    } else if is_deleted_file {
        // Show only the old file panel
        let visible_height = main_area.height.saturating_sub(2) as usize;
        let old_context = compute_context_lines(&diff.old_content, &diff.filename, scroll as usize, &settings.context, settings.tab_width);
        let context_count = old_context.len();
        let content_height = visible_height.saturating_sub(context_count);

        let visible_lines: Vec<&DiffLine> = side_by_side
            .iter()
            .skip(scroll as usize)
            .take(content_height)
            .collect();

        let mut old_lines: Vec<Line> = Vec::new();
        if settings.context.enabled && context_count > 0 {
            render_context_lines(&old_context, context_count, &mut old_lines, &diff.filename);
        }

        for (i, diff_line) in visible_lines.iter().enumerate() {
            let line_idx = scroll as usize + i;
            if let Some((num, text)) = &diff_line.old_line {
                let prefix = format!("{:4} | ", num);
                let mut spans: Vec<Span> = vec![Span::styled(
                    prefix,
                    Style::default()
                        .fg(t.ui.line_number)
                        .bg(t.diff.deleted_bg),
                )];
                let matches = search_state.get_matches_for_line(line_idx, MatchPanel::Old);
                spans.extend(apply_search_highlight(text, &diff.filename, Some(t.diff.deleted_bg), &matches));
                old_lines.push(Line::from(spans));
            }
        }

        let old_para = Paragraph::new(old_lines)
            .scroll((0, h_scroll))
            .block(
                Block::default()
                    .title(" [2] Deleted File ")
                    .borders(Borders::ALL)
                    .border_style(diff_title_style.patch(Style::default().fg(t.ui.status_deleted))),
            );
        frame.render_widget(old_para, main_area);
    } else {
        // Standard side-by-side view (or fullscreen mode)
        let (old_area, new_area) = match diff_fullscreen {
            DiffFullscreen::OldOnly => (Some(main_area), None),
            DiffFullscreen::NewOnly => (None, Some(main_area)),
            DiffFullscreen::None => {
                let content_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(main_area);
                (Some(content_chunks[0]), Some(content_chunks[1]))
            }
        };

        // Compute context lines for old and new panels using tree-sitter
        let old_context = compute_context_lines(&diff.old_content, &diff.filename, scroll as usize, &settings.context, settings.tab_width);
        let new_context = compute_context_lines(&diff.new_content, &diff.filename, scroll as usize, &settings.context, settings.tab_width);
        let context_count = old_context.len().max(new_context.len());

        let reference_area = old_area.or(new_area).unwrap_or(main_area);
        let visible_height = reference_area.height.saturating_sub(2) as usize;
        let scroll_usize = scroll as usize;

        // Adjust visible lines to account for context lines
        let content_height = visible_height.saturating_sub(context_count);
        let visible_lines: Vec<&DiffLine> = side_by_side
            .iter()
            .skip(scroll_usize)
            .take(content_height)
            .collect();

        let mut old_lines: Vec<Line> = Vec::new();
        let mut new_lines: Vec<Line> = Vec::new();

        // Render context lines first (if enabled)
        if settings.context.enabled && context_count > 0 {
            if old_area.is_some() {
                render_context_lines(&old_context, context_count, &mut old_lines, &diff.filename);
            }
            if new_area.is_some() {
                render_context_lines(&new_context, context_count, &mut new_lines, &diff.filename);
            }
        }

        for (i, diff_line) in visible_lines.iter().enumerate() {
            let line_idx = scroll_usize + i;
            let (old_bg, new_bg) = match diff_line.change_type {
                ChangeType::Equal => (None, None),
                ChangeType::Delete => (Some(t.diff.deleted_bg), None),
                ChangeType::Insert => (None, Some(t.diff.added_bg)),
            };

            if old_area.is_some() {
                let mut old_spans: Vec<Span> = Vec::new();
                match &diff_line.old_line {
                    Some((num, text)) => {
                        let prefix = format!("{:4} | ", num);
                        old_spans.push(Span::styled(
                            prefix,
                            Style::default()
                                .fg(t.ui.line_number)
                                .bg(old_bg.unwrap_or(Color::Reset)),
                        ));
                        let matches = search_state.get_matches_for_line(line_idx, MatchPanel::Old);
                        old_spans.extend(apply_search_highlight(text, &diff.filename, old_bg, &matches));
                    }
                    None => {
                        old_spans.push(Span::styled("     |", Style::default().fg(t.ui.line_number)));
                    }
                }
                old_lines.push(Line::from(old_spans));
            }

            if new_area.is_some() {
                let mut new_spans: Vec<Span> = Vec::new();
                match &diff_line.new_line {
                    Some((num, text)) => {
                        let prefix = format!("{:4} | ", num);
                        new_spans.push(Span::styled(
                            prefix,
                            Style::default()
                                .fg(t.ui.line_number)
                                .bg(new_bg.unwrap_or(Color::Reset)),
                        ));
                        let matches = search_state.get_matches_for_line(line_idx, MatchPanel::New);
                        new_spans.extend(apply_search_highlight(text, &diff.filename, new_bg, &matches));
                    }
                    None => {
                        new_spans.push(Span::styled("     |", Style::default().fg(t.ui.line_number)));
                    }
                }
                new_lines.push(Line::from(new_spans));
            }
        }

        if let Some(area) = old_area {
            let old_para = Paragraph::new(old_lines)
                .scroll((0, h_scroll))
                .block(
                    Block::default()
                        .title(" [2] Old ")
                        .borders(Borders::ALL)
                        .border_style(diff_title_style.patch(Style::default().fg(t.ui.status_deleted))),
                );
            frame.render_widget(old_para, area);
        }

        if let Some(area) = new_area {
            let new_para = Paragraph::new(new_lines)
                .scroll((0, h_scroll))
                .block(
                    Block::default()
                        .title(" New ")
                        .borders(Borders::ALL)
                        .border_style(diff_title_style.patch(Style::default().fg(t.ui.status_added))),
                );
            frame.render_widget(new_para, area);
        }
    }

    // Render footer
    let footer_area = chunks[1];
    let bg = t.ui.footer_bg;

    if search_state.is_active() {
        // Show search input in footer
        use crate::diff_ui::search::SearchMode;
        let prefix = match search_state.mode {
            SearchMode::InputForward => "/",
            SearchMode::InputBackward => "?",
            SearchMode::Inactive => "",
        };
        let search_spans = vec![
            Span::styled(prefix, Style::default().fg(t.ui.highlight).bg(bg)),
            Span::styled(&search_state.query, Style::default().fg(t.ui.text_primary).bg(bg)),
            Span::styled("_", Style::default().fg(t.ui.text_muted).bg(bg)),
        ];
        let remaining_width = footer_area.width as usize - prefix.len() - search_state.query.len() - 1;
        let mut spans = search_spans;
        spans.push(Span::styled(" ".repeat(remaining_width), Style::default().bg(bg)));
        let footer = Paragraph::new(Line::from(spans)).style(Style::default().bg(bg));
        frame.render_widget(footer, footer_area);
    } else {
        // Build left spans (common to both search results and normal footer)
        let watch_indicator = if watching { " watching" } else { "" };
        let max_filename_len = if search_state.has_query() {
            (area.width as usize).saturating_sub(80).min(40)
        } else {
            (area.width as usize).saturating_sub(60).min(50)
        };
        let truncated_filename = truncate_middle(&diff.filename, max_filename_len);
        let viewed_indicator = if viewed_files.contains(&current_file) { " ✓" } else { "" };

        let left_spans = vec![
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(
                format!(" {} ", branch),
                Style::default().fg(t.ui.footer_branch_fg).bg(t.ui.footer_branch_bg),
            ),
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(truncated_filename, Style::default().fg(t.ui.text_secondary).bg(bg)),
            Span::styled(viewed_indicator, Style::default().fg(t.ui.viewed).bg(bg)),
            Span::styled(watch_indicator, Style::default().fg(t.ui.watching).bg(bg)),
        ];

        let (center_spans, right_spans) = if search_state.has_query() {
            let match_count = search_state.match_count();
            let current_idx = search_state.current_match_index().map(|i| i + 1).unwrap_or(0);
            let search_info = if match_count > 0 {
                format!("[{}/{}] /{}", current_idx, match_count, search_state.query)
            } else {
                format!("[0/0] /{}", search_state.query)
            };
            (
                vec![Span::styled(search_info, Style::default().fg(t.ui.highlight).bg(bg))],
                vec![Span::styled(" n/N navigate ", Style::default().fg(t.ui.text_muted).bg(bg))],
            )
        } else {
            (
                vec![
                    Span::styled(format!("+{}", line_stats.added), Style::default().fg(t.ui.stats_added).bg(bg)),
                    Span::styled(" ", Style::default().bg(bg)),
                    Span::styled(format!("-{}", line_stats.removed), Style::default().fg(t.ui.stats_removed).bg(bg)),
                    Span::styled(" ", Style::default().bg(bg)),
                    Span::styled(
                        format!("({} {})", hunk_count, if hunk_count == 1 { "hunk" } else { "hunks" }),
                        Style::default().fg(t.ui.text_muted).bg(bg),
                    ),
                ],
                vec![Span::styled(" ? help ", Style::default().fg(t.ui.text_muted).bg(bg))],
            )
        };

        let left_line = Line::from(left_spans);
        let center_line = Line::from(center_spans);
        let right_line = Line::from(right_spans);

        let footer_width = footer_area.width as usize;
        let left_len = left_line.width();
        let center_len = center_line.width();
        let right_len = right_line.width();

        let center_pos = footer_width / 2;
        let center_start = center_pos.saturating_sub(center_len / 2);
        let left_padding = center_start.saturating_sub(left_len);
        let right_padding = footer_width.saturating_sub(center_start + center_len + right_len);

        let mut final_spans: Vec<Span> = left_line.spans;
        final_spans.push(Span::styled(" ".repeat(left_padding), Style::default().bg(bg)));
        final_spans.extend(center_line.spans);
        final_spans.push(Span::styled(" ".repeat(right_padding), Style::default().bg(bg)));
        final_spans.extend(right_line.spans);

        let footer = Paragraph::new(Line::from(final_spans)).style(Style::default().bg(bg));
        frame.render_widget(footer, footer_area);
    }
}

fn render_context_lines(
    context: &[ContextLine],
    total_count: usize,
    lines: &mut Vec<Line>,
    filename: &str,
) {
    let t = theme::get();
    let context_bg = t.diff.context_bg;
    
    for i in 0..total_count {
        if let Some(cl) = context.get(i) {
            let prefix = format!("{:4} ~ ", cl.line_number);
            let mut spans: Vec<Span> = vec![Span::styled(
                prefix,
                Style::default().fg(t.ui.line_number).bg(context_bg),
            )];
            spans.extend(highlight_line_spans(&cl.content, filename, Some(context_bg)));
            lines.push(Line::from(spans));
        } else {
            // Empty context line placeholder (when other panel has more context lines)
            lines.push(Line::from(vec![Span::styled(
                "     ~".to_string(),
                Style::default().fg(t.ui.line_number).bg(context_bg),
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
    sidebar_h_scroll: u16,
    viewed_files: &HashSet<usize>,
    is_focused: bool,
) {
    let t = theme::get();
    let visible_height = area.height.saturating_sub(2) as usize;
    let lines: Vec<Line> = sidebar_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let (prefix, status_symbol, status_color, name, is_current_file, is_viewed) = match item {
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
                    (format!("{}{}", indent, marker), "▼".to_string(), None, format!(" {}", name), false, all_children_viewed && has_children)
                }
                SidebarItem::File {
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
                        crate::diff_ui::types::FileStatus::Modified => Some(t.ui.status_modified),
                        crate::diff_ui::types::FileStatus::Added => Some(t.ui.status_added),
                        crate::diff_ui::types::FileStatus::Deleted => Some(t.ui.status_deleted),
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

            let is_selected = i == sidebar_selected;
            let base_style = if is_selected {
                Style::default()
                    .fg(t.ui.selection_fg)
                    .bg(if is_focused {
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

    let border_color = if is_focused {
        t.ui.border_focused
    } else {
        t.ui.border_unfocused
    };

    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(sidebar_scroll)
        .take(visible_height)
        .collect();

    let para = Paragraph::new(visible_lines)
        .scroll((0, sidebar_h_scroll))
        .block(
            Block::default()
                .title(" [1] Files ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        );

    frame.render_widget(para, area);
}
