use std::collections::HashSet;

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::command::diff::context::{compute_context_lines, ContextLine};
use crate::command::diff::diff_algo::hunk_comment_lines;
use crate::command::diff::highlight::{highlight_line_spans, FileHighlighter};
use crate::command::diff::search::{MatchPanel, SearchState};
use crate::command::diff::theme;
use crate::command::diff::review_thread::{AnnotationTarget, Thread};
use crate::command::diff::{annotation::AnnotationEditor, state::TreeCache};

/// Screen rect of a rendered inline thread overlay, plus the screen `y` rows of
/// any `⋯` truncation indicators inside it (clicking those opens the thread modal).
#[derive(Clone, Debug)]
pub struct ThreadHitRect {
    pub thread_id: String,
    pub area: Rect,
    pub expand_rows: Vec<u16>,
}

/// One overlay slot in the rendered diff: either a thread or the active inline
/// editor (a new thread, a reply, or a comment being edited).
enum OverlaySlot<'a, 'e> {
    /// `rows` are the thread's screen rows, without the gutter column.
    Saved {
        thread: &'a Thread,
        rows: Vec<Line<'static>>,
        expand_rows: Vec<usize>,
    },
    Editor(&'e AnnotationEditor<'e>),
}

impl<'a, 'e> OverlaySlot<'a, 'e> {
    fn height(&self) -> usize {
        match self {
            OverlaySlot::Saved { rows, .. } => rows.len(),
            OverlaySlot::Editor(e) => e.desired_height(),
        }
    }

    fn target(&self) -> &AnnotationTarget {
        match self {
            OverlaySlot::Saved { thread, .. } => &thread.target,
            OverlaySlot::Editor(e) => &e.target,
        }
    }
}

/// Overlay slots of one file, for its file-level threads or for its line-range
/// threads. The active editor follows the thread it replies to; while it edits a
/// comment, that comment is left out of the thread. An editor starting a new
/// thread comes last. `width_for` gives the overlay width for a target.
#[allow(clippy::too_many_arguments)]
fn build_slots<'a, 'e>(
    threads: &'a [Thread],
    editor: Option<&'e AnnotationEditor<'e>>,
    filename: &str,
    file_level: bool,
    width_for: &dyn Fn(&AnnotationTarget) -> u16,
    visible_height: usize,
    tab_width: usize,
    t: &theme::Theme,
) -> Vec<OverlaySlot<'a, 'e>> {
    let is_wanted_level =
        |target: &AnnotationTarget| matches!(target, AnnotationTarget::File) == file_level;
    let mut editor = editor.filter(|e| e.filename == filename && is_wanted_level(&e.target));
    let mut slots: Vec<OverlaySlot<'a, 'e>> = Vec::new();

    let open_file_threads = if file_level {
        threads
            .iter()
            .filter(|thread| {
                thread.filename == filename && is_wanted_level(&thread.target) && !thread.resolved
            })
            .count()
            .max(1)
    } else {
        1
    };

    for thread in threads
        .iter()
        .filter(|thread| thread.filename == filename && is_wanted_level(&thread.target))
    {
        let editor_in_thread = editor.filter(|e| e.thread_id.as_deref() == Some(&thread.id));
        let edited_comment = editor_in_thread.and_then(|e| e.comment_id.as_deref());
        let rows_width = width_for(&thread.target).saturating_sub(1) as usize;
        let max_rows = inline_thread_max_rows(
            visible_height,
            file_level,
            open_file_threads,
            editor_in_thread.map(AnnotationEditor::desired_height),
        );
        let rendered = thread_rows(thread, edited_comment, rows_width, max_rows, tab_width, t);
        if !rendered.rows.is_empty() {
            slots.push(OverlaySlot::Saved {
                thread,
                rows: rendered.rows,
                expand_rows: rendered.expand_rows,
            });
        }
        if let Some(e) = editor_in_thread {
            slots.push(OverlaySlot::Editor(e));
            editor = None;
        }
    }
    if let Some(e) = editor {
        slots.push(OverlaySlot::Editor(e));
    }
    slots
}

fn inline_thread_max_rows(
    visible_height: usize,
    file_level: bool,
    open_file_threads: usize,
    editor_height: Option<usize>,
) -> usize {
    let base_cap = if file_level {
        (visible_height * 40 / 100 / open_file_threads.max(1)).max(6)
    } else {
        (visible_height * 60 / 100).max(6)
    }
    .min(visible_height);

    match editor_height {
        Some(editor_h) => {
            let remaining_screen = visible_height.saturating_sub(editor_h);
            base_cap
                .saturating_sub(editor_h)
                .max(4)
                .min(remaining_screen)
                .max(3)
        }
        None => base_cap.max(3),
    }
}

fn char_count(text: &str) -> usize {
    text.chars().count()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if char_count(text) <= max_chars {
        text.to_string()
    } else {
        let kept: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{}…", kept)
    }
}

struct RenderedThread {
    rows: Vec<Line<'static>>,
    expand_rows: Vec<usize>,
}

fn truncation_indicator_row(
    count: usize,
    singular_label: &str,
    text_width: usize,
    t: &theme::Theme,
) -> Vec<Span<'static>> {
    let suffix = if count == 1 { "" } else { "s" };
    let with_hint = format!("⋯ {} {}{} (t to view)", count, singular_label, suffix);
    let text = if Span::raw(&with_hint).width() <= text_width {
        with_hint
    } else {
        let short = format!("⋯ {} {}{}", count, singular_label, suffix);
        super::markdown::truncate_with_ellipsis(&short, text_width)
    };
    vec![Span::styled(text, Style::default().fg(t.ui.text_muted).italic())]
}

/// Screen rows of a thread, each `width` characters wide: a box holding comments
/// under their authors (capped to `max_rows`, dropping older comments first), or
/// a single row when the thread is resolved. The comment `edited_comment` is left
/// out. Empty when nothing is left to show.
fn thread_rows(
    thread: &Thread,
    edited_comment: Option<&str>,
    width: usize,
    max_rows: usize,
    tab_width: usize,
    t: &theme::Theme,
) -> RenderedThread {
    let comments: Vec<_> = thread
        .comments
        .iter()
        .filter(|comment| Some(comment.id.as_str()) != edited_comment)
        .collect();
    let Some(first_comment) = comments.first() else {
        return RenderedThread {
            rows: Vec::new(),
            expand_rows: Vec::new(),
        };
    };
    let muted = Style::default().fg(t.ui.text_muted);

    if thread.resolved {
        let summary = format!(
            " ✓ resolved · {}: {}",
            first_comment.author_name,
            first_comment.body.lines().next().unwrap_or_default()
        );
        let summary = super::markdown::truncate_with_ellipsis(&summary, width);
        let pad = width.saturating_sub(Span::raw(&summary).width());
        return RenderedThread {
            rows: vec![Line::from(Span::styled(
                format!("{}{}", summary, " ".repeat(pad)),
                muted,
            ))],
            expand_rows: Vec::new(),
        };
    }

    let border = Style::default().fg(t.ui.highlight);
    let inner_width = width.saturating_sub(2);
    let text_width = inner_width.saturating_sub(1);
    let mut rows = Vec::new();
    let mut expand_rows = Vec::new();

    let mut badges = Vec::new();
    if thread.outdated {
        badges.push("outdated");
    }
    if thread.awaiting_agent() {
        badges.push("awaiting agent");
    }
    let badge = if badges.is_empty() {
        String::new()
    } else {
        truncate_chars(&format!(" {} ", badges.join(" · ")), inner_width.saturating_sub(1))
    };
    let leading_dashes = usize::from(!badge.is_empty());
    let trailing_dashes = inner_width.saturating_sub(leading_dashes + char_count(&badge));
    rows.push(Line::from(vec![
        Span::styled(format!("┌{}", "─".repeat(leading_dashes)), border),
        Span::styled(badge, Style::default().fg(t.ui.status_modified)),
        Span::styled(format!("{}┐", "─".repeat(trailing_dashes)), border),
    ]));

    let boxed = |spans: Vec<Span<'static>>| {
        let used: usize = spans.iter().map(Span::width).sum();
        let mut row = vec![Span::styled("│ ", border)];
        row.extend(spans);
        row.push(Span::raw(" ".repeat(text_width.saturating_sub(used))));
        row.push(Span::styled("│", border));
        Line::from(row)
    };

    let rendered_comments: Vec<Vec<Vec<Span<'static>>>> = comments
        .iter()
        .map(|comment| {
            super::markdown::render_comment_rows(comment, &thread.filename, text_width, tab_width, t)
        })
        .collect();
    let total_inner: usize = rendered_comments.iter().map(Vec::len).sum();
    let max_inner = max_rows.saturating_sub(2).max(1);

    if total_inner <= max_inner {
        for comment_rows in rendered_comments {
            for row in comment_rows {
                rows.push(boxed(row));
            }
        }
    } else {
        let budget_for_whole = max_inner.saturating_sub(1);
        let mut kept_from_end = 0usize;
        let mut used = 0usize;
        for comment_rows in rendered_comments.iter().rev() {
            if used + comment_rows.len() <= budget_for_whole {
                used += comment_rows.len();
                kept_from_end += 1;
            } else {
                break;
            }
        }

        if kept_from_end > 0 {
            let hidden_comments = rendered_comments.len() - kept_from_end;
            expand_rows.push(rows.len());
            rows.push(boxed(truncation_indicator_row(
                hidden_comments,
                "earlier comment",
                text_width,
                t,
            )));
            for comment_rows in &rendered_comments[hidden_comments..] {
                for row in comment_rows {
                    rows.push(boxed(row.clone()));
                }
            }
        } else if let Some(last) = rendered_comments.last() {
            let hidden_comments = rendered_comments.len().saturating_sub(1);
            let mut remaining = max_inner;
            if hidden_comments > 0 && remaining > 0 {
                expand_rows.push(rows.len());
                rows.push(boxed(truncation_indicator_row(
                    hidden_comments,
                    "earlier comment",
                    text_width,
                    t,
                )));
                remaining -= 1;
            }
            if remaining > 0 {
                let keep = remaining.saturating_sub(1).min(last.len());
                let hidden_lines = last.len().saturating_sub(keep);
                for row in &last[..keep] {
                    rows.push(boxed(row.clone()));
                }
                if hidden_lines > 0 {
                    expand_rows.push(rows.len());
                    rows.push(boxed(truncation_indicator_row(
                        hidden_lines,
                        "more line",
                        text_width,
                        t,
                    )));
                }
            }
        }
    }

    rows.push(Line::from(Span::styled(
        format!("└{}┘", "─".repeat(inner_width)),
        border,
    )));
    RenderedThread { rows, expand_rows }
}
use crate::command::diff::types::{
    ChangeType, DiffFullscreen, DiffLine, DiffPanelFocus, DiffViewSettings, FileDiff, FocusedPanel,
    InlineSegment, Selection, SelectionMode, SidebarItem,
};
use crate::command::diff::PrInfo;

