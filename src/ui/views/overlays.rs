use ratatui::{
    prelude::*,
    widgets::{Clear, Paragraph},
};

use crate::diff::{DiffLine, DiffPanelFocus};
use crate::review::{AnnotationTarget, Thread};
use crate::ui::coordinates::ThreadHitRect;
use crate::ui::editor::AnnotationEditor;
use crate::ui::markdown::{render_comment_rows, truncate_with_ellipsis};
use crate::ui::theme;

pub enum OverlaySlot<'a, 'e> {
    Saved {
        thread: &'a Thread,
        rows: Vec<Line<'static>>,
        expand_rows: Vec<usize>,
    },
    Editor(&'e AnnotationEditor<'e>),
}

impl<'a, 'e> OverlaySlot<'a, 'e> {
    pub fn height(&self) -> usize {
        match self {
            Self::Saved { rows, .. } => rows.len(),
            Self::Editor(e) => e.desired_height(),
        }
    }

    pub fn target(&self) -> &AnnotationTarget {
        match self {
            Self::Saved { thread, .. } => &thread.target,
            Self::Editor(e) => &e.target,
        }
    }
}

pub struct SlotBuildCtx<'a> {
    pub filename: &'a str,
    pub file_level: bool,
    pub visible_height: usize,
    pub tab_width: usize,
    pub theme: &'a theme::Theme,
}

