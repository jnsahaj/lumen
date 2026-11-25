use std::collections::HashSet;

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::diff::context::compute_context_lines;
use crate::diff::highlight::FileHighlighter;
use crate::diff::{
    hunk_comment_lines, ChangeType, DiffFullscreen, DiffPanelFocus, FileDiff, InlineSegment, PrInfo,
};
use crate::review::AnnotationTarget;
use crate::search::MatchPanel;
use crate::ui::coordinates::{PanelLayout, ThreadHitRect};
use crate::ui::editor::AnnotationEditor;
use crate::ui::state::{AppState, FocusedPanel};
use crate::ui::theme;
use crate::ui::views::diff_lines::{
    apply_selection_to_spans, blank_diff_line, blend_with_selection, generate_stripe_pattern,
    get_selection_range_for_line, hidden_lines_label, highlight_diff_line, make_indicator_span,
    pad_line_bg, push_wrapped_diff_line, render_context_lines, segments_to_emphasis_ranges,
    separator_line, top_line_position, DiffLineStyle, LineHighlightParams,
};
use crate::ui::views::footer::{render_footer, FooterData, ReviewStatus};
use crate::ui::views::header::render_stacked_header;
use crate::ui::views::overlays::{
    build_slots, compute_target_index_ranges, file_slots_height, is_in_ann_range,
    render_overlay_slot, OverlayRenderTarget, OverlaySlot, SlotBuildCtx,
};
use crate::ui::views::sidebar::render_sidebar;

pub struct DiffRenderInput<'a> {
    pub state: &'a AppState,
    pub commit_ref: &'a str,
    pub pr_info: Option<&'a PrInfo>,
    pub old_highlighter: &'a FileHighlighter,
    pub new_highlighter: &'a FileHighlighter,
    pub editor: Option<&'a AnnotationEditor<'a>>,
    pub review: &'a ReviewStatus<'a>,
}

#[derive(Default)]
pub struct DiffRenderOutput {
    pub content_row_offset: usize,
    pub overlay_gaps: Vec<(usize, usize)>,
    pub annotation_rects: Vec<ThreadHitRect>,
    pub editor_rect: Option<Rect>,
}

pub fn render_empty_state(frame: &mut Frame, watching: bool, repository_name: Option<&str>) {
    let watch_hint = if watching {
        " (watching for changes...)"
    } else {
        ""
    };
    let title = match repository_name {
        Some(name) => format!(" Git Review · {name} "),
        None => " Git Review ".to_string(),
    };
    let msg = Paragraph::new(format!("No changes detected.{watch_hint}"))
        .block(Block::default().title(title).borders(Borders::ALL));
    frame.render_widget(msg, frame.area());
}

pub fn render_selection_tooltip(
    frame: &mut Frame,
    state: &AppState,
    content_row_offset: usize,
    gaps: &[(usize, usize)],
) {
    let t = theme::get();
    let term = frame.area();
    let header_h: u16 = u16::from(state.stacked_mode);
    let sidebar_w: u16 = if state.show_sidebar {
        (term.width / 4).clamp(20, 35)
    } else {
        0
    };
    let effective_fullscreen = state
        .file_diffs
        .get(state.current_file)
        .map_or(state.diff_fullscreen, |d| {
            d.effective_fullscreen(state.diff_fullscreen)
        });
    let layout = PanelLayout::calculate(
        term.width,
        sidebar_w,
        state.show_sidebar,
        effective_fullscreen,
    );

    let sel = &state.selection;
    let (_, sel_end) = sel.normalized_range();
    let scroll_usize = state.scroll as usize;
    if sel_end.line < scroll_usize {
        return;
    }

    let content_y = sel_end.line - scroll_usize;
    let cum_gaps: u16 = gaps
        .iter()
        .filter(|&&(after_line, _)| after_line < content_y)
        .map(|&(_, gap_h)| gap_h as u16)
        .sum();
    let screen_y = header_h + 1 + content_row_offset as u16 + content_y as u16 + cum_gaps + 1;

    let (panel_x, panel_w) = match sel.panel {
        DiffPanelFocus::Old => (layout.old_panel_x, layout.old_panel_width),
        DiffPanelFocus::New => (layout.new_panel_x, layout.new_panel_width),
        DiffPanelFocus::None => (0, 0),
    };
    if panel_w == 0 || screen_y >= term.height.saturating_sub(1) {
        return;
    }

    let tip_w: u16 = 27;
    let cx = layout.content_x_offset(sel.panel);
    let tip_x = (panel_x + cx).min(panel_x + panel_w.saturating_sub(tip_w + 1));
    let tip_area = Rect::new(tip_x, screen_y, tip_w.min(panel_w), 1);

    let bg = t.ui.footer_branch_bg;
    let key_style = Style::default().fg(t.ui.text_primary).bg(bg).bold();
    let desc_style = Style::default().fg(t.ui.text_muted).bg(bg);
    let tip_line = Line::from(vec![
        Span::styled(" i", key_style),
        Span::styled(" annotate ", desc_style),
        Span::styled("y", key_style),
        Span::styled(" copy ", desc_style),
        Span::styled("esc", key_style),
        Span::styled("   ", desc_style),
    ]);

    frame.render_widget(Clear, tip_area);
    frame.render_widget(
        Paragraph::new(tip_line).style(Style::default().bg(bg)),
        tip_area,
    );
}

