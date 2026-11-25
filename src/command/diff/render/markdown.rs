use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::prelude::*;

use crate::command::diff::highlight::highlight_code_lines;
use crate::command::diff::render::diff_view::expand_tabs_in_spans;
use crate::command::diff::review_thread::{Author, Comment};
use crate::command::diff::theme;

enum Container {
    BlockQuote,
    List { next_number: Option<u64> },
    Item { marker: String, first_line_pending: bool },
}

struct CodeBlockBuffer {
    language: Option<String>,
    content: String,
}

struct LinkState {
    url: String,
    label: String,
}

#[derive(Clone)]
struct StyledGrapheme {
    text: String,
    style: Style,
    width: usize,
}

/// Renders Markdown comment `body` into wrapped rows of styled spans, each with
/// display width at most `width`. Inline code and untagged fenced code blocks
/// are highlighted with the language rules of `filename`; tagged fenced blocks
/// use the requested language when bundled.
pub fn render_markdown(
    body: &str,
    width: usize,
    base_fg: Color,
    filename: &str,
    tab_width: usize,
    t: &theme::Theme,
) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(body, options).into_offset_iter();

    let mut renderer = MarkdownRenderer {
        source: body,
        width,
        filename,
        tab_width,
        theme: t,
        rows: Vec::new(),
        containers: Vec::new(),
        style_stack: vec![Style::default().fg(base_fg)],
        inline_spans: Vec::new(),
        code_block: None,
        link_stack: Vec::new(),
        last_content_end: None,
    };

    for (event, range) in parser {
        renderer.handle_event(event, range);
    }
    renderer.flush_inline();
    renderer.rows
}

/// Renders one review comment as wrapped rows of styled spans, each with display
/// width at most `width`: its author/time header row followed by its Markdown body.
pub fn render_comment_rows(
    comment: &Comment,
    filename: &str,
    width: usize,
    tab_width: usize,
    t: &theme::Theme,
) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let (name_color, body_color) = match comment.author {
        Author::Human => (t.ui.highlight, t.ui.text_secondary),
        Author::Agent => (t.ui.status_added, t.ui.text_primary),
    };
    let muted = Style::default().fg(t.ui.text_muted);
    let time_suffix = format!(" · {}", comment.format_time());
    let queued_suffix = if comment.queued { " · queued" } else { "" };
    let suffix_width = str_width(&time_suffix) + str_width(queued_suffix);
    let name = truncate_with_ellipsis(&comment.author_name, width.saturating_sub(suffix_width));
    let mut header = vec![
        Span::styled(name, Style::default().fg(name_color).bold()),
        Span::styled(time_suffix, muted),
    ];
    if comment.queued {
        header.push(Span::styled(
            queued_suffix,
            Style::default().fg(t.ui.status_modified),
        ));
    }

    let mut rows = vec![truncate_spans(&header, width)];
    rows.extend(render_markdown(
        &comment.body,
        width,
        body_color,
        filename,
        tab_width,
        t,
    ));
    rows
}

/// Renders a raw code snippet (e.g. a thread's anchored lines) highlighted with
/// `filename`'s language rules, wrapped to `width` with a muted left bar and `code_bg`.
pub fn render_code_snippet(
    code: &str,
    filename: &str,
    width: usize,
    tab_width: usize,
    t: &theme::Theme,
) -> Vec<Vec<Span<'static>>> {
    let normalized = code.replace("\r\n", "\n");
    let trimmed = normalized.strip_suffix('\n').unwrap_or(&normalized);
    let code_style = Style::default().bg(t.ui.code_bg);
    let highlighted_lines = highlight_code_lines(trimmed, None, filename, code_style);
    let mut rows = Vec::new();
    for line_spans in highlighted_lines {
        rows.extend(wrap_code_line(line_spans, &[], &[], width, tab_width, t));
    }
    rows
}

