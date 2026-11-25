use ratatui::prelude::*;

use crate::diff::context::ContextLine;
use crate::diff::highlight::{highlight_line_spans, FileHighlighter};
use crate::diff::{ChangeType, DiffLine, DiffPanelFocus, InlineSegment};
use crate::ui::coordinates::{Selection, SelectionMode};
use crate::ui::markdown::expand_tabs_in_spans;
use crate::ui::theme;

const SELECTION_TINT: Color = Color::Rgb(80, 120, 180);
const SELECTION_ALPHA: f32 = 0.4;

pub struct DiffLineStyle {
    pub old_bg: Option<Color>,
    pub old_gutter_bg: Option<Color>,
    pub old_gutter_fg: Option<Color>,
    pub new_bg: Option<Color>,
    pub new_gutter_bg: Option<Color>,
    pub new_gutter_fg: Option<Color>,
}

impl DiffLineStyle {
    pub fn for_change_type(change_type: ChangeType, bg: Color, t: &theme::Theme) -> Self {
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

pub fn generate_stripe_pattern(width: usize) -> String {
    "╱".repeat(width)
}

pub fn pad_line_bg<'a>(mut spans: Vec<Span<'a>>, target_width: usize, bg: Color) -> Vec<Span<'a>> {
    let current: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if current < target_width {
        spans.push(Span::styled(
            " ".repeat(target_width - current),
            Style::default().bg(bg),
        ));
    }
    spans
}

pub fn separator_line(label: &str, width: usize, t: &theme::Theme) -> Line<'static> {
    let text = format!("      {label}");
    let padding = width.saturating_sub(text.chars().count());
    Line::from(Span::styled(
        format!("{text}{}", " ".repeat(padding)),
        Style::default().fg(t.ui.text_muted).bg(t.diff.context_bg),
    ))
}

pub fn hidden_lines_label(hidden_lines: usize) -> String {
    format!("⋯ {hidden_lines} unchanged lines")
}

pub fn top_line_position(rows: &[DiffLine], scroll: usize, panel: DiffPanelFocus) -> usize {
    rows.iter()
        .skip(scroll)
        .find_map(|row| row.line_number(panel))
        .map_or(0, |line| line - 1)
}

pub fn blank_diff_line(width: usize, bg: Color) -> Line<'static> {
    Line::from(vec![Span::styled(
        " ".repeat(width),
        Style::default().bg(bg),
    )])
}