use super::footer::{render_footer, FooterData, ReviewStatus};
use super::sidebar::render_sidebar;

/// Render the header bar for stacked diff mode showing commit info with navigation arrows
fn render_stacked_header(
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

    // Styles for arrows and hints
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

    // Commit info - for jj show change_id, for git show short SHA
    let (commit_id_label, commit_msg) = if let Some(c) = commit {
        let id_str = if let Some(ref change_id) = c.change_id {
            // jj: show change_id (first 8 chars) + short commit id
            format!("{} {}", &change_id[..8.min(change_id.len())], c.short_id)
        } else {
            // git: just show short SHA
            c.short_id.clone()
        };
        (id_str, c.summary.clone())
    } else {
        ("?".to_string(), "No commit".to_string())
    };

    // Build center content: [jj] [1/6]  id  message
    let vcs_indicator = format!(" {} ", vcs_name);
    let nav_indicator = format!(" {}/{} ", index + 1, total);
    let id_label = format!(" {} ", commit_id_label);

    // Reserve space for arrows, hints, vcs indicator, and id
    let available_for_msg =
        (area.width as usize).saturating_sub(60 + vcs_indicator.len() + id_label.len());

    let truncated_msg = if commit_msg.len() > available_for_msg {
        format!(
            "{}...",
            &commit_msg[..available_for_msg.saturating_sub(3).max(0)]
        )
    } else {
        commit_msg
    };

    // Build center spans: [vcs] [1/6] [id] message
    let badge_style = Style::default().bg(t.ui.footer_branch_bg);
    let spacer_style = Style::default().bg(bg);
    let center_spans = vec![
        Span::styled(&vcs_indicator, badge_style.fg(t.ui.text_muted)),
        Span::styled(" ", spacer_style),
        Span::styled(&nav_indicator, badge_style.fg(t.ui.highlight)),
        Span::styled(" ", spacer_style),
        Span::styled(&id_label, badge_style.fg(t.ui.footer_branch_fg)),
        Span::styled("  ", spacer_style),
        Span::styled(
            &truncated_msg,
            Style::default().fg(t.ui.text_secondary).bg(bg),
        ),
    ];

    // Calculate widths for centering
    let center_width: usize = vcs_indicator.len()
        + 1
        + nav_indicator.len()
        + 1
        + id_label.len()
        + 2
        + truncated_msg.chars().count();
    // " ‹ " + " ctrl+h " = 12 chars, same for right side
    let side_width = 12;

    let total_content_width = side_width * 2 + center_width;
    let total_padding = (area.width as usize).saturating_sub(total_content_width);
    let left_padding = total_padding / 2;
    let right_padding = total_padding - left_padding;

    // Build final line with centered content
    let mut spans = vec![
        // Left side: arrow and hint
        Span::styled(" ‹ ", left_style),
        Span::styled(" ctrl+h ", dimmed_style),
        // Left padding
        Span::styled(" ".repeat(left_padding), Style::default().bg(bg)),
    ];

    // Add center content
    spans.extend(center_spans);

    // Right padding and right side
    spans.push(Span::styled(
        " ".repeat(right_padding),
        Style::default().bg(bg),
    ));
    spans.push(Span::styled(" ctrl+l ", dimmed_style));
    spans.push(Span::styled(" › ", right_style));

    let header = Paragraph::new(Line::from(spans)).style(Style::default().bg(bg));
    frame.render_widget(header, area);
}

/// Generates a diagonal stripe pattern for empty placeholder lines in the diff view.
/// The pattern uses forward slashes to create a visual distinction for empty areas.
fn generate_stripe_pattern(width: usize) -> String {
    "╱".repeat(width)
}

/// Append a trailing span of bg-colored spaces so the line fills `target_width` cells.
/// Used so diff line backgrounds extend to the right edge of the panel even when the
/// content is shorter than the viewport or scrolled horizontally.
fn pad_line_bg<'a>(mut spans: Vec<Span<'a>>, target_width: usize, bg: Color) -> Vec<Span<'a>> {
    let current: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if current < target_width {
        spans.push(Span::styled(
            " ".repeat(target_width - current),
            Style::default().bg(bg),
        ));
    }
    spans
}

fn diff_spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

fn styled_chars(spans: Vec<Span<'_>>) -> Vec<(char, Style)> {
    spans
        .into_iter()
        .flat_map(|span| {
            let style = span.style;
            span.content
                .to_string()
                .chars()
                .map(move |ch| (ch, style))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn chars_to_spans(chars: &[(char, Style)]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut iter = chars.iter();
    let Some(&(first_ch, first_style)) = iter.next() else {
        return spans;
    };

    let mut current_style = first_style;
    let mut buffer = String::from(first_ch);
    for &(ch, style) in iter {
        if style == current_style {
            buffer.push(ch);
        } else {
            spans.push(Span::styled(std::mem::take(&mut buffer), current_style));
            buffer.push(ch);
            current_style = style;
        }
    }
    spans.push(Span::styled(buffer, current_style));
    spans
}

fn leading_indent_width_chars(chars: &[(char, Style)]) -> usize {
    chars.iter().take_while(|(ch, _)| *ch == ' ').count()
}

fn wrap_break_len(chars: &[(char, Style)], max_width: usize) -> (usize, usize) {
    if chars.len() <= max_width {
        return (chars.len(), chars.len());
    }

    let mut break_at = max_width;
    for idx in 1..=max_width.min(chars.len().saturating_sub(1)) {
        if chars[idx].0.is_whitespace() {
            break_at = idx;
        }
    }

    if break_at <= max_width
        && chars
            .get(break_at)
            .is_some_and(|(ch, _)| ch.is_whitespace())
    {
        let mut next_start = break_at;
        while next_start < chars.len() && chars[next_start].0.is_whitespace() {
            next_start += 1;
        }
        (break_at, next_start)
    } else {
        (max_width, max_width)
    }
}

fn wrapped_diff_lines(
    prefix_spans: Vec<Span<'_>>,
    content_spans: Vec<Span<'_>>,
    target_width: usize,
    bg: Color,
    wrap: bool,
) -> Vec<Line<'static>> {
    if !wrap {
        let mut spans: Vec<Span<'static>> = Vec::new();
        spans.extend(
            prefix_spans
                .into_iter()
                .map(|s| Span::styled(s.content.to_string(), s.style)),
        );
        spans.extend(
            content_spans
                .into_iter()
                .map(|s| Span::styled(s.content.to_string(), s.style)),
        );
        return vec![Line::from(pad_line_bg(spans, target_width, bg))];
    }

    let prefix_width = diff_spans_width(&prefix_spans);
    let content_width = target_width
        .saturating_sub(prefix_width)
        .saturating_sub(1)
        .max(1);
    let chars = styled_chars(content_spans);
    let prefix: Vec<Span<'static>> = prefix_spans
        .iter()
        .map(|s| Span::styled(s.content.to_string(), s.style))
        .collect();
    let blank_prefix: Vec<Span<'static>> = prefix_spans
        .iter()
        .filter_map(|span| {
            let width = span.content.chars().count();
            (width > 0).then(|| Span::styled(" ".repeat(width), span.style))
        })
        .collect();

    if chars.is_empty() {
        return vec![Line::from(pad_line_bg(prefix, target_width, bg))];
    }

    let indent = leading_indent_width_chars(&chars).min(content_width.saturating_sub(1));
    let indent_span =
        (indent > 0).then(|| Span::styled(" ".repeat(indent), Style::default().bg(bg)));

    let mut lines = Vec::new();
    let mut start = 0;
    let mut first = true;
    while start < chars.len() {
        let available = if first {
            content_width
        } else {
            content_width.saturating_sub(indent).max(1)
        };
        let remaining = &chars[start..];
        let (take, next_start_delta) = wrap_break_len(remaining, available);
        let mut line_spans = if first {
            prefix.clone()
        } else {
            let mut spans = blank_prefix.clone();
            if let Some(indent_span) = &indent_span {
                spans.push(indent_span.clone());
            }
            spans
        };
        line_spans.extend(chars_to_spans(&remaining[..take]));
        lines.push(Line::from(pad_line_bg(line_spans, target_width, bg)));

        if next_start_delta == 0 {
            break;
        }
        start += next_start_delta;
        first = false;
    }

    lines
}

fn push_wrapped_diff_line(
    lines: &mut Vec<Line<'static>>,
    prefix_spans: Vec<Span<'_>>,
    content_spans: Vec<Span<'_>>,
    target_width: usize,
    bg: Color,
    wrap: bool,
) {
    lines.extend(wrapped_diff_lines(
        prefix_spans,
        content_spans,
        target_width,
        bg,
        wrap,
    ));
}

/// The row shown where hunks-only view hides unchanged lines. `label` says how many.
fn separator_line(label: &str, width: usize, t: &theme::Theme) -> Line<'static> {
    // Under the line numbers, where the code starts.
    let text = format!("      {}", label);
    let padding = width.saturating_sub(char_count(&text));
    Line::from(Span::styled(
        format!("{}{}", text, " ".repeat(padding)),
        Style::default().fg(t.ui.text_muted).bg(t.diff.context_bg),
    ))
}

fn hidden_lines_label(hidden_lines: usize) -> String {
    format!("⋯ {} unchanged lines", hidden_lines)
}

/// The position in the file, counted from 0, of the first line a panel shows from row
/// `scroll` on. A row is not a line: one side can have gaps, and rows can be hidden.
fn top_line_position(rows: &[DiffLine], scroll: usize, panel: DiffPanelFocus) -> usize {
    rows.iter()
        .skip(scroll)
        .find_map(|row| row.line_number(panel))
        .map_or(0, |line| line - 1)
}

fn blank_diff_line(width: usize, bg: Color) -> Line<'static> {
    Line::from(vec![Span::styled(
        " ".repeat(width),
        Style::default().bg(bg),
    )])
}

