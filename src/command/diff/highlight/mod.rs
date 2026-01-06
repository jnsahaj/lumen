mod config;
mod queries;

use std::collections::HashMap;
use std::path::Path;

use ratatui::prelude::*;
use tree_sitter_highlight::{HighlightEvent, Highlighter};

use super::theme;
use config::{LanguageConfig, CONFIGS, HIGHLIGHT_NAMES};

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
    CONFIGS.iter().find(|(e, _)| *e == ext).map(|(_, c)| c)
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

pub fn init() {
    let _ = &*CONFIGS;
    #[cfg(debug_assertions)]
    {
        let extensions: Vec<&str> = CONFIGS.iter().map(|(ext, _)| *ext).collect();
        eprintln!("[DEBUG] Loaded highlight configs for: {:?}", extensions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_configs_load() {
        let extensions: Vec<&str> = CONFIGS.iter().map(|(ext, _)| *ext).collect();
        assert!(extensions.contains(&"rs"), "Rust config should be loaded");
        assert!(
            extensions.contains(&"ts"),
            "TypeScript config should be loaded"
        );
        assert!(extensions.contains(&"tsx"), "TSX config should be loaded");
        assert!(
            extensions.contains(&"js"),
            "JavaScript config should be loaded"
        );
        assert!(extensions.contains(&"py"), "Python config should be loaded");
        assert!(extensions.contains(&"go"), "Go config should be loaded");
        assert!(extensions.contains(&"json"), "JSON config should be loaded");
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
