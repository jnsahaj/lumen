mod cache;
mod config;
mod queries;

use std::path::Path;
use std::sync::Arc;

use ratatui::prelude::*;
use tree_sitter_highlight::{HighlightEvent, Highlighter};

use super::theme;
use config::{LanguageConfig, HIGHLIGHT_NAMES, LANGUAGES};

pub use cache::HighlightCache;

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

fn get_config_for_file(filename: &str) -> Option<&'static LanguageConfig> {
    let ext = Path::new(filename).extension().and_then(|e| e.to_str())?;
    LANGUAGES
        .iter()
        .find(|language| language.extensions.contains(&ext))?
        .config()
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

/// Marks a span with no highlight in `FileHighlighter::span_highlights`.
const NO_HIGHLIGHT: u8 = u8::MAX;

/// Pre-computed highlights for an entire file, organized by line number.
/// This allows proper highlighting of multi-line constructs like JSDoc comments.
///
/// Spans are stored as byte ranges into the file content rather than one
/// `String` per token, which keeps a highlighter at roughly the size of the
/// source instead of ~10x.
#[derive(Default)]
pub struct FileHighlighter {
    content: Arc<str>,
    /// (start, end) byte offsets into `content`, in source order. A span
    /// never crosses a line break.
    spans: Vec<(u32, u32)>,
    /// Index into `HIGHLIGHT_NAMES` for each span, or `NO_HIGHLIGHT`.
    span_highlights: Vec<u8>,
    /// `line_spans[n - 1]` is the index of the first span on 1-based line `n`.
    line_spans: Vec<u32>,
}

impl FileHighlighter {
    /// Create a new FileHighlighter by analyzing the entire file content.
    #[cfg(test)]
    pub fn new(content: &str, filename: &str) -> Self {
        Self::from_shared(Arc::from(content), filename)
    }

    /// Like `new`, but keeps a reference to `content` instead of copying it.
    pub fn from_shared(content: Arc<str>, filename: &str) -> Self {
        let Some(lang_config) = get_config_for_file(filename) else {
            return Self::default();
        };
        // Offsets are stored as u32.
        if content.len() > u32::MAX as usize {
            return Self::default();
        }
        let Some((spans, span_highlights, line_spans)) = Self::build_spans(&content, lang_config)
        else {
            return Self::default();
        };

        Self {
            content,
            spans,
            span_highlights,
            line_spans,
        }
    }

    #[allow(clippy::type_complexity)]
    fn build_spans(
        content: &str,
        lang_config: &LanguageConfig,
    ) -> Option<(Vec<(u32, u32)>, Vec<u8>, Vec<u32>)> {
        let mut highlighter = Highlighter::new();
        let highlights = highlighter
            .highlight(&lang_config.config, content.as_bytes(), None, |_| None)
            .ok()?;

        // Build a map of byte offset -> line number (1-based)
        let mut line_starts: Vec<usize> = vec![0];
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

        let mut spans: Vec<(u32, u32)> = Vec::new();
        let mut span_highlights: Vec<u8> = Vec::new();
        let mut line_spans: Vec<u32> = Vec::new();
        let mut push = |line: usize, start: usize, end: usize, highlight: Option<usize>| {
            if start == end {
                return;
            }
            while line_spans.len() < line {
                line_spans.push(spans.len() as u32);
            }
            spans.push((start as u32, end as u32));
            span_highlights.push(highlight.map_or(NO_HIGHLIGHT, |h| h as u8));
        };
        let mut current_highlight: Option<usize> = None;

        for event in highlights.flatten() {
            match event {
                HighlightEvent::Source { start, end } => {
                    // Split text by newlines and assign to correct lines
                    let mut current_line = byte_to_line(start);
                    let mut line_start = start;

                    for (i, b) in content.as_bytes()[start..end].iter().enumerate() {
                        if *b == b'\n' {
                            push(current_line, line_start, start + i, current_highlight);
                            current_line += 1;
                            line_start = start + i + 1;
                        }
                    }

                    // Handle remaining text after last newline
                    push(current_line, line_start, end, current_highlight);
                }
                HighlightEvent::HighlightStart(h) => {
                    current_highlight = Some(h.0);
                }
                HighlightEvent::HighlightEnd => {
                    current_highlight = None;
                }
            }
        }

        Some((spans, span_highlights, line_spans))
    }