pub fn render_diff(frame: &mut Frame, input: &DiffRenderInput<'_>) -> DiffRenderOutput {
    let state = input.state;
    let diff = &state.file_diffs[state.current_file];
    let area = frame.area();
    let t = theme::get();
    let h_scroll = if state.settings.wrap {
        0
    } else {
        state.h_scroll
    };

    let (content_area, footer_area) = if state.stacked_mode {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);
        render_stacked_header(
            frame,
            chunks[0],
            state.current_commit(),
            state.current_commit_index,
            state.stacked_commits.len(),
            state.vcs_name,
        );
        (chunks[1], chunks[2])
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        (chunks[0], chunks[1])
    };

    let main_area = if state.show_sidebar {
        let sidebar_width = (area.width / 4).clamp(20, 35);
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(sidebar_width), Constraint::Min(0)])
            .split(content_area);
        render_sidebar(frame, main_chunks[0], state);
        main_chunks[1]
    } else {
        content_area
    };

    let border_style = Style::default().fg(t.ui.border_unfocused);
    let title_style = if state.focused_panel == FocusedPanel::DiffView {
        Style::default().fg(t.ui.border_focused)
    } else {
        Style::default().fg(t.ui.border_unfocused)
    };

    let empty_viewed_hunks = HashSet::new();
    let viewed_hunks = state
        .viewed_hunks
        .get(&diff.filename)
        .unwrap_or(&empty_viewed_hunks);

    let output = if diff.is_binary {
        let message = Line::from(vec![Span::styled(
            "Binary file - not displayed",
            Style::default().fg(t.ui.text_muted),
        )]);
        let para = Paragraph::new(vec![message])
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title(Line::styled(format!(" {} ", diff.filename), title_style))
                    .borders(Borders::ALL)
                    .border_style(border_style),
            );
        frame.render_widget(para, main_area);
        DiffRenderOutput::default()
    } else if diff.is_new_file() {
        render_single_panel(
            frame,
            main_area,
            input,
            diff,
            viewed_hunks,
            SinglePanelKind::Added,
            h_scroll,
            title_style,
            border_style,
        )
    } else if diff.is_deleted_file() {
        render_single_panel(
            frame,
            main_area,
            input,
            diff,
            viewed_hunks,
            SinglePanelKind::Deleted,
            h_scroll,
            title_style,
            border_style,
        )
    } else {
        render_side_by_side(
            frame,
            main_area,
            input,
            diff,
            viewed_hunks,
            h_scroll,
            title_style,
            border_style,
        )
    };

    if state.show_sidebar {
        draw_sidebar_junctions(frame, main_area, t.ui.border_unfocused);
    }

    let line_stats = if diff.is_binary {
        crate::diff::LineStats::default()
    } else {
        state.line_stats()
    };
    render_footer(
        frame,
        footer_area,
        FooterData {
            filename: &diff.filename,
            commit_ref: input.commit_ref,
            pr_info: input.pr_info,
            watching: state.watching,
            current_file: state.current_file,
            viewed_files: &state.viewed_files,
            line_stats_added: line_stats.added,
            line_stats_removed: line_stats.removed,
            hunk_count: if diff.is_binary {
                0
            } else {
                state.hunks_ref().len()
            },
            focused_hunk: if diff.is_binary {
                None
            } else {
                state.focused_hunk
            },
            search_state: &state.search_state,
            area_width: area.width,
            review: input.review,
        },
    );

    output
}

fn draw_sidebar_junctions(frame: &mut Frame, main_area: Rect, border_color: Color) {
    let junction_x = main_area.x;
    let top_y = main_area.y;
    let bottom_y = main_area.y + main_area.height.saturating_sub(1);
    let buf = frame.buffer_mut();
    if let Some(cell) = buf.cell_mut(Position::new(junction_x, top_y)) {
        cell.set_char('┬');
        cell.set_fg(border_color);
    }
    if bottom_y > top_y {
        if let Some(cell) = buf.cell_mut(Position::new(junction_x, bottom_y)) {
            cell.set_char('┴');
            cell.set_fg(border_color);
        }
    }
}

