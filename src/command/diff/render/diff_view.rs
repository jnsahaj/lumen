use std::collections::HashSet;

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::command::diff::context::{compute_context_lines, ContextLine};
use crate::command::diff::diff_algo::compute_side_by_side;
use crate::command::diff::highlight::highlight_line_spans;
use crate::command::diff::search::{MatchPanel, SearchState};
use crate::command::diff::theme;
use crate::command::diff::types::{
    ChangeType, DiffFullscreen, DiffLine, DiffViewSettings, FileDiff, FocusedPanel, SidebarItem,
};
use crate::command::diff::PrInfo;

use super::footer::{render_footer, FooterData};
use super::sidebar::render_sidebar;

/// Generates a diagonal stripe pattern for empty placeholder lines in the diff view.
/// The pattern uses forward slashes to create a visual distinction for empty areas.
fn generate_stripe_pattern(width: usize) -> String {
    "╱".repeat(width)
}

pub struct LineStats {
    pub added: usize,
    pub removed: usize,
}

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

    let base_spans = highlight_line_spans(text, filename, bg);
    let mut result: Vec<Span<'a>> = Vec::new();
    let mut char_pos = 0;

    for span in base_spans {
        let span_text = span.content.to_string();
        let span_len = span_text.len();
        let span_end = char_pos + span_len;

        let mut current_pos = 0;
        let mut remaining = span_text.as_str();

        for &(match_start, match_end, is_current) in match_ranges {
            if match_end <= char_pos || match_start >= span_end {
                continue;
            }

            let rel_start = match_start.saturating_sub(char_pos);
            let rel_end = (match_end - char_pos).min(span_len);

            if rel_start > current_pos {
                let before = &remaining[..(rel_start - current_pos)];
                if !before.is_empty() {
                    result.push(Span::styled(before.to_string(), span.style));
                }
            }

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
            ChangeType::Modified => {
                // A modified line counts as both a removal and an addition
                added += 1;
                removed += 1;
            }
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
    let msg = Paragraph::new(format!("No changes detected.{}", watch_hint))
        .block(Block::default().title(" Git Review ").borders(Borders::ALL));
    frame.render_widget(msg, frame.area());
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
            spans.extend(highlight_line_spans(
                &cl.content,
                filename,
                Some(context_bg),
            ));
            lines.push(Line::from(spans));
        } else {
            lines.push(Line::from(vec![Span::styled(
                "     ~".to_string(),
                Style::default().fg(t.ui.line_number).bg(context_bg),
            )]));
        }
    }
}

