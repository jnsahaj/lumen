use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph},
};

use crate::diff::ChangeType;
use crate::search::global::{GlobalSearchEntry, GlobalSearchState, LineChange, ScoredResult};
use crate::search::MatchPanel;
use crate::ui::markdown::expand_tabs_in_spans;
use crate::ui::theme::{self, Theme, ThemeMode};
use crate::ui::views::modal::ModalResult;

pub fn handle_global_search_input(
    state: &mut GlobalSearchState,
    key: KeyEvent,
    terminal_height: u16,
) -> Option<ModalResult> {
    let visible_rows = (terminal_height as usize).saturating_sub(4);
    let page_step = (visible_rows / 2).max(5);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let cmd =
        key.modifiers.contains(KeyModifiers::SUPER) || key.modifiers.contains(KeyModifiers::META);

    let is_down = matches!(key.code, KeyCode::Down)
        || (ctrl && matches!(key.code, KeyCode::Char('n') | KeyCode::Char('k')));
    let is_up = matches!(key.code, KeyCode::Up)
        || (ctrl && matches!(key.code, KeyCode::Char('p') | KeyCode::Char('j')));
    let is_page_down =
        matches!(key.code, KeyCode::PageDown) || (ctrl && matches!(key.code, KeyCode::Char('d')));

    if is_down {
        state.move_down(visible_rows);
        return None;
    }
    if is_up {
        state.move_up(visible_rows);
        return None;
    }
    if is_page_down {
        state.page_down(page_step);
        return None;
    }

    match key.code {
        KeyCode::Esc => Some(ModalResult::Dismissed),
        KeyCode::Char('c') if ctrl => Some(ModalResult::Dismissed),
        KeyCode::Enter => state.current_entry().map(|e| ModalResult::JumpToLine {
            file_index: e.file_index,
            line_no: e.line_no,
            panel: e.panel,
        }),
        KeyCode::PageUp => {
            state.page_up(page_step);
            None
        }
        KeyCode::Home => {
            state.jump_top(visible_rows);
            None
        }
        KeyCode::End => {
            state.jump_bottom(visible_rows);
            None
        }
        KeyCode::Char('u') if ctrl => {
            state.clear_query();
            None
        }
        KeyCode::Backspace if cmd => {
            state.clear_query();
            None
        }
        KeyCode::Backspace if alt => {
            state.erase_query_word();
            None
        }
        KeyCode::Char('w') if ctrl => {
            state.erase_query_word();
            None
        }
        KeyCode::Backspace => {
            state.pop_char();
            None
        }
        KeyCode::Char(c) if !ctrl && !alt => {
            state.push_char(c);
            None
        }
        _ => None,
    }
}

pub fn handle_global_search_mouse(
    state: &mut GlobalSearchState,
    mouse: MouseEvent,
    terminal_width: u16,
    terminal_height: u16,
) -> bool {
    let pane_boundary = (terminal_width as u32 * 50 / 100) as u16;
    let in_left = mouse.column < pane_boundary;
    let list_y_start: u16 = 3;
    let visible_rows = (terminal_height as usize).saturating_sub(4);
    let horizontal = mouse.modifiers.contains(KeyModifiers::SHIFT);

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if in_left && mouse.row >= list_y_start && mouse.row + 1 < terminal_height {
                let row_in_list = (mouse.row - list_y_start) as usize;
                let target = state.list_scroll + row_in_list;
                if target < state.results.len() {
                    state.select(target, visible_rows);
                }
            }
            true
        }
        MouseEventKind::ScrollDown => {
            if in_left {
                if horizontal {
                    state.scroll_list_x(4);
                } else {
                    state.scroll_list_y(3, visible_rows);
                }
            } else if horizontal {
                state.scroll_preview_x(4);
            } else {
                state.scroll_preview_y(3);
            }
            true
        }
        MouseEventKind::ScrollUp => {
            if in_left {
                if horizontal {
                    state.scroll_list_x(-4);
                } else {
                    state.scroll_list_y(-3, visible_rows);
                }
            } else if horizontal {
                state.scroll_preview_x(-4);
            } else {
                state.scroll_preview_y(-3);
            }
            true
        }
        MouseEventKind::ScrollLeft => {
            if in_left {
                state.scroll_list_x(-4);
            } else {
                state.scroll_preview_x(-4);
            }
            true
        }
        MouseEventKind::ScrollRight => {
            if in_left {
                state.scroll_list_x(4);
            } else {
                state.scroll_preview_x(4);
            }
            true
        }
        _ => false,
    }
}

