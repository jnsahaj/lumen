use std::collections::HashSet;

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::command::diff::context::{compute_context_lines, ContextLine};
use crate::command::diff::diff_algo::compute_side_by_side;
use crate::command::diff::highlight::{highlight_line_spans, FileHighlighter};
use crate::command::diff::search::{MatchPanel, SearchState};
use crate::command::diff::state::HunkAnnotation;
use crate::command::diff::theme;
use crate::command::diff::types::{
    ChangeType, DiffFullscreen, DiffLine, DiffViewSettings, FileDiff, FocusedPanel, InlineSegment,
    SidebarItem,
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
    let center_spans = vec![
        Span::styled(
            vcs_indicator.clone(),
            Style::default()
                .fg(t.ui.text_muted)
                .bg(t.ui.footer_branch_bg),
        ),
        Span::styled(" ", Style::default().bg(bg)),
        Span::styled(
            nav_indicator.clone(),
            Style::default()
                .fg(t.ui.highlight)
                .bg(t.ui.footer_branch_bg),
        ),
        Span::styled(" ", Style::default().bg(bg)),
        Span::styled(
            id_label.clone(),
            Style::default()
                .fg(t.ui.footer_branch_fg)
                .bg(t.ui.footer_branch_bg),
        ),
        Span::styled("  ", Style::default().bg(bg)),
        Span::styled(
            truncated_msg.clone(),
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

/// Render annotation overlays for single-panel views (new file, deleted file)
fn render_annotation_overlays(
    frame: &mut Frame,
    overlays: &[(usize, &HunkAnnotation)],
    area: Rect,
    bg: Color,
    t: &crate::command::diff::theme::Theme,
) {
    let content_start_y = area.y + 1;
    let content_x = area.x + 1;
    let content_width = area.width.saturating_sub(2);

    for (line_pos, annotation) in overlays {
        let screen_y = content_start_y + *line_pos as u16;
        let content_lines: Vec<&str> = annotation.content.lines().collect();
        let num_lines = content_lines.len() + 2;

        if screen_y >= area.y + area.height {
            continue;
        }

        let available_height = (area.y + area.height).saturating_sub(screen_y) as usize;
        if available_height == 0 {
            continue;
        }

        let overlay_height = num_lines.min(available_height) as u16;
        let overlay_area = Rect::new(content_x, screen_y, content_width, overlay_height);

        frame.render_widget(ratatui::widgets::Clear, overlay_area);

        let mut ann_lines: Vec<Line> = Vec::new();
        let note_style = Style::default().fg(t.ui.text_muted).italic();
        let border_style_ann = Style::default().fg(t.ui.border_unfocused);
        let border_width = content_width.saturating_sub(3) as usize;

        ann_lines.push(Line::from(vec![Span::styled(
            format!(" ┌{}┐", "─".repeat(border_width)),
            border_style_ann,
        )]));

        for content_line in content_lines.iter().take(available_height.saturating_sub(2)) {
            let content_width_inner = border_width.saturating_sub(1);
            let padded_content = format!("{:<width$}", content_line, width = content_width_inner);
            ann_lines.push(Line::from(vec![
                Span::styled(" │ ", border_style_ann),
                Span::styled(padded_content, note_style),
                Span::styled("│", border_style_ann),
            ]));
        }

        if ann_lines.len() < available_height {
            ann_lines.push(Line::from(vec![Span::styled(
                format!(" └{}┘", "─".repeat(border_width)),
                border_style_ann,
            )]));
        }

        let ann_para = Paragraph::new(ann_lines).style(Style::default().bg(bg));
        frame.render_widget(ann_para, overlay_area);
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
    stacked_mode: bool,
    stacked_commit: Option<&StackedCommitInfo>,
    stacked_index: usize,
    stacked_total: usize,
    vcs_name: &str,
    annotations: &[HunkAnnotation],
) {
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
                branch,
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
        return;
    }

    let side_by_side =
        compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
    let line_stats = compute_line_stats(&side_by_side);

    // Pre-compute highlights for the entire file to properly handle multi-line constructs
    // like JSDoc comments that span multiple lines
    let old_highlighter = FileHighlighter::new(&diff.old_content, &diff.filename);
    let new_highlighter = FileHighlighter::new(&diff.new_content, &diff.filename);

    let is_new_file = diff.old_content.is_empty() && !diff.new_content.is_empty();
    let is_deleted_file = !diff.old_content.is_empty() && diff.new_content.is_empty();

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

        // Check if there's an annotation for this file (hunk 0 for new files)
        let annotation = annotations
            .iter()
            .find(|a| a.file_index == current_file && a.hunk_index == 0);

        // Calculate how much space we need for annotation (shown at top)
        let annotation_height = annotation
            .map(|a| a.content.lines().count() + 2)
            .unwrap_or(0);

        // Reserve space for annotation at top
        let base_content_height = visible_height.saturating_sub(context_count);
        let content_height = base_content_height.saturating_sub(annotation_height);

        let visible_lines: Vec<&DiffLine> = side_by_side
            .iter()
            .skip(scroll_usize)
            .take(content_height)
            .collect();

        let mut new_lines: Vec<Line> = Vec::new();
        let mut annotation_overlays: Vec<(usize, &HunkAnnotation)> = Vec::new();

        if settings.context.enabled && context_count > 0 {
            render_context_lines(
                &new_context,
                context_count,
                &mut new_lines,
                &diff.filename,
                &new_highlighter,
                settings.tab_width,
            );
        }

        // For new files, show annotation at top (after context lines)
        if let Some(annotation) = annotation {
            let content_lines: Vec<&str> = annotation.content.lines().collect();
            let num_lines = content_lines.len() + 2;
            let annotation_start = new_lines.len();
            for _ in 0..num_lines {
                new_lines.push(Line::from(vec![Span::raw("")]));
            }
            annotation_overlays.push((annotation_start, annotation));
        }

        for (i, diff_line) in visible_lines.iter().enumerate() {
            let line_idx = scroll_usize + i;
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
                    Some(&new_highlighter),
                    Some(*num),
                    settings.tab_width,
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

        // Render annotation overlays
        render_annotation_overlays(frame, &annotation_overlays, main_area, bg, t);
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

        // Check if there's an annotation for this file (hunk 0 for deleted files)
        let annotation = annotations
            .iter()
            .find(|a| a.file_index == current_file && a.hunk_index == 0);

        // Calculate how much space we need for annotation (shown at top)
        let annotation_height = annotation
            .map(|a| a.content.lines().count() + 2)
            .unwrap_or(0);

        // Reserve space for annotation at top
        let base_content_height = visible_height.saturating_sub(context_count);
        let content_height = base_content_height.saturating_sub(annotation_height);

        let visible_lines: Vec<&DiffLine> = side_by_side
            .iter()
            .skip(scroll_usize)
            .take(content_height)
            .collect();

        let mut old_lines: Vec<Line> = Vec::new();
        let mut annotation_overlays: Vec<(usize, &HunkAnnotation)> = Vec::new();

        if settings.context.enabled && context_count > 0 {
            render_context_lines(
                &old_context,
                context_count,
                &mut old_lines,
                &diff.filename,
                &old_highlighter,
                settings.tab_width,
            );
        }

        // For deleted files, show annotation at top (after context lines)
        if let Some(annotation) = annotation {
            let content_lines: Vec<&str> = annotation.content.lines().collect();
            let num_lines = content_lines.len() + 2;
            let annotation_start = old_lines.len();
            for _ in 0..num_lines {
                old_lines.push(Line::from(vec![Span::raw("")]));
            }
            annotation_overlays.push((annotation_start, annotation));
        }

        for (i, diff_line) in visible_lines.iter().enumerate() {
            let line_idx = scroll_usize + i;
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
                    Some(&old_highlighter),
                    Some(*num),
                    settings.tab_width,
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

        // Render annotation overlays
        render_annotation_overlays(frame, &annotation_overlays, main_area, bg, t);
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
        let mut annotation_overlays: Vec<(usize, &HunkAnnotation)> = Vec::new();

        if settings.context.enabled && context_count > 0 {
            if old_area.is_some() {
                render_context_lines(
                    &old_context,
                    context_count,
                    &mut old_lines,
                    &diff.filename,
                    &old_highlighter,
                    settings.tab_width,
                );
            }
            if new_area.is_some() {
                render_context_lines(
                    &new_context,
                    context_count,
                    &mut new_lines,
                    &diff.filename,
                    &new_highlighter,
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

        // Find the hunk index for a given line, returns None if the line is not in a hunk
        let get_hunk_for_line = |line_idx: usize| -> Option<usize> {
            for (hunk_idx, &hunk_start) in hunks.iter().enumerate() {
                let hunk_end = hunks.get(hunk_idx + 1).copied().unwrap_or(usize::MAX);
                if line_idx >= hunk_start && line_idx < hunk_end {
                    return Some(hunk_idx);
                }
            }
            None
        };


        // Check if this line is the last changed line of a hunk (before Equal or end of visible area)
        let is_last_changed_line_of_hunk = |line_idx: usize, lines: &[&DiffLine]| -> Option<usize> {
            let current_idx_in_slice = line_idx.saturating_sub(scroll_usize);
            if current_idx_in_slice >= lines.len() {
                return None;
            }
            let current_line = lines[current_idx_in_slice];
            // Current line must be a change
            if matches!(current_line.change_type, ChangeType::Equal) {
                return None;
            }
            // Check next line
            let next_idx = current_idx_in_slice + 1;
            let is_last = if next_idx >= lines.len() {
                // End of visible lines - check if we're at the end of file or hunk
                true
            } else {
                matches!(lines[next_idx].change_type, ChangeType::Equal)
            };
            if is_last {
                get_hunk_for_line(line_idx)
            } else {
                None
            }
        };

        for (i, diff_line) in visible_lines.iter().enumerate() {
            let line_idx = scroll_usize + i;
            let in_focused = is_in_focused_hunk(line_idx, diff_line.change_type);
            let (old_bg, old_gutter_bg, old_gutter_fg, new_bg, new_gutter_bg, new_gutter_fg) =
                match diff_line.change_type {
                    ChangeType::Equal => (
                        Some(bg),               // old_bg: Set to theme background
                        Some(bg),               // old_gutter_bg
                        Some(t.ui.line_number), // old_gutter_fg
                        Some(bg),               // new_bg: Set to theme background
                        Some(bg),               // new_gutter_bg
                        Some(t.ui.line_number), // new_gutter_
                    ),
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
                    Some((num, _text)) => {
                        let prefix = format!("{:4} ", num);
                        old_spans.push(Span::styled(
                            prefix,
                            Style::default()
                                .fg(old_gutter_fg.unwrap_or(t.ui.line_number))
                                .bg(old_gutter_bg.unwrap_or(Color::Reset)),
                        ));
                        let matches = search_state.get_matches_for_line(line_idx, MatchPanel::Old);

                        // Use word-level rendering for modified lines if segments are available
                        if matches!(diff_line.change_type, ChangeType::Modified) {
                            if let Some(ref segments) = diff_line.old_segments {
                                let emphasis_ranges = segments_to_emphasis_ranges(segments);
                                old_spans.extend(apply_word_emphasis_highlight(
                                    _text,
                                    &diff.filename,
                                    old_bg,
                                    t.diff.deleted_word_bg,
                                    &emphasis_ranges,
                                    &matches,
                                    Some(&old_highlighter),
                                    Some(*num),
                                    settings.tab_width,
                                ));
                            } else {
                                old_spans.extend(apply_search_highlight(
                                    _text,
                                    &diff.filename,
                                    old_bg,
                                    &matches,
                                    Some(&old_highlighter),
                                    Some(*num),
                                    settings.tab_width,
                                ));
                            }
                        } else {
                            old_spans.extend(apply_search_highlight(
                                _text,
                                &diff.filename,
                                old_bg,
                                &matches,
                                Some(&old_highlighter),
                                Some(*num),
                                settings.tab_width,
                            ));
                        }
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
                    Some((num, _text)) => {
                        let prefix = format!("{:4} ", num);
                        new_spans.push(Span::styled(
                            prefix,
                            Style::default()
                                .fg(new_gutter_fg.unwrap_or(t.ui.line_number))
                                .bg(new_gutter_bg.unwrap_or(Color::Reset)),
                        ));
                        let matches = search_state.get_matches_for_line(line_idx, MatchPanel::New);

                        // Use word-level rendering for modified lines if segments are available
                        if matches!(diff_line.change_type, ChangeType::Modified) {
                            if let Some(ref segments) = diff_line.new_segments {
                                let emphasis_ranges = segments_to_emphasis_ranges(segments);
                                new_spans.extend(apply_word_emphasis_highlight(
                                    _text,
                                    &diff.filename,
                                    new_bg,
                                    t.diff.added_word_bg,
                                    &emphasis_ranges,
                                    &matches,
                                    Some(&new_highlighter),
                                    Some(*num),
                                    settings.tab_width,
                                ));
                            } else {
                                new_spans.extend(apply_search_highlight(
                                    _text,
                                    &diff.filename,
                                    new_bg,
                                    &matches,
                                    Some(&new_highlighter),
                                    Some(*num),
                                    settings.tab_width,
                                ));
                            }
                        } else {
                            new_spans.extend(apply_search_highlight(
                                _text,
                                &diff.filename,
                                new_bg,
                                &matches,
                                Some(&new_highlighter),
                                Some(*num),
                                settings.tab_width,
                            ));
                        }
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

            // Check if we need to add annotation content rows after this line
            if let Some(hunk_idx) = is_last_changed_line_of_hunk(line_idx, &visible_lines) {
                if let Some(annotation) = annotations
                    .iter()
                    .find(|a| a.file_index == current_file && a.hunk_index == hunk_idx)
                {
                    // Add annotation content - track position for overlay rendering
                    let content_lines: Vec<&str> = annotation.content.lines().collect();
                    let num_lines = content_lines.len() + 2; // +2 for top and bottom borders

                    // Use the rendered panel's line count for position
                    let line_pos = if old_area.is_some() {
                        old_lines.len()
                    } else {
                        new_lines.len()
                    };

                    // Add placeholder lines to both panels to make space
                    for _ in 0..num_lines {
                        if old_area.is_some() {
                            old_lines.push(Line::from(vec![Span::raw("")]));
                        }
                        if new_area.is_some() {
                            new_lines.push(Line::from(vec![Span::raw("")]));
                        }
                    }

                    annotation_overlays.push((line_pos, annotation));
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

        // Render annotation overlays spanning both panels (or single panel when fullscreened)
        let render_area = old_area.or(new_area).unwrap_or(main_area);
        let content_start_y = render_area.y + 1;
        let content_x = render_area.x + 1;
        let content_width = if old_area.is_some() && new_area.is_some() {
            old_area.unwrap().width + new_area.unwrap().width - 2 // -2 for shared border
        } else {
            render_area.width.saturating_sub(2)
        };

        for (line_pos, annotation) in annotation_overlays {
            let screen_y = content_start_y + line_pos as u16;
            let content_lines: Vec<&str> = annotation.content.lines().collect();
            let num_lines = content_lines.len() + 2; // +2 for top and bottom borders

            // Check if annotation is visible
            if screen_y >= main_area.y + main_area.height {
                continue;
            }

            let available_height = (main_area.y + main_area.height).saturating_sub(screen_y) as usize;
            if available_height == 0 {
                continue;
            }

            let overlay_height = num_lines.min(available_height) as u16;
            let overlay_area = Rect::new(content_x, screen_y, content_width, overlay_height);

            // Clear the area first
            frame.render_widget(ratatui::widgets::Clear, overlay_area);

            // Build annotation lines
            let mut ann_lines: Vec<Line> = Vec::new();
            let note_style = Style::default().fg(t.ui.text_muted).italic();
            let border_style_ann = Style::default().fg(t.ui.border_unfocused);
            let border_width = content_width.saturating_sub(3) as usize;

            // Add top border
            ann_lines.push(Line::from(vec![
                Span::styled(format!(" ┌{}┐", "─".repeat(border_width)), border_style_ann),
            ]));

            // Add content lines
            for content_line in content_lines.iter().take(available_height.saturating_sub(2)) {
                // Pad content to fill width, leaving room for borders
                let content_width_inner = border_width.saturating_sub(1);
                let padded_content = format!("{:<width$}", content_line, width = content_width_inner);
                ann_lines.push(Line::from(vec![
                    Span::styled(" │ ", border_style_ann),
                    Span::styled(padded_content, note_style),
                    Span::styled("│", border_style_ann),
                ]));
            }

            // Add bottom border if there's room
            if ann_lines.len() < available_height {
                ann_lines.push(Line::from(vec![
                    Span::styled(format!(" └{}┘", "─".repeat(border_width)), border_style_ann),
                ]));
            }

            let ann_para = Paragraph::new(ann_lines).style(Style::default().bg(bg));
            frame.render_widget(ann_para, overlay_area);
        }
    }

    render_footer(
        frame,
        footer_area,
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