pub(super) fn truncate_with_ellipsis(text: &str, max_width: usize) -> String {
    if str_width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    let keep_budget = max_width.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0usize;
    for g in Span::raw(text).styled_graphemes(Style::default()) {
        let w = str_width(g.symbol);
        if used + w > keep_budget {
            break;
        }
        out.push_str(g.symbol);
        used += w;
    }
    out.push('…');
    out
}

struct MarkdownRenderer<'a> {
    source: &'a str,
    width: usize,
    filename: &'a str,
    tab_width: usize,
    theme: &'a theme::Theme,
    rows: Vec<Vec<Span<'static>>>,
    containers: Vec<Container>,
    style_stack: Vec<Style>,
    inline_spans: Vec<Span<'static>>,
    code_block: Option<CodeBlockBuffer>,
    link_stack: Vec<LinkState>,
    last_content_end: Option<usize>,
}

impl<'a> MarkdownRenderer<'a> {
    fn current_style(&self) -> Style {
        self.style_stack
            .iter()
            .fold(Style::default(), |acc, layer| acc.patch(*layer))
    }

    fn muted_style(&self) -> Style {
        Style::default().fg(self.theme.ui.text_muted)
    }

    fn start_block(&mut self, start: usize) {
        self.flush_inline();
        self.maybe_emit_blank_line(start);
    }

    /// Emits a blank separator row when the source left at least one blank line
    /// between the previous emitted content and the block starting at `start`.
    fn maybe_emit_blank_line(&mut self, start: usize) {
        if self.rows.is_empty() {
            return;
        }
        let Some(prev_end) = self.last_content_end else {
            return;
        };
        if start < prev_end || start > self.source.len() {
            return;
        }
        let gap = &self.source[prev_end..start];
        let newlines = gap.bytes().filter(|&b| b == b'\n').count();
        let prev_ended_with_newline =
            prev_end > 0 && self.source.as_bytes()[prev_end - 1] == b'\n';
        let required_newlines = if prev_ended_with_newline { 1 } else { 2 };
        if newlines >= required_newlines {
            let prefix = self.blank_line_prefix();
            self.rows.push(truncate_spans(&prefix, self.width));
            self.last_content_end = None;
        }
    }