pub fn render_global_search(frame: &mut Frame, area: Rect, title: &str, state: &GlobalSearchState) {
    let t = theme::get();
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let count = state.result_count();
    let total = state.total_indexed();
    let results_title = if state.query.is_empty() {
        format!(" {title} · {total} lines ")
    } else {
        format!(" {title} · {count}/{total} ")
    };
    let results_block = Block::default()
        .title(results_title)
        .title_style(Style::default().fg(t.ui.text_secondary))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.ui.border_focused));
    let results_inner = results_block.inner(cols[0]);
    frame.render_widget(results_block, cols[0]);

    let left_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(results_inner);

    let prompt_line = Line::from(vec![
        Span::styled("❯ ", Style::default().fg(t.ui.border_focused).bold()),
        Span::styled(&state.query, Style::default().fg(t.ui.text_primary)),
        Span::styled("█", Style::default().fg(t.ui.border_focused)),
    ]);
    frame.render_widget(Paragraph::new(prompt_line), left_rows[0]);

    let sep = "─".repeat(left_rows[1].width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(
            sep,
            Style::default().fg(t.ui.border_unfocused),
        )),
        left_rows[1],
    );

    let list_area = left_rows[2];
    let visible = list_area.height as usize;
    let scroll = state
        .list_scroll
        .min(state.results.len().saturating_sub(visible.max(1)));

    let list_items: Vec<ListItem> = state
        .results
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible)
        .filter_map(|(i, r)| {
            let entry = state.entries.get(r.entry_index)?;
            let is_selected = i == state.selected;
            Some(build_result_row(
                entry,
                r,
                is_selected,
                t,
                list_area.width,
                state.list_scroll_x,
            ))
        })
        .collect();
    frame.render_widget(List::new(list_items), list_area);

    let preview_title = state
        .current_entry()
        .map(|e| format!(" {}:{} ", e.filename, e.line_no))
        .unwrap_or_else(|| " preview ".to_string());
    let preview_block = Block::default()
        .title(preview_title)
        .title_style(Style::default().fg(t.ui.text_secondary))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(t.ui.border_unfocused))
        .style(Style::default().bg(t.ui.bg));
    let preview_inner = preview_block.inner(cols[1]);
    frame.render_widget(preview_block, cols[1]);

    render_preview_pane(frame, preview_inner, state, t);
}

fn build_result_row(
    entry: &GlobalSearchEntry,
    result: &ScoredResult,
    is_selected: bool,
    t: &Theme,
    width: u16,
    scroll_x: usize,
) -> ListItem<'static> {
    let (sym_char, sym_color) = match entry.change {
        LineChange::Added => ('+', t.ui.stats_added),
        LineChange::Removed => ('-', t.ui.stats_removed),
        LineChange::Modified => ('~', t.ui.status_modified),
        LineChange::Equal => (' ', t.ui.text_muted),
    };

    let bg = is_selected.then_some(t.ui.selection_bg);
    let apply_bg = |mut s: Style| -> Style {
        if let Some(bg) = bg {
            s = s.bg(bg);
        }
        s
    };

    let body_color = if is_selected {
        t.ui.selection_fg
    } else {
        t.ui.text_primary
    };
    let meta_color = if is_selected {
        t.ui.selection_fg
    } else {
        t.ui.text_muted
    };

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(16);
    spans.push(if is_selected {
        Span::styled(
            "❯ ",
            apply_bg(Style::default().fg(t.ui.border_focused).bold()),
        )
    } else {
        Span::styled("  ", apply_bg(Style::default()))
    });
    spans.push(Span::styled(
        format!("{sym_char} "),
        apply_bg(Style::default().fg(sym_color).bold()),
    ));

    let filename_len = entry.filename.chars().count();
    let lineno_str_len = entry.line_no.to_string().chars().count();
    let prefix_len = filename_len + 1 + lineno_str_len + 2;

    let mut to_skip = scroll_x;
    let mut match_iter = result.match_indices.iter().peekable();
    let mut emitted_chars = 0;
    let mut run = String::new();
    let mut run_style: Option<Style> = None;

    for (i, ch) in entry.haystack.chars().enumerate() {
        let is_match = match match_iter.peek() {
            Some(&&idx) if idx as usize == i => {
                match_iter.next();
                true
            }
            _ => false,
        };
        if to_skip > 0 {
            to_skip -= 1;
            continue;
        }
        let fg = if is_match {
            t.ui.search_match_fg
        } else if i < prefix_len {
            meta_color
        } else {
            body_color
        };
        let mut style = Style::default().fg(fg);
        if is_match {
            style = style.bold();
        }
        let style = apply_bg(style);
        if run_style != Some(style) {
            if let Some(prev) = run_style.take() {
                if !run.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut run), prev));
                }
            }
            run_style = Some(style);
        }
        run.push(ch);
        emitted_chars += 1;
    }
    if let Some(prev) = run_style {
        if !run.is_empty() {
            spans.push(Span::styled(run, prev));
        }
    }

    if is_selected {
        let pad = (width as usize).saturating_sub(4 + emitted_chars);
        if pad > 0 {
            spans.push(Span::styled(
                " ".repeat(pad),
                Style::default().bg(t.ui.selection_bg),
            ));
        }
    }
    ListItem::new(Line::from(spans))
}

