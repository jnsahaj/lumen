use std::collections::HashSet;

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::command::diff::context::{compute_context_lines, ContextLine};
use crate::command::diff::highlight::{highlight_line_spans, FileHighlighter};
use crate::command::diff::search::{MatchPanel, SearchState};
use crate::command::diff::state::{Annotation, AnnotationTarget};
use crate::command::diff::theme;
use crate::command::diff::types::{
    ChangeType, DiffFullscreen, DiffLine, DiffPanelFocus, DiffViewSettings, FileDiff, FocusedPanel,
    InlineSegment, Selection, SelectionMode, SidebarItem,
};
use crate::command::diff::PrInfo;

use super::footer::{render_footer, FooterData};
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
        Span::styled(&truncated_msg, Style::default().fg(t.ui.text_secondary).bg(bg)),
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

pub struct LineStats {
    pub added: usize,
    pub removed: usize,
}

fn expand_tabs_in_spans<'a>(spans: Vec<Span<'a>>, tab_width: usize) -> Vec<Span<'a>> {
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
            let Color::Rgb(sr, sg, sb) = SELECTION_TINT else { return base };
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
    fn for_change_type(change_type: ChangeType, bg: Color, t: &crate::command::diff::theme::Theme) -> Self {
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

/// Compute side_by_side index ranges for line-range annotations.
/// Returns `(first_sbs_idx, last_sbs_idx, panel)` for each annotation that has matching lines.
fn compute_ann_index_ranges(
    line_annotations: &[&Annotation],
    side_by_side: &[DiffLine],
) -> Vec<(usize, usize, DiffPanelFocus)> {
    let mut ranges = Vec::new();
    for ann in line_annotations {
        if let AnnotationTarget::LineRange { panel, start_line, end_line, .. } = &ann.target {
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
        Span::styled(indicator.content, indicator.style.bg(blend_with_selection(ind_bg)))
    } else {
        indicator
    }
}

/// Total rendered height of file-level annotation overlays.
fn file_annotation_height(annotations: &[&Annotation]) -> usize {
    annotations.iter().map(|a| a.content.lines().count() + 2).sum()
}

/// Render annotation overlays at specified positions.
///
/// This function renders annotation boxes that can span single or multiple panels.
/// The `content_x`, `content_start_y`, `content_width`, and `max_area` parameters
/// allow flexible positioning for both single-panel and side-by-side views.
fn render_annotation_overlays(
    frame: &mut Frame,
    overlays: &[(usize, &Annotation)],
    content_x: u16,
    content_start_y: u16,
    content_width: u16,
    max_area: Rect,
    bg: Color,
    t: &crate::command::diff::theme::Theme,
    suppress_gutter: bool,
) {
    // Annotation accent color — a subtle but visible tint
    let ann_accent = t.ui.highlight;

    for (line_pos, annotation) in overlays {
        let screen_y = content_start_y + *line_pos as u16;
        let content_lines: Vec<&str> = annotation.content.lines().collect();
        let num_lines = content_lines.len() + 2; // +2 for top and bottom borders

        // Check if annotation is visible
        if screen_y >= max_area.y + max_area.height {
            continue;
        }

        let available_height = (max_area.y + max_area.height).saturating_sub(screen_y) as usize;
        if available_height == 0 {
            continue;
        }

        let overlay_height = num_lines.min(available_height) as u16;
        let overlay_area = Rect::new(content_x, screen_y, content_width, overlay_height);

        // Clear the area first
        frame.render_widget(ratatui::widgets::Clear, overlay_area);

        // Build annotation lines
        let mut ann_lines: Vec<Line> = Vec::new();
        let note_style = Style::default().fg(t.ui.text_secondary);
        let border_style_ann = Style::default().fg(ann_accent);
        let indicator_style = Style::default().fg(ann_accent);
        let border_width = content_width.saturating_sub(3) as usize;

        // For line-range annotations, show a gutter indicator connecting to the lines above.
        // Suppressed for new-panel annotations in side-by-side mode, where the indicator
        // is rendered on the shared border via buffer_mut() instead.
        let has_gutter = !suppress_gutter && matches!(annotation.target, AnnotationTarget::LineRange { .. });

        // Add top border — gutter indicator continues into the box
        if has_gutter {
            ann_lines.push(Line::from(vec![
                Span::styled("▍", indicator_style),
                Span::styled(format!("┌{}┐", "─".repeat(border_width)), border_style_ann),
            ]));
        } else {
            ann_lines.push(Line::from(vec![Span::styled(
                format!(" ┌{}┐", "─".repeat(border_width)),
                border_style_ann,
            )]));
        }

        // Add content lines
        for content_line in content_lines.iter().take(available_height.saturating_sub(2)) {
            let content_width_inner = border_width.saturating_sub(1);
            let padded_content = format!("{:<width$}", content_line, width = content_width_inner);
            if has_gutter {
                ann_lines.push(Line::from(vec![
                    Span::styled("▍", indicator_style),
                    Span::styled("│ ", border_style_ann),
                    Span::styled(padded_content, note_style),
                    Span::styled("│", border_style_ann),
                ]));
            } else {
                ann_lines.push(Line::from(vec![
                    Span::styled(" │ ", border_style_ann),
                    Span::styled(padded_content, note_style),
                    Span::styled("│", border_style_ann),
                ]));
            }
        }

        // Add bottom border with time if there's room
        if ann_lines.len() < available_height {
            let time_str = annotation.format_time();
            let time_with_padding = format!(" {} ", time_str);
            let time_len = time_with_padding.len();
            let dashes_before = border_width.saturating_sub(time_len + 1);
            if has_gutter {
                ann_lines.push(Line::from(vec![
                    Span::styled("▍", indicator_style),
                    Span::styled(format!("└{}", "─".repeat(dashes_before)), border_style_ann),
                    Span::styled(time_with_padding, Style::default().fg(t.ui.text_muted)),
                    Span::styled("─┘", border_style_ann),
                ]));
            } else {
                ann_lines.push(Line::from(vec![
                    Span::styled(format!(" └{}", "─".repeat(dashes_before)), border_style_ann),
                    Span::styled(time_with_padding, Style::default().fg(t.ui.text_muted)),
                    Span::styled("─┘", border_style_ann),
                ]));
            }
        }

        let ann_para = Paragraph::new(ann_lines).style(Style::default().bg(bg));
        frame.render_widget(ann_para, overlay_area);
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
    annotations: &[Annotation],
    selection: &Selection,
    old_highlighter: &FileHighlighter,
    new_highlighter: &FileHighlighter,
) -> (usize, Vec<(usize, usize)>) {
    let area = frame.area();
    let t = theme::get();
    let bg = t.ui.bg;

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
            },
        );
        return (0, Vec::new());
    }

    // side_by_side is now passed as a parameter (pre-computed and cached)
    let line_stats = compute_line_stats(side_by_side);

    let is_new_file = diff.old_content.is_empty() && !diff.new_content.is_empty();
    let is_deleted_file = !diff.old_content.is_empty() && diff.new_content.is_empty();

    // Track how many non-diff rows are at the top (context lines + file annotations)
    let content_row_offset: usize;
    // Track inline annotation overlay gaps for mouse coordinate mapping
    let mut overlay_gaps: Vec<(usize, usize)> = Vec::new();

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
        let scroll_usize = scroll as usize;

        // Get file-level annotations for this file
        let file_annotations: Vec<&Annotation> = annotations
            .iter()
            .filter(|a| a.filename == diff.filename && matches!(a.target, AnnotationTarget::File))
            .collect();

        // Collect line-range annotations for this file (New panel only)
        let line_annotations: Vec<&Annotation> = annotations
            .iter()
            .filter(|a| a.filename == diff.filename && matches!(a.target, AnnotationTarget::LineRange { .. }))
            .collect();

        let annotation_height = file_annotation_height(&file_annotations);
        content_row_offset = context_count + annotation_height;

        let base_content_height = visible_height.saturating_sub(context_count);
        let content_height = base_content_height.saturating_sub(annotation_height);

        let visible_lines: Vec<&DiffLine> = side_by_side
            .iter()
            .skip(scroll_usize)
            .take(content_height)
            .collect();

        let mut new_lines: Vec<Line> = Vec::new();
        let mut annotation_overlays: Vec<(usize, &Annotation)> = Vec::new();

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

        // Show file-level annotations at top
        for annotation in &file_annotations {
            let content_lines: Vec<&str> = annotation.content.lines().collect();
            let num_lines = content_lines.len() + 2;
            let annotation_start = new_lines.len();
            for _ in 0..num_lines {
                new_lines.push(Line::from(vec![Span::raw("")]));
            }
            annotation_overlays.push((annotation_start, annotation));
        }

        let ann_index_ranges = compute_ann_index_ranges(&line_annotations, side_by_side);
        let annotation_indicator_style = Style::default().fg(t.ui.highlight);
        let focus_style = Style::default().fg(t.ui.border_focused);

        for (i, diff_line) in visible_lines.iter().enumerate() {
            let line_idx = scroll_usize + i;
            let new_selection_range = get_selection_range_for_line(line_idx, DiffPanelFocus::New, selection);
            let in_annotation = is_in_ann_range(line_idx, None, &ann_index_ranges);
            let new_line_selected = new_selection_range.map_or(false, |(s, e)| s == 0 && e == usize::MAX);

            if let Some((num, text)) = &diff_line.new_line {
                let mut spans: Vec<Span> = Vec::new();
                spans.push(make_indicator_span(false, in_annotation, new_line_selected, bg, focus_style, annotation_indicator_style));

                let prefix = format!("{:4} ", num);
                let gutter_bg = t.diff.added_gutter_bg;
                let gutter_bg = if new_line_selected {
                    blend_with_selection(gutter_bg)
                } else {
                    gutter_bg
                };
                spans.push(Span::styled(
                    prefix,
                    Style::default()
                        .fg(t.diff.added_gutter_fg)
                        .bg(gutter_bg),
                ));
                let matches = search_state.get_matches_for_line(line_idx, MatchPanel::New);
                let content_spans = apply_search_highlight(
                    text,
                    &diff.filename,
                    Some(t.diff.added_bg),
                    &matches,
                    Some(new_highlighter),
                    Some(*num),
                    settings.tab_width,
                );
                let content_spans = apply_selection_to_spans(
                    content_spans,
                    new_selection_range,
                    t.diff.added_bg,
                );
                spans.extend(content_spans);
                new_lines.push(Line::from(spans));
            }

            // Check if this line is the end_line for any line-range annotation
            for annotation in &line_annotations {
                if let AnnotationTarget::LineRange { panel, end_line, .. } = &annotation.target {
                    if diff_line.line_number(*panel) == Some(*end_line) {
                        let num_ann_lines = annotation.content.lines().count() + 2;
                        let line_pos = new_lines.len();
                        overlay_gaps.push((i, num_ann_lines));
                        for _ in 0..num_ann_lines {
                            new_lines.push(Line::from(vec![Span::raw("")]));
                        }
                        annotation_overlays.push((line_pos, annotation));
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

        // Render annotation overlays
        let content_x = main_area.x + 1;
        let content_start_y = main_area.y + 1;
        let content_width = main_area.width.saturating_sub(2);
        render_annotation_overlays(frame, &annotation_overlays, content_x, content_start_y, content_width, main_area, bg, t, false);
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
        let scroll_usize = scroll as usize;

        // Get file-level annotations for this file
        let file_annotations: Vec<&Annotation> = annotations
            .iter()
            .filter(|a| a.filename == diff.filename && matches!(a.target, AnnotationTarget::File))
            .collect();

        // Collect line-range annotations for this file (Old panel only)
        let line_annotations: Vec<&Annotation> = annotations
            .iter()
            .filter(|a| a.filename == diff.filename && matches!(a.target, AnnotationTarget::LineRange { .. }))
            .collect();

        let annotation_height = file_annotation_height(&file_annotations);
        content_row_offset = context_count + annotation_height;

        let base_content_height = visible_height.saturating_sub(context_count);
        let content_height = base_content_height.saturating_sub(annotation_height);

        let visible_lines: Vec<&DiffLine> = side_by_side
            .iter()
            .skip(scroll_usize)
            .take(content_height)
            .collect();

        let mut old_lines: Vec<Line> = Vec::new();
        let mut annotation_overlays: Vec<(usize, &Annotation)> = Vec::new();

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

        // Show file-level annotations at top
        for annotation in &file_annotations {
            let content_lines: Vec<&str> = annotation.content.lines().collect();
            let num_lines = content_lines.len() + 2;
            let annotation_start = old_lines.len();
            for _ in 0..num_lines {
                old_lines.push(Line::from(vec![Span::raw("")]));
            }
            annotation_overlays.push((annotation_start, annotation));
        }

        let ann_index_ranges = compute_ann_index_ranges(&line_annotations, side_by_side);
        let annotation_indicator_style = Style::default().fg(t.ui.highlight);
        let focus_style = Style::default().fg(t.ui.border_focused);

        for (i, diff_line) in visible_lines.iter().enumerate() {
            let line_idx = scroll_usize + i;
            let old_selection_range = get_selection_range_for_line(line_idx, DiffPanelFocus::Old, selection);
            let in_annotation = is_in_ann_range(line_idx, None, &ann_index_ranges);
            let old_line_selected = old_selection_range.map_or(false, |(s, e)| s == 0 && e == usize::MAX);

            if let Some((num, text)) = &diff_line.old_line {
                let mut spans: Vec<Span> = Vec::new();
                spans.push(make_indicator_span(false, in_annotation, old_line_selected, bg, focus_style, annotation_indicator_style));

                let prefix = format!("{:4} ", num);
                let gutter_bg = t.diff.deleted_gutter_bg;
                let gutter_bg = if old_line_selected {
                    blend_with_selection(gutter_bg)
                } else {
                    gutter_bg
                };
                spans.push(Span::styled(
                    prefix,
                    Style::default()
                        .fg(t.diff.deleted_gutter_fg)
                        .bg(gutter_bg),
                ));
                let matches = search_state.get_matches_for_line(line_idx, MatchPanel::Old);
                let content_spans = apply_search_highlight(
                    text,
                    &diff.filename,
                    Some(t.diff.deleted_bg),
                    &matches,
                    Some(old_highlighter),
                    Some(*num),
                    settings.tab_width,
                );
                let content_spans = apply_selection_to_spans(
                    content_spans,
                    old_selection_range,
                    t.diff.deleted_bg,
                );
                spans.extend(content_spans);
                old_lines.push(Line::from(spans));
            }

            // Check if this line is the end_line for any line-range annotation
            for annotation in &line_annotations {
                if let AnnotationTarget::LineRange { panel, end_line, .. } = &annotation.target {
                    if diff_line.line_number(*panel) == Some(*end_line) {
                        let num_ann_lines = annotation.content.lines().count() + 2;
                        let line_pos = old_lines.len();
                        overlay_gaps.push((i, num_ann_lines));
                        for _ in 0..num_ann_lines {
                            old_lines.push(Line::from(vec![Span::raw("")]));
                        }
                        annotation_overlays.push((line_pos, annotation));
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

        // Render annotation overlays
        let content_x = main_area.x + 1;
        let content_start_y = main_area.y + 1;
        let content_width = main_area.width.saturating_sub(2);
        render_annotation_overlays(frame, &annotation_overlays, content_x, content_start_y, content_width, main_area, bg, t, false);
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
        let mut annotation_overlays: Vec<(usize, &Annotation)> = Vec::new();

        // Collect file-level annotations
        let file_annotations: Vec<&Annotation> = annotations
            .iter()
            .filter(|a| a.filename == diff.filename && matches!(a.target, AnnotationTarget::File))
            .collect();

        // Collect line-range annotations for this file
        let line_annotations: Vec<&Annotation> = annotations
            .iter()
            .filter(|a| a.filename == diff.filename && matches!(a.target, AnnotationTarget::LineRange { .. }))
            .collect();

        let file_ann_height = file_annotation_height(&file_annotations);
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

        // Add file-level annotations at the top (after context lines)
        for annotation in &file_annotations {
            let content_lines_count = annotation.content.lines().count();
            let num_lines = content_lines_count + 2;
            let annotation_start = old_lines.len(); // same position in both panels
            for _ in 0..num_lines {
                if old_area.is_some() {
                    old_lines.push(Line::from(vec![Span::raw("")]));
                }
                if new_area.is_some() {
                    new_lines.push(Line::from(vec![Span::raw("")]));
                }
            }
            annotation_overlays.push((annotation_start, annotation));
        }

        let ann_index_ranges = compute_ann_index_ranges(&line_annotations, side_by_side);

        // Track rendered rows for new-panel border annotation markers (paragraph-relative)
        let mut border_marker_rows: Vec<usize> = Vec::new();
        let mut rendered_row = content_row_offset;

        let focus_style = Style::default().fg(t.ui.border_focused);
        let annotation_indicator_style = Style::default().fg(t.ui.highlight);

        for (i, diff_line) in visible_lines.iter().enumerate() {
            let line_idx = scroll_usize + i;
            let in_focused = is_in_focused_hunk(line_idx, diff_line.change_type);
            let style = DiffLineStyle::for_change_type(diff_line.change_type, bg, t);

            let old_selection_range = get_selection_range_for_line(line_idx, DiffPanelFocus::Old, selection);
            let new_selection_range = get_selection_range_for_line(line_idx, DiffPanelFocus::New, selection);

            let old_in_annotation = is_in_ann_range(line_idx, Some(DiffPanelFocus::Old), &ann_index_ranges);
            let new_in_annotation = is_in_ann_range(line_idx, Some(DiffPanelFocus::New), &ann_index_ranges);

            if new_in_annotation {
                border_marker_rows.push(rendered_row);
            }
            rendered_row += 1;

            let old_line_selected = old_selection_range.map_or(false, |(s, e)| s == 0 && e == usize::MAX);
            let new_line_selected = new_selection_range.map_or(false, |(s, e)| s == 0 && e == usize::MAX);

            if old_area.is_some() {
                let mut old_spans: Vec<Span> = Vec::new();
                old_spans.push(make_indicator_span(in_focused, old_in_annotation, old_line_selected, bg, focus_style, annotation_indicator_style));
                match &diff_line.old_line {
                    Some((num, _text)) => {
                        let prefix = format!("{:4} ", num);
                        let gutter_bg = style.old_gutter_bg.unwrap_or(Color::Reset);
                        let gutter_bg = if old_line_selected { blend_with_selection(if gutter_bg == Color::Reset { bg } else { gutter_bg }) } else { gutter_bg };
                        old_spans.push(Span::styled(
                            prefix,
                            Style::default()
                                .fg(style.old_gutter_fg.unwrap_or(t.ui.line_number))
                                .bg(gutter_bg),
                        ));
                        let matches = search_state.get_matches_for_line(line_idx, MatchPanel::Old);

                        // Use word-level rendering for modified lines if segments are available
                        let content_spans = if matches!(diff_line.change_type, ChangeType::Modified) {
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
                        old_spans.extend(content_spans);
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
                    new_spans.push(make_indicator_span(in_focused, new_in_annotation, new_line_selected, bg, focus_style, annotation_indicator_style));
                }
                match &diff_line.new_line {
                    Some((num, _text)) => {
                        let prefix = format!("{:4} ", num);
                        let gutter_bg = style.new_gutter_bg.unwrap_or(Color::Reset);
                        let gutter_bg = if new_line_selected { blend_with_selection(if gutter_bg == Color::Reset { bg } else { gutter_bg }) } else { gutter_bg };
                        new_spans.push(Span::styled(
                            prefix,
                            Style::default()
                                .fg(style.new_gutter_fg.unwrap_or(t.ui.line_number))
                                .bg(gutter_bg),
                        ));
                        let matches = search_state.get_matches_for_line(line_idx, MatchPanel::New);

                        // Use word-level rendering for modified lines if segments are available
                        let content_spans = if matches!(diff_line.change_type, ChangeType::Modified) {
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
                        new_spans.extend(content_spans);
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

            // Check if this line is the end_line for any line-range annotation
            for annotation in &line_annotations {
                if let AnnotationTarget::LineRange { panel, end_line, .. } = &annotation.target {
                    if diff_line.line_number(*panel) == Some(*end_line) {
                        let num_lines = annotation.content.lines().count() + 2;

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
                            rendered_row += 1;
                        }

                        annotation_overlays.push((line_pos, annotation));
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

        // Render annotation gutter markers on the shared border for new panel annotations
        if let (Some(old_a), Some(_)) = (old_area, new_area) {
            let border_x = old_a.x + old_a.width - 1; // Right border of old panel
            let content_start = old_a.y + 1; // After top border of block
            let buf = frame.buffer_mut();

            for &para_row in &border_marker_rows {
                let screen_row = content_start + para_row as u16;
                if screen_row < old_a.y + old_a.height - 1 {
                    if let Some(cell) = buf.cell_mut(ratatui::layout::Position::new(border_x, screen_row)) {
                        cell.set_fg(t.ui.highlight);
                        cell.set_char('▎');
                    }
                }
            }
        }

        // Render annotation overlays per-panel for line-range annotations,
        // spanning both panels for file-level annotations
        let is_side_by_side = old_area.is_some() && new_area.is_some();
        for &(line_pos, annotation) in &annotation_overlays {
            let (overlay_x, overlay_width, overlay_start_y, suppress_gutter) = match &annotation.target {
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
                    (area.x + 1, area.width.saturating_sub(2), area.y + 1, suppress)
                }
            };
            render_annotation_overlays(
                frame,
                &[(line_pos, annotation)],
                overlay_x,
                overlay_start_y,
                overlay_width,
                main_area,
                bg,
                t,
                suppress_gutter,
            );
        }

        // Extend border markers through new-panel annotation overlay rows on the shared border
        if let (Some(old_a), Some(_)) = (old_area, new_area) {
            let border_x = old_a.x + old_a.width - 1;
            let content_start = old_a.y + 1;
            let buf = frame.buffer_mut();

            for &(line_pos, annotation) in &annotation_overlays {
                if let AnnotationTarget::LineRange { panel, .. } = &annotation.target {
                    if matches!(panel, DiffPanelFocus::New | DiffPanelFocus::None) {
                        let content_lines_count = annotation.content.lines().count();
                        let num_rows = content_lines_count + 2;
                        for row_offset in 0..num_rows {
                            let screen_row = content_start + line_pos as u16 + row_offset as u16;
                            if screen_row < old_a.y + old_a.height - 1 {
                                if let Some(cell) = buf.cell_mut(ratatui::layout::Position::new(border_x, screen_row)) {
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
        },
    );

    (content_row_offset, overlay_gaps)
}
