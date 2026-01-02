mod config;
mod queries;

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
}
