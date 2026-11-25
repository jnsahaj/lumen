mod config;
mod queries;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use ratatui::prelude::*;
use tree_sitter_highlight::{HighlightEvent, Highlighter};

use crate::ui::theme;
use config::{config_for_extension, config_for_language, LanguageConfig, HIGHLIGHT_NAMES};

pub fn highlight_color(index: usize) -> Color {
    let syntax = &theme::get().syntax;
    match HIGHLIGHT_NAMES.get(index) {
        Some(&"comment") => syntax.comment,
        Some(&"keyword") => syntax.keyword,
        Some(&"string" | &"string.special") => syntax.string,
        Some(&"number" | &"constant" | &"constant.builtin") => syntax.number,
        Some(&"function" | &"function.builtin" | &"function.method") => syntax.function,
        Some(&"function.macro") => syntax.function_macro,
        Some(&"type" | &"type.builtin" | &"constructor") => syntax.r#type,
        Some(&"variable.builtin") => syntax.variable_builtin,
        Some(&"variable.member" | &"property") => syntax.variable_member,
        Some(&"module") => syntax.module,
        Some(&"operator") => syntax.operator,
        Some(&"tag") => syntax.tag,
        Some(&"attribute") => syntax.attribute,
        Some(&"label") => syntax.label,
        Some(&"punctuation" | &"punctuation.bracket" | &"punctuation.delimiter") => {
            syntax.punctuation
        }
        _ => syntax.default_text,
    }
}

/// Above 1 MB, whole-file parsing takes long enough to be felt (~0.15s/MB per side).
const WHOLE_FILE_PARSE_LIMIT: usize = 1024 * 1024;

pub fn too_large_to_parse(old_content: &str, new_content: &str) -> bool {
    old_content.len().max(new_content.len()) > WHOLE_FILE_PARSE_LIMIT
}

fn get_config_for_file(filename: &str) -> Option<&'static LanguageConfig> {
    let ext = Path::new(filename).extension().and_then(|e| e.to_str())?;
    config_for_extension(ext)
}

fn highlight_code(code: &str, filename: &str) -> Vec<(String, Option<usize>)> {
    let Some(lang_config) = get_config_for_file(filename) else {
        return code.lines().map(|l| (l.to_string(), None)).collect();
    };

    SNIPPET_HIGHLIGHTER.with(|cell| {
        let mut highlighter = cell.borrow_mut();
        let Ok(highlights) =
            highlighter.highlight(&lang_config.config, code.as_bytes(), None, |_| None)
        else {
            return code.lines().map(|l| (l.to_string(), None)).collect();
        };

        let mut result = Vec::new();
        let mut stack: Vec<usize> = Vec::new();

        for event in highlights.flatten() {
            match event {
                HighlightEvent::Source { start, end } => {
                    result.push((code[start..end].to_string(), stack.last().copied()));
                }
                HighlightEvent::HighlightStart(h) => stack.push(h.0),
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
            }
        }
        result
    })
}

/// Pre-computed highlights for an entire file, organized by 1-based line number.
#[derive(Default)]
pub struct FileHighlighter {
    line_highlights: HashMap<usize, Vec<(String, Option<usize>)>>,
}

impl FileHighlighter {
    pub fn new(content: &str, filename: &str) -> Self {
        let Some(lang_config) = get_config_for_file(filename) else {
            return Self::default();
        };

        let mut highlighter = Highlighter::new();
        let Ok(highlights) =
            highlighter.highlight(&lang_config.config, content.as_bytes(), None, |_| None)
        else {
            return Self::default();
        };

        let mut line_starts = vec![0usize];
        for (i, b) in content.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }

        let byte_to_line = |byte_offset: usize| -> usize {
            match line_starts.binary_search(&byte_offset) {
                Ok(line) => line + 1,
                Err(line) => line,
            }
        };

        let mut line_highlights: HashMap<usize, Vec<(String, Option<usize>)>> = HashMap::new();
        let mut stack: Vec<usize> = Vec::new();