#[allow(clippy::too_many_arguments)]
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
    branch: &str,
    pr_info: Option<&PrInfo>,
    focused_hunk: Option<usize>,
    hunks: &[usize],
) {
    let area = frame.area();
    let side_by_side =
        compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
    let line_stats = compute_line_stats(&side_by_side);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    let main_area = if show_sidebar {
        let sidebar_width = (area.width / 4).min(35).max(20);
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(sidebar_width), Constraint::Min(0)])
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

    let is_new_file = diff.old_content.is_empty() && !diff.new_content.is_empty();
    let is_deleted_file = !diff.old_content.is_empty() && diff.new_content.is_empty();

    let t = theme::get();
    let border_style = Style::default().fg(t.ui.border_unfocused);
    let title_style = if focused_panel == FocusedPanel::DiffView {
        Style::default().fg(t.ui.border_focused)
    } else {
        Style::default().fg(t.ui.border_unfocused)
    };

    if is_new_file {
        let visible_height = main_area.height.saturating_sub(2) as usize;
        let new_context = compute_context_lines(
            &diff.new_content,
            &diff.filename,
            scroll as usize,
            &settings.context,
            settings.tab_width,
        );
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
                let prefix = format!("{:4}  ", num);
                let mut spans: Vec<Span> = vec![Span::styled(
                    prefix,
                    Style::default()
                        .fg(t.diff.added_gutter_fg)
                        .bg(t.diff.added_gutter_bg),
                )];
                let matches = search_state.get_matches_for_line(line_idx, MatchPanel::New);
                spans.extend(apply_search_highlight(
                    text,
                    &diff.filename,
                    Some(t.diff.added_bg),
                    &matches,
                ));
                new_lines.push(Line::from(spans));
            }
        }

        let new_para = Paragraph::new(new_lines).scroll((0, h_scroll)).block(
            Block::default()
                .title(Line::styled(" [2] New File ", title_style))
                .borders(Borders::ALL)
                .border_style(border_style),
        );
        frame.render_widget(new_para, main_area);
    } else if is_deleted_file {
        let visible_height = main_area.height.saturating_sub(2) as usize;
        let old_context = compute_context_lines(
            &diff.old_content,
            &diff.filename,
            scroll as usize,
            &settings.context,
            settings.tab_width,
        );
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
                let prefix = format!("{:4}  ", num);
                let mut spans: Vec<Span> = vec![Span::styled(
                    prefix,
                    Style::default()
                        .fg(t.diff.deleted_gutter_fg)
                        .bg(t.diff.deleted_gutter_bg),
                )];
                let matches = search_state.get_matches_for_line(line_idx, MatchPanel::Old);
                spans.extend(apply_search_highlight(
                    text,
                    &diff.filename,
                    Some(t.diff.deleted_bg),
                    &matches,
                ));
                old_lines.push(Line::from(spans));
            }
        }

        let old_para = Paragraph::new(old_lines).scroll((0, h_scroll)).block(
            Block::default()
                .title(Line::styled(" [2] Deleted File ", title_style))
                .borders(Borders::ALL)
                .border_style(border_style),
        );
        frame.render_widget(old_para, main_area);
    } else {
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

        let old_context = compute_context_lines(
            &diff.old_content,
            &diff.filename,
            scroll as usize,
            &settings.context,
            settings.tab_width,
        );
        let new_context = compute_context_lines(
            &diff.new_content,
            &diff.filename,
            scroll as usize,
            &settings.context,
            settings.tab_width,
        );
        let context_count = old_context.len().max(new_context.len());

        let reference_area = old_area.or(new_area).unwrap_or(main_area);
        let visible_height = reference_area.height.saturating_sub(2) as usize;
        let scroll_usize = scroll as usize;

        let content_height = visible_height.saturating_sub(context_count);
        let visible_lines: Vec<&DiffLine> = side_by_side
            .iter()
            .skip(scroll_usize)
            .take(content_height)
            .collect();

        let mut old_lines: Vec<Line> = Vec::new();
        let mut new_lines: Vec<Line> = Vec::new();

        if settings.context.enabled && context_count > 0 {
            if old_area.is_some() {
                render_context_lines(&old_context, context_count, &mut old_lines, &diff.filename);
            }
            if new_area.is_some() {
                render_context_lines(&new_context, context_count, &mut new_lines, &diff.filename);
            }
        }

        let is_in_focused_hunk = |line_idx: usize, change_type: ChangeType| -> bool {
            if matches!(change_type, ChangeType::Equal) {
                return false;
            }
            if let Some(hunk_idx) = focused_hunk {
                if let Some(&hunk_start) = hunks.get(hunk_idx) {
                    let hunk_end = hunks.get(hunk_idx + 1).copied().unwrap_or(usize::MAX);
                    return line_idx >= hunk_start && line_idx < hunk_end;
                }
            }
            false
        };

        for (i, diff_line) in visible_lines.iter().enumerate() {
            let line_idx = scroll_usize + i;
            let in_focused = is_in_focused_hunk(line_idx, diff_line.change_type);
            let (old_bg, old_gutter_bg, old_gutter_fg, new_bg, new_gutter_bg, new_gutter_fg) =
                match diff_line.change_type {
                    ChangeType::Equal => (None, None, None, None, None, None),
                    ChangeType::Delete => (
                        Some(t.diff.deleted_bg),
                        Some(t.diff.deleted_gutter_bg),
                        Some(t.diff.deleted_gutter_fg),
                        None,
                        None,
                        None,
                    ),
                    ChangeType::Insert => (
                        None,
                        None,
                        None,
                        Some(t.diff.added_bg),
                        Some(t.diff.added_gutter_bg),
                        Some(t.diff.added_gutter_fg),
                    ),
                    ChangeType::Modified => (
                        Some(t.diff.deleted_bg),
                        Some(t.diff.deleted_gutter_bg),
                        Some(t.diff.deleted_gutter_fg),
                        Some(t.diff.added_bg),
                        Some(t.diff.added_gutter_bg),
                        Some(t.diff.added_gutter_fg),
                    ),
                };

            let focus_indicator = if in_focused { "▎" } else { " " };
            let focus_style = Style::default().fg(t.ui.border_focused);

            if old_area.is_some() {
                let mut old_spans: Vec<Span> = Vec::new();
                old_spans.push(Span::styled(focus_indicator, focus_style));
                match &diff_line.old_line {
                    Some((num, text)) => {
                        let prefix = format!("{:4} ", num);
                        old_spans.push(Span::styled(
                            prefix,
                            Style::default()
                                .fg(old_gutter_fg.unwrap_or(t.ui.line_number))
                                .bg(old_gutter_bg.unwrap_or(Color::Reset)),
                        ));
                        let matches = search_state.get_matches_for_line(line_idx, MatchPanel::Old);
                        old_spans.extend(apply_search_highlight(
                            text,
                            &diff.filename,
                            old_bg,
                            &matches,
                        ));
                    }
                    None => {
                        let panel_width = old_area.map(|a| a.width as usize).unwrap_or(80);
                        let content_width = panel_width.saturating_sub(8);
                        let pattern = generate_stripe_pattern(content_width);
                        old_spans.push(Span::styled(
                            "     ",
                            Style::default().fg(t.diff.empty_placeholder_fg),
                        ));
                        old_spans.push(Span::styled(
                            pattern,
                            Style::default().fg(t.diff.empty_placeholder_fg),
                        ));
                    }
                }
                old_lines.push(Line::from(old_spans));
            }

            if new_area.is_some() {
                let mut new_spans: Vec<Span> = Vec::new();
                if old_area.is_none() {
                    new_spans.push(Span::styled(focus_indicator, focus_style));
                }
                match &diff_line.new_line {
                    Some((num, text)) => {
                        let prefix = if old_area.is_some() {
                            format!("{:4} ", num)
                        } else {
                            format!("{:4} ", num)
                        };
                        new_spans.push(Span::styled(
                            prefix,
                            Style::default()
                                .fg(new_gutter_fg.unwrap_or(t.ui.line_number))
                                .bg(new_gutter_bg.unwrap_or(Color::Reset)),
                        ));
                        let matches = search_state.get_matches_for_line(line_idx, MatchPanel::New);
                        new_spans.extend(apply_search_highlight(
                            text,
                            &diff.filename,
                            new_bg,
                            &matches,
                        ));
                    }
                    None => {
                        let panel_width = new_area.map(|a| a.width as usize).unwrap_or(80);
                        let content_width = panel_width.saturating_sub(8);
                        let pattern = generate_stripe_pattern(content_width);
                        new_spans.push(Span::styled(
                            "     ",
                            Style::default().fg(t.diff.empty_placeholder_fg),
                        ));
                        new_spans.push(Span::styled(
                            pattern,
                            Style::default().fg(t.diff.empty_placeholder_fg),
                        ));
                    }
                }
                new_lines.push(Line::from(new_spans));
            }
        }

        if let Some(area) = old_area {
            let old_para = Paragraph::new(old_lines).scroll((0, h_scroll)).block(
                Block::default()
                    .title(Line::styled(" [2] Old ", title_style))
                    .borders(Borders::ALL)
                    .border_style(border_style),
            );
            frame.render_widget(old_para, area);
        }

        if let Some(area) = new_area {
            // When both panels are shown, new panel has no left border to share with old panel
            let new_borders = if old_area.is_some() {
                Borders::TOP | Borders::RIGHT | Borders::BOTTOM
            } else {
                Borders::ALL
            };
            let new_para = Paragraph::new(new_lines).scroll((0, h_scroll)).block(
                Block::default()
                    .title(Line::styled(" New ", title_style))
                    .borders(new_borders)
                    .border_style(border_style),
            );
            frame.render_widget(new_para, area);
        }
    }

    render_footer(
        frame,
        chunks[1],
        FooterData {
            filename: &diff.filename,
            branch,
            pr_info,
            watching,
            current_file,
            viewed_files,
            line_stats_added: line_stats.added,
            line_stats_removed: line_stats.removed,
            hunk_count,
            focused_hunk,
            search_state,
            area_width: area.width,
        },
    );
}
