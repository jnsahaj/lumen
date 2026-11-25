mod config;
mod queries;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use ratatui::prelude::*;
use tree_sitter_highlight::{HighlightEvent, Highlighter};

use super::theme;
use config::{config_for_extension, config_for_language, LanguageConfig, HIGHLIGHT_NAMES};

pub fn highlight_color(index: usize) -> Color {
    let t = theme::get();
    let syntax = &t.syntax;
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

/// Above this size, parsing a whole file takes long enough to be felt: about
/// 0.15 s per megabyte and per side.
const WHOLE_FILE_PARSE_LIMIT: usize = 1024 * 1024;

/// Whether a file with these two sides is too large to be parsed as a whole. Its
/// lines are then coloured one by one, which is instant but approximate: a string or
/// comment spanning several lines is not seen as one. Function lines are not pinned.
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

    let mut highlighter = Highlighter::new();
    let highlights = highlighter.highlight(&lang_config.config, code.as_bytes(), None, |_| None);

    let Ok(highlights) = highlights else {
        return code.lines().map(|l| (l.to_string(), None)).collect();
    };

    let mut result: Vec<(String, Option<usize>)> = Vec::new();
    let mut current_highlight: Option<usize> = None;

    for event in highlights.flatten() {
        match event {
            HighlightEvent::Source { start, end } => {
                let text = &code[start..end];
                result.push((text.to_string(), current_highlight));
            }
            HighlightEvent::HighlightStart(h) => {
                current_highlight = Some(h.0);
            }
            HighlightEvent::HighlightEnd => {
                current_highlight = None;
            }
        }
    }

    result
}

/// Pre-computed highlights for an entire file, organized by line number.
/// This allows proper highlighting of multi-line constructs like JSDoc comments.
#[derive(Default)]
pub struct FileHighlighter {
    /// Map from 1-based line number to list of (text, highlight_index) spans
    line_highlights: HashMap<usize, Vec<(String, Option<usize>)>>,
}