    fn blank_line_prefix(&self) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        for container in &self.containers {
            match container {
                Container::BlockQuote => spans.push(Span::styled("▎ ", self.muted_style())),
                Container::Item { marker, .. } => {
                    spans.push(Span::raw(" ".repeat(str_width(marker))));
                }
                Container::List { .. } => {}
            }
        }
        spans
    }

    fn take_prefixes(&mut self) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
        let muted = self.muted_style();
        let mut first = Vec::new();
        let mut cont = Vec::new();
        for container in &mut self.containers {
            match container {
                Container::BlockQuote => {
                    first.push(Span::styled("▎ ", muted));
                    cont.push(Span::styled("▎ ", muted));
                }
                Container::Item {
                    marker,
                    first_line_pending,
                } => {
                    let indent = " ".repeat(str_width(marker));
                    if *first_line_pending {
                        first.push(Span::styled(marker.clone(), muted));
                        *first_line_pending = false;
                    } else {
                        first.push(Span::raw(indent.clone()));
                    }
                    cont.push(Span::raw(indent));
                }
                Container::List { .. } => {}
            }
        }
        (first, cont)
    }

    fn flush_inline(&mut self) {
        if self.inline_spans.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.inline_spans);
        let (first_prefix, cont_prefix) = self.take_prefixes();
        let wrapped = wrap_prose_line(
            spans,
            &first_prefix,
            &cont_prefix,
            self.width,
            self.tab_width,
        );
        self.rows.extend(wrapped);
    }

    fn push_inline_break(&mut self) {
        if self.inline_spans.is_empty() {
            let (first_prefix, _) = self.take_prefixes();
            self.rows.push(truncate_spans(&first_prefix, self.width));
        } else {
            self.flush_inline();
        }
    }

    fn push_multiline_text(&mut self, text: &str, flush_trailing_newline: bool) {
        let normalized = text.replace("\r\n", "\n");
        let trailing_nl = flush_trailing_newline && normalized.ends_with('\n');
        let body = if trailing_nl {
            normalized.strip_suffix('\n').unwrap_or(&normalized)
        } else {
            normalized.as_str()
        };
        let style = self.current_style();
        for (idx, line) in body.split('\n').enumerate() {
            if idx > 0 {
                if let Some(link) = self.link_stack.last_mut() {
                    link.label.push(' ');
                }
                self.push_inline_break();
            }
            if !line.is_empty() {
                if let Some(link) = self.link_stack.last_mut() {
                    link.label.push_str(line);
                }
                self.inline_spans
                    .push(Span::styled(line.to_string(), style));
            }
        }
        if trailing_nl {
            self.push_inline_break();
        }
    }

    fn handle_event(&mut self, event: Event<'_>, range: std::ops::Range<usize>) {
        if let Some(buf) = self.code_block.as_mut() {
            match event {
                Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                    buf.content.push_str(&text);
                    return;
                }
                Event::End(TagEnd::CodeBlock) => {
                    let buf = self.code_block.take().unwrap();
                    self.emit_code_block(buf);
                    self.last_content_end = Some(range.end);
                    return;
                }
                _ => return,
            }
        }

        match event {
            Event::Start(tag) => self.handle_start(tag, range.start),
            Event::End(tag_end) => self.handle_end(tag_end, range.end),
            Event::Text(text) => {
                self.push_multiline_text(&text, false);
                self.last_content_end = Some(range.end);
            }
            Event::Html(text) => {
                self.push_multiline_text(&text, true);
                self.last_content_end = Some(range.end);
            }
            Event::InlineHtml(text) => {
                self.push_multiline_text(&text, false);
                self.last_content_end = Some(range.end);
            }
            Event::Code(code) => {
                let normalized = code.replace("\r\n", " ").replace('\n', " ");
                if let Some(link) = self.link_stack.last_mut() {
                    link.label.push_str(&normalized);
                }
                let active = self.current_style();
                let code_style = Style::default()
                    .bg(self.theme.ui.code_bg)
                    .add_modifier(active.add_modifier);
                let mut lines =
                    highlight_code_lines(&normalized, None, self.filename, code_style);
                if let Some(line) = lines.pop() {
                    self.inline_spans.extend(line);
                }
                self.last_content_end = Some(range.end);
            }
            Event::SoftBreak | Event::HardBreak => {
                self.push_inline_break();
            }
            Event::Rule => {
                self.start_block(range.start);
                let (prefix, _) = self.take_prefixes();
                let prefix = truncate_spans(&prefix, self.width.saturating_sub(1));
                let rule_width = self.width.saturating_sub(spans_width(&prefix));
                let mut row = prefix;
                row.push(Span::styled(
                    "─".repeat(rule_width),
                    Style::default().fg(self.theme.ui.border_unfocused),
                ));
                self.rows.push(row);
                self.last_content_end = Some(range.end);
            }
            Event::TaskListMarker(checked) => {
                let checkbox = if checked { "☑ " } else { "☐ " };
                if let Some(Container::Item { marker, .. }) = self
                    .containers
                    .iter_mut()
                    .rev()
                    .find(|c| matches!(c, Container::Item { .. }))
                {
                    *marker = checkbox.to_string();
                }
                self.last_content_end = Some(range.end);
            }
            _ => {}
        }
    }

    fn handle_start(&mut self, tag: Tag<'_>, start: usize) {
        match tag {
            Tag::Paragraph | Tag::HtmlBlock => self.start_block(start),
            Tag::Heading { .. } => {
                self.start_block(start);
                self.style_stack
                    .push(Style::default().fg(self.theme.ui.border_focused).bold());
            }
            Tag::BlockQuote(_) => {
                self.start_block(start);
                self.containers.push(Container::BlockQuote);
                self.style_stack
                    .push(Style::default().fg(self.theme.ui.text_muted).italic());
            }
            Tag::CodeBlock(kind) => {
                self.start_block(start);
                let language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        let trimmed = info.trim();
                        (!trimmed.is_empty()).then(|| trimmed.to_string())
                    }
                    CodeBlockKind::Indented => None,
                };
                self.code_block = Some(CodeBlockBuffer {
                    language,
                    content: String::new(),
                });
            }
            Tag::List(start_number) => {
                self.start_block(start);
                self.containers.push(Container::List {
                    next_number: start_number,
                });
            }
            Tag::Item => {
                self.start_block(start);
                let marker = match self
                    .containers
                    .iter_mut()
                    .rev()
                    .find_map(|c| match c {
                        Container::List { next_number } => Some(next_number),
                        _ => None,
                    }) {
                    Some(Some(number)) => {
                        let label = format!("{}. ", *number);
                        *number += 1;
                        label
                    }
                    _ => "• ".to_string(),
                };
                self.containers.push(Container::Item {
                    marker,
                    first_line_pending: true,
                });
            }
            Tag::Emphasis => {
                self.style_stack.push(Style::default().italic());
            }
            Tag::Strong => {
                self.style_stack.push(Style::default().bold());
            }
            Tag::Strikethrough => {
                self.style_stack
                    .push(Style::default().fg(self.theme.ui.text_muted).crossed_out());
            }
            Tag::Link { dest_url, .. } => {
                self.link_stack.push(LinkState {
                    url: dest_url.into_string(),
                    label: String::new(),
                });
                self.style_stack
                    .push(Style::default().fg(self.theme.ui.border_focused).underlined());
            }
            _ => {}
        }
    }

    fn handle_end(&mut self, tag_end: TagEnd, end: usize) {
        match tag_end {
            TagEnd::Paragraph | TagEnd::HtmlBlock => {
                self.flush_inline();
                self.last_content_end = Some(end);
            }
            TagEnd::Heading(_) => {
                self.flush_inline();
                self.style_stack.pop();
                self.last_content_end = Some(end);
            }
            // Container close ranges in pulldown-cmark can extend past trailing
            // blank lines to the start of the next block, so leave
            // `last_content_end` at the last emitted leaf content.
            TagEnd::BlockQuote(_) => {
                self.flush_inline();
                self.containers.pop();
                self.style_stack.pop();
            }
            TagEnd::List(_) | TagEnd::Item => {
                self.flush_inline();
                self.containers.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.style_stack.pop();
            }
            TagEnd::Link => {
                self.style_stack.pop();
                if let Some(link) = self.link_stack.pop() {
                    if !link.url.is_empty() && link.url != link.label {
                        self.inline_spans.push(Span::styled(
                            format!(" ({})", link.url),
                            self.muted_style(),
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    fn emit_code_block(&mut self, buf: CodeBlockBuffer) {
        let normalized = buf.content.replace("\r\n", "\n");
        let code = normalized.strip_suffix('\n').unwrap_or(&normalized);
        let code_style = Style::default().bg(self.theme.ui.code_bg);
        let highlighted_lines =
            highlight_code_lines(code, buf.language.as_deref(), self.filename, code_style);

        for line_spans in highlighted_lines {
            let (first_prefix, cont_prefix) = self.take_prefixes();
            let wrapped = wrap_code_line(
                line_spans,
                &first_prefix,
                &cont_prefix,
                self.width,
                self.tab_width,
                self.theme,
            );
            self.rows.extend(wrapped);
        }
    }
}

fn str_width(text: &str) -> usize {
    Span::raw(text).width()
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(Span::width).sum()
}

fn spans_to_graphemes(spans: Vec<Span<'static>>, tab_width: usize) -> Vec<StyledGrapheme> {
    let expanded = expand_tabs_in_spans(spans, tab_width);
    let mut graphemes = Vec::new();
    for span in expanded {
        for g in span.styled_graphemes(Style::default()) {
            graphemes.push(StyledGrapheme {
                width: str_width(g.symbol),
                text: g.symbol.to_string(),
                style: g.style,
            });
        }
    }
    graphemes
}

fn graphemes_to_spans(graphemes: &[StyledGrapheme]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut iter = graphemes.iter();
    let Some(first) = iter.next() else {
        return spans;
    };
    let mut current_style = first.style;
    let mut buffer = first.text.clone();
    for g in iter {
        if g.style == current_style {
            buffer.push_str(&g.text);
        } else {
            spans.push(Span::styled(std::mem::take(&mut buffer), current_style));
            buffer.push_str(&g.text);
            current_style = g.style;
        }
    }
    spans.push(Span::styled(buffer, current_style));
    spans
}

fn truncate_spans(spans: &[Span<'static>], max_width: usize) -> Vec<Span<'static>> {
    if spans_width(spans) <= max_width {
        return spans.to_vec();
    }
    let mut kept = Vec::new();
    let mut used = 0;
    for span in spans {
        for g in span.styled_graphemes(Style::default()) {
            let w = str_width(g.symbol);
            if used + w > max_width {
                return graphemes_to_spans(&kept);
            }
            kept.push(StyledGrapheme {
                text: g.symbol.to_string(),
                style: g.style,
                width: w,
            });
            used += w;
        }
    }
    graphemes_to_spans(&kept)
}

fn wrap_prose_line(
    spans: Vec<Span<'static>>,
    first_prefix: &[Span<'static>],
    cont_prefix: &[Span<'static>],
    width: usize,
    tab_width: usize,
) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let prefix_cap = width.saturating_sub(1);
    let first_prefix = truncate_spans(first_prefix, prefix_cap);
    let cont_prefix = truncate_spans(cont_prefix, prefix_cap);
    let first_avail = width.saturating_sub(spans_width(&first_prefix)).max(1);
    let cont_avail = width.saturating_sub(spans_width(&cont_prefix)).max(1);

    let graphemes = spans_to_graphemes(spans, tab_width);
    if graphemes.is_empty() {
        return vec![first_prefix];
    }

    let mut tokens: Vec<(bool, Vec<StyledGrapheme>)> = Vec::new();
    for g in graphemes {
        let is_space = g.text == " ";
        if let Some((last_is_space, buf)) = tokens.last_mut() {
            if *last_is_space == is_space {
                buf.push(g);
                continue;
            }
        }
        tokens.push((is_space, vec![g]));
    }

    let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
    let mut line_graphemes: Vec<StyledGrapheme> = Vec::new();
    let mut line_width = 0usize;
    let mut pending_spaces: Vec<StyledGrapheme> = Vec::new();
    let mut pending_space_width = 0usize;
    let mut is_first = true;

    let flush_row = |rows: &mut Vec<Vec<Span<'static>>>,
                     line_graphemes: &mut Vec<StyledGrapheme>,
                     line_width: &mut usize,
                     is_first: &mut bool| {
        let mut row = if *is_first {
            first_prefix.clone()
        } else {
            cont_prefix.clone()
        };
        row.extend(graphemes_to_spans(line_graphemes));
        rows.push(row);
        line_graphemes.clear();
        *line_width = 0;
        *is_first = false;
    };

    for (is_space, token) in tokens {
        let token_width: usize = token.iter().map(|g| g.width).sum();
        if is_space {
            pending_spaces.extend(token);
            pending_space_width += token_width;
            continue;
        }

        let avail = if is_first { first_avail } else { cont_avail };
        if !line_graphemes.is_empty() && line_width + pending_space_width + token_width > avail {
            flush_row(&mut rows, &mut line_graphemes, &mut line_width, &mut is_first);
            pending_spaces.clear();
            pending_space_width = 0;
        } else if !pending_spaces.is_empty() {
            if !line_graphemes.is_empty() && line_width + pending_space_width <= avail {
                line_graphemes.append(&mut pending_spaces);
                line_width += pending_space_width;
            } else {
                pending_spaces.clear();
            }
            pending_space_width = 0;
        }

        let mut remaining = token.as_slice();
        while !remaining.is_empty() {
            let avail = if is_first { first_avail } else { cont_avail };
            let space_left = avail.saturating_sub(line_width);
            let rem_width: usize = remaining.iter().map(|g| g.width).sum();
            if rem_width <= space_left {
                line_graphemes.extend_from_slice(remaining);
                line_width += rem_width;
                break;
            }
            if !line_graphemes.is_empty() {
                flush_row(&mut rows, &mut line_graphemes, &mut line_width, &mut is_first);
                continue;
            }
            let mut take = 0usize;
            let mut take_width = 0usize;
            for (idx, g) in remaining.iter().enumerate() {
                if take > 0 && take_width + g.width > avail {
                    break;
                }
                take = idx + 1;
                take_width += g.width;
            }
            line_graphemes.extend_from_slice(&remaining[..take]);
            line_width += take_width;
            remaining = &remaining[take..];
            if !remaining.is_empty() {
                flush_row(&mut rows, &mut line_graphemes, &mut line_width, &mut is_first);
            }
        }
    }

    if !line_graphemes.is_empty() || rows.is_empty() {
        flush_row(&mut rows, &mut line_graphemes, &mut line_width, &mut is_first);
    }
    rows
}

fn wrap_code_line(
    spans: Vec<Span<'static>>,
    first_outer: &[Span<'static>],
    cont_outer: &[Span<'static>],
    width: usize,
    tab_width: usize,
    t: &theme::Theme,
) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let code_bg = t.ui.code_bg;
    let bar = Span::styled("▎ ", Style::default().fg(t.ui.border_unfocused));
    let prefix_cap = width.saturating_sub(1);

    let make_prefix = |outer: &[Span<'static>]| {
        let mut p = outer.to_vec();
        p.push(bar.clone());
        truncate_spans(&p, prefix_cap)
    };
    let first_prefix = make_prefix(first_outer);
    let cont_prefix = make_prefix(cont_outer);

    let code_width = |prefix: &[Span<'static>]| {
        let rem = width.saturating_sub(spans_width(prefix));
        if rem >= 2 {
            rem - 1
        } else {
            rem.max(1)
        }
    };

    let graphemes = spans_to_graphemes(spans, tab_width);
    if graphemes.is_empty() {
        let avail = code_width(&first_prefix);
        let mut row = first_prefix;
        row.push(Span::styled(
            " ".repeat(avail),
            Style::default().bg(code_bg),
        ));
        return vec![row];
    }

    let mut rows = Vec::new();
    let mut start = 0usize;
    let mut is_first = true;
    while start < graphemes.len() {
        let prefix = if is_first { &first_prefix } else { &cont_prefix };
        let avail = code_width(prefix);
        let slice = &graphemes[start..];

        let mut fit_len = 0usize;
        let mut fit_width = 0usize;
        let mut last_space: Option<(usize, usize)> = None;
        let mut saw_non_space = false;

        for (idx, g) in slice.iter().enumerate() {
            if fit_len > 0 && fit_width + g.width > avail {
                break;
            }
            if g.text == " " {
                if saw_non_space {
                    last_space = Some((idx, fit_width));
                }
            } else {
                saw_non_space = true;
            }
            fit_len = idx + 1;
            fit_width += g.width;
        }

        let (take_len, take_width, next_offset) = if start + fit_len >= graphemes.len() {
            (fit_len, fit_width, fit_len)
        } else if let Some((space_idx, width_before_space)) = last_space {
            let mut next = space_idx + 1;
            while next < slice.len() && slice[next].text == " " {
                next += 1;
            }
            (space_idx, width_before_space, next)
        } else {
            (fit_len, fit_width, fit_len)
        };

        let mut row = prefix.clone();
        row.extend(graphemes_to_spans(&slice[..take_len]));
        let pad = avail.saturating_sub(take_width);
        if pad > 0 {
            row.push(Span::styled(" ".repeat(pad), Style::default().bg(code_bg)));
        }
        rows.push(row);

        if next_offset == 0 {
            break;
        }
        start += next_offset;
        is_first = false;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(row: &[Span<'_>]) -> String {
        row.iter().map(|s| s.content.as_ref()).collect()
    }

    fn rows_text(rows: &[Vec<Span<'_>>]) -> Vec<String> {
        rows.iter().map(|r| row_text(r)).collect()
    }

    fn assert_width_bound(rows: &[Vec<Span<'_>>], width: usize) {
        for (idx, row) in rows.iter().enumerate() {
            let w = spans_width(row);
            assert!(
                w <= width,
                "row {} width {} exceeded max {}: {:?}",
                idx,
                w,
                width,
                row_text(row)
            );
            for span in row {
                assert!(
                    !span.content.contains('\n') && !span.content.contains('\r'),
                    "row {} contained raw newline in span {:?}",
                    idx,
                    span.content
                );
            }
        }
    }

    #[test]
    fn test_plain_text_wrapping_and_newlines() {
        let t = theme::get();
        let rows = render_markdown(
            "Hello world\nSecond line\n\nNew paragraph",
            20,
            t.ui.text_primary,
            "src/a.rs",
            4,
            t,
        );
        assert_eq!(
            rows_text(&rows),
            vec!["Hello world", "Second line", "", "New paragraph"]
        );
        assert_width_bound(&rows, 20);

        let tight = render_markdown(
            "Changes:\n- one\n- two",
            30,
            t.ui.text_primary,
            "src/a.rs",
            4,
            t,
        );
        assert_eq!(rows_text(&tight), vec!["Changes:", "• one", "• two"]);

        let spaced = render_markdown(
            "Changes:\n\n- one\n- two\n\nAfter list",
            30,
            t.ui.text_primary,
            "src/a.rs",
            4,
            t,
        );
        assert_eq!(
            rows_text(&spaced),
            vec!["Changes:", "", "• one", "• two", "", "After list"]
        );

        let loose = render_markdown(
            "- one\n\n- two",
            30,
            t.ui.text_primary,
            "src/a.rs",
            4,
            t,
        );
        assert_eq!(rows_text(&loose), vec!["• one", "", "• two"]);
    }

    #[test]
    fn test_inline_formatting_and_snake_case_preserved() {
        let t = theme::get();
        let rows = render_markdown(
            "Use **bold**, *italic*, ~~old~~, and foo_bar_baz with <thread-id>.",
            100,
            t.ui.text_primary,
            "src/a.py",
            4,
            t,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(
            row_text(&rows[0]),
            "Use bold, italic, old, and foo_bar_baz with <thread-id>."
        );
        assert!(rows[0]
            .iter()
            .any(|s| s.content == "bold" && s.style.add_modifier.contains(Modifier::BOLD)));
        assert!(rows[0]
            .iter()
            .any(|s| s.content == "italic" && s.style.add_modifier.contains(Modifier::ITALIC)));
        assert!(rows[0]
            .iter()
            .any(|s| s.content == "old" && s.style.add_modifier.contains(Modifier::CROSSED_OUT)));
        assert!(rows[0]
            .iter()
            .all(|s| !s.content.contains("bar") || !s.style.add_modifier.contains(Modifier::ITALIC)));
    }

    #[test]
    fn test_html_blocks_and_multiline_inline_html_split_lines_without_raw_newlines() {
        let t = theme::get();
        let body = "Before\n\n<details>\n<summary>Details</summary>\nBody\n</details>\n\n<thread-id>\n\nInline <!-- one\ntwo --> end";
        let rows = render_markdown(body, 40, t.ui.text_primary, "src/a.rs", 4, t);
        assert_width_bound(&rows, 40);
        assert_eq!(
            rows_text(&rows),
            vec![
                "Before",
                "",
                "<details>",
                "<summary>Details</summary>",
                "Body",
                "</details>",
                "",
                "<thread-id>",
                "",
                "Inline <!-- one",
                "two --> end",
            ]
        );
    }

    #[test]
    fn test_inline_code_highlighted_with_file_language_and_inherits_bold() {
        let t = theme::get();
        let rows = render_markdown(
            "Try `def xxx` and **`return 1`** here.",
            80,
            t.ui.text_primary,
            "src/app.py",
            4,
            t,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(row_text(&rows[0]), "Try def xxx and return 1 here.");

        let def_span = rows[0].iter().find(|s| s.content == "def").expect("def");
        assert_eq!(def_span.style.fg, Some(t.syntax.keyword));
        assert_eq!(def_span.style.bg, Some(t.ui.code_bg));

        let ret_span = rows[0]
            .iter()
            .find(|s| s.content == "return")
            .expect("return");
        assert_eq!(ret_span.style.fg, Some(t.syntax.keyword));
        assert_eq!(ret_span.style.bg, Some(t.ui.code_bg));
        assert!(ret_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_fenced_code_blocks_use_fence_language_or_file_fallback() {
        let t = theme::get();
        let body = "Bare:\n```\ndef foo():\n    return 1\n```\n\nTagged:\n```rust\nfn main() {}\n```";
        let rows = render_markdown(body, 40, t.ui.text_primary, "src/app.py", 4, t);
        assert_width_bound(&rows, 40);

        let def_row = rows
            .iter()
            .find(|r| row_text(r).contains("def foo():"))
            .expect("python row");
        assert!(def_row
            .iter()
            .any(|s| s.content == "def" && s.style.fg == Some(t.syntax.keyword)));
        assert!(def_row.iter().any(|s| s.style.bg == Some(t.ui.code_bg)));

        let fn_row = rows
            .iter()
            .find(|r| row_text(r).contains("fn main()"))
            .expect("rust row");
        assert!(fn_row
            .iter()
            .any(|s| s.content == "fn" && s.style.fg == Some(t.syntax.keyword)));
    }

    #[test]
    fn test_lists_headings_blockquotes_links_and_rules() {
        let t = theme::get();
        let body = "## Plan\n\n- first bullet item that wraps cleanly\n  - nested item\n1. numbered item\n- [x] done\n\n> quoted note\n\nSee [docs](https://example.com) and <https://example.com>.\n\n---";
        let rows = render_markdown(body, 28, t.ui.text_primary, "src/a.rs", 4, t);
        assert_width_bound(&rows, 28);

        let text = rows_text(&rows);
        assert_eq!(text[0], "Plan");
        assert!(rows[0][0].style.add_modifier.contains(Modifier::BOLD));
        assert!(text.iter().any(|line| line.starts_with("• first bullet")));
        assert!(text.iter().any(|line| line.starts_with("  wraps cleanly")));
        assert!(text.iter().any(|line| line.starts_with("  • nested")));
        assert!(text.iter().any(|line| line.starts_with("1. numbered")));
        let done_idx = text
            .iter()
            .position(|line| line.starts_with("☑ done"))
            .expect("done item");
        assert_eq!(text[done_idx + 1], "");
        assert!(text[done_idx + 2].starts_with("▎ quoted note"));
        let joined = text.join(" ");
        assert!(joined.contains("docs (https://example.com)"));
        assert!(!joined.contains("(https://example.com) (https://example.com)"));
        assert!(text.last().unwrap().chars().all(|c| c == '─'));
    }

    #[test]
    fn test_narrow_width_tabs_and_multi_codepoint_emojis_never_overflow() {
        let t = theme::get();
        let body = "⚠️⚠️⚠️⚠️⚠️ ⚠️ SuperLongUnbreakableIdentifier\twith\ttabs and 🧑‍💻\n\n```\n\tlet x = \"⚠️ 🧑‍💻 long code line\";\n```";
        for width in [6, 10, 16, 30] {
            let rows = render_markdown(body, width, t.ui.text_primary, "src/a.rs", 4, t);
            assert_width_bound(&rows, width);
            for row in &rows {
                let text = row_text(row);
                assert!(
                    !text.contains('\u{26A0}') || text.contains("⚠️"),
                    "warning emoji grapheme was split at width {}: {:?}",
                    width,
                    text
                );
            }
        }
    }
}