pub fn build_slots<'a, 'e>(
    threads: &'a [Thread],
    editor: Option<&'e AnnotationEditor<'e>>,
    ctx: &SlotBuildCtx<'_>,
    width_for: &dyn Fn(&AnnotationTarget) -> u16,
) -> Vec<OverlaySlot<'a, 'e>> {
    let is_wanted_level =
        |target: &AnnotationTarget| matches!(target, AnnotationTarget::File) == ctx.file_level;
    let mut editor = editor.filter(|e| e.filename == ctx.filename && is_wanted_level(&e.target));
    let mut slots: Vec<OverlaySlot<'a, 'e>> = Vec::new();

    let open_file_threads = if ctx.file_level {
        threads
            .iter()
            .filter(|thread| {
                thread.filename == ctx.filename
                    && is_wanted_level(&thread.target)
                    && !thread.resolved
            })
            .count()
            .max(1)
    } else {
        1
    };

    for thread in threads
        .iter()
        .filter(|thread| thread.filename == ctx.filename && is_wanted_level(&thread.target))
    {
        let editor_in_thread = editor.filter(|e| e.thread_id.as_deref() == Some(&thread.id));
        let edited_comment = editor_in_thread.and_then(|e| e.comment_id.as_deref());
        let rows_width = width_for(&thread.target).saturating_sub(1) as usize;
        let max_rows = inline_thread_max_rows(
            ctx.visible_height,
            ctx.file_level,
            open_file_threads,
            editor_in_thread.map(AnnotationEditor::desired_height),
        );
        let rendered = thread_rows(
            thread,
            edited_comment,
            rows_width,
            max_rows,
            ctx.tab_width,
            ctx.theme,
        );
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
    let with_hint = format!("⋯ {count} {singular_label}{suffix} (t to view)");
    let text = if Span::raw(&with_hint).width() <= text_width {
        with_hint
    } else {
        let short = format!("⋯ {count} {singular_label}{suffix}");
        truncate_with_ellipsis(&short, text_width)
    };
    vec![Span::styled(
        text,
        Style::default().fg(t.ui.text_muted).italic(),
    )]
}

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
        let summary = truncate_with_ellipsis(&summary, width);
        let pad = width.saturating_sub(Span::raw(&summary).width());
        return RenderedThread {
            rows: vec![Line::from(Span::styled(
                format!("{summary}{}", " ".repeat(pad)),
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
        truncate_with_ellipsis(
            &format!(" {} ", badges.join(" · ")),
            inner_width.saturating_sub(1),
        )
    };
    let leading_dashes = usize::from(!badge.is_empty());
    let trailing_dashes = inner_width.saturating_sub(leading_dashes + Span::raw(&badge).width());
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
        .map(|comment| render_comment_rows(comment, &thread.filename, text_width, tab_width, t))
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

pub fn compute_target_index_ranges(
    slots: &[OverlaySlot],
    side_by_side: &[DiffLine],
) -> Vec<(usize, usize, DiffPanelFocus)> {
    let mut ranges = Vec::new();
    for slot in slots {
        if let AnnotationTarget::LineRange {
            panel,
            start_line,
            end_line,
        } = slot.target()
        {
            let mut first_idx = None;
            let mut last_idx = None;
            for (idx, dl) in side_by_side.iter().enumerate() {
                if let Some(n) = dl.line_number(*panel) {
                    if n >= *start_line && n <= *end_line {
                        first_idx.get_or_insert(idx);
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

pub fn is_in_ann_range(
    sbs_idx: usize,
    panel: Option<DiffPanelFocus>,
    ranges: &[(usize, usize, DiffPanelFocus)],
) -> bool {
    ranges.iter().any(|(first, last, ann_panel)| {
        sbs_idx >= *first && sbs_idx <= *last && panel.is_none_or(|p| *ann_panel == p)
    })
}

pub fn file_slots_height(slots: &[OverlaySlot]) -> usize {
    slots.iter().map(OverlaySlot::height).sum()
}

pub struct OverlayRenderTarget {
    pub x: u16,
    pub start_y: u16,
    pub width: u16,
    pub max_area: Rect,
    pub suppress_gutter: bool,
}

pub fn render_overlay_slot(
    frame: &mut Frame,
    line_pos: usize,
    slot: &OverlaySlot,
    target: &OverlayRenderTarget,
    t: &theme::Theme,
    annotation_rects: &mut Vec<ThreadHitRect>,
    editor_rect: &mut Option<Rect>,
) {
    let inner_bottom = target.max_area.y + target.max_area.height.saturating_sub(1);
    let screen_y = target.start_y + line_pos as u16;
    if screen_y >= inner_bottom {
        return;
    }
    let available = inner_bottom.saturating_sub(screen_y) as usize;
    if available == 0 {
        return;
    }
    let height = slot.height().min(available) as u16;
    let area = Rect::new(target.x, screen_y, target.width, height);
    let has_gutter =
        !target.suppress_gutter && matches!(slot.target(), AnnotationTarget::LineRange { .. });

    match slot {
        OverlaySlot::Saved {
            thread,
            rows,
            expand_rows,
        } => {
            frame.render_widget(Clear, area);
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
            frame.render_widget(
                Paragraph::new(lines).style(Style::default().bg(t.ui.bg)),
                area,
            );
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
            editor.render_inline(frame, area, t.ui.highlight, t.ui.bg, has_gutter);
            *editor_rect = Some(area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::Comment;

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
            assert_eq!(row.width(), width, "row {idx} misaligned");
            for span in &row.spans {
                assert!(!span.content.contains('\n') && !span.content.contains('\r'));
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
        thread.comments.push(Comment::agent(
            "second comment".to_string(),
            "pi".to_string(),
        ));
        thread
            .comments
            .push(Comment::human("third comment".to_string(), false));
        thread.comments.push(Comment::agent(
            "fourth comment".to_string(),
            "pi".to_string(),
        ));

        let width = 46;
        let rendered = thread_rows(&thread, None, width, 7, 4, t);
        assert_eq!(rendered.rows.len(), 7);
        assert_eq!(rendered.expand_rows, vec![1]);
        for (idx, row) in rendered.rows.iter().enumerate() {
            assert_eq!(row.width(), width, "row {idx} misaligned");
        }

        let texts: Vec<String> = rendered.rows.iter().map(line_text).collect();
        assert!(texts[1].contains("⋯ 2 earlier comments (t to view)"));
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
            .map(|n| format!("line {n:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut thread = Thread::new(
            "src/app.rs".to_string(),
            AnnotationTarget::File,
            None,
            Comment::agent(long_body, "pi".to_string()),
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
        assert!(single_texts[6].contains("⋯ 11 more lines (t to view)"));

        thread
            .comments
            .insert(0, Comment::human("short opener".to_string(), false));
        let multi = thread_rows(&thread, None, width, 8, 4, t);
        assert_eq!(multi.rows.len(), 8);
        assert_eq!(multi.expand_rows, vec![1, 6]);
        let multi_texts: Vec<String> = multi.rows.iter().map(line_text).collect();
        assert!(multi_texts[1].contains("⋯ 1 earlier comment (t to view)"));
        assert!(multi_texts[6].contains("⋯ 12 more lines (t to view)"));
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
        thread
            .comments
            .push(Comment::human("follow up".to_string(), false));

        let width_for = |_: &AnnotationTarget| 50u16;
        let visible_height = 30;
        let ctx = SlotBuildCtx {
            filename: "src/app.rs",
            file_level: true,
            visible_height,
            tab_width: 4,
            theme: t,
        };

        let without_editor = build_slots(std::slice::from_ref(&thread), None, &ctx, &width_for);
        assert_eq!(without_editor.len(), 1);
        assert!(without_editor[0].height() <= visible_height * 40 / 100);

        let editor = AnnotationEditor::for_thread(&thread);
        let with_editor = build_slots(
            std::slice::from_ref(&thread),
            Some(&editor),
            &ctx,
            &width_for,
        );
        assert_eq!(with_editor.len(), 2);
        let combined: usize = with_editor.iter().map(OverlaySlot::height).sum();
        assert!(combined <= visible_height * 40 / 100);
    }
}