pub struct LineStats {
    pub added: usize,
    pub removed: usize,
}

pub(super) fn expand_tabs_in_spans<'a>(spans: Vec<Span<'a>>, tab_width: usize) -> Vec<Span<'a>> {
    if tab_width == 0 {
        let mut col = 0;
        return spans
            .into_iter()
            .map(|span| {
                if !span.content.contains('\t') {
                    col += span.content.chars().count();
                    return span;
                }
                let mut buf = String::new();
                for ch in span.content.chars() {
                    if ch == '\t' {
                        continue;
                    }
                    buf.push(ch);
                    col += 1;
                }
                Span::styled(buf, span.style)
            })
            .collect();
    }

    let mut col = 0;
    let mut out: Vec<Span<'a>> = Vec::with_capacity(spans.len());
    for span in spans {
        if !span.content.contains('\t') {
            col += span.content.chars().count();
            out.push(span);
            continue;
        }
        let mut buf = String::new();
        for ch in span.content.chars() {
            if ch == '\t' {
                let spaces = tab_width - (col % tab_width);
                for _ in 0..spaces {
                    buf.push(' ');
                }
                col += spaces;
            } else {
                buf.push(ch);
                col += 1;
            }
        }
        out.push(Span::styled(buf, span.style));
    }
    out
}

fn apply_search_highlight<'a>(
    text: &str,
    filename: &str,
    bg: Option<Color>,
    match_ranges: &[(usize, usize, bool)],
    highlighter: Option<&FileHighlighter>,
    line_number: Option<usize>,
    tab_width: usize,
) -> Vec<Span<'a>> {
    let t = theme::get();

    // Use FileHighlighter if available for proper multi-line construct highlighting
    let base_spans = if let (Some(hl), Some(line_num)) = (highlighter, line_number) {
        let spans = hl.get_line_spans(line_num, bg);
        if spans.is_empty() {
            // Fallback if highlighter doesn't have this line
            highlight_line_spans(text, filename, bg)
        } else {
            spans
        }
    } else {
        highlight_line_spans(text, filename, bg)
    };
    let base_spans = expand_tabs_in_spans(base_spans, tab_width);

    if match_ranges.is_empty() {
        return base_spans;
    }
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

/// Convert InlineSegments to emphasis ranges (start, end) positions.
fn segments_to_emphasis_ranges(segments: &[InlineSegment]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut pos = 0;
    for segment in segments {
        let len = segment.text.len();
        if segment.emphasized {
            ranges.push((pos, pos + len));
        }
        pos += len;
    }
    ranges
}

/// Check if a color is "muted" (low luminosity) and would have poor contrast
/// on a colored background. Returns true for grays and dark colors.
fn is_muted_color(color: Color) -> bool {
    match color {
        Color::Rgb(r, g, b) => {
            // Calculate relative luminance (simplified)
            let luminance = (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000;
            // Also check if it's grayish (low saturation)
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let saturation = if max == 0 {
                0
            } else {
                (max - min) as u32 * 100 / max as u32
            };
            // Muted = low luminance OR (medium luminance AND low saturation)
            luminance < 140 || (luminance < 180 && saturation < 30)
        }
        Color::DarkGray | Color::Gray => true,
        _ => false,
    }
}

/// Boost a muted foreground color to improve contrast on emphasized backgrounds.
fn boost_muted_fg(fg: Color, default_text: Color) -> Color {
    if is_muted_color(fg) {
        // Use a brighter version - the default text color is usually good
        default_text
    } else {
        fg
    }
}

/// Apply syntax highlighting with word-level emphasis backgrounds.
/// This preserves syntax colors while overlaying emphasis backgrounds for changed words.
fn apply_word_emphasis_highlight<'a>(
    text: &str,
    filename: &str,
    line_bg: Option<Color>,
    word_emphasis_bg: Color,
    emphasis_ranges: &[(usize, usize)],
    search_ranges: &[(usize, usize, bool)],
    highlighter: Option<&FileHighlighter>,
    line_number: Option<usize>,
    tab_width: usize,
) -> Vec<Span<'a>> {
    let t = theme::get();

    // Get syntax-highlighted base spans
    let base_spans = if let (Some(hl), Some(line_num)) = (highlighter, line_number) {
        let spans = hl.get_line_spans(line_num, line_bg);
        if spans.is_empty() {
            highlight_line_spans(text, filename, line_bg)
        } else {
            spans
        }
    } else {
        highlight_line_spans(text, filename, line_bg)
    };
    let base_spans = expand_tabs_in_spans(base_spans, tab_width);

    if emphasis_ranges.is_empty() && search_ranges.is_empty() {
        return base_spans;
    }

    let mut result: Vec<Span<'a>> = Vec::new();
    let mut byte_pos = 0;

    for span in base_spans {
        let span_text = span.content.to_string();
        let span_byte_len = span_text.len();

        // Build a list of (byte_offset, char) for safe UTF-8 iteration
        let char_indices: Vec<(usize, char)> = span_text.char_indices().collect();
        if char_indices.is_empty() {
            byte_pos += span_byte_len;
            continue;
        }

        let mut idx = 0;
        while idx < char_indices.len() {
            let (byte_offset, _) = char_indices[idx];
            let global_pos = byte_pos + byte_offset;

            // Check if we're in a search match (takes priority)
            let search_match = search_ranges
                .iter()
                .find(|(start, end, _)| global_pos >= *start && global_pos < *end);

            // Check if we're in an emphasis range
            let in_emphasis = emphasis_ranges
                .iter()
                .any(|(start, end)| global_pos >= *start && global_pos < *end);

            // Determine background and style for this character
            let (bg, fg, bold) = if let Some((_, _, is_current)) = search_match {
                if *is_current {
                    (t.ui.search_current_bg, t.ui.search_current_fg, true)
                } else {
                    (t.ui.search_match_bg, t.ui.search_match_fg, true)
                }
            } else if in_emphasis {
                // Emphasis: use word highlight background, boost muted fg colors for contrast
                let original_fg = span.style.fg.unwrap_or(t.syntax.default_text);
                let boosted_fg = boost_muted_fg(original_fg, t.syntax.default_text);
                (word_emphasis_bg, boosted_fg, false)
            } else {
                // Normal: use line background with original foreground
                (
                    span.style.bg.unwrap_or(line_bg.unwrap_or(Color::Reset)),
                    span.style.fg.unwrap_or(t.syntax.default_text),
                    false,
                )
            };

            // Find the end of this run (same style)
            let mut run_end_idx = idx + 1;
            while run_end_idx < char_indices.len() {
                let (next_byte_offset, _) = char_indices[run_end_idx];
                let next_global_pos = byte_pos + next_byte_offset;

                let next_search = search_ranges
                    .iter()
                    .find(|(start, end, _)| next_global_pos >= *start && next_global_pos < *end);

                let next_in_emphasis = emphasis_ranges
                    .iter()
                    .any(|(start, end)| next_global_pos >= *start && next_global_pos < *end);

                let same_style = match (search_match, next_search) {
                    (Some((_, _, c1)), Some((_, _, c2))) => c1 == c2,
                    (None, None) => in_emphasis == next_in_emphasis,
                    _ => false,
                };

                if !same_style {
                    break;
                }
                run_end_idx += 1;
            }

            // Get the byte range for this run
            let run_start_byte = byte_offset;
            let run_end_byte = if run_end_idx < char_indices.len() {
                char_indices[run_end_idx].0
            } else {
                span_byte_len
            };

            // Push this run
            let run_text = &span_text[run_start_byte..run_end_byte];
            let mut style = Style::default().fg(fg).bg(bg);
            if bold {
                style = style.bold();
            }
            result.push(Span::styled(run_text.to_string(), style));

            idx = run_end_idx;
        }

        byte_pos += span_byte_len;
    }

    result
}

/// Selection tint color - a subtle blue that blends with any diff background
const SELECTION_TINT: Color = Color::Rgb(80, 120, 180);
const SELECTION_ALPHA: f32 = 0.4;

/// Blend a base background color with a selection tint.
#[inline]
fn blend_with_selection(base: Color) -> Color {
    match base {
        Color::Rgb(br, bg, bb) => {
            let Color::Rgb(sr, sg, sb) = SELECTION_TINT else {
                return base;
            };
            let r = ((br as f32) * (1.0 - SELECTION_ALPHA) + (sr as f32) * SELECTION_ALPHA) as u8;
            let g = ((bg as f32) * (1.0 - SELECTION_ALPHA) + (sg as f32) * SELECTION_ALPHA) as u8;
            let b = ((bb as f32) * (1.0 - SELECTION_ALPHA) + (sb as f32) * SELECTION_ALPHA) as u8;
            Color::Rgb(r, g, b)
        }
        _ => SELECTION_TINT,
    }
}

/// Check if a line position is within the selection range for a given panel.
/// Returns the column range that's selected on this line, or None if not selected.
#[inline]
fn get_selection_range_for_line(
    line_idx: usize,
    panel: DiffPanelFocus,
    selection: &Selection,
) -> Option<(usize, usize)> {
    if !selection.is_active() || selection.panel != panel {
        return None;
    }

    let (start, end) = selection.normalized_range();

    if line_idx < start.line || line_idx > end.line {
        return None;
    }

    match selection.mode {
        SelectionMode::Line => {
            // Line mode: entire line is selected
            Some((0, usize::MAX))
        }
        SelectionMode::Character => {
            if start.line == end.line {
                // Single line selection
                Some((start.column, end.column))
            } else if line_idx == start.line {
                // First line of multi-line selection
                Some((start.column, usize::MAX))
            } else if line_idx == end.line {
                // Last line of multi-line selection
                Some((0, end.column))
            } else {
                // Middle line - entire line selected
                Some((0, usize::MAX))
            }
        }
        SelectionMode::None => None,
    }
}