impl FileHighlighter {
    /// Create a new FileHighlighter by analyzing the entire file content.
    pub fn new(content: &str, filename: &str) -> Self {
        let Some(lang_config) = get_config_for_file(filename) else {
            return Self::default();
        };

        let mut highlighter = Highlighter::new();
        let highlights =
            highlighter.highlight(&lang_config.config, content.as_bytes(), None, |_| None);

        let Ok(highlights) = highlights else {
            return Self::default();
        };

        // Build a map of byte offset -> line number (1-based)
        let mut line_starts: Vec<usize> = vec![0];
        for (i, c) in content.char_indices() {
            if c == '\n' {
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
        let mut current_highlight: Option<usize> = None;

        for event in highlights.flatten() {
            match event {
                HighlightEvent::Source { start, end } => {
                    let text = &content[start..end];

                    // Split text by newlines and assign to correct lines
                    let start_line = byte_to_line(start);
                    let mut current_line = start_line;
                    let mut line_start = 0;

                    for (i, c) in text.char_indices() {
                        if c == '\n' {
                            let line_text = &text[line_start..i];
                            if !line_text.is_empty() {
                                line_highlights
                                    .entry(current_line)
                                    .or_default()
                                    .push((line_text.to_string(), current_highlight));
                            }
                            // Add the newline as a separate span (usually not displayed)
                            line_highlights
                                .entry(current_line)
                                .or_default()
                                .push(("\n".to_string(), current_highlight));
                            current_line += 1;
                            line_start = i + 1;
                        }
                    }

                    // Handle remaining text after last newline
                    if line_start < text.len() {
                        let remaining = &text[line_start..];
                        line_highlights
                            .entry(current_line)
                            .or_default()
                            .push((remaining.to_string(), current_highlight));
                    }
                }
                HighlightEvent::HighlightStart(h) => {
                    current_highlight = Some(h.0);
                }
                HighlightEvent::HighlightEnd => {
                    current_highlight = None;
                }
            }
        }

        Self { line_highlights }
    }

    /// Get highlighted spans for a specific line (1-based line number).
    pub fn get_line_spans<'a>(&self, line_number: usize, bg: Option<Color>) -> Vec<Span<'a>> {
        let bg_color = bg.unwrap_or(Color::Reset);
        let default_fg = theme::get().syntax.default_text;

        self.line_highlights
            .get(&line_number)
            .map(|spans| {
                spans
                    .iter()
                    .filter(|(text, _)| *text != "\n") // Skip newline markers
                    .map(|(text, highlight_idx)| {
                        let fg = highlight_idx.map(highlight_color).unwrap_or(default_fg);
                        Span::styled(text.clone(), Style::default().fg(fg).bg(bg_color))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Check if this highlighter has any highlights (i.e., was successfully created)
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.line_highlights.is_empty()
    }
}

/// Legacy function for single-line highlighting.
/// For multi-line constructs (like JSDoc comments), use FileHighlighter instead.
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

/// Highlights `code` line by line using `language` when provided and non-empty,
/// otherwise falling back to the language rules for `fallback_filename`.
/// Preserves every line (including empty lines) and merges `base_style` with
/// each token's syntax foreground color.
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
    fn test_rust_highlighting() {
        let code = r#"fn main() {
    let x = 42;
    println!("Hello");
}"#;
        let result = highlight_code(code, "test.rs");
        assert!(
            !result.is_empty(),
            "Rust highlighting should produce output"
        );
        let has_highlights = result.iter().any(|(_, h)| h.is_some());
        assert!(has_highlights, "Rust code should have syntax highlights");
    }

    #[test]
    fn test_typescript_highlighting() {
        let code = r#"const x: number = 42;
function hello(): string {
    return "world";
}"#;
        let result = highlight_code(code, "test.ts");
        assert!(
            !result.is_empty(),
            "TypeScript highlighting should produce output"
        );
        let has_highlights = result.iter().any(|(_, h)| h.is_some());
        assert!(
            has_highlights,
            "TypeScript code should have syntax highlights"
        );
    }

    #[test]
    fn test_python_highlighting() {
        let code = r#"def hello():
    x = 42
    return "world"
"#;
        let result = highlight_code(code, "test.py");
        assert!(
            !result.is_empty(),
            "Python highlighting should produce output"
        );
        let has_highlights = result.iter().any(|(_, h)| h.is_some());
        assert!(has_highlights, "Python code should have syntax highlights");
    }

    #[test]
    fn test_elixir_highlighting() {
        let code = r#"defmodule Hello do
  def greet(name) do
    IO.puts("Hello, #{name}!")
  end
end
"#;
        let result = highlight_code(code, "test.ex");
        assert!(!result.is_empty(), "Elixir highlighting should produce output");
        let has_highlights = result.iter().any(|(_, h)| h.is_some());
        assert!(has_highlights, "Elixir code should have syntax highlights");
    }

    #[test]
    fn test_java_highlighting() {
        let code = r#"package com.example;

public class Hello {
    public static void main(String[] args) {
        System.out.println("Hello, Java!");
    }
}
"#;
        let result = highlight_code(code, "test.java");
        assert!(!result.is_empty(), "Java highlighting should produce output");
        let has_highlights = result.iter().any(|(_, h)| h.is_some());
        assert!(has_highlights, "Java code should have syntax highlights");
    }

    #[test]
    fn test_zig_highlighting() {
        let code = r#"const std = @import("std");

pub fn main() !void {
    const answer: u32 = 42;
    std.debug.print("Hello, Zig!\n", .{});
}
"#;
        let result = highlight_code(code, "test.zig");
        assert!(
            !result.is_empty(),
            "Zig highlighting should produce output"
        );
        let has_highlights = result.iter().any(|(_, h)| h.is_some());
        assert!(has_highlights, "Zig code should have syntax highlights");
    }

    #[test]
    fn test_zig_file_highlighter_payload_binding() {
        use config::HIGHLIGHT_NAMES;

        let code = r#"const std = @import("std");

pub fn unwrap(maybe: ?u32) void {
    if (maybe) |value| {
        std.debug.print("{}\n", .{value});
    }
}
"#;

        let highlighter = FileHighlighter::new(code, "test.zig");
        assert!(!highlighter.is_empty(), "Highlighter should have content");

        let parameter_idx = HIGHLIGHT_NAMES
            .iter()
            .position(|&n| n == "variable.parameter")
            .expect("variable.parameter highlight should exist");
        let parameter_color = highlight_color(parameter_idx);

        let payload_line = highlighter.get_line_spans(4, None);
        let raw_result = highlight_code(code, "test.zig");
        let payload_span = payload_line
            .iter()
            .find(|span| span.content == "value")
            .expect("Payload binding should appear on line 4");

        let payload_binding = raw_result
            .windows(3)
            .find(|window| window[0].0 == "|" && window[1].0 == "value" && window[2].0 == "|")
            .expect("Payload binding should be tokenized as |value|");

        assert_eq!(
            payload_binding[1].1,
            Some(parameter_idx),
            "Payload binding should be highlighted as variable.parameter"
        );

        assert_eq!(
            payload_span.style.fg,
            Some(parameter_color),
            "FileHighlighter should preserve the payload binding highlight"
        );
    }

    #[test]
    fn test_c_highlighting() {
        let code = r#"#include <stdio.h>

int main(void) {
    int x = 42;
    printf("Hello, %d\n", x);
    return 0;
}
"#;
        let result = highlight_code(code, "test.c");
        assert!(
            !result.is_empty(),
            "C highlighting should produce output"
        );
        let has_highlights = result.iter().any(|(_, h)| h.is_some());
        assert!(has_highlights, "C code should have syntax highlights");
    }

    #[test]
    fn test_cpp_highlighting() {
        use config::HIGHLIGHT_NAMES;
        let code = r#"#include <vector>

namespace ns {
    template<typename T>
    T add(T a, T b) {
        return a + b;
    }
}

int main() {
    return ns::add<int>(1, 2);
}
"#;
        let result = highlight_code(code, "test.cpp");
        assert!(
            !result.is_empty(),
            "C++ highlighting should produce output"
        );
        let has_highlights = result.iter().any(|(_, h)| h.is_some());
        assert!(has_highlights, "C++ code should have syntax highlights");

        let keyword_idx = HIGHLIGHT_NAMES.iter().position(|&n| n == "keyword");
        let has_template_keyword = result
            .iter()
            .any(|(t, h)| *t == "template" && *h == keyword_idx);
        assert!(
            has_template_keyword,
            "C++ 'template' should be highlighted as keyword"
        );

        let has_namespace_keyword = result
            .iter()
            .any(|(t, h)| *t == "namespace" && *h == keyword_idx);
        assert!(
            has_namespace_keyword,
            "C++ 'namespace' should be highlighted as keyword"
        );
    }

    #[test]
    fn test_rust_comment_highlighting() {
        use config::HIGHLIGHT_NAMES;
        // Test all Rust comment types plus operators that could conflict
        let code = r#"/// Outer doc comment
//! Inner doc comment  
// Regular comment
/* Block comment */
/** Outer doc block */
/*! Inner doc block */
fn main() {
    let x = 1 / 2;
    let y = !true;
}"#;
        let result = highlight_code(code, "test.rs");

        let comment_idx = HIGHLIGHT_NAMES.iter().position(|&n| n == "comment");
        let operator_idx = HIGHLIGHT_NAMES.iter().position(|&n| n == "operator");

        // Check that doc comment "/" marker is highlighted as comment (not operator)
        let doc_slash = result.iter().find(|(t, _)| *t == "/").map(|(_, h)| h);
        assert_eq!(
            doc_slash,
            Some(&comment_idx),
            "Doc comment '/' should be highlighted as comment"
        );

        // Check division operator is still highlighted correctly
        let has_div_operator = result
            .iter()
            .any(|(t, h)| t.contains("/") && *h == operator_idx);
        assert!(
            has_div_operator,
            "Division '/' in expressions should be highlighted as operator"
        );

        // Check negation operator is still highlighted correctly
        let has_neg_operator = result
            .iter()
            .any(|(t, h)| t.contains("!") && *h == operator_idx);
        assert!(
            has_neg_operator,
            "Negation '!' should be highlighted as operator"
        );
    }

    #[test]
    fn test_ts_comment_highlighting() {
        use config::HIGHLIGHT_NAMES;
        // Test real-world JSDoc multi-line comment
        let code = r#"/**
 * Clone a git-backed template into a new chat's sandbox.
 *
 * Only clones the source files - repo creation and push happens lazily
 * via tryCreateAndPushNewRepo on first auto-push.
 *
 * Orchestrates:
 * 1. Generate chat ID and get sandbox
 * 2. Clone source template to CLEAN directory
 * 3. Remove .git directory (so it's treated as fresh files)
 */
function foo() {}"#;
        let result = highlight_code(code, "test.ts");

        let comment_idx = HIGHLIGHT_NAMES.iter().position(|&n| n == "comment");
        let has_comment_highlight = result.iter().any(|(_, h)| *h == comment_idx);
        assert!(
            has_comment_highlight,
            "Comments should be highlighted as 'comment'"
        );
    }

    #[test]
    fn test_file_highlighter_multiline_jsdoc() {
        use config::HIGHLIGHT_NAMES;
        // Test that FileHighlighter properly handles multi-line JSDoc comments
        let code = r#"/**
 * Clone a git-backed template into a new chat's sandbox.
 *
 * Only clones the source files - repo creation and push happens lazily
 * via tryCreateAndPushNewRepo on first auto-push.
 *
 * Orchestrates:
 * 1. Generate chat ID and get sandbox
 * 2. Clone source template to CLEAN directory
 * 3. Remove .git directory (so it's treated as fresh files)
 */
function foo() {}"#;

        let highlighter = FileHighlighter::new(code, "test.ts");
        assert!(!highlighter.is_empty(), "Highlighter should have content");

        let comment_idx = HIGHLIGHT_NAMES.iter().position(|&n| n == "comment");

        // Check that each line of the comment is highlighted as a comment
        // Lines 1-11 should be comment, line 12 should be code
        for line_num in 1..=11 {
            let spans = highlighter.get_line_spans(line_num, None);
            // Each line should have at least one span
            assert!(!spans.is_empty(), "Line {} should have spans", line_num);
            // All spans should be highlighted as comment
            for span in &spans {
                let fg = span.style.fg;
                let comment_color = highlight_color(comment_idx.unwrap());
                assert_eq!(
                    fg,
                    Some(comment_color),
                    "Line {} should be highlighted as comment, got {:?}",
                    line_num,
                    fg
                );
            }
        }

        // Line 12 should contain "function" which is a keyword
        let line12_spans = highlighter.get_line_spans(12, None);
        let has_function = line12_spans.iter().any(|s| s.content.contains("function"));
        assert!(has_function, "Line 12 should contain 'function'");
    }

    #[test]
    fn test_highlight_code_lines_for_inline_fragments_and_fences() {
        let t = theme::get();
        let bg = t.ui.code_bg;
        let base = Style::default().bg(bg).bold();

        let py_lines = highlight_code_lines("def xxx", None, "src/app.py", base);
        assert_eq!(py_lines.len(), 1);
        let def_span = py_lines[0]
            .iter()
            .find(|s| s.content == "def")
            .expect("def token");
        assert_eq!(def_span.style.fg, Some(t.syntax.keyword));
        assert_eq!(def_span.style.bg, Some(bg));
        assert!(def_span.style.add_modifier.contains(Modifier::BOLD));

        let ts_lines =
            highlight_code_lines("zero(\"EUR\") + this.subtotal()", None, "src/cart.ts", Style::default());
        assert_eq!(ts_lines.len(), 1);
        assert!(ts_lines[0]
            .iter()
            .any(|s| s.content == "zero" && s.style.fg == Some(t.syntax.function)));
        assert!(ts_lines[0]
            .iter()
            .any(|s| s.content == "\"EUR\"" && s.style.fg == Some(t.syntax.string)));
        assert!(ts_lines[0]
            .iter()
            .any(|s| s.content == "subtotal" && s.style.fg == Some(t.syntax.variable_member)));

        let override_lines =
            highlight_code_lines("fn main() {}\n\nlet x = 1;", Some("rust"), "src/app.py", Style::default());
        assert_eq!(override_lines.len(), 3);
        assert!(override_lines[0]
            .iter()
            .any(|s| s.content == "fn" && s.style.fg == Some(t.syntax.keyword)));
        assert!(override_lines[1].is_empty());

        let unknown_lines =
            highlight_code_lines("SELECT * FROM t", Some("sql"), "src/app.py", Style::default().bg(bg));
        assert_eq!(unknown_lines.len(), 1);
        assert_eq!(unknown_lines[0][0].style.fg, Some(t.syntax.default_text));
        assert_eq!(unknown_lines[0][0].style.bg, Some(bg));
    }
}
