use std::collections::HashSet;

use ratatui::{prelude::*, widgets::Paragraph};

use crate::command::diff::search::{SearchMode, SearchState};
use crate::command::diff::theme;
use crate::command::diff::PrInfo;

pub struct FooterData<'a> {
    pub filename: &'a str,
    pub branch: &'a str,
    pub pr_info: Option<&'a PrInfo>,
    pub watching: bool,
    pub current_file: usize,
    pub viewed_files: &'a HashSet<usize>,
    pub line_stats_added: usize,
    pub line_stats_removed: usize,
    pub hunk_count: usize,
    pub search_state: &'a SearchState,
    pub area_width: u16,
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

pub fn render_footer(frame: &mut Frame, footer_area: Rect, data: FooterData) {
    let t = theme::get();
    let bg = t.ui.footer_bg;

    if data.search_state.is_active() {
        let prefix = match data.search_state.mode {
            SearchMode::InputForward => "/",
            SearchMode::Inactive => "",
        };
        let search_spans = vec![
            Span::styled(prefix, Style::default().fg(t.ui.highlight).bg(bg)),
            Span::styled(
                &data.search_state.query,
                Style::default().fg(t.ui.text_primary).bg(bg),
            ),
            Span::styled("_", Style::default().fg(t.ui.text_muted).bg(bg)),
        ];
        let remaining_width =
            footer_area.width as usize - prefix.len() - data.search_state.query.len() - 1;
        let mut spans = search_spans;
        spans.push(Span::styled(
            " ".repeat(remaining_width),
            Style::default().bg(bg),
        ));
        let footer = Paragraph::new(Line::from(spans)).style(Style::default().bg(bg));
        frame.render_widget(footer, footer_area);
    } else {
        let watch_indicator = if data.watching { " watching" } else { "" };
        let max_filename_len = if data.search_state.has_query() {
            (data.area_width as usize).saturating_sub(80).min(40)
        } else {
            (data.area_width as usize).saturating_sub(60).min(50)
        };
        let truncated_filename = truncate_middle(data.filename, max_filename_len);
        let viewed_indicator = if data.viewed_files.contains(&data.current_file) {
            " ✓"
        } else {
            ""
        };

        let left_spans = if let Some(pr) = data.pr_info {
            // PR mode: show "base <- head #123" or "owner:base <- owner:head #123" for forks
            let is_fork = pr
                .head_repo_owner
                .as_ref()
                .map_or(true, |head_owner| head_owner != &pr.base_repo_owner);

            let base_label = if is_fork {
                format!(" {}:{} ", pr.base_repo_owner, pr.base_ref)
            } else {
                format!(" {} ", pr.base_ref)
            };

            let head_label = if is_fork {
                match &pr.head_repo_owner {
                    Some(owner) => format!(" {}:{} ", owner, pr.head_ref),
                    None => format!(" {} ", pr.head_ref), // Fork was deleted
                }
            } else {
                format!(" {} ", pr.head_ref)
            };

            vec![
                Span::styled(" ", Style::default().bg(bg)),
                Span::styled(
                    base_label,
                    Style::default()
                        .fg(t.ui.footer_branch_fg)
                        .bg(t.ui.footer_branch_bg),
                ),
                Span::styled(" <- ", Style::default().fg(t.ui.text_muted).bg(bg)),
                Span::styled(
                    head_label,
                    Style::default()
                        .fg(t.ui.footer_branch_fg)
                        .bg(t.ui.footer_branch_bg),
                ),
                Span::styled(" ", Style::default().bg(bg)),
                Span::styled(
                    truncated_filename,
                    Style::default().fg(t.ui.text_secondary).bg(bg),
                ),
                Span::styled(viewed_indicator, Style::default().fg(t.ui.viewed).bg(bg)),
            ]
        } else {
            // Normal diff mode: show branch name
            vec![
                Span::styled(" ", Style::default().bg(bg)),
                Span::styled(
                    format!(" {} ", data.branch),
                    Style::default()
                        .fg(t.ui.footer_branch_fg)
                        .bg(t.ui.footer_branch_bg),
                ),
                Span::styled(" ", Style::default().bg(bg)),
                Span::styled(
                    truncated_filename,
                    Style::default().fg(t.ui.text_secondary).bg(bg),
                ),
                Span::styled(viewed_indicator, Style::default().fg(t.ui.viewed).bg(bg)),
                Span::styled(watch_indicator, Style::default().fg(t.ui.watching).bg(bg)),
            ]
        };

        let (center_spans, right_spans) = if data.search_state.has_query() {
            let match_count = data.search_state.match_count();
            let current_idx = data
                .search_state
                .current_match_index()
                .map(|i| i + 1)
                .unwrap_or(0);
            let search_info = if match_count > 0 {
                format!(
                    "[{}/{}] /{}",
                    current_idx, match_count, data.search_state.query
                )
            } else {
                format!("[0/0] /{}", data.search_state.query)
            };
            (
                vec![Span::styled(
                    search_info,
                    Style::default().fg(t.ui.highlight).bg(bg),
                )],
                vec![Span::styled(
                    " n/N navigate ",
                    Style::default().fg(t.ui.text_muted).bg(bg),
                )],
            )
        } else {
            (
                vec![
                    Span::styled(
                        format!("+{}", data.line_stats_added),
                        Style::default().fg(t.ui.stats_added).bg(bg),
                    ),
                    Span::styled(" ", Style::default().bg(bg)),
                    Span::styled(
                        format!("-{}", data.line_stats_removed),
                        Style::default().fg(t.ui.stats_removed).bg(bg),
                    ),
                    Span::styled(" ", Style::default().bg(bg)),
                    Span::styled(
                        format!(
                            "({} {})",
                            data.hunk_count,
                            if data.hunk_count == 1 {
                                "hunk"
                            } else {
                                "hunks"
                            }
                        ),
                        Style::default().fg(t.ui.text_muted).bg(bg),
                    ),
                ],
                vec![Span::styled(
                    " ? help ",
                    Style::default().fg(t.ui.text_muted).bg(bg),
                )],
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
        final_spans.push(Span::styled(
            " ".repeat(left_padding),
            Style::default().bg(bg),
        ));
        final_spans.extend(center_line.spans);
        final_spans.push(Span::styled(
            " ".repeat(right_padding),
            Style::default().bg(bg),
        ));
        final_spans.extend(right_line.spans);

        let footer = Paragraph::new(Line::from(final_spans)).style(Style::default().bg(bg));
        frame.render_widget(footer, footer_area);
    }
}