    /// (text, highlight index) for each span on a 1-based line.
    fn line_tokens(&self, line_number: usize) -> impl Iterator<Item = (&str, Option<usize>)> {
        let bound = |line: usize| {
            self.line_spans
                .get(line)
                .map_or(self.spans.len(), |&i| i as usize)
        };
        let range = match line_number.checked_sub(1) {
            Some(line) => bound(line)..bound(line + 1),
            None => 0..0,
        };
        range.map(|i| {
            let (start, end) = self.spans[i];
            let highlight = self.span_highlights[i];
            (
                &self.content[start as usize..end as usize],
                (highlight != NO_HIGHLIGHT).then_some(highlight as usize),
            )
        })
    }

    /// Get highlighted spans for a specific line (1-based line number).
    pub fn get_line_spans<'a>(&self, line_number: usize, bg: Option<Color>) -> Vec<Span<'a>> {
        let bg_color = bg.unwrap_or(Color::Reset);
        let default_fg = theme::get().syntax.default_text;

        self.line_tokens(line_number)
            .map(|(text, highlight_idx)| {
                let fg = highlight_idx.map(highlight_color).unwrap_or(default_fg);
                Span::styled(text.to_string(), Style::default().fg(fg).bg(bg_color))
            })
            .collect()
    }

    /// Check if this highlighter has any highlights (i.e., was successfully created)
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
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

/// Compile the config for `filename`'s language now, so tests with deadlines
/// don't also time the query compilation.
#[cfg(test)]
pub fn preload(filename: &str) {
    let _ = get_config_for_file(filename);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The original one-`String`-per-token construction, kept as the oracle
    /// for the compact representation.
    fn reference_line_tokens(
        content: &str,
        filename: &str,
    ) -> std::collections::HashMap<usize, Vec<(String, Option<usize>)>> {
        let config = get_config_for_file(filename).unwrap();
        let mut highlighter = Highlighter::new();
        let events = highlighter
            .highlight(&config.config, content.as_bytes(), None, |_| None)
            .unwrap();
        let mut lines = std::collections::HashMap::<usize, Vec<_>>::new();
        let mut current = None;
        for event in events.flatten() {
            match event {
                HighlightEvent::Source { start, end } => {
                    let mut line = content[..start].matches('\n').count() + 1;
                    for (i, piece) in content[start..end].split('\n').enumerate() {
                        if i > 0 {
                            line += 1;
                        }
                        if !piece.is_empty() {
                            lines
                                .entry(line)
                                .or_default()
                                .push((piece.to_string(), current));
                        }
                    }
                }
                HighlightEvent::HighlightStart(h) => current = Some(h.0),
                HighlightEvent::HighlightEnd => current = None,
            }
        }
        lines
    }

    #[test]
    fn test_file_highlighter_matches_reference() {
        let samples = [
            ("mod.rs", include_str!("mod.rs").to_string()),
            ("config.rs", include_str!("config.rs").to_string()),
            ("block.ts", "/**\n * doc\n *\n */\n\n\nconst s = `a\nb`;".to_string()),
            ("crlf.rs", "fn main() {\r\n    let x = 1;\r\n}\r\n".to_string()),
            ("unicode.py", "# héllo wörld ✓\nx = \"日本語\"\n\n".to_string()),
            ("empty.rs", String::new()),
        ];
        for (filename, content) in samples {
            let highlighter = FileHighlighter::new(&content, filename);
            let reference = reference_line_tokens(&content, filename);
            // One past the end and line 0 must come back empty too.
            for line in 0..=content.lines().count() + 2 {
                let actual: Vec<(String, Option<usize>)> = highlighter
                    .line_tokens(line)
                    .map(|(text, h)| (text.to_string(), h))
                    .collect();
                let expected = reference.get(&line).cloned().unwrap_or_default();
                assert_eq!(actual, expected, "{filename} line {line}");
            }
        }
    }

    #[test]
    fn test_all_configs_load() {
        for language in &LANGUAGES {
            assert!(
                language.config().is_some(),
                "config for {:?} should load",
                language.extensions
            );
        }
        for ext in [
            "ts", "tsx", "js", "jsx", "rs", "json", "py", "go", "css", "html", "toml", "sh",
            "bash", "md", "mdx", "cs", "rb", "ex", "exs", "java", "zig", "c", "h", "cpp", "cc",
            "cxx", "hpp", "hh", "hxx",
        ] {
            assert!(
                get_config_for_file(&format!("file.{ext}")).is_some(),
                ".{ext} should have a highlight config"
            );
        }
    }

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
}