pub fn render_context_lines(
    context: &[ContextLine],
    total_count: usize,
    lines: &mut Vec<Line<'static>>,
    filename: &str,
    highlighter: &FileHighlighter,
    tab_width: usize,
) {
    let t = theme::get();
    let context_bg = t.diff.context_bg;

    for i in 0..total_count {
        if let Some(cl) = context.get(i) {
            let prefix = format!("{:4} ~ ", cl.line_number);
            let mut spans: Vec<Span<'static>> = vec![Span::styled(
                prefix,
                Style::default().fg(t.ui.line_number).bg(context_bg),
            )];
            let hl_spans = expand_tabs_in_spans(
                highlighter.get_line_spans(cl.line_number, Some(context_bg)),
                tab_width,
            );
            if hl_spans.is_empty() {
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

pub fn segments_to_emphasis_ranges(segments: &[InlineSegment]) -> Vec<(usize, usize)> {
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

fn is_muted_color(color: Color) -> bool {
    match color {
        Color::Rgb(r, g, b) => {
            let luminance = (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000;
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let saturation = if max == 0 {
                0
            } else {
                (max - min) as u32 * 100 / max as u32
            };
            luminance < 140 || (luminance < 180 && saturation < 30)
        }
        Color::DarkGray | Color::Gray => true,
        _ => false,
    }
}

fn boost_muted_fg(fg: Color, default_text: Color) -> Color {
    if is_muted_color(fg) {
        default_text
    } else {
        fg
    }
}

pub struct LineHighlightParams<'a> {
    pub text: &'a str,
    pub filename: &'a str,
    pub line_bg: Option<Color>,
    pub word_emphasis: Option<(Color, &'a [(usize, usize)])>,
    pub search_ranges: &'a [(usize, usize, bool)],
    pub highlighter: Option<&'a FileHighlighter>,
    pub line_number: Option<usize>,
    pub tab_width: usize,
}

/// Applies syntax highlighting plus optional search matches and word-level diff emphasis
/// using UTF-8 char-boundary iteration.
pub fn highlight_diff_line(params: &LineHighlightParams<'_>) -> Vec<Span<'static>> {
    let t = theme::get();
    let base_spans = if let (Some(hl), Some(line_num)) = (params.highlighter, params.line_number) {
        let spans = hl.get_line_spans(line_num, params.line_bg);
        if spans.is_empty() {
            highlight_line_spans(params.text, params.filename, params.line_bg)
        } else {
            spans
        }
    } else {
        highlight_line_spans(params.text, params.filename, params.line_bg)
    };
    let base_spans = expand_tabs_in_spans(base_spans, params.tab_width);

    let emphasis = params
        .word_emphasis
        .filter(|(_, ranges)| !ranges.is_empty());
    if emphasis.is_none() && params.search_ranges.is_empty() {
        return base_spans;
    }

    let mut result: Vec<Span<'static>> = Vec::new();
    let mut byte_pos = 0usize;

    for span in base_spans {
        let span_text = span.content.to_string();
        let span_byte_len = span_text.len();
        let char_indices: Vec<(usize, char)> = span_text.char_indices().collect();
        if char_indices.is_empty() {
            byte_pos += span_byte_len;
            continue;
        }

        let style_at = |global_pos: usize| -> Style {
            if let Some(&(_, _, is_current)) = params
                .search_ranges
                .iter()
                .find(|&&(s, e, _)| global_pos >= s && global_pos < e)
            {
                let (fg, bg) = if is_current {
                    (t.ui.search_current_fg, t.ui.search_current_bg)
                } else {
                    (t.ui.search_match_fg, t.ui.search_match_bg)
                };
                return Style::default().fg(fg).bg(bg).bold();
            }
            if let Some((word_bg, ranges)) = emphasis {
                if ranges
                    .iter()
                    .any(|&(s, e)| global_pos >= s && global_pos < e)
                {
                    let original_fg = span.style.fg.unwrap_or(t.syntax.default_text);
                    let boosted_fg = boost_muted_fg(original_fg, t.syntax.default_text);
                    return Style::default().fg(boosted_fg).bg(word_bg);
                }
            }
            span.style
        };

        let mut idx = 0;
        while idx < char_indices.len() {
            let run_start_byte = char_indices[idx].0;
            let run_style = style_at(byte_pos + run_start_byte);
            let mut run_end_idx = idx + 1;
            while run_end_idx < char_indices.len() {
                let next_style = style_at(byte_pos + char_indices[run_end_idx].0);
                if next_style != run_style {
                    break;
                }
                run_end_idx += 1;
            }
            let run_end_byte = if run_end_idx < char_indices.len() {
                char_indices[run_end_idx].0
            } else {
                span_byte_len
            };
            result.push(Span::styled(
                span_text[run_start_byte..run_end_byte].to_string(),
                run_style,
            ));
            idx = run_end_idx;
        }

        byte_pos += span_byte_len;
    }

    result
}

#[inline]
pub fn blend_with_selection(base: Color) -> Color {
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

#[inline]
pub fn get_selection_range_for_line(
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
        SelectionMode::Line => Some((0, usize::MAX)),
        SelectionMode::Character => {
            if start.line == end.line {
                Some((start.column, end.column))
            } else if line_idx == start.line {
                Some((start.column, usize::MAX))
            } else if line_idx == end.line {
                Some((0, end.column))
            } else {
                Some((0, usize::MAX))
            }
        }
        SelectionMode::None => None,
    }
}

#[inline]
pub fn apply_selection_to_spans<'a>(
    spans: Vec<Span<'a>>,
    selection_range: Option<(usize, usize)>,
    default_bg: Color,
) -> Vec<Span<'a>> {
    let Some((sel_start, sel_end)) = selection_range else {
        return spans;
    };

    if sel_start == 0 && sel_end == usize::MAX {
        return spans
            .into_iter()
            .map(|span| {
                let bg = span.style.bg.unwrap_or(default_bg);
                Span::styled(span.content, span.style.bg(blend_with_selection(bg)))
            })
            .collect();
    }

    let mut result = Vec::with_capacity(spans.len() * 2);
    let mut col = 0usize;

    for span in spans {
        let text = span.content.to_string();
        let span_len = text.chars().count();
        let span_end = col + span_len;

        if span_end <= sel_start || col >= sel_end {
            result.push(Span::styled(text, span.style));
            col = span_end;
            continue;
        }

        let bg = span.style.bg.unwrap_or(default_bg);
        let selected_bg = blend_with_selection(bg);
        let chars: Vec<char> = text.chars().collect();

        if col < sel_start {
            let before: String = chars[..(sel_start - col)].iter().collect();
            result.push(Span::styled(before, span.style));
        }
        let sel_start_in_span = sel_start.saturating_sub(col);
        let sel_end_in_span = (sel_end - col).min(span_len);
        if sel_start_in_span < sel_end_in_span {
            let selected: String = chars[sel_start_in_span..sel_end_in_span].iter().collect();
            result.push(Span::styled(selected, span.style.bg(selected_bg)));
        }
        if sel_end < span_end {
            let after: String = chars[(sel_end - col)..].iter().collect();
            result.push(Span::styled(after, span.style));
        }

        col = span_end;
    }

    result
}

pub fn make_indicator_span(
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

fn wrap_break_len(chars: &[(char, Style)], max_width: usize) -> (usize, usize) {
    if chars.len() <= max_width {
        return (chars.len(), chars.len());
    }

    let mut break_at = max_width;
    for (idx, (ch, _)) in chars
        .iter()
        .enumerate()
        .take(max_width.min(chars.len().saturating_sub(1)) + 1)
        .skip(1)
    {
        if ch.is_whitespace() {
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

pub fn push_wrapped_diff_line(
    lines: &mut Vec<Line<'static>>,
    prefix_spans: Vec<Span<'static>>,
    content_spans: Vec<Span<'static>>,
    target_width: usize,
    bg: Color,
    wrap: bool,
) {
    if !wrap {
        let mut spans = prefix_spans;
        spans.extend(content_spans);
        lines.push(Line::from(pad_line_bg(spans, target_width, bg)));
        return;
    }

    let prefix_width = diff_spans_width(&prefix_spans);
    let content_width = target_width
        .saturating_sub(prefix_width)
        .saturating_sub(1)
        .max(1);
    let chars = styled_chars(content_spans);
    let blank_prefix: Vec<Span<'static>> = prefix_spans
        .iter()
        .filter_map(|span| {
            let width = span.content.chars().count();
            (width > 0).then(|| Span::styled(" ".repeat(width), span.style))
        })
        .collect();

    if chars.is_empty() {
        lines.push(Line::from(pad_line_bg(prefix_spans, target_width, bg)));
        return;
    }

    let indent = chars
        .iter()
        .take_while(|(ch, _)| *ch == ' ')
        .count()
        .min(content_width.saturating_sub(1));
    let indent_span =
        (indent > 0).then(|| Span::styled(" ".repeat(indent), Style::default().bg(bg)));

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
            prefix_spans.clone()
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
}
