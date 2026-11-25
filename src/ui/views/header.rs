use ratatui::{prelude::*, widgets::Paragraph};

use crate::ui::markdown::truncate_with_ellipsis;
use crate::ui::theme;
use crate::vcs::StackedCommitInfo;

pub fn render_stacked_header(
    frame: &mut Frame,
    area: Rect,
    commit: Option<&StackedCommitInfo>,
    index: usize,
    total: usize,
    vcs_name: &str,
) {
    let t = theme::get();
    let bg = t.ui.bg;

    let can_go_prev = index > 0;
    let can_go_next = index < total.saturating_sub(1);

    let active_style = Style::default().fg(t.ui.text_primary).bg(bg);
    let dimmed_style = Style::default().fg(t.ui.text_muted).bg(bg);

    let left_style = if can_go_prev {
        active_style
    } else {
        dimmed_style
    };
    let right_style = if can_go_next {
        active_style
    } else {
        dimmed_style
    };

    let (commit_id_label, commit_msg) = if let Some(c) = commit {
        let id_str = if let Some(ref change_id) = c.change_id {
            let short_change: String = change_id.chars().take(8).collect();
            format!("{short_change} {}", c.short_id)
        } else {
            c.short_id.clone()
        };
        (id_str, c.summary.as_str())
    } else {
        ("?".to_string(), "No commit")
    };

    let vcs_indicator = format!(" {vcs_name} ");
    let nav_indicator = format!(" {}/{total} ", index + 1);
    let id_label = format!(" {commit_id_label} ");

    let available_for_msg = (area.width as usize)
        .saturating_sub(60 + Span::raw(&vcs_indicator).width() + Span::raw(&id_label).width());
    let truncated_msg = truncate_with_ellipsis(commit_msg, available_for_msg);

    let badge_style = Style::default().bg(t.ui.footer_branch_bg);
    let spacer_style = Style::default().bg(bg);
    let center_spans = vec![
        Span::styled(vcs_indicator, badge_style.fg(t.ui.text_muted)),
        Span::styled(" ", spacer_style),
        Span::styled(nav_indicator, badge_style.fg(t.ui.highlight)),
        Span::styled(" ", spacer_style),
        Span::styled(id_label, badge_style.fg(t.ui.footer_branch_fg)),
        Span::styled("  ", spacer_style),
        Span::styled(
            truncated_msg,
            Style::default().fg(t.ui.text_secondary).bg(bg),
        ),
    ];

    let center_width: usize = center_spans.iter().map(Span::width).sum();
    let side_width = 12;
    let total_padding = (area.width as usize).saturating_sub(side_width * 2 + center_width);
    let left_padding = total_padding / 2;
    let right_padding = total_padding - left_padding;

    let mut spans = vec![
        Span::styled(" ‹ ", left_style),
        Span::styled(" ctrl+h ", dimmed_style),
        Span::styled(" ".repeat(left_padding), Style::default().bg(bg)),
    ];
    spans.extend(center_spans);
    spans.push(Span::styled(
        " ".repeat(right_padding),
        Style::default().bg(bg),
    ));
    spans.push(Span::styled(" ctrl+l ", dimmed_style));
    spans.push(Span::styled(" › ", right_style));

    let header = Paragraph::new(Line::from(spans)).style(Style::default().bg(bg));
    frame.render_widget(header, area);
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;

    #[test]
    fn test_stacked_header_truncates_multibyte_summary_safely() {
        let commit = StackedCommitInfo {
            commit_id: "abcdef1234567890".to_string(),
            short_id: "abcdef1".to_string(),
            change_id: Some("日本語チェンジID1234".to_string()),
            summary: "feat: améliorer le rendu des diffs avec des émojis ✨🚀 et beaucoup de texte"
                .to_string(),
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).unwrap();
        terminal
            .draw(|frame| {
                render_stacked_header(frame, frame.area(), Some(&commit), 0, 3, "jj");
            })
            .unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(rendered.contains("1/3"));
        assert!(rendered.contains("abcdef1"));
    }
}
