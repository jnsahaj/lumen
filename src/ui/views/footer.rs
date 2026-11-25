use std::collections::HashSet;

use ratatui::{prelude::*, widgets::Paragraph};

use crate::diff::PrInfo;
use crate::search::{SearchMode, SearchState};
use crate::ui::markdown::truncate_with_ellipsis;
use crate::ui::theme;

pub struct ReviewStatus<'a> {
    pub agent: &'a str,
    pub queued_comments: usize,
    pub notice: Option<&'a str>,
}

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
    pub review: &'a ReviewStatus<'a>,
}

/// Abbreviates leading directory names to their first character so the filename remains
/// legible in narrow footers (e.g. `alpha/beta/gamma/config.rs` -> `a/b/gamma/config.rs`).
pub fn truncate_path(path: &str, max_width: usize) -> String {
    if Span::raw(path).width() <= max_width {
        return path.to_string();
    }

    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= 1 {
        return truncate_with_ellipsis(path, max_width);
    }

    let filename = parts[parts.len() - 1];
    let dirs = &parts[..parts.len() - 1];

    for abbrev_count in 0..=dirs.len() {
        let mut result_parts: Vec<String> = Vec::with_capacity(parts.len());
        for (i, dir) in dirs.iter().enumerate() {
            if i < abbrev_count {
                if let Some(first_char) = dir.chars().next() {
                    result_parts.push(first_char.to_string());
                }
            } else {
                result_parts.push((*dir).to_string());
            }
        }
        result_parts.push(filename.to_string());

        let candidate = result_parts.join("/");
        if Span::raw(&candidate).width() <= max_width {
            return candidate;
        }
    }

    let abbreviated_dirs: Vec<String> = dirs
        .iter()
        .filter_map(|d| d.chars().next().map(|c| c.to_string()))
        .collect();
    let prefix = if abbreviated_dirs.is_empty() {
        String::new()
    } else {
        format!("{}/", abbreviated_dirs.join("/"))
    };

    let prefix_width = Span::raw(&prefix).width();
    let remaining = max_width.saturating_sub(prefix_width);
    if remaining > 0 {
        format!("{prefix}{}", truncate_with_ellipsis(filename, remaining))
    } else {
        truncate_with_ellipsis(&format!("{prefix}{filename}"), max_width)
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
        let mut spans = vec![
            Span::styled(prefix, Style::default().fg(t.ui.highlight).bg(bg)),
            Span::styled(
                &data.search_state.query,
                Style::default().fg(t.ui.text_primary).bg(bg),
            ),
            Span::styled("_", Style::default().fg(t.ui.text_muted).bg(bg)),
        ];
        let used_width: usize = spans.iter().map(Span::width).sum();
        let remaining_width = (footer_area.width as usize).saturating_sub(used_width);
        if remaining_width > 0 {
            spans.push(Span::styled(
                " ".repeat(remaining_width),
                Style::default().bg(bg),
            ));
        }
        let footer = Paragraph::new(Line::from(spans)).style(Style::default().bg(bg));
        frame.render_widget(footer, footer_area);
        return;
    }

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
        let is_fork = pr.head_repo_owner.as_ref() != Some(&pr.base_repo_owner);
        let base_label = if is_fork {
            format!(" {}:{} ", pr.base_repo_owner, pr.base_ref)
        } else {
            format!(" {} ", pr.base_ref)
        };
        let head_label = if is_fork {
            match &pr.head_repo_owner {
                Some(owner) => format!(" {owner}:{} ", pr.head_ref),
                None => format!(" {} ", pr.head_ref),
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
        spans.push(Span::styled(
            format!(" · {}", data.review.agent),
            Style::default().fg(t.ui.text_muted).bg(bg),
        ));
        if data.review.queued_comments > 0 {
            spans.push(Span::styled(
                format!(" · {} queued", data.review.queued_comments),
                Style::default().fg(t.ui.status_modified).bg(bg),
            ));
        }
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
                "[{current_idx}/{match_count}] /{} ",
                data.search_state.query
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
    } else if let Some(notice) = data.review.notice {
        vec![Span::styled(
            format!("{notice} "),
            Style::default().fg(t.ui.highlight).bg(bg),
        )]
    } else {
        let noun = if data.hunk_count == 1 {
            "hunk"
        } else {
            "hunks"
        };
        let hunk_label = match data.focused_hunk {
            Some(idx) => format!("({}/{} {noun}) ", idx + 1, data.hunk_count),
            None => format!("({} {noun}) ", data.hunk_count),
        };
        vec![
            Span::styled(hunk_label, Style::default().fg(t.ui.text_muted).bg(bg)),
            Span::styled(" ? help ", Style::default().fg(t.ui.text_muted).bg(bg)),
        ]
    };

    let left_line = Line::from(left_spans);
    let right_line = Line::from(right_spans);
    let footer_width = footer_area.width as usize;
    let padding = footer_width
        .saturating_sub(left_line.width() + right_line.width())
        .max(1);

    let mut final_spans: Vec<Span> = left_line.spans;
    final_spans.push(Span::styled(" ".repeat(padding), Style::default().bg(bg)));
    final_spans.extend(right_line.spans);

    let footer = Paragraph::new(Line::from(final_spans)).style(Style::default().bg(bg));
    frame.render_widget(footer, footer_area);
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;

    #[test]
    fn test_truncate_path_abbreviates_dirs_and_handles_multibyte_utf8() {
        assert_eq!(truncate_path("src/main.rs", 30), "src/main.rs");
        assert_eq!(
            truncate_path("alpha/beta/gamma/config.rs", 20),
            "a/b/gamma/config.rs"
        );
        let wide = truncate_path("données/résumé_très_long_fichier.rs", 14);
        assert!(Span::raw(&wide).width() <= 14, "got {wide:?}");
        assert!(truncate_path("日本語ファイル名.rs", 6).ends_with('…'));
        assert_eq!(truncate_path("a/b/c.rs", 0), "");
    }

    #[test]
    fn test_active_search_footer_does_not_underflow_on_long_query() {
        let mut search = SearchState::default();
        search.start_forward();
        search.query = "a_very_long_search_query_exceeding_terminal_width_日本語".to_string();

        let viewed = HashSet::new();
        let review = ReviewStatus {
            agent: "no agent",
            queued_comments: 0,
            notice: None,
        };
        let mut terminal = Terminal::new(TestBackend::new(20, 1)).unwrap();
        terminal
            .draw(|frame| {
                render_footer(
                    frame,
                    frame.area(),
                    FooterData {
                        filename: "src/lib.rs",
                        commit_ref: "main",
                        pr_info: None,
                        watching: false,
                        current_file: 0,
                        viewed_files: &viewed,
                        line_stats_added: 0,
                        line_stats_removed: 0,
                        hunk_count: 0,
                        focused_hunk: None,
                        search_state: &search,
                        area_width: 20,
                        review: &review,
                    },
                );
            })
            .unwrap();
    }
}