fn render_preview_pane(frame: &mut Frame, area: Rect, state: &GlobalSearchState, t: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let Some(entry) = state.current_entry() else {
        if !state.query.is_empty() {
            let msg = Line::from(Span::styled(
                "  no matches",
                Style::default().fg(t.ui.text_muted).bg(t.ui.bg).italic(),
            ));
            frame.render_widget(Paragraph::new(msg), area);
        }
        return;
    };

    let body_width = area.width as usize;
    let body_height = area.height as usize;
    let tab_width = state.files.get(entry.file_index).map_or(4, |f| f.tab_width);

    #[derive(Clone, Copy)]
    enum Side {
        Old,
        New,
    }
    #[derive(Clone, Copy)]
    struct PreviewRowMeta {
        sbs_idx: usize,
        side: Side,
        is_cursor: bool,
    }

    state.with_sbs(entry.file_index, |sbs| {
        let mut metas: Vec<PreviewRowMeta> = Vec::with_capacity(sbs.len() + sbs.len() / 4);
        let mut pending: Vec<PreviewRowMeta> = Vec::new();
        let mut cursor_idx: Option<usize> = None;

        let flush = |metas: &mut Vec<PreviewRowMeta>,
                     pending: &mut Vec<PreviewRowMeta>,
                     cursor_idx: &mut Option<usize>| {
            for p in pending.drain(..) {
                if p.is_cursor {
                    *cursor_idx = Some(metas.len());
                }
                metas.push(p);
            }
        };

        for (sbs_idx, sbs_line) in sbs.iter().enumerate() {
            let is_match_row = sbs_idx == entry.sbs_line_index;
            match sbs_line.change_type {
                ChangeType::Equal => {
                    flush(&mut metas, &mut pending, &mut cursor_idx);
                    if sbs_line.new_line.is_some() {
                        let is_cursor = is_match_row && entry.panel == MatchPanel::New;
                        if is_cursor {
                            cursor_idx = Some(metas.len());
                        }
                        metas.push(PreviewRowMeta {
                            sbs_idx,
                            side: Side::New,
                            is_cursor,
                        });
                    }
                }
                ChangeType::Insert => {
                    if sbs_line.new_line.is_some() {
                        pending.push(PreviewRowMeta {
                            sbs_idx,
                            side: Side::New,
                            is_cursor: is_match_row && entry.panel == MatchPanel::New,
                        });
                    }
                }
                ChangeType::Delete => {
                    if sbs_line.old_line.is_some() {
                        let is_cursor = is_match_row && entry.panel == MatchPanel::Old;
                        if is_cursor {
                            cursor_idx = Some(metas.len());
                        }
                        metas.push(PreviewRowMeta {
                            sbs_idx,
                            side: Side::Old,
                            is_cursor,
                        });
                    }
                }
                ChangeType::Modified => {
                    if sbs_line.old_line.is_some() {
                        let is_cursor = is_match_row && entry.panel == MatchPanel::Old;
                        if is_cursor {
                            cursor_idx = Some(metas.len());
                        }
                        metas.push(PreviewRowMeta {
                            sbs_idx,
                            side: Side::Old,
                            is_cursor,
                        });
                    }
                    if sbs_line.new_line.is_some() {
                        pending.push(PreviewRowMeta {
                            sbs_idx,
                            side: Side::New,
                            is_cursor: is_match_row && entry.panel == MatchPanel::New,
                        });
                    }
                }
            }
        }
        flush(&mut metas, &mut pending, &mut cursor_idx);

        let total_rows = metas.len();
        let base_drop = cursor_idx
            .map(|i| i.saturating_sub(body_height / 2))
            .unwrap_or(0);
        let max_drop = total_rows.saturating_sub(body_height);
        let drop = ((base_drop as i32 + state.preview_scroll_y).max(0) as usize).min(max_drop);
        let end = (drop + body_height).min(total_rows);

        let lines: Vec<Line> = metas[drop..end]
            .iter()
            .filter_map(|m| {
                let sbs_line = &sbs[m.sbs_idx];
                let (line_no, text, sym, row_bg, gutter_bg, panel) = match m.side {
                    Side::Old => {
                        let (ln, text) = sbs_line.old_line.as_ref()?;
                        (
                            *ln,
                            text.as_str(),
                            '-',
                            t.diff.deleted_bg,
                            t.diff.deleted_gutter_bg,
                            MatchPanel::Old,
                        )
                    }
                    Side::New => {
                        let (ln, text) = sbs_line.new_line.as_ref()?;
                        let (sym, row_bg, gutter_bg) = match sbs_line.change_type {
                            ChangeType::Equal => (' ', t.ui.bg, t.ui.bg),
                            ChangeType::Insert | ChangeType::Modified => {
                                ('+', t.diff.added_bg, t.diff.added_gutter_bg)
                            }
                            ChangeType::Delete => return None,
                        };
                        (*ln, text.as_str(), sym, row_bg, gutter_bg, MatchPanel::New)
                    }
                };
                Some(make_preview_row(
                    &PreviewRowInput {
                        state,
                        file_index: entry.file_index,
                        panel,
                        line_no,
                        text,
                        sym,
                        row_bg,
                        gutter_bg,
                        is_cursor: m.is_cursor,
                        width: body_width,
                        scroll_x: state.preview_scroll_x,
                        tab_width,
                    },
                    t,
                ))
            })
            .collect();

        frame.render_widget(Paragraph::new(lines), area);
    });
}