        for event in highlights.flatten() {
            match event {
                HighlightEvent::Source { start, end } => {
                    let text = &content[start..end];
                    let highlight = stack.last().copied();
                    let mut current_line = byte_to_line(start);

                    for (part_idx, part) in text.split('\n').enumerate() {
                        if part_idx > 0 {
                            current_line += 1;
                        }
                        if !part.is_empty() {
                            line_highlights
                                .entry(current_line)
                                .or_default()
                                .push((part.to_string(), highlight));
                        }
                    }
                }
                HighlightEvent::HighlightStart(h) => stack.push(h.0),
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
            }
        }

        Self { line_highlights }
    }

    pub fn get_line_spans<'a>(&self, line_number: usize, bg: Option<Color>) -> Vec<Span<'a>> {
        let bg_color = bg.unwrap_or(Color::Reset);
        let default_fg = theme::get().syntax.default_text;

        self.line_highlights
            .get(&line_number)
            .map(|spans| {
                spans
                    .iter()
                    .map(|(text, highlight_idx)| {
                        let fg = highlight_idx.map(highlight_color).unwrap_or(default_fg);
                        Span::styled(text.clone(), Style::default().fg(fg).bg(bg_color))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.line_highlights.is_empty()
    }
}

pub fn highlight_line_spans<'a>(line: &str, filename: &str, bg: Option<Color>) -> Vec<Span<'a>> {
    let highlighted = highlight_code(line, filename);
    let bg_color = bg.unwrap_or(Color::Reset);
    let default_fg = theme::get().syntax.default_text;

    highlighted
        .into_iter()
        .map(|(text, highlight_idx)| {
            let fg = highlight_idx.map(highlight_color).unwrap_or(default_fg);
            Span::styled(text, Style::default().fg(fg).bg(bg_color))
        })
        .collect()
}

thread_local! {
    static SNIPPET_HIGHLIGHTER: RefCell<Highlighter> = RefCell::new(Highlighter::new());
}

fn plain_code_lines(code: &str, base_style: Style) -> Vec<Vec<Span<'static>>> {
    let style = base_style.fg(theme::get().syntax.default_text);
    code.split('\n')
        .map(|line| {
            if line.is_empty() {
                Vec::new()
            } else {
                vec![Span::styled(line.to_string(), style)]
            }
        })
        .collect()
}

/// Highlights `code` line by line using `language` when provided, else `fallback_filename`.
pub fn highlight_code_lines(
    code: &str,
    language: Option<&str>,
    fallback_filename: &str,
    base_style: Style,
) -> Vec<Vec<Span<'static>>> {
    let lang_config = match language.map(str::trim).filter(|tag| !tag.is_empty()) {
        Some(tag) => config_for_language(tag),
        None => get_config_for_file(fallback_filename),
    };
    let Some(lang_config) = lang_config else {
        return plain_code_lines(code, base_style);
    };

    let default_fg = theme::get().syntax.default_text;
    SNIPPET_HIGHLIGHTER.with(|cell| {
        let mut highlighter = cell.borrow_mut();
        let Ok(highlights) =
            highlighter.highlight(&lang_config.config, code.as_bytes(), None, |_| None)
        else {
            return plain_code_lines(code, base_style);
        };

        let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new()];
        let mut stack: Vec<usize> = Vec::new();

        for event in highlights.flatten() {
            match event {
                HighlightEvent::Source { start, end } => {
                    let fg = stack
                        .last()
                        .copied()
                        .map(highlight_color)
                        .unwrap_or(default_fg);
                    let style = base_style.fg(fg);
                    let text = &code[start..end];
                    for (i, part) in text.split('\n').enumerate() {
                        if i > 0 {
                            lines.push(Vec::new());
                        }
                        if !part.is_empty() {
                            lines
                                .last_mut()
                                .unwrap()
                                .push(Span::styled(part.to_string(), style));
                        }
                    }
                }
                HighlightEvent::HighlightStart(h) => stack.push(h.0),
                HighlightEvent::HighlightEnd => {
                    stack.pop();
                }
            }
        }
        lines
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_doc_comment_and_operator_highlighting() {
        let code =
            "/// Outer doc\n//! Inner doc\nfn main() {\n    let x = 1 / 2;\n    let y = !true;\n}";
        let result = highlight_code(code, "test.rs");
        let comment_idx = HIGHLIGHT_NAMES.iter().position(|&n| n == "comment");
        let operator_idx = HIGHLIGHT_NAMES.iter().position(|&n| n == "operator");

        assert_eq!(
            result.iter().find(|(t, _)| *t == "/").map(|(_, h)| h),
            Some(&comment_idx)
        );
        assert!(result
            .iter()
            .any(|(t, h)| t.contains('/') && *h == operator_idx));
        assert!(result
            .iter()
            .any(|(t, h)| t.contains('!') && *h == operator_idx));
    }

    #[test]
    fn test_file_highlighter_multiline_jsdoc() {
        let code = "/**\n * Line 2\n * Line 3\n */\nfunction foo() {}";
        let highlighter = FileHighlighter::new(code, "test.ts");
        assert!(!highlighter.is_empty());

        let comment_color = highlight_color(
            HIGHLIGHT_NAMES
                .iter()
                .position(|&n| n == "comment")
                .unwrap(),
        );
        for line_num in 1..=4 {
            let spans = highlighter.get_line_spans(line_num, None);
            assert!(!spans.is_empty());
            assert!(spans.iter().all(|s| s.style.fg == Some(comment_color)));
        }
        let line5 = highlighter.get_line_spans(5, None);
        assert!(line5.iter().any(|s| s.content.contains("function")));
    }

    #[test]
    fn test_zig_payload_binding_and_cpp_keywords() {
        let zig = "pub fn unwrap(maybe: ?u32) void {\n    if (maybe) |value| {}\n}\n";
        let param_idx = HIGHLIGHT_NAMES
            .iter()
            .position(|&n| n == "variable.parameter")
            .unwrap();
        let raw = highlight_code(zig, "test.zig");
        let binding = raw
            .windows(3)
            .find(|w| w[0].0 == "|" && w[1].0 == "value" && w[2].0 == "|")
            .expect("payload binding");
        assert_eq!(binding[1].1, Some(param_idx));

        let cpp = "namespace ns { template<typename T> T add(T a, T b) { return a + b; } }";
        let kw_idx = HIGHLIGHT_NAMES.iter().position(|&n| n == "keyword");
        let cpp_hl = highlight_code(cpp, "test.cpp");
        assert!(cpp_hl.iter().any(|(t, h)| t == "template" && *h == kw_idx));
        assert!(cpp_hl.iter().any(|(t, h)| t == "namespace" && *h == kw_idx));
    }

    #[test]
    fn test_highlight_code_lines_for_inline_fragments_and_fences() {
        let t = theme::get();
        let bg = t.ui.code_bg;
        let base = Style::default().bg(bg).bold();

        let py_lines = highlight_code_lines("def xxx", None, "src/app.py", base);
        let def_span = py_lines[0].iter().find(|s| s.content == "def").unwrap();
        assert_eq!(def_span.style.fg, Some(t.syntax.keyword));
        assert_eq!(def_span.style.bg, Some(bg));
        assert!(def_span.style.add_modifier.contains(Modifier::BOLD));

        let override_lines = highlight_code_lines(
            "fn main() {}\n\nlet x = 1;",
            Some("rust"),
            "src/app.py",
            Style::default(),
        );
        assert_eq!(override_lines.len(), 3);
        assert!(override_lines[0]
            .iter()
            .any(|s| s.content == "fn" && s.style.fg == Some(t.syntax.keyword)));
        assert!(override_lines[1].is_empty());

        let unknown_lines = highlight_code_lines(
            "SELECT * FROM t",
            Some("sql"),
            "src/app.py",
            Style::default().bg(bg),
        );
        assert_eq!(unknown_lines[0][0].style.fg, Some(t.syntax.default_text));
        assert_eq!(unknown_lines[0][0].style.bg, Some(bg));
    }
}