/// Apply selection highlighting to spans. Only processes spans if selection is active.
/// For efficiency, spans fully outside the selection range are passed through unchanged.
#[inline]
fn apply_selection_to_spans<'a>(
    spans: Vec<Span<'a>>,
    selection_range: Option<(usize, usize)>,
    default_bg: Color,
) -> Vec<Span<'a>> {
    let Some((sel_start, sel_end)) = selection_range else {
        return spans;
    };

    // Line mode or full line selected - apply to all spans (fast path)
    if sel_start == 0 && sel_end == usize::MAX {
        return spans
            .into_iter()
            .map(|span| {
                let bg = span.style.bg.unwrap_or(default_bg);
                Span::styled(span.content, span.style.bg(blend_with_selection(bg)))
            })
            .collect();
    }

    // Character mode - need to apply selection per-character
    let mut result = Vec::with_capacity(spans.len() * 2);
    let mut col = 0usize;

    for span in spans {
        let text = span.content.to_string();
        let span_len = text.chars().count();
        let span_end = col + span_len;

        // Fast path: span fully before or after selection
        if span_end <= sel_start || col >= sel_end {
            result.push(Span::styled(text, span.style));
            col = span_end;
            continue;
        }

        // Span intersects with selection - need to split
        let bg = span.style.bg.unwrap_or(default_bg);
        let selected_bg = blend_with_selection(bg);
        let chars: Vec<char> = text.chars().collect();

        // Part before selection
        if col < sel_start {
            let before_len = sel_start - col;
            let before: String = chars[..before_len].iter().collect();
            result.push(Span::styled(before, span.style));
        }

        // Selected part
        let sel_start_in_span = sel_start.saturating_sub(col);
        let sel_end_in_span = (sel_end - col).min(span_len);
        if sel_start_in_span < sel_end_in_span {
            let selected: String = chars[sel_start_in_span..sel_end_in_span].iter().collect();
            result.push(Span::styled(selected, span.style.bg(selected_bg)));
        }

        // Part after selection
        if sel_end < span_end {
            let after_start = sel_end - col;
            let after: String = chars[after_start..].iter().collect();
            result.push(Span::styled(after, span.style));
        }

        col = span_end;
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
                added += 1;
                removed += 1;
            }
            ChangeType::Equal => {}
        }
    }
    LineStats { added, removed }
}

/// Style configuration for rendering a diff line's gutter and background.
struct DiffLineStyle {
    old_bg: Option<Color>,
    old_gutter_bg: Option<Color>,
    old_gutter_fg: Option<Color>,
    new_bg: Option<Color>,
    new_gutter_bg: Option<Color>,
    new_gutter_fg: Option<Color>,
}

impl DiffLineStyle {
    fn for_change_type(
        change_type: ChangeType,
        bg: Color,
        t: &crate::command::diff::theme::Theme,
    ) -> Self {
        match change_type {
            ChangeType::Equal => Self {
                old_bg: Some(bg),
                old_gutter_bg: Some(bg),
                old_gutter_fg: Some(t.ui.line_number),
                new_bg: Some(bg),
                new_gutter_bg: Some(bg),
                new_gutter_fg: Some(t.ui.line_number),
            },
            ChangeType::Delete => Self {
                old_bg: Some(t.diff.deleted_bg),
                old_gutter_bg: Some(t.diff.deleted_gutter_bg),
                old_gutter_fg: Some(t.diff.deleted_gutter_fg),
                new_bg: None,
                new_gutter_bg: None,
                new_gutter_fg: None,
            },
            ChangeType::Insert => Self {
                old_bg: None,
                old_gutter_bg: None,
                old_gutter_fg: None,
                new_bg: Some(t.diff.added_bg),
                new_gutter_bg: Some(t.diff.added_gutter_bg),
                new_gutter_fg: Some(t.diff.added_gutter_fg),
            },
            ChangeType::Modified => Self {
                old_bg: Some(t.diff.deleted_bg),
                old_gutter_bg: Some(t.diff.deleted_gutter_bg),
                old_gutter_fg: Some(t.diff.deleted_gutter_fg),
                new_bg: Some(t.diff.added_bg),
                new_gutter_bg: Some(t.diff.added_gutter_bg),
                new_gutter_fg: Some(t.diff.added_gutter_fg),
            },
        }
    }
}

pub fn render_empty_state(frame: &mut Frame, watching: bool, repository_name: Option<&str>) {
    let watch_hint = if watching {
        " (watching for changes...)"
    } else {
        ""
    };
    let title = match repository_name {
        Some(name) => format!(" Git Review · {} ", name),
        None => " Git Review ".to_string(),
    };
    let msg = Paragraph::new(format!("No changes detected.{}", watch_hint))
        .block(Block::default().title(title).borders(Borders::ALL));
    frame.render_widget(msg, frame.area());
}

fn render_context_lines(
    context: &[ContextLine],
    total_count: usize,
    lines: &mut Vec<Line>,
    filename: &str,
    highlighter: &FileHighlighter,
    tab_width: usize,
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
            // Use FileHighlighter for proper multi-line construct highlighting
            let hl_spans = expand_tabs_in_spans(
                highlighter.get_line_spans(cl.line_number, Some(context_bg)),
                tab_width,
            );
            if hl_spans.is_empty() {
                // Fallback to line-by-line highlighting
                spans.extend(highlight_line_spans(
                    &cl.content,
                    filename,
                    Some(context_bg),
                ));
            } else {
                spans.extend(hl_spans);
            }
            lines.push(Line::from(spans));
        } else {
            lines.push(Line::from(vec![Span::styled(
                "     ~".to_string(),
                Style::default().fg(t.ui.line_number).bg(context_bg),
            )]));
        }
    }
}

use crate::vcs::StackedCommitInfo;

/// Compute side_by_side index ranges for line-range targets.
/// Returns `(first_sbs_idx, last_sbs_idx, panel)` for each target that has matching lines.
fn compute_target_index_ranges(
    targets: &[AnnotationTarget],
    side_by_side: &[DiffLine],
) -> Vec<(usize, usize, DiffPanelFocus)> {
    let mut ranges = Vec::new();
    for target in targets {
        if let AnnotationTarget::LineRange { panel, start_line, end_line, .. } = target {
            let mut first_idx: Option<usize> = None;
            let mut last_idx: Option<usize> = None;
            for (idx, dl) in side_by_side.iter().enumerate() {
                if let Some(n) = dl.line_number(*panel) {
                    if n >= *start_line && n <= *end_line {
                        if first_idx.is_none() {
                            first_idx = Some(idx);
                        }
                        last_idx = Some(idx);
                    }
                }
            }
            if let (Some(first), Some(last)) = (first_idx, last_idx) {
                ranges.push((first, last, *panel));
            }
        }
    }
    ranges
}

/// Collect line-range targets from a slot list.
fn slot_targets(slots: &[OverlaySlot]) -> Vec<AnnotationTarget> {
    slots.iter().map(|s| s.target().clone()).collect()
}

/// Check if a side_by_side index falls within any annotation range, optionally filtering by panel.
fn is_in_ann_range(
    sbs_idx: usize,
    panel: Option<DiffPanelFocus>,
    ranges: &[(usize, usize, DiffPanelFocus)],
) -> bool {
    ranges.iter().any(|(first, last, ann_panel)| {
        sbs_idx >= *first && sbs_idx <= *last && panel.map_or(true, |p| *ann_panel == p)
    })
}

/// Build the focus/annotation indicator span for a diff line.
fn make_indicator_span(
    in_focused: bool,
    in_annotation: bool,
    line_selected: bool,
    bg: Color,
    focus_style: Style,
    annotation_style: Style,
) -> Span<'static> {
    let indicator = if in_focused {
        Span::styled("▎", focus_style)
    } else if in_annotation {
        Span::styled("▍", annotation_style)
    } else {
        Span::styled(" ", Style::default())
    };
    if line_selected {
        let ind_bg = indicator.style.bg.unwrap_or(bg);
        Span::styled(
            indicator.content,
            indicator.style.bg(blend_with_selection(ind_bg)),
        )
    } else {
        indicator
    }
}

/// Total rendered height of file-level overlay slots.
fn file_slots_height(slots: &[OverlaySlot]) -> usize {
    slots.iter().map(|s| s.height()).sum()
}

