use std::collections::HashSet;

use ratatui::{prelude::*, widgets::Paragraph};

use crate::command::diff::search::{SearchMode, SearchState};
use crate::command::diff::theme;
use crate::command::diff::PrInfo;

pub struct FooterData<'a> {
    pub filename: &'a str,
    pub commit_ref: &'a str,
    pub pr_info: Option<&'a PrInfo>,
    pub watching: bool,
    pub current_file: usize,
    pub viewed_files: &'a HashSet<usize>,
    pub line_stats_added: usize,
    pub line_stats_removed: usize,
    pub hunk_count: usize,
    pub focused_hunk: Option<usize>,
    pub search_state: &'a SearchState,
    pub area_width: u16,
}

/// Truncates a file path by abbreviating directory names to their first character.
/// For example: "aadfadf/bsdff/casdfdsf/config.rs" -> "a/b/casdfdsf/config.rs"
/// Keeps the last directory component and filename intact when possible.
pub fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        return path.to_string();
    }

    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= 1 {
        // No directories, just truncate the filename at the end
        if path.len() > max_len {
            return format!("{}...", &path[..max_len.saturating_sub(3)]);
        }
        return path.to_string();
    }

    // Start by abbreviating directories from the beginning
    let filename = parts[parts.len() - 1];
    let dirs = &parts[..parts.len() - 1];

    // Try progressively abbreviating more directories from the start
    for abbrev_count in 0..=dirs.len() {
        let mut result_parts: Vec<String> = Vec::new();

        // Abbreviate first `abbrev_count` directories to first char
        for (i, dir) in dirs.iter().enumerate() {
            if i < abbrev_count {
                // Abbreviate to first character
                if let Some(first_char) = dir.chars().next() {
                    result_parts.push(first_char.to_string());
                }
            } else {
                result_parts.push((*dir).to_string());
            }
        }
        result_parts.push(filename.to_string());

        let result = result_parts.join("/");
        if result.len() <= max_len {
            return result;
        }
    }

    // If still too long after abbreviating all dirs, truncate the filename
    let abbreviated_dirs: Vec<String> = dirs
        .iter()
        .filter_map(|d| d.chars().next().map(|c| c.to_string()))
        .collect();
    let prefix = if abbreviated_dirs.is_empty() {
        String::new()
    } else {
        format!("{}/", abbreviated_dirs.join("/"))
    };

    let remaining = max_len.saturating_sub(prefix.len());
    if remaining > 3 && filename.len() > remaining {
        format!("{}{}...", prefix, &filename[..remaining.saturating_sub(3)])
    } else {
        format!("{}{}", prefix, filename)
    }
}

pub fn render_footer(frame: &mut Frame, footer_area: Rect, data: FooterData) {
    let t = theme::get();
    let bg = t.ui.bg;

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
        let truncated_filename = truncate_path(data.filename, max_filename_len);
        let viewed_indicator = if data.viewed_files.contains(&data.current_file) {
            " ✓"
        } else {
            ""
        };

        // Build stats spans (shown after filename)
        let stats_spans: Vec<Span> = if data.search_state.has_query() {
            vec![]
        } else {
            vec![
                Span::styled(" ", Style::default().bg(bg)),
                Span::styled(
                    format!("+{}", data.line_stats_added),
                    Style::default().fg(t.ui.stats_added).bg(bg),
                ),
                Span::styled(" ", Style::default().bg(bg)),
                Span::styled(
                    format!("-{}", data.line_stats_removed),
                    Style::default().fg(t.ui.stats_removed).bg(bg),
                ),
            ]
        };

        let left_spans = if let Some(pr) = data.pr_info {
            // PR mode: show "base <- head #123" or "owner:base <- owner:head #123" for forks
            let is_fork = pr.head_repo_owner.as_ref() != Some(&pr.base_repo_owner);

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

            let mut spans = vec![
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
            ];
            spans.extend(stats_spans);
            spans
        } else {
            // Normal diff mode: show commit reference
            let mut spans = vec![
                Span::styled(" ", Style::default().bg(bg)),
                Span::styled(
                    format!(" {} ", data.commit_ref),
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
            ];
            spans.extend(stats_spans);
            spans.push(Span::styled(
                watch_indicator,
                Style::default().fg(t.ui.watching).bg(bg),
            ));
            spans
        };

        let right_spans: Vec<Span> = if data.search_state.has_query() {
            let match_count = data.search_state.match_count();
            let current_idx = data
                .search_state
                .current_match_index()
                .map(|i| i + 1)
                .unwrap_or(0);
            let search_info = if match_count > 0 {
                format!(
                    "[{}/{}] /{} ",
                    current_idx, match_count, data.search_state.query
                )
            } else {
                format!("[0/0] /{} ", data.search_state.query)
            };
            vec![
                Span::styled(search_info, Style::default().fg(t.ui.highlight).bg(bg)),
                Span::styled(
                    " n/N navigate ",
                    Style::default().fg(t.ui.text_muted).bg(bg),
                ),
            ]
        } else {
            vec![
                Span::styled(
                    if let Some(idx) = data.focused_hunk {
                        format!(
                            "({}/{} {}) ",
                            idx + 1,
                            data.hunk_count,
                            if data.hunk_count == 1 {
                                "hunk"
                            } else {
                                "hunks"
                            }
                        )
                    } else {
                        format!(
                            "({} {}) ",
                            data.hunk_count,
                            if data.hunk_count == 1 {
                                "hunk"
                            } else {
                                "hunks"
                            }
                        )
                    },
                    Style::default().fg(t.ui.text_muted).bg(bg),
                ),
                Span::styled(" ? help ", Style::default().fg(t.ui.text_muted).bg(bg)),
            ]
        };

        let left_line = Line::from(left_spans);
        let right_line = Line::from(right_spans);

        let footer_width = footer_area.width as usize;
        let left_len = left_line.width();
        let right_len = right_line.width();

        // Simple left-aligned layout with right section
        let padding = footer_width.saturating_sub(left_len + right_len);

        let mut final_spans: Vec<Span> = left_line.spans;
        final_spans.push(Span::styled(" ".repeat(padding), Style::default().bg(bg)));
        final_spans.extend(right_line.spans);

        let footer = Paragraph::new(Line::from(final_spans)).style(Style::default().bg(bg));
        frame.render_widget(footer, footer_area);
    }
}