#[derive(Clone, Copy)]
enum SinglePanelKind {
    Added,
    Deleted,
}

#[allow(clippy::too_many_arguments)]
fn render_single_panel(
    frame: &mut Frame,
    main_area: Rect,
    input: &DiffRenderInput<'_>,
    diff: &FileDiff,
    viewed_hunks: &HashSet<usize>,
    kind: SinglePanelKind,
    h_scroll: u16,
    title_style: Style,
    border_style: Style,
) -> DiffRenderOutput {
    let state = input.state;
    let t = theme::get();
    let bg = t.ui.bg;
    let side_by_side = state.side_by_side_ref();
    let hunks = state.hunks_ref();

    let (
        title,
        panel_focus,
        match_panel,
        source,
        tree_map,
        highlighter,
        line_color_bg,
        gutter_bg_base,
        gutter_fg,
    ) = match kind {
        SinglePanelKind::Added => (
            " [2] New File ",
            DiffPanelFocus::New,
            MatchPanel::New,
            &diff.new_content,
            &state.cached_trees.new_file_trees,
            input.new_highlighter,
            t.diff.added_bg,
            t.diff.added_gutter_bg,
            t.diff.added_gutter_fg,
        ),
        SinglePanelKind::Deleted => (
            " [2] Deleted File ",
            DiffPanelFocus::Old,
            MatchPanel::Old,
            &diff.old_content,
            &state.cached_trees.old_file_trees,
            input.old_highlighter,
            t.diff.deleted_bg,
            t.diff.deleted_gutter_bg,
            t.diff.deleted_gutter_fg,
        ),
    };

    let visible_height = main_area.height.saturating_sub(2) as usize;
    let scroll_usize = state.scroll as usize;
    let context = compute_context_lines(
        source,
        &diff.filename,
        tree_map,
        top_line_position(side_by_side, scroll_usize, panel_focus),
        &state.settings.context,
        state.settings.tab_width,
    );
    let context_count = context.len();

    let slot_width = |_: &AnnotationTarget| main_area.width.saturating_sub(2);
    let file_ctx = SlotBuildCtx {
        filename: &diff.filename,
        file_level: true,
        visible_height,
        tab_width: state.settings.tab_width,
        theme: t,
    };
    let line_ctx = SlotBuildCtx {
        file_level: false,
        ..file_ctx
    };
    let file_slots = build_slots(&state.threads, input.editor, &file_ctx, &slot_width);
    let line_slots = build_slots(&state.threads, input.editor, &line_ctx, &slot_width);

    let annotation_height = file_slots_height(&file_slots);
    let content_row_offset = context_count + annotation_height;
    let content_height = visible_height
        .saturating_sub(context_count)
        .saturating_sub(annotation_height);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut slot_overlays: Vec<(usize, &OverlaySlot)> = Vec::new();
    let mut overlay_gaps: Vec<(usize, usize)> = Vec::new();

    if state.settings.context.enabled && context_count > 0 {
        render_context_lines(
            &context,
            context_count,
            &mut lines,
            &diff.filename,
            highlighter,
            state.settings.tab_width,
        );
    }

    for slot in &file_slots {
        let slot_start = lines.len();
        for _ in 0..slot.height() {
            lines.push(Line::from(vec![Span::raw("")]));
        }
        slot_overlays.push((slot_start, slot));
    }

    let ann_index_ranges = compute_target_index_ranges(&line_slots, side_by_side);
    let annotation_indicator_style = Style::default().fg(t.ui.highlight);
    let focus_style = Style::default().fg(t.ui.border_focused);
    let row_target_width = (main_area.width as usize).saturating_sub(2) + h_scroll as usize;

    for (i, diff_line) in side_by_side
        .iter()
        .skip(scroll_usize)
        .take(content_height)
        .enumerate()
    {
        let line_idx = scroll_usize + i;
        let selection_range = get_selection_range_for_line(line_idx, panel_focus, &state.selection);
        let in_annotation = is_in_ann_range(line_idx, None, &ann_index_ranges);
        let line_selected = selection_range.is_some_and(|(s, e)| s == 0 && e == usize::MAX);
        let hunk_viewed = !matches!(diff_line.change_type, ChangeType::Equal)
            && hunks
                .iter()
                .rposition(|&h| h <= line_idx)
                .is_some_and(|idx| viewed_hunks.contains(&idx));

        let side_line = match kind {
            SinglePanelKind::Added => diff_line.new_line.as_ref(),
            SinglePanelKind::Deleted => diff_line.old_line.as_ref(),
        };
        if let Some((num, text)) = side_line {
            let gutter_bg = if line_selected {
                blend_with_selection(gutter_bg_base)
            } else {
                gutter_bg_base
            };
            let prefix_spans = vec![
                make_indicator_span(
                    false,
                    in_annotation,
                    line_selected,
                    bg,
                    focus_style,
                    annotation_indicator_style,
                ),
                Span::styled(
                    format!("{num:4} "),
                    Style::default().fg(gutter_fg).bg(gutter_bg),
                ),
            ];
            let matches = state
                .search_state
                .get_matches_for_line(line_idx, match_panel);
            let line_bg = if hunk_viewed { bg } else { line_color_bg };
            let content_spans = highlight_diff_line(&LineHighlightParams {
                text,
                filename: &diff.filename,
                line_bg: Some(line_bg),
                word_emphasis: None,
                search_ranges: &matches,
                highlighter: Some(highlighter),
                line_number: Some(*num),
                tab_width: state.settings.tab_width,
            });
            let content_spans = apply_selection_to_spans(content_spans, selection_range, line_bg);
            push_wrapped_diff_line(
                &mut lines,
                prefix_spans,
                content_spans,
                row_target_width,
                line_bg,
                state.settings.wrap,
            );
        }

        for slot in &line_slots {
            if let AnnotationTarget::LineRange {
                panel, end_line, ..
            } = slot.target()
            {
                if diff_line.line_number(*panel) == Some(*end_line) {
                    let num_ann_lines = slot.height();
                    let line_pos = lines.len();
                    overlay_gaps.push((i, num_ann_lines));
                    for _ in 0..num_ann_lines {
                        lines.push(Line::from(vec![Span::raw("")]));
                    }
                    slot_overlays.push((line_pos, slot));
                }
            }
        }
    }

    let para = Paragraph::new(lines).scroll((0, h_scroll)).block(
        Block::default()
            .title(Line::styled(title, title_style))
            .borders(Borders::ALL)
            .border_style(border_style),
    );
    frame.render_widget(para, main_area);

    let mut annotation_rects = Vec::new();
    let mut editor_rect = None;
    let target = OverlayRenderTarget {
        x: main_area.x + 1,
        start_y: main_area.y + 1,
        width: main_area.width.saturating_sub(2),
        max_area: main_area,
        suppress_gutter: false,
    };
    for &(line_pos, slot) in &slot_overlays {
        render_overlay_slot(
            frame,
            line_pos,
            slot,
            &target,
            t,
            &mut annotation_rects,
            &mut editor_rect,
        );
    }

    DiffRenderOutput {
        content_row_offset,
        overlay_gaps,
        annotation_rects,
        editor_rect,
    }
}