struct PreviewRowInput<'a> {
    state: &'a GlobalSearchState,
    file_index: usize,
    panel: MatchPanel,
    line_no: usize,
    text: &'a str,
    sym: char,
    row_bg: Color,
    gutter_bg: Color,
    is_cursor: bool,
    width: usize,
    scroll_x: usize,
    tab_width: usize,
}

fn make_preview_row(input: &PreviewRowInput<'_>, t: &Theme) -> Line<'static> {
    let (row_bg, gutter_bg) = if input.is_cursor {
        let fallback = cursor_row_fallback_bg(t);
        (
            brighten_bg(input.row_bg, 18).unwrap_or(fallback),
            brighten_bg(input.gutter_bg, 18).unwrap_or(fallback),
        )
    } else {
        (input.row_bg, input.gutter_bg)
    };

    let accent = if input.is_cursor {
        Span::styled("▌", Style::default().fg(t.ui.border_focused).bg(gutter_bg))
    } else {
        Span::styled(" ", Style::default().bg(gutter_bg))
    };

    let gutter_fg = match input.sym {
        '+' => t.diff.added_gutter_fg,
        '-' => t.diff.deleted_gutter_fg,
        _ => t.ui.line_number,
    };
    let gutter_text = format!("{:>4} ", input.line_no);
    let gutter_style = Style::default().fg(gutter_fg).bg(gutter_bg);

    let highlighted = input.state.highlighted_line_spans(
        input.file_index,
        input.panel,
        input.line_no,
        Some(row_bg),
    );
    let body_spans: Vec<Span<'static>> = if highlighted.is_empty() {
        vec![Span::styled(
            input.text.to_string(),
            Style::default().fg(t.ui.text_primary).bg(row_bg),
        )]
    } else {
        expand_tabs_in_spans(highlighted, input.tab_width)
    };

    let avail = input.width.saturating_sub(6);
    let mut row: Vec<Span<'static>> = Vec::with_capacity(body_spans.len() + 3);
    row.push(accent);
    row.push(Span::styled(gutter_text, gutter_style));

    let mut to_skip = input.scroll_x;
    let mut remaining = avail;
    let mut used_chars = 0;
    for sp in body_spans {
        if remaining == 0 {
            break;
        }
        let count = sp.content.chars().count();
        if to_skip >= count {
            to_skip -= count;
            continue;
        }
        let visible_text: String = if to_skip > 0 {
            let s: String = sp.content.chars().skip(to_skip).collect();
            to_skip = 0;
            s
        } else {
            sp.content.to_string()
        };
        let visible_count = visible_text.chars().count();
        if visible_count <= remaining {
            used_chars += visible_count;
            remaining -= visible_count;
            row.push(Span::styled(visible_text, sp.style));
        } else {
            let truncated: String = visible_text.chars().take(remaining).collect();
            used_chars += remaining;
            remaining = 0;
            row.push(Span::styled(truncated, sp.style));
        }
    }

    let pad = avail.saturating_sub(used_chars);
    if pad > 0 {
        row.push(Span::styled(" ".repeat(pad), Style::default().bg(row_bg)));
    }

    Line::from(row)
}

fn brighten_bg(c: Color, amount: u8) -> Option<Color> {
    match c {
        Color::Rgb(r, g, b) => Some(Color::Rgb(
            r.saturating_add(amount),
            g.saturating_add(amount),
            b.saturating_add(amount),
        )),
        _ => None,
    }
}

fn cursor_row_fallback_bg(t: &Theme) -> Color {
    match t.mode {
        ThemeMode::Dark => Color::Rgb(55, 58, 68),
        ThemeMode::Light => Color::Rgb(225, 228, 232),
    }
}