/// Render a single overlay slot (thread or editor) at a given position.
///
/// A thread's screen rect is appended to `annotation_rects` so mouse handlers can
/// hit-test clicks against it. Line-range overlays get a gutter marker connecting
/// them to the lines above, unless `suppress_gutter` is set: for new-panel overlays
/// in side-by-side mode the marker is drawn on the shared border instead.
#[allow(clippy::too_many_arguments)]
fn render_overlay_slot(
    frame: &mut Frame,
    line_pos: usize,
    slot: &OverlaySlot,
    content_x: u16,
    content_start_y: u16,
    content_width: u16,
    max_area: Rect,
    bg: Color,
    t: &crate::command::diff::theme::Theme,
    suppress_gutter: bool,
    annotation_rects: &mut Vec<ThreadHitRect>,
    editor_rect: &mut Option<Rect>,
) {
    let inner_bottom = max_area.y + max_area.height.saturating_sub(1);
    let screen_y = content_start_y + line_pos as u16;
    if screen_y >= inner_bottom {
        return;
    }
    let available = inner_bottom.saturating_sub(screen_y) as usize;
    if available == 0 {
        return;
    }
    let height = slot.height().min(available) as u16;
    let area = Rect::new(content_x, screen_y, content_width, height);
    let has_gutter = !suppress_gutter && matches!(slot.target(), AnnotationTarget::LineRange { .. });

    match slot {
        OverlaySlot::Saved {
            thread,
            rows,
            expand_rows,
        } => {
            frame.render_widget(ratatui::widgets::Clear, area);
            let gutter = if has_gutter { "▍" } else { " " };
            let lines: Vec<Line> = rows
                .iter()
                .take(available)
                .map(|row| {
                    let mut spans = vec![Span::styled(gutter, Style::default().fg(t.ui.highlight))];
                    spans.extend(row.spans.iter().cloned());
                    Line::from(spans)
                })
                .collect();
            frame.render_widget(Paragraph::new(lines).style(Style::default().bg(bg)), area);
            let visible_expand_rows = expand_rows
                .iter()
                .copied()
                .filter(|&row_idx| row_idx < available)
                .map(|row_idx| screen_y + row_idx as u16)
                .collect();
            annotation_rects.push(ThreadHitRect {
                thread_id: thread.id.clone(),
                area,
                expand_rows: visible_expand_rows,
            });
        }
        OverlaySlot::Editor(editor) => {
            editor.render_inline(frame, area, t.ui.highlight, bg, has_gutter);
            *editor_rect = Some(area);
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Returns `(content_row_offset, annotation_overlay_gaps)`.
/// - `content_row_offset`: the number of non-diff rows at the top
///   (context lines + file annotation placeholders), used for mouse coordinate mapping.
/// - `annotation_overlay_gaps`: list of `(content_line_after, gap_height)` pairs describing
///   inline annotation overlay gaps within the content area.
pub fn render_diff(
    frame: &mut Frame,
    diff: &FileDiff,
    _file_diffs: &[FileDiff],
    trees: &TreeCache,
    sidebar_items: &[SidebarItem],
    sidebar_visible: &[usize],
    collapsed_dirs: &HashSet<String>,
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
    commit_ref: &str,
    pr_info: Option<&PrInfo>,
    focused_hunk: Option<usize>,
    hunks: &[usize],
    stacked_mode: bool,
    stacked_commit: Option<&StackedCommitInfo>,
    stacked_index: usize,
    stacked_total: usize,
    side_by_side: &[DiffLine],
    vcs_name: &str,
    annotations: &[Thread],
    selection: &Selection,
    old_highlighter: &FileHighlighter,
    new_highlighter: &FileHighlighter,
    viewed_hunks: &HashSet<usize>,
    total_added: usize,
    total_removed: usize,
    editor: Option<&AnnotationEditor>,
    review: &ReviewStatus,
) -> (usize, Vec<(usize, usize)>, Vec<ThreadHitRect>, Option<Rect>) {
    let area = frame.area();
    let t = theme::get();
    let bg = t.ui.bg;
    let h_scroll = if settings.wrap { 0 } else { h_scroll };

    // Layout: header (if stacked) + main content + footer
    let (content_area, footer_area) = if stacked_mode {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Header
                Constraint::Min(0),    // Main content
                Constraint::Length(1), // Footer
            ])
            .split(area);

        // Render stacked header
        render_stacked_header(
            frame,
            chunks[0],
            stacked_commit,
            stacked_index,
            stacked_total,
            vcs_name,
        );

        (chunks[1], chunks[2])
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        (chunks[0], chunks[1])
    };

    let main_area = if show_sidebar {
        let sidebar_width = (area.width / 4).clamp(20, 35);
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(sidebar_width), Constraint::Min(0)])
            .split(content_area);

        render_sidebar(
            frame,
            main_chunks[0],
            sidebar_items,
            sidebar_visible,
            collapsed_dirs,
            current_file,
            sidebar_selected,
            sidebar_scroll,
            sidebar_h_scroll,
            viewed_files,
            focused_panel == FocusedPanel::Sidebar,
            _file_diffs.len(),
            total_added,
            total_removed,
        );

        main_chunks[1]
    } else {
        content_area
    };

    // Handle binary files - show a message instead of trying to diff
    if diff.is_binary {
        let border_style = Style::default().fg(t.ui.border_unfocused);
        let title_style = if focused_panel == FocusedPanel::DiffView {
            Style::default().fg(t.ui.border_focused)
        } else {
            Style::default().fg(t.ui.border_unfocused)
        };

        let message = Line::from(vec![Span::styled(
            "Binary file - not displayed",
            Style::default().fg(t.ui.text_muted),
        )]);
        let para = Paragraph::new(vec![message])
            .alignment(ratatui::layout::Alignment::Center)
            .block(
                Block::default()
                    .title(Line::styled(format!(" {} ", diff.filename), title_style))
                    .borders(Borders::ALL)
                    .border_style(border_style),
            );
        frame.render_widget(para, main_area);

        render_footer(
            frame,
            footer_area,
            FooterData {
                filename: &diff.filename,
                commit_ref,
                pr_info,
                watching,
                current_file,
                viewed_files,
                line_stats_added: 0,
                line_stats_removed: 0,
                hunk_count: 0,
                focused_hunk: None,
                search_state,
                area_width: area.width,
                review,
            },
        );
        return (0, Vec::new(), Vec::new(), None);
    }

    // side_by_side is now passed as a parameter (pre-computed and cached)
    let line_stats = compute_line_stats(side_by_side);

    let is_new_file = diff.old_content.is_empty() && !diff.new_content.is_empty();
    let is_deleted_file = !diff.old_content.is_empty() && diff.new_content.is_empty();

    // Track how many non-diff rows are at the top (context lines + file annotations)
    let content_row_offset: usize;
    // Track inline annotation overlay gaps for mouse coordinate mapping
    let mut overlay_gaps: Vec<(usize, usize)> = Vec::new();
    // Track rendered annotation screen rects for click hit-testing
    let mut annotation_rects: Vec<ThreadHitRect> = Vec::new();
    // Tracks the screen rect of the inline editor, when one is rendered this frame.
    let mut editor_rect: Option<Rect> = None;

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
            &trees.new_file_trees,
            scroll as usize,
            &settings.context,
            settings.tab_width,
        );
        let context_count = new_context.len();
        let scroll_usize = scroll as usize;

        let slot_width = |_: &AnnotationTarget| main_area.width.saturating_sub(2);
        let file_slots = build_slots(
            annotations,
            editor,
            &diff.filename,
            true,
            &slot_width,
            visible_height,
            settings.tab_width,
            t,
        );
        let line_slots = build_slots(
            annotations,
            editor,
            &diff.filename,
            false,
            &slot_width,
            visible_height,
            settings.tab_width,
            t,
        );

        let annotation_height = file_slots_height(&file_slots);
        content_row_offset = context_count + annotation_height;

        let base_content_height = visible_height.saturating_sub(context_count);
        let content_height = base_content_height.saturating_sub(annotation_height);

        let visible_lines: Vec<&DiffLine> = side_by_side
            .iter()
            .skip(scroll_usize)
            .take(content_height)
            .collect();

        let mut new_lines: Vec<Line> = Vec::new();
        let mut slot_overlays: Vec<(usize, &OverlaySlot)> = Vec::new();

        if settings.context.enabled && context_count > 0 {
            render_context_lines(
                &new_context,
                context_count,
                &mut new_lines,
                &diff.filename,
                new_highlighter,
                settings.tab_width,
            );
        }

        // Show file-level slots at top (annotations + editor when file-targeted)
        for slot in &file_slots {
            let num_lines = slot.height();
            let slot_start = new_lines.len();
            for _ in 0..num_lines {
                new_lines.push(Line::from(vec![Span::raw("")]));
            }
            slot_overlays.push((slot_start, slot));
        }

        let line_targets = slot_targets(&line_slots);
        let ann_index_ranges = compute_target_index_ranges(&line_targets, side_by_side);
        let annotation_indicator_style = Style::default().fg(t.ui.highlight);
        let focus_style = Style::default().fg(t.ui.border_focused);

        let row_target_width = (main_area.width as usize).saturating_sub(2) + h_scroll as usize;

        for (i, diff_line) in visible_lines.iter().enumerate() {
            let line_idx = scroll_usize + i;
            let new_selection_range =
                get_selection_range_for_line(line_idx, DiffPanelFocus::New, selection);
            let in_annotation = is_in_ann_range(line_idx, None, &ann_index_ranges);
            let new_line_selected =
                new_selection_range.map_or(false, |(s, e)| s == 0 && e == usize::MAX);
            let hunk_viewed = {
                let h_idx = hunks.iter().rposition(|&h| h <= line_idx);
                !matches!(diff_line.change_type, ChangeType::Equal)
                    && h_idx.map_or(false, |idx| viewed_hunks.contains(&idx))
            };

            if let Some((num, text)) = &diff_line.new_line {
                let mut spans: Vec<Span> = Vec::new();
                spans.push(make_indicator_span(
                    false,
                    in_annotation,
                    new_line_selected,
                    bg,
                    focus_style,
                    annotation_indicator_style,
                ));

                let prefix = format!("{:4} ", num);
                let gutter_bg = t.diff.added_gutter_bg;
                let gutter_bg = if new_line_selected {
                    blend_with_selection(gutter_bg)
                } else {
                    gutter_bg
                };
                spans.push(Span::styled(
                    prefix,
                    Style::default().fg(t.diff.added_gutter_fg).bg(gutter_bg),
                ));
                let matches = search_state.get_matches_for_line(line_idx, MatchPanel::New);
                let line_bg = if hunk_viewed { bg } else { t.diff.added_bg };
                let content_spans = apply_search_highlight(
                    text,
                    &diff.filename,
                    Some(line_bg),
                    &matches,
                    Some(new_highlighter),
                    Some(*num),
                    settings.tab_width,
                );
                let content_spans =
                    apply_selection_to_spans(content_spans, new_selection_range, line_bg);
                push_wrapped_diff_line(
                    &mut new_lines,
                    spans,
                    content_spans,
                    row_target_width,
                    line_bg,
                    settings.wrap,
                );
            }

            // Check if this line is the end_line for any line-range slot
            for slot in &line_slots {
                if let AnnotationTarget::LineRange { panel, end_line, .. } = slot.target() {
                    if diff_line.line_number(*panel) == Some(*end_line) {
                        let num_ann_lines = slot.height();
                        let line_pos = new_lines.len();
                        overlay_gaps.push((i, num_ann_lines));
                        for _ in 0..num_ann_lines {
                            new_lines.push(Line::from(vec![Span::raw("")]));
                        }
                        slot_overlays.push((line_pos, slot));
                    }
                }
            }
        }

        let new_para = Paragraph::new(new_lines).scroll((0, h_scroll)).block(
            Block::default()
                .title(Line::styled(" [2] New File ", title_style))
                .borders(Borders::ALL)
                .border_style(border_style),
        );
        frame.render_widget(new_para, main_area);

        // Render overlay slots (saved annotations + active editor)
        let content_x = main_area.x + 1;
        let content_start_y = main_area.y + 1;
        let content_width = main_area.width.saturating_sub(2);
        for &(line_pos, slot) in &slot_overlays {
            render_overlay_slot(
                frame,
                line_pos,
                slot,
                content_x,
                content_start_y,
                content_width,
                main_area,
                bg,
                t,
                false,
                &mut annotation_rects,
                &mut editor_rect,
            );
        }
    } else if is_deleted_file {
        let visible_height = main_area.height.saturating_sub(2) as usize;
        let old_context = compute_context_lines(
            &diff.old_content,
            &diff.filename,
            &trees.old_file_trees,
            scroll as usize,
            &settings.context,
            settings.tab_width,
        );
        let context_count = old_context.len();
        let scroll_usize = scroll as usize;

        let slot_width = |_: &AnnotationTarget| main_area.width.saturating_sub(2);
        let file_slots = build_slots(
            annotations,
            editor,
            &diff.filename,
            true,
            &slot_width,
            visible_height,
            settings.tab_width,
            t,
        );
        let line_slots = build_slots(
            annotations,
            editor,
            &diff.filename,
            false,
            &slot_width,
            visible_height,
            settings.tab_width,
            t,
        );

        let annotation_height = file_slots_height(&file_slots);
        content_row_offset = context_count + annotation_height;

        let base_content_height = visible_height.saturating_sub(context_count);
        let content_height = base_content_height.saturating_sub(annotation_height);

        let visible_lines: Vec<&DiffLine> = side_by_side
            .iter()
            .skip(scroll_usize)
            .take(content_height)
            .collect();

        let mut old_lines: Vec<Line> = Vec::new();
        let mut slot_overlays: Vec<(usize, &OverlaySlot)> = Vec::new();

        if settings.context.enabled && context_count > 0 {
            render_context_lines(
                &old_context,
                context_count,
                &mut old_lines,
                &diff.filename,
                old_highlighter,
                settings.tab_width,
            );
        }

        // Show file-level slots at top (annotations + editor when file-targeted)
        for slot in &file_slots {
            let num_lines = slot.height();
            let slot_start = old_lines.len();
            for _ in 0..num_lines {
                old_lines.push(Line::from(vec![Span::raw("")]));
            }
            slot_overlays.push((slot_start, slot));
        }

        let line_targets = slot_targets(&line_slots);
        let ann_index_ranges = compute_target_index_ranges(&line_targets, side_by_side);
        let annotation_indicator_style = Style::default().fg(t.ui.highlight);
        let focus_style = Style::default().fg(t.ui.border_focused);

        let row_target_width = (main_area.width as usize).saturating_sub(2) + h_scroll as usize;

        for (i, diff_line) in visible_lines.iter().enumerate() {
            let line_idx = scroll_usize + i;
            let old_selection_range =
                get_selection_range_for_line(line_idx, DiffPanelFocus::Old, selection);
            let in_annotation = is_in_ann_range(line_idx, None, &ann_index_ranges);
            let old_line_selected =
                old_selection_range.map_or(false, |(s, e)| s == 0 && e == usize::MAX);
            let hunk_viewed = {
                let h_idx = hunks.iter().rposition(|&h| h <= line_idx);
                !matches!(diff_line.change_type, ChangeType::Equal)
                    && h_idx.map_or(false, |idx| viewed_hunks.contains(&idx))
            };

            if let Some((num, text)) = &diff_line.old_line {
                let mut spans: Vec<Span> = Vec::new();
                spans.push(make_indicator_span(
                    false,
                    in_annotation,
                    old_line_selected,
                    bg,
                    focus_style,
                    annotation_indicator_style,
                ));

                let prefix = format!("{:4} ", num);
                let gutter_bg = t.diff.deleted_gutter_bg;
                let gutter_bg = if old_line_selected {
                    blend_with_selection(gutter_bg)
                } else {
                    gutter_bg
                };
                spans.push(Span::styled(
                    prefix,
                    Style::default().fg(t.diff.deleted_gutter_fg).bg(gutter_bg),
                ));
                let matches = search_state.get_matches_for_line(line_idx, MatchPanel::Old);
                let line_bg = if hunk_viewed { bg } else { t.diff.deleted_bg };
                let content_spans = apply_search_highlight(
                    text,
                    &diff.filename,
                    Some(line_bg),
                    &matches,
                    Some(old_highlighter),
                    Some(*num),
                    settings.tab_width,
                );
                let content_spans =
                    apply_selection_to_spans(content_spans, old_selection_range, line_bg);
                push_wrapped_diff_line(
                    &mut old_lines,
                    spans,
                    content_spans,
                    row_target_width,
                    line_bg,
                    settings.wrap,
                );
            }

            // Check if this line is the end_line for any line-range slot
            for slot in &line_slots {
                if let AnnotationTarget::LineRange { panel, end_line, .. } = slot.target() {
                    if diff_line.line_number(*panel) == Some(*end_line) {
                        let num_ann_lines = slot.height();
                        let line_pos = old_lines.len();
                        overlay_gaps.push((i, num_ann_lines));
                        for _ in 0..num_ann_lines {
                            old_lines.push(Line::from(vec![Span::raw("")]));
                        }
                        slot_overlays.push((line_pos, slot));
                    }
                }
            }
        }

        let old_para = Paragraph::new(old_lines).scroll((0, h_scroll)).block(
            Block::default()
                .title(Line::styled(" [2] Deleted File ", title_style))
                .borders(Borders::ALL)
                .border_style(border_style),
        );
        frame.render_widget(old_para, main_area);

        // Render overlay slots (saved annotations + active editor)
        let content_x = main_area.x + 1;
        let content_start_y = main_area.y + 1;
        let content_width = main_area.width.saturating_sub(2);
        for &(line_pos, slot) in &slot_overlays {
            render_overlay_slot(
                frame,
                line_pos,
                slot,
                content_x,
                content_start_y,
                content_width,
                main_area,
                bg,
                t,
                false,
                &mut annotation_rects,
                &mut editor_rect,
            );
        }
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
            &trees.old_file_trees,
            top_line_position(side_by_side, scroll as usize, DiffPanelFocus::Old),
            &settings.context,
            settings.tab_width,
        );
        let new_context = compute_context_lines(
            &diff.new_content,
            &diff.filename,
            &trees.new_file_trees,
            top_line_position(side_by_side, scroll as usize, DiffPanelFocus::New),
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
        let mut slot_overlays: Vec<(usize, &OverlaySlot)> = Vec::new();

        let is_side_by_side = old_area.is_some() && new_area.is_some();
        // Where the overlay for a target is drawn: x, width, first row, and whether
        // its gutter marker is left to the shared border.
        let slot_geometry = |target: &AnnotationTarget| -> (u16, u16, u16, bool) {
            match target {
                AnnotationTarget::File => {
                    // File-level: span both panels
                    let render_area = old_area.or(new_area).unwrap_or(main_area);
                    let x = render_area.x + 1;
                    let w = if is_side_by_side {
                        old_area.unwrap().width + new_area.unwrap().width - 2
                    } else {
                        render_area.width.saturating_sub(2)
                    };
                    (x, w, render_area.y + 1, false)
                }
                AnnotationTarget::LineRange { panel, .. } => {
                    // Line-range: render in the specific panel
                    let is_new_panel = matches!(panel, DiffPanelFocus::New | DiffPanelFocus::None);
                    let area = match panel {
                        DiffPanelFocus::Old => old_area.unwrap_or(main_area),
                        _ => new_area.unwrap_or(main_area),
                    };
                    // In side-by-side mode, new panel annotations use border markers on the
                    // shared border (via buffer_mut) instead of an inline gutter indicator,
                    // so suppress the gutter in the overlay.
                    let suppress = is_new_panel && is_side_by_side;
                    (
                        area.x + 1,
                        area.width.saturating_sub(2),
                        area.y + 1,
                        suppress,
                    )
                }
            }
        };
        let slot_width = |target: &AnnotationTarget| slot_geometry(target).1;
        let file_slots = build_slots(
            annotations,
            editor,
            &diff.filename,
            true,
            &slot_width,
            visible_height,
            settings.tab_width,
            t,
        );
        let line_slots = build_slots(
            annotations,
            editor,
            &diff.filename,
            false,
            &slot_width,
            visible_height,
            settings.tab_width,
            t,
        );

        let file_ann_height = file_slots_height(&file_slots);
        content_row_offset = context_count + file_ann_height;

        if settings.context.enabled && context_count > 0 {
            if old_area.is_some() {
                render_context_lines(
                    &old_context,
                    context_count,
                    &mut old_lines,
                    &diff.filename,
                    old_highlighter,
                    settings.tab_width,
                );
            }
            if new_area.is_some() {
                render_context_lines(
                    &new_context,
                    context_count,
                    &mut new_lines,
                    &diff.filename,
                    new_highlighter,
                    settings.tab_width,
                );
            }
        }

        let hunk_index_for_line =
            |line_idx: usize| -> Option<usize> { hunks.iter().rposition(|&h| h <= line_idx) };
        let is_line_in_viewed_hunk = |line_idx: usize, change_type: ChangeType| -> bool {
            if matches!(change_type, ChangeType::Equal) {
                return false;
            }
            match hunk_index_for_line(line_idx) {
                Some(idx) => viewed_hunks.contains(&idx),
                None => false,
            }
        };

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

        // Add file-level slots at the top (after context lines)
        for slot in &file_slots {
            let num_lines = slot.height();
            let slot_start = old_lines.len(); // same position in both panels
            for _ in 0..num_lines {
                if old_area.is_some() {
                    old_lines.push(Line::from(vec![Span::raw("")]));
                }
                if new_area.is_some() {
                    new_lines.push(Line::from(vec![Span::raw("")]));
                }
            }
            slot_overlays.push((slot_start, slot));
        }

        let line_targets = slot_targets(&line_slots);
        let ann_index_ranges = compute_target_index_ranges(&line_targets, side_by_side);

        // The focus bar marks the lines a comment on the focused hunk would be attached to.
        let commented_side = focused_hunk
            .and_then(|hunk_idx| hunks.get(hunk_idx))
            .and_then(|&hunk_start| hunk_comment_lines(side_by_side, hunk_start, diff_fullscreen))
            .map(|(panel, _, _)| panel);

        // Rows of the old panel's paragraph where the border it shares with the new panel
        // carries a new-side thread marker, or the focus bar. Used when both are shown.
        let mut border_marker_rows: Vec<usize> = Vec::new();
        let mut border_focus_rows: Vec<usize> = Vec::new();

        let focus_style = Style::default().fg(t.ui.border_focused);
        let annotation_indicator_style = Style::default().fg(t.ui.highlight);

        // Old panel always has full borders (Borders::ALL); the new panel drops its left
        // border when shown alongside the old panel (sharing it with the old panel's right).
        let old_row_target_width = old_area
            .map(|a| (a.width as usize).saturating_sub(2))
            .unwrap_or(0)
            + h_scroll as usize;
        let new_row_target_width = new_area
            .map(|a| {
                let borders = if old_area.is_some() { 1 } else { 2 };
                (a.width as usize).saturating_sub(borders)
            })
            .unwrap_or(0)
            + h_scroll as usize;

        for (i, diff_line) in visible_lines.iter().enumerate() {
            let line_idx = scroll_usize + i;
            if diff_line.hidden_lines > 0 {
                let label = hidden_lines_label(diff_line.hidden_lines);
                if old_area.is_some() {
                    old_lines.push(separator_line(&label, old_row_target_width, t));
                }
                if new_area.is_some() {
                    // Said once per row: on the old side when both are shown.
                    let label = if old_area.is_some() { "" } else { label.as_str() };
                    new_lines.push(separator_line(label, new_row_target_width, t));
                }
                continue;
            }
            let in_focused = is_in_focused_hunk(line_idx, diff_line.change_type);
            let hunk_viewed = is_line_in_viewed_hunk(line_idx, diff_line.change_type);
            let mut style = DiffLineStyle::for_change_type(diff_line.change_type, bg, t);
            if hunk_viewed {
                // Drop colored bg from the code area; keep the gutter colored.
                if style.old_bg.is_some() {
                    style.old_bg = Some(bg);
                }
                if style.new_bg.is_some() {
                    style.new_bg = Some(bg);
                }
            }

            let old_selection_range =
                get_selection_range_for_line(line_idx, DiffPanelFocus::Old, selection);
            let new_selection_range =
                get_selection_range_for_line(line_idx, DiffPanelFocus::New, selection);

            let old_in_annotation =
                is_in_ann_range(line_idx, Some(DiffPanelFocus::Old), &ann_index_ranges);
            let new_in_annotation =
                is_in_ann_range(line_idx, Some(DiffPanelFocus::New), &ann_index_ranges);

            // On the commented side, next to its lines. When that side is not shown,
            // on every row of the hunk in the panel that is.
            let focus_on_old = in_focused
                && match commented_side {
                    Some(DiffPanelFocus::Old) => diff_line.old_line.is_some(),
                    _ => new_area.is_none(),
                };
            let focus_on_new = in_focused
                && match commented_side {
                    Some(DiffPanelFocus::New) => diff_line.new_line.is_some(),
                    _ => old_area.is_none(),
                };


            let old_line_selected =
                old_selection_range.map_or(false, |(s, e)| s == 0 && e == usize::MAX);
            let new_line_selected =
                new_selection_range.map_or(false, |(s, e)| s == 0 && e == usize::MAX);
            let old_start_len = old_lines.len();
            let new_start_len = new_lines.len();

            if old_area.is_some() {
                let mut old_spans: Vec<Span> = Vec::new();
                old_spans.push(make_indicator_span(
                    focus_on_old,
                    old_in_annotation,
                    old_line_selected,
                    bg,
                    focus_style,
                    annotation_indicator_style,
                ));
                let mut old_line_pushed = false;
                match &diff_line.old_line {
                    Some((num, _text)) => {
                        let prefix = format!("{:4} ", num);
                        let gutter_bg = style.old_gutter_bg.unwrap_or(Color::Reset);
                        let gutter_bg = if old_line_selected {
                            blend_with_selection(if gutter_bg == Color::Reset {
                                bg
                            } else {
                                gutter_bg
                            })
                        } else {
                            gutter_bg
                        };
                        old_spans.push(Span::styled(
                            prefix,
                            Style::default()
                                .fg(style.old_gutter_fg.unwrap_or(t.ui.line_number))
                                .bg(gutter_bg),
                        ));
                        let matches = search_state.get_matches_for_line(line_idx, MatchPanel::Old);

                        // Use word-level rendering for modified lines if segments are available
                        let content_spans = if matches!(diff_line.change_type, ChangeType::Modified)
                            && !hunk_viewed
                        {
                            if let Some(ref segments) = diff_line.old_segments {
                                let emphasis_ranges = segments_to_emphasis_ranges(segments);
                                apply_word_emphasis_highlight(
                                    _text,
                                    &diff.filename,
                                    style.old_bg,
                                    t.diff.deleted_word_bg,
                                    &emphasis_ranges,
                                    &matches,
                                    Some(old_highlighter),
                                    Some(*num),
                                    settings.tab_width,
                                )
                            } else {
                                apply_search_highlight(
                                    _text,
                                    &diff.filename,
                                    style.old_bg,
                                    &matches,
                                    Some(old_highlighter),
                                    Some(*num),
                                    settings.tab_width,
                                )
                            }
                        } else {
                            apply_search_highlight(
                                _text,
                                &diff.filename,
                                style.old_bg,
                                &matches,
                                Some(old_highlighter),
                                Some(*num),
                                settings.tab_width,
                            )
                        };
                        // Apply selection highlighting
                        let content_spans = apply_selection_to_spans(
                            content_spans,
                            old_selection_range,
                            style.old_bg.unwrap_or(bg),
                        );
                        push_wrapped_diff_line(
                            &mut old_lines,
                            old_spans.clone(),
                            content_spans,
                            old_row_target_width,
                            style.old_bg.unwrap_or(bg),
                            settings.wrap,
                        );
                        old_line_pushed = true;
                    }
                    None => {
                        // Fill the whole row (including the line-number gutter) with stripes,
                        // extending past the visible viewport so horizontal scroll stays striped.
                        let current_len: usize =
                            old_spans.iter().map(|s| s.content.chars().count()).sum();
                        let stripe_width = old_row_target_width.saturating_sub(current_len);
                        old_spans.push(Span::styled(
                            generate_stripe_pattern(stripe_width),
                            Style::default().fg(t.diff.empty_placeholder_fg),
                        ));
                    }
                }
                if !old_line_pushed {
                    let old_spans =
                        pad_line_bg(old_spans, old_row_target_width, style.old_bg.unwrap_or(bg));
                    old_lines.push(Line::from(old_spans));
                }
            }

            if new_area.is_some() {
                let mut new_spans: Vec<Span> = Vec::new();
                if old_area.is_none() {
                    new_spans.push(make_indicator_span(
                        focus_on_new,
                        new_in_annotation,
                        new_line_selected,
                        bg,
                        focus_style,
                        annotation_indicator_style,
                    ));
                }
                let mut new_line_pushed = false;
                match &diff_line.new_line {
                    Some((num, _text)) => {
                        let prefix = format!("{:4} ", num);
                        let gutter_bg = style.new_gutter_bg.unwrap_or(Color::Reset);
                        let gutter_bg = if new_line_selected {
                            blend_with_selection(if gutter_bg == Color::Reset {
                                bg
                            } else {
                                gutter_bg
                            })
                        } else {
                            gutter_bg
                        };
                        new_spans.push(Span::styled(
                            prefix,
                            Style::default()
                                .fg(style.new_gutter_fg.unwrap_or(t.ui.line_number))
                                .bg(gutter_bg),
                        ));
                        let matches = search_state.get_matches_for_line(line_idx, MatchPanel::New);

                        // Use word-level rendering for modified lines if segments are available
                        let content_spans = if matches!(diff_line.change_type, ChangeType::Modified)
                            && !hunk_viewed
                        {
                            if let Some(ref segments) = diff_line.new_segments {
                                let emphasis_ranges = segments_to_emphasis_ranges(segments);
                                apply_word_emphasis_highlight(
                                    _text,
                                    &diff.filename,
                                    style.new_bg,
                                    t.diff.added_word_bg,
                                    &emphasis_ranges,
                                    &matches,
                                    Some(new_highlighter),
                                    Some(*num),
                                    settings.tab_width,
                                )
                            } else {
                                apply_search_highlight(
                                    _text,
                                    &diff.filename,
                                    style.new_bg,
                                    &matches,
                                    Some(new_highlighter),
                                    Some(*num),
                                    settings.tab_width,
                                )
                            }
                        } else {
                            apply_search_highlight(
                                _text,
                                &diff.filename,
                                style.new_bg,
                                &matches,
                                Some(new_highlighter),
                                Some(*num),
                                settings.tab_width,
                            )
                        };
                        // Apply selection highlighting
                        let content_spans = apply_selection_to_spans(
                            content_spans,
                            new_selection_range,
                            style.new_bg.unwrap_or(bg),
                        );
                        push_wrapped_diff_line(
                            &mut new_lines,
                            new_spans.clone(),
                            content_spans,
                            new_row_target_width,
                            style.new_bg.unwrap_or(bg),
                            settings.wrap,
                        );
                        new_line_pushed = true;
                    }
                    None => {
                        // Fill the whole row (including the line-number gutter) with stripes,
                        // extending past the visible viewport so horizontal scroll stays striped.
                        let current_len: usize =
                            new_spans.iter().map(|s| s.content.chars().count()).sum();
                        let stripe_width = new_row_target_width.saturating_sub(current_len);
                        new_spans.push(Span::styled(
                            generate_stripe_pattern(stripe_width),
                            Style::default().fg(t.diff.empty_placeholder_fg),
                        ));
                    }
                }
                if !new_line_pushed {
                    let new_spans =
                        pad_line_bg(new_spans, new_row_target_width, style.new_bg.unwrap_or(bg));
                    new_lines.push(Line::from(new_spans));
                }
            }

            let old_row_count = old_lines.len().saturating_sub(old_start_len);
            let new_row_count = new_lines.len().saturating_sub(new_start_len);
            let row_count = old_row_count.max(new_row_count);
            if old_area.is_some() {
                for _ in old_row_count..row_count {
                    old_lines.push(blank_diff_line(
                        old_row_target_width,
                        style.old_bg.unwrap_or(bg),
                    ));
                }
            }
            if new_area.is_some() {
                for _ in new_row_count..row_count {
                    new_lines.push(blank_diff_line(
                        new_row_target_width,
                        style.new_bg.unwrap_or(bg),
                    ));
                }
            }
            // A wrapped row takes several screen lines: all of them are marked.
            let screen_lines = old_start_len..old_start_len + row_count;
            if new_in_annotation {
                border_marker_rows.extend(screen_lines.clone());
            }
            if focus_on_new {
                border_focus_rows.extend(screen_lines);
            }

            // Check if this line is the end_line for any line-range slot
            for slot in &line_slots {
                if let AnnotationTarget::LineRange { panel, end_line, .. } = slot.target() {
                    if diff_line.line_number(*panel) == Some(*end_line) {
                        let num_lines = slot.height();

                        let line_pos = if old_area.is_some() {
                            old_lines.len()
                        } else {
                            new_lines.len()
                        };

                        // Record gap for mouse coordinate mapping:
                        // `i` is the visible content line index after which this overlay appears
                        overlay_gaps.push((i, num_lines));

                        // Add placeholder lines to both panels to keep them in sync
                        for _ in 0..num_lines {
                            if old_area.is_some() {
                                old_lines.push(Line::from(vec![Span::raw("")]));
                            }
                            if new_area.is_some() {
                                new_lines.push(Line::from(vec![Span::raw("")]));
                            }
                        }

                        slot_overlays.push((line_pos, slot));
                    }
                }
            }
        }

        if let Some(area) = old_area {
            let old_para = Paragraph::new(old_lines)
                .style(Style::default().bg(bg))
                .scroll((0, h_scroll))
                .block(
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
            let new_para = Paragraph::new(new_lines)
                .style(Style::default().bg(bg))
                .scroll((0, h_scroll))
                .block(
                    Block::default()
                        .title(Line::styled(" New ", title_style))
                        .borders(new_borders)
                        .style(Style::default().bg(bg))
                        .border_style(border_style),
                );
            frame.render_widget(new_para, area);
        }

        // The new panel has no column of its own for markers when both panels are shown:
        // its annotation markers and its focus bar go on the border it shares with the
        // old panel. The focus bar is drawn last, as it wins in a panel's own column.
        if let (Some(old_a), Some(_)) = (old_area, new_area) {
            let border_x = old_a.x + old_a.width - 1; // Right border of old panel
            let content_start = old_a.y + 1; // After top border of block
            let buf = frame.buffer_mut();

            let markers = border_marker_rows.iter().map(|&row| (row, t.ui.highlight));
            let focus_bar = border_focus_rows.iter().map(|&row| (row, t.ui.border_focused));
            for (para_row, color) in markers.chain(focus_bar) {
                let screen_row = content_start + para_row as u16;
                if screen_row < old_a.y + old_a.height - 1 {
                    if let Some(cell) =
                        buf.cell_mut(ratatui::layout::Position::new(border_x, screen_row))
                    {
                        cell.set_fg(color);
                        cell.set_char('▎');
                    }
                }
            }
        }

        // Render overlay slots (saved annotations + active editor) per-panel for line-range,
        // spanning both panels for file-level.
        for &(line_pos, slot) in &slot_overlays {
            let (overlay_x, overlay_width, overlay_start_y, suppress_gutter) =
                slot_geometry(slot.target());
            render_overlay_slot(
                frame,
                line_pos,
                slot,
                overlay_x,
                overlay_start_y,
                overlay_width,
                main_area,
                bg,
                t,
                suppress_gutter,
                &mut annotation_rects,
                &mut editor_rect,
            );
        }

        // Extend border markers through new-panel slot overlay rows on the shared border
        if let (Some(old_a), Some(_)) = (old_area, new_area) {
            let border_x = old_a.x + old_a.width - 1;
            let content_start = old_a.y + 1;
            let buf = frame.buffer_mut();

            for &(line_pos, slot) in &slot_overlays {
                if let AnnotationTarget::LineRange { panel, .. } = slot.target() {
                    if matches!(panel, DiffPanelFocus::New | DiffPanelFocus::None) {
                        let num_rows = slot.height();
                        for row_offset in 0..num_rows {
                            let screen_row = content_start + line_pos as u16 + row_offset as u16;
                            if screen_row < old_a.y + old_a.height - 1 {
                                if let Some(cell) = buf
                                    .cell_mut(ratatui::layout::Position::new(border_x, screen_row))
                                {
                                    cell.set_fg(t.ui.highlight);
                                    cell.set_char('▎');
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Merge the sidebar's right border with the diff panel's left border into
    // a single shared line by overwriting the corner cells with T-junctions.
    if show_sidebar {
        let junction_x = main_area.x;
        let top_y = main_area.y;
        let bottom_y = main_area.y + main_area.height.saturating_sub(1);
        let border_color = t.ui.border_unfocused;
        let buf = frame.buffer_mut();
        if let Some(cell) = buf.cell_mut(ratatui::layout::Position::new(junction_x, top_y)) {
            cell.set_char('┬');
            cell.set_fg(border_color);
        }
        if bottom_y > top_y {
            if let Some(cell) = buf.cell_mut(ratatui::layout::Position::new(junction_x, bottom_y)) {
                cell.set_char('┴');
                cell.set_fg(border_color);
            }
        }
    }

    render_footer(
        frame,
        footer_area,
        FooterData {
            filename: &diff.filename,
            commit_ref,
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
            review,
        },
    );

    (content_row_offset, overlay_gaps, annotation_rects, editor_rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::diff::review_thread::Comment;

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn test_thread_rows_aligns_box_borders_for_markdown_comments() {
        let t = theme::get();
        let mut thread = Thread::new(
            "src/app.py".to_string(),
            AnnotationTarget::File,
            None,
            Comment::agent(
                "⚠️ Use `def foo()` and **bold** text:\n<details>\n<summary>More</summary>\n</details>\n```\ndef foo():\n    return \"🧑‍💻\"\n```"
                    .to_string(),
                "pi".to_string(),
            ),
        );
        let width = 30;
        let rendered = thread_rows(&thread, None, width, 40, 4, t);
        assert!(rendered.rows.len() >= 7);
        assert!(rendered.expand_rows.is_empty());
        for (idx, row) in rendered.rows.iter().enumerate() {
            assert_eq!(row.width(), width, "row {} misaligned", idx);
            for span in &row.spans {
                assert!(
                    !span.content.contains('\n') && !span.content.contains('\r'),
                    "row {} contained raw newline in span {:?}",
                    idx,
                    span.content
                );
            }
        }

        thread.resolved = true;
        let resolved = thread_rows(&thread, None, width, 40, 4, t);
        assert_eq!(resolved.rows.len(), 1);
        assert!(resolved.expand_rows.is_empty());
    }

    #[test]
    fn test_thread_rows_tail_keeps_latest_comments_and_hides_earlier_ones() {
        let t = theme::get();
        let mut thread = Thread::new(
            "src/app.rs".to_string(),
            AnnotationTarget::File,
            None,
            Comment::human("first comment".to_string(), false),
        );
        thread.comments.push(Comment::agent("second comment".to_string(), "pi".to_string()));
        thread.comments.push(Comment::human("third comment".to_string(), false));
        thread.comments.push(Comment::agent("fourth comment".to_string(), "pi".to_string()));

        // Each comment takes 2 rows (header + 1-line body) = 8 inner rows total.
        // Cap to 7 total rows (5 inner): 1 indicator + 2 latest comments (4 rows).
        let width = 46;
        let rendered = thread_rows(&thread, None, width, 7, 4, t);
        assert_eq!(rendered.rows.len(), 7);
        assert_eq!(rendered.expand_rows, vec![1]);
        for (idx, row) in rendered.rows.iter().enumerate() {
            assert_eq!(row.width(), width, "row {} misaligned", idx);
        }

        let texts: Vec<String> = rendered.rows.iter().map(line_text).collect();
        assert!(texts[1].contains("⋯ 2 earlier comments (t to view)"), "got: {}", texts[1]);
        let all = texts.join("\n");
        assert!(!all.contains("first comment"));
        assert!(!all.contains("second comment"));
        assert!(all.contains("third comment"));
        assert!(all.contains("fourth comment"));
        assert!(texts.last().unwrap().starts_with('└'));
    }

    #[test]
    fn test_thread_rows_single_long_comment_keeps_top_and_marks_more_lines() {
        let t = theme::get();
        let long_body: String = (1..=15)
            .map(|n| format!("line {:02}", n))
            .collect::<Vec<_>>()
            .join("\n");
        let mut thread = Thread::new(
            "src/app.rs".to_string(),
            AnnotationTarget::File,
            None,
            Comment::agent(long_body.clone(), "pi".to_string()),
        );

        let width = 46;
        let single = thread_rows(&thread, None, width, 8, 4, t);
        assert_eq!(single.rows.len(), 8);
        assert_eq!(single.expand_rows, vec![6]);
        let single_texts: Vec<String> = single.rows.iter().map(line_text).collect();
        assert!(!single_texts.join("\n").contains("earlier comment"));
        assert!(single_texts[1].contains("pi"));
        assert!(single_texts[2].contains("line 01"));
        assert!(single_texts[5].contains("line 04"));
        assert!(single_texts[6].contains("⋯ 11 more lines (t to view)"), "got: {}", single_texts[6]);
        for (idx, row) in single.rows.iter().enumerate() {
            assert_eq!(row.width(), width, "single row {} misaligned", idx);
        }

        // Now add an earlier comment before the long latest comment.
        thread.comments.insert(0, Comment::human("short opener".to_string(), false));
        let multi = thread_rows(&thread, None, width, 8, 4, t);
        assert_eq!(multi.rows.len(), 8);
        assert_eq!(multi.expand_rows, vec![1, 6]);
        let multi_texts: Vec<String> = multi.rows.iter().map(line_text).collect();
        assert!(multi_texts[1].contains("⋯ 1 earlier comment (t to view)"), "got: {}", multi_texts[1]);
        assert!(multi_texts[2].contains("pi"));
        assert!(multi_texts[3].contains("line 01"));
        assert!(multi_texts[5].contains("line 03"));
        assert!(multi_texts[6].contains("⋯ 12 more lines (t to view)"), "got: {}", multi_texts[6]);
    }

    #[test]
    fn test_build_slots_reserves_height_for_editor_and_caps_file_threads() {
        let t = theme::get();
        let mut thread = Thread::new(
            "src/app.rs".to_string(),
            AnnotationTarget::File,
            None,
            Comment::agent("line\n".repeat(40), "pi".to_string()),
        );
        thread.comments.push(Comment::human("follow up".to_string(), false));

        let width_for = |_: &AnnotationTarget| 50u16;
        let visible_height = 30;

        let without_editor = build_slots(
            std::slice::from_ref(&thread),
            None,
            "src/app.rs",
            true,
            &width_for,
            visible_height,
            4,
            t,
        );
        assert_eq!(without_editor.len(), 1);
        assert!(
            without_editor[0].height() <= visible_height * 40 / 100,
            "file-level thread took {} of {} rows",
            without_editor[0].height(),
            visible_height
        );

        let editor = AnnotationEditor::for_thread(&thread);
        let with_editor = build_slots(
            std::slice::from_ref(&thread),
            Some(&editor),
            "src/app.rs",
            true,
            &width_for,
            visible_height,
            4,
            t,
        );
        assert_eq!(with_editor.len(), 2);
        let combined: usize = with_editor.iter().map(OverlaySlot::height).sum();
        assert!(
            combined <= visible_height * 40 / 100,
            "saved ({}) + editor ({}) exceeded budget",
            with_editor[0].height(),
            with_editor[1].height()
        );
    }
}