struct PanelRowParams<'a> {
    show_indicator: bool,
    focus_bar: bool,
    in_annotation: bool,
    line_selected: bool,
    selection_range: Option<(usize, usize)>,
    line: Option<&'a (usize, String)>,
    segments: Option<&'a Vec<InlineSegment>>,
    change_type: ChangeType,
    hunk_viewed: bool,
    line_bg: Option<Color>,
    gutter_bg: Option<Color>,
    gutter_fg: Option<Color>,
    word_bg: Color,
    match_panel: MatchPanel,
    line_idx: usize,
    highlighter: &'a FileHighlighter,
    row_target_width: usize,
}

fn render_panel_row(
    out: &mut Vec<Line<'static>>,
    params: &PanelRowParams<'_>,
    filename: &str,
    state: &AppState,
    t: &theme::Theme,
) {
    let bg = t.ui.bg;
    let focus_style = Style::default().fg(t.ui.border_focused);
    let annotation_style = Style::default().fg(t.ui.highlight);

    let mut prefix_spans: Vec<Span<'static>> = Vec::new();
    if params.show_indicator {
        prefix_spans.push(make_indicator_span(
            params.focus_bar,
            params.in_annotation,
            params.line_selected,
            bg,
            focus_style,
            annotation_style,
        ));
    }

    let Some((num, text)) = params.line else {
        let current_len: usize = prefix_spans.iter().map(|s| s.content.chars().count()).sum();
        let stripe_width = params.row_target_width.saturating_sub(current_len);
        prefix_spans.push(Span::styled(
            generate_stripe_pattern(stripe_width),
            Style::default().fg(t.diff.empty_placeholder_fg),
        ));
        let padded = pad_line_bg(
            prefix_spans,
            params.row_target_width,
            params.line_bg.unwrap_or(bg),
        );
        out.push(Line::from(padded));
        return;
    };

    let raw_gutter_bg = params.gutter_bg.unwrap_or(Color::Reset);
    let gutter_bg = if params.line_selected {
        blend_with_selection(if raw_gutter_bg == Color::Reset {
            bg
        } else {
            raw_gutter_bg
        })
    } else {
        raw_gutter_bg
    };
    prefix_spans.push(Span::styled(
        format!("{num:4} "),
        Style::default()
            .fg(params.gutter_fg.unwrap_or(t.ui.line_number))
            .bg(gutter_bg),
    ));

    let matches = state
        .search_state
        .get_matches_for_line(params.line_idx, params.match_panel);
    let emphasis_storage = (matches!(params.change_type, ChangeType::Modified)
        && !params.hunk_viewed)
        .then_some(params.segments)
        .flatten()
        .map(|segs| segments_to_emphasis_ranges(segs));
    let word_emphasis = emphasis_storage
        .as_deref()
        .map(|ranges| (params.word_bg, ranges));

    let content_spans = highlight_diff_line(&LineHighlightParams {
        text,
        filename,
        line_bg: params.line_bg,
        word_emphasis,
        search_ranges: &matches,
        highlighter: Some(params.highlighter),
        line_number: Some(*num),
        tab_width: state.settings.tab_width,
    });
    let content_spans = apply_selection_to_spans(
        content_spans,
        params.selection_range,
        params.line_bg.unwrap_or(bg),
    );
    push_wrapped_diff_line(
        out,
        prefix_spans,
        content_spans,
        params.row_target_width,
        params.line_bg.unwrap_or(bg),
        state.settings.wrap,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_side_by_side(
    frame: &mut Frame,
    main_area: Rect,
    input: &DiffRenderInput<'_>,
    diff: &FileDiff,
    viewed_hunks: &HashSet<usize>,
    h_scroll: u16,
    title_style: Style,
    border_style: Style,
) -> DiffRenderOutput {
    let state = input.state;
    let t = theme::get();
    let bg = t.ui.bg;
    let side_by_side = state.side_by_side_ref();
    let hunks = state.hunks_ref();

    let (old_area, new_area) = match state.diff_fullscreen {
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

    let scroll_usize = state.scroll as usize;
    let old_context = compute_context_lines(
        &diff.old_content,
        &diff.filename,
        &state.cached_trees.old_file_trees,
        top_line_position(side_by_side, scroll_usize, DiffPanelFocus::Old),
        &state.settings.context,
        state.settings.tab_width,
    );
    let new_context = compute_context_lines(
        &diff.new_content,
        &diff.filename,
        &state.cached_trees.new_file_trees,
        top_line_position(side_by_side, scroll_usize, DiffPanelFocus::New),
        &state.settings.context,
        state.settings.tab_width,
    );
    let context_count = old_context.len().max(new_context.len());

    let reference_area = old_area.or(new_area).unwrap_or(main_area);
    let visible_height = reference_area.height.saturating_sub(2) as usize;
    let content_height = visible_height.saturating_sub(context_count);

    let is_side_by_side = old_area.is_some() && new_area.is_some();
    let slot_geometry = |target: &AnnotationTarget| -> OverlayRenderTarget {
        match target {
            AnnotationTarget::File => {
                let render_area = old_area.or(new_area).unwrap_or(main_area);
                let width = if is_side_by_side {
                    old_area
                        .unwrap()
                        .width
                        .saturating_add(new_area.unwrap().width)
                        .saturating_sub(2)
                } else {
                    render_area.width.saturating_sub(2)
                };
                OverlayRenderTarget {
                    x: render_area.x + 1,
                    start_y: render_area.y + 1,
                    width,
                    max_area: main_area,
                    suppress_gutter: false,
                }
            }
            AnnotationTarget::LineRange { panel, .. } => {
                let is_new_panel = matches!(panel, DiffPanelFocus::New | DiffPanelFocus::None);
                let panel_area = match panel {
                    DiffPanelFocus::Old => old_area.unwrap_or(main_area),
                    _ => new_area.unwrap_or(main_area),
                };
                OverlayRenderTarget {
                    x: panel_area.x + 1,
                    start_y: panel_area.y + 1,
                    width: panel_area.width.saturating_sub(2),
                    max_area: main_area,
                    suppress_gutter: is_new_panel && is_side_by_side,
                }
            }
        }
    };

    let slot_width = |target: &AnnotationTarget| slot_geometry(target).width;
    let file_ctx = SlotBuildCtx {
        filename: &diff.filename,
        file_level: true,
        visible_height,
        tab_width: state.settings.tab_width,
        theme: t,
    };
    let line_ctx = SlotBuildCtx {
        file_level: false,
        ..file_ctx
    };
    let file_slots = build_slots(&state.threads, input.editor, &file_ctx, &slot_width);
    let line_slots = build_slots(&state.threads, input.editor, &line_ctx, &slot_width);

    let content_row_offset = context_count + file_slots_height(&file_slots);
    let mut old_lines: Vec<Line<'static>> = Vec::new();
    let mut new_lines: Vec<Line<'static>> = Vec::new();
    let mut slot_overlays: Vec<(usize, &OverlaySlot)> = Vec::new();
    let mut overlay_gaps: Vec<(usize, usize)> = Vec::new();

    if state.settings.context.enabled && context_count > 0 {
        if old_area.is_some() {
            render_context_lines(
                &old_context,
                context_count,
                &mut old_lines,
                &diff.filename,
                input.old_highlighter,
                state.settings.tab_width,
            );
        }
        if new_area.is_some() {
            render_context_lines(
                &new_context,
                context_count,
                &mut new_lines,
                &diff.filename,
                input.new_highlighter,
                state.settings.tab_width,
            );
        }
    }

    for slot in &file_slots {
        let slot_start = old_lines.len().max(new_lines.len());
        for _ in 0..slot.height() {
            if old_area.is_some() {
                old_lines.push(Line::from(vec![Span::raw("")]));
            }
            if new_area.is_some() {
                new_lines.push(Line::from(vec![Span::raw("")]));
            }
        }
        slot_overlays.push((slot_start, slot));
    }

    let ann_index_ranges = compute_target_index_ranges(&line_slots, side_by_side);
    let commented_side = state
        .focused_hunk
        .and_then(|hunk_idx| hunks.get(hunk_idx))
        .and_then(|&hunk_start| hunk_comment_lines(side_by_side, hunk_start, state.diff_fullscreen))
        .map(|(panel, _, _)| panel);

    let mut border_marker_rows: Vec<usize> = Vec::new();
    let mut border_focus_rows: Vec<usize> = Vec::new();

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

    let is_in_focused_hunk = |line_idx: usize, change_type: ChangeType| -> bool {
        if matches!(change_type, ChangeType::Equal) {
            return false;
        }
        if let Some(hunk_idx) = state.focused_hunk {
            if let Some(&hunk_start) = hunks.get(hunk_idx) {
                let hunk_end = hunks.get(hunk_idx + 1).copied().unwrap_or(usize::MAX);
                return line_idx >= hunk_start && line_idx < hunk_end;
            }
        }
        false
    };

    for (i, diff_line) in side_by_side
        .iter()
        .skip(scroll_usize)
        .take(content_height)
        .enumerate()
    {
        let line_idx = scroll_usize + i;
        if diff_line.hidden_lines > 0 {
            let label = hidden_lines_label(diff_line.hidden_lines);
            if old_area.is_some() {
                old_lines.push(separator_line(&label, old_row_target_width, t));
            }
            if new_area.is_some() {
                let label = if old_area.is_some() {
                    ""
                } else {
                    label.as_str()
                };
                new_lines.push(separator_line(label, new_row_target_width, t));
            }
            continue;
        }

        let in_focused = is_in_focused_hunk(line_idx, diff_line.change_type);
        let hunk_viewed = !matches!(diff_line.change_type, ChangeType::Equal)
            && hunks
                .iter()
                .rposition(|&h| h <= line_idx)
                .is_some_and(|idx| viewed_hunks.contains(&idx));
        let mut style = DiffLineStyle::for_change_type(diff_line.change_type, bg, t);
        if hunk_viewed {
            if style.old_bg.is_some() {
                style.old_bg = Some(bg);
            }
            if style.new_bg.is_some() {
                style.new_bg = Some(bg);
            }
        }

        let old_selection_range =
            get_selection_range_for_line(line_idx, DiffPanelFocus::Old, &state.selection);
        let new_selection_range =
            get_selection_range_for_line(line_idx, DiffPanelFocus::New, &state.selection);
        let old_in_annotation =
            is_in_ann_range(line_idx, Some(DiffPanelFocus::Old), &ann_index_ranges);
        let new_in_annotation =
            is_in_ann_range(line_idx, Some(DiffPanelFocus::New), &ann_index_ranges);

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

        let old_start_len = old_lines.len();
        let new_start_len = new_lines.len();

        if old_area.is_some() {
            render_panel_row(
                &mut old_lines,
                &PanelRowParams {
                    show_indicator: true,
                    focus_bar: focus_on_old,
                    in_annotation: old_in_annotation,
                    line_selected: old_selection_range
                        .is_some_and(|(s, e)| s == 0 && e == usize::MAX),
                    selection_range: old_selection_range,
                    line: diff_line.old_line.as_ref(),
                    segments: diff_line.old_segments.as_ref(),
                    change_type: diff_line.change_type,
                    hunk_viewed,
                    line_bg: style.old_bg,
                    gutter_bg: style.old_gutter_bg,
                    gutter_fg: style.old_gutter_fg,
                    word_bg: t.diff.deleted_word_bg,
                    match_panel: MatchPanel::Old,
                    line_idx,
                    highlighter: input.old_highlighter,
                    row_target_width: old_row_target_width,
                },
                &diff.filename,
                state,
                t,
            );
        }

        if new_area.is_some() {
            render_panel_row(
                &mut new_lines,
                &PanelRowParams {
                    show_indicator: old_area.is_none(),
                    focus_bar: focus_on_new,
                    in_annotation: new_in_annotation,
                    line_selected: new_selection_range
                        .is_some_and(|(s, e)| s == 0 && e == usize::MAX),
                    selection_range: new_selection_range,
                    line: diff_line.new_line.as_ref(),
                    segments: diff_line.new_segments.as_ref(),
                    change_type: diff_line.change_type,
                    hunk_viewed,
                    line_bg: style.new_bg,
                    gutter_bg: style.new_gutter_bg,
                    gutter_fg: style.new_gutter_fg,
                    word_bg: t.diff.added_word_bg,
                    match_panel: MatchPanel::New,
                    line_idx,
                    highlighter: input.new_highlighter,
                    row_target_width: new_row_target_width,
                },
                &diff.filename,
                state,
                t,
            );
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

        let screen_lines = old_start_len..old_start_len + row_count;
        if new_in_annotation {
            border_marker_rows.extend(screen_lines.clone());
        }
        if focus_on_new {
            border_focus_rows.extend(screen_lines);
        }

        for slot in &line_slots {
            if let AnnotationTarget::LineRange {
                panel, end_line, ..
            } = slot.target()
            {
                if diff_line.line_number(*panel) == Some(*end_line) {
                    let num_lines = slot.height();
                    let line_pos = if old_area.is_some() {
                        old_lines.len()
                    } else {
                        new_lines.len()
                    };
                    overlay_gaps.push((i, num_lines));
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

    // Paint new-panel markers and focus bar onto the shared border in side-by-side mode.
    if let (Some(old_a), Some(_)) = (old_area, new_area) {
        if old_a.width >= 2 && old_a.height >= 2 {
            let border_x = old_a.x + old_a.width - 1;
            let content_start = old_a.y + 1;
            let buf = frame.buffer_mut();

            let markers = border_marker_rows.iter().map(|&row| (row, t.ui.highlight));
            let focus_bar = border_focus_rows
                .iter()
                .map(|&row| (row, t.ui.border_focused));
            for (para_row, color) in markers.chain(focus_bar) {
                let screen_row = content_start.saturating_add(para_row as u16);
                if screen_row < old_a.y + old_a.height - 1 {
                    if let Some(cell) = buf.cell_mut(Position::new(border_x, screen_row)) {
                        cell.set_fg(color);
                        cell.set_char('▎');
                    }
                }
            }
        }
    }

    let mut annotation_rects = Vec::new();
    let mut editor_rect = None;
    for &(line_pos, slot) in &slot_overlays {
        let target = slot_geometry(slot.target());
        render_overlay_slot(
            frame,
            line_pos,
            slot,
            &target,
            t,
            &mut annotation_rects,
            &mut editor_rect,
        );
    }

    if let (Some(old_a), Some(_)) = (old_area, new_area) {
        if old_a.width >= 2 && old_a.height >= 2 {
            let border_x = old_a.x + old_a.width - 1;
            let content_start = old_a.y + 1;
            let buf = frame.buffer_mut();

            for &(line_pos, slot) in &slot_overlays {
                if let AnnotationTarget::LineRange { panel, .. } = slot.target() {
                    if matches!(panel, DiffPanelFocus::New | DiffPanelFocus::None) {
                        for row_offset in 0..slot.height() {
                            let screen_row = content_start
                                .saturating_add(line_pos as u16)
                                .saturating_add(row_offset as u16);
                            if screen_row < old_a.y + old_a.height - 1 {
                                if let Some(cell) =
                                    buf.cell_mut(Position::new(border_x, screen_row))
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

    DiffRenderOutput {
        content_row_offset,
        overlay_gaps,
        annotation_rects,
        editor_rect,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;
    use crate::diff::FileStatus;
    use crate::review::Comment;

    fn render_state(state: &mut AppState, width: u16, height: u16) -> (String, DiffRenderOutput) {
        state.update_search_matches();
        state.get_highlighters();
        let (old_hl, new_hl) = state.highlighters_ref().unwrap();
        let review = ReviewStatus {
            agent: "pi idle",
            queued_comments: state.queued_count(),
            notice: None,
        };
        let mut output = DiffRenderOutput::default();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                output = render_diff(
                    frame,
                    &DiffRenderInput {
                        state,
                        commit_ref: "main",
                        pr_info: None,
                        old_highlighter: old_hl,
                        new_highlighter: new_hl,
                        editor: None,
                        review: &review,
                    },
                );
            })
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        (screen, output)
    }

    #[test]
    fn test_render_diff_side_by_side_hunks_only_and_inline_thread_hit_rect() {
        let old: String = (1..=30).map(|n| format!("line {n}\n")).collect();
        let new = old.replace("line 15\n", "line 15 café updated\n");
        let mut state = AppState::new(
            vec![FileDiff::from_contents("src/lib.rs".into(), old, new)],
            None,
        );
        let (thread_id, _) = state.add_thread(
            "src/lib.rs".into(),
            AnnotationTarget::LineRange {
                panel: DiffPanelFocus::New,
                start_line: 15,
                end_line: 15,
            },
            "check unicode café handling".into(),
        );
        state.search_state.query = "café".to_string();
        state.mark_search_dirty();

        let (screen, output) = render_state(&mut state, 120, 30);
        assert!(screen.contains("[1] Files"));
        assert!(screen.contains("[2] Old"));
        assert!(screen.contains("New"));
        assert!(screen.contains("unchanged lines"));
        assert!(screen.contains("line 15 café updated"));
        assert!(screen.contains("check unicode café handling"));
        assert!(screen.contains("1 queued"));
        assert_eq!(output.annotation_rects.len(), 1);
        assert_eq!(output.annotation_rects[0].thread_id, thread_id);

        state.set_hunks_only(false, 25);
        let (full_screen, _) = render_state(&mut state, 120, 30);
        assert!(!full_screen.contains("unchanged lines"));
    }

    #[test]
    fn test_render_diff_added_deleted_binary_and_empty_states() {
        let mut added = AppState::new(
            vec![FileDiff {
                filename: "new.rs".into(),
                old_content: String::new(),
                new_content: "fn hello() {}\n".into(),
                status: FileStatus::Added,
                is_binary: false,
            }],
            None,
        );
        let (added_screen, _) = render_state(&mut added, 100, 18);
        assert!(added_screen.contains("[2] New File"));
        assert!(added_screen.contains("fn hello()"));

        let mut deleted = AppState::new(
            vec![FileDiff {
                filename: "old.rs".into(),
                old_content: "fn goodbye() {}\n".into(),
                new_content: String::new(),
                status: FileStatus::Deleted,
                is_binary: false,
            }],
            None,
        );
        let (deleted_screen, _) = render_state(&mut deleted, 100, 18);
        assert!(deleted_screen.contains("[2] Deleted File"));
        assert!(deleted_screen.contains("fn goodbye()"));

        let mut binary = AppState::new(
            vec![FileDiff {
                filename: "image.png".into(),
                old_content: "\0PNG".into(),
                new_content: "\0PNG2".into(),
                status: FileStatus::Modified,
                is_binary: true,
            }],
            None,
        );
        let (binary_screen, _) = render_state(&mut binary, 100, 18);
        assert!(binary_screen.contains("Binary file - not displayed"));

        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal
            .draw(|frame| render_empty_state(frame, true, Some("api")))
            .unwrap();
        let empty_screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(empty_screen.contains("Git Review · api"));
        assert!(empty_screen.contains("No changes detected. (watching for changes...)"));
    }

    #[test]
    fn test_render_diff_long_inline_thread_records_expand_row() {
        let mut state = AppState::new(
            vec![FileDiff::from_contents(
                "a.rs".into(),
                "a\n".into(),
                "b\n".into(),
            )],
            None,
        );
        let (id, _) = state.add_thread("a.rs".into(), AnnotationTarget::File, "one".into());
        for i in 0..8 {
            state.threads[0]
                .comments
                .push(Comment::agent(format!("reply {i}"), "pi".into()));
        }
        let (screen, output) = render_state(&mut state, 100, 22);
        assert!(screen.contains("earlier comment"));
        assert_eq!(output.annotation_rects.len(), 1);
        assert_eq!(output.annotation_rects[0].thread_id, id);
        assert!(!output.annotation_rects[0].expand_rows.is_empty());
    }
}
