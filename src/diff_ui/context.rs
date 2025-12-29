use std::path::Path;

use once_cell::sync::Lazy;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

/// Configuration for context lines feature
#[derive(Clone)]
pub struct ContextConfig {
    pub enabled: bool,
    pub max_lines: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_lines: 5,
        }
    }
}

/// Represents a context line with its content and original line number
#[derive(Clone, Debug)]
pub struct ContextLine {
    pub line_number: usize,
    pub content: String,
}

// Context queries for each supported language
// These define which AST nodes should be shown as context

const RUST_CONTEXT_QUERY: &str = r#"
(function_item) @context
(impl_item) @context
(struct_item) @context
(enum_item) @context
(trait_item) @context
(mod_item) @context
(match_expression) @context
(for_expression) @context
(while_expression) @context
(loop_expression) @context
(if_expression) @context
(closure_expression) @context
"#;

const TYPESCRIPT_CONTEXT_QUERY: &str = r#"
(function_declaration) @context
(method_definition) @context
(class_declaration) @context
(interface_declaration) @context
(for_statement) @context
(for_in_statement) @context
(while_statement) @context
(do_statement) @context
(if_statement) @context
(switch_statement) @context
(try_statement) @context
(arrow_function) @context
(function_expression) @context
"#;

const JAVASCRIPT_CONTEXT_QUERY: &str = r#"
(function_declaration) @context
(method_definition) @context
(class_declaration) @context
(for_statement) @context
(for_in_statement) @context
(while_statement) @context
(do_statement) @context
(if_statement) @context
(switch_statement) @context
(try_statement) @context
(arrow_function) @context
(function_expression) @context
"#;

const PYTHON_CONTEXT_QUERY: &str = r#"
(function_definition) @context
(class_definition) @context
(for_statement) @context
(while_statement) @context
(if_statement) @context
(try_statement) @context
(with_statement) @context
(match_statement) @context
"#;

const GO_CONTEXT_QUERY: &str = r#"
(function_declaration) @context
(method_declaration) @context
(type_declaration) @context
(for_statement) @context
(if_statement) @context
(switch_statement) @context
(select_statement) @context
"#;

struct LanguageContext {
    language: Language,
    query: Query,
}

static LANGUAGE_CONTEXTS: Lazy<Vec<(&'static str, LanguageContext)>> = Lazy::new(|| {
    let mut contexts = Vec::new();

    // Rust
    let rust_lang: Language = tree_sitter_rust::LANGUAGE.into();
    if let Ok(query) = Query::new(&rust_lang, RUST_CONTEXT_QUERY) {
        contexts.push((
            "rs",
            LanguageContext {
                language: rust_lang,
                query,
            },
        ));
    }

    // TypeScript
    let ts_lang: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    if let Ok(query) = Query::new(&ts_lang, TYPESCRIPT_CONTEXT_QUERY) {
        contexts.push((
            "ts",
            LanguageContext {
                language: ts_lang,
                query,
            },
        ));
    }

    // TSX
    let tsx_lang: Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    if let Ok(query) = Query::new(&tsx_lang, TYPESCRIPT_CONTEXT_QUERY) {
        contexts.push((
            "tsx",
            LanguageContext {
                language: tsx_lang,
                query,
            },
        ));
    }

    // JavaScript
    let js_lang: Language = tree_sitter_javascript::LANGUAGE.into();
    if let Ok(query) = Query::new(&js_lang, JAVASCRIPT_CONTEXT_QUERY) {
        contexts.push((
            "js",
            LanguageContext {
                language: js_lang.clone(),
                query,
            },
        ));
    }

    // JSX (same as JS)
    if let Ok(query) = Query::new(&js_lang, JAVASCRIPT_CONTEXT_QUERY) {
        contexts.push((
            "jsx",
            LanguageContext {
                language: js_lang,
                query,
            },
        ));
    }

    // Python
    let py_lang: Language = tree_sitter_python::LANGUAGE.into();
    if let Ok(query) = Query::new(&py_lang, PYTHON_CONTEXT_QUERY) {
        contexts.push((
            "py",
            LanguageContext {
                language: py_lang,
                query,
            },
        ));
    }

    // Go
    let go_lang: Language = tree_sitter_go::LANGUAGE.into();
    if let Ok(query) = Query::new(&go_lang, GO_CONTEXT_QUERY) {
        contexts.push((
            "go",
            LanguageContext {
                language: go_lang,
                query,
            },
        ));
    }

    contexts
});

fn get_language_context(filename: &str) -> Option<&'static LanguageContext> {
    let ext = Path::new(filename).extension().and_then(|e| e.to_str())?;
    LANGUAGE_CONTEXTS
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, ctx)| ctx)
}

/// Compute context lines for a given scroll position using tree-sitter AST.
///
/// This function parses the source code, finds all context-worthy nodes (functions,
/// classes, loops, etc.), and returns the ones that contain the scroll position
/// but start above it (i.e., not fully visible).
pub fn compute_context_lines(
    source: &str,
    filename: &str,
    scroll_position: usize,
    config: &ContextConfig,
) -> Vec<ContextLine> {
    if !config.enabled || source.is_empty() || scroll_position == 0 {
        return Vec::new();
    }

    let Some(lang_ctx) = get_language_context(filename) else {
        return Vec::new();
    };

    let mut parser = Parser::new();
    if parser.set_language(&lang_ctx.language).is_err() {
        return Vec::new();
    }

    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };

    let mut cursor = QueryCursor::new();

    let lines: Vec<&str> = source.lines().collect();

    // Collect all context nodes that contain the scroll position but start above it
    let mut context_nodes: Vec<(usize, usize, String)> = Vec::new(); // (start_line, end_line, first_line_content)

    let mut matches = cursor.matches(&lang_ctx.query, tree.root_node(), source.as_bytes());
    while let Some(m) = matches.next() {
        for capture in m.captures {
            let node = capture.node;
            let start_line = node.start_position().row;
            let end_line = node.end_position().row;

            // Node must:
            // 1. Start before the scroll position (not visible at top)
            // 2. End at or after the scroll position (still contains current view)
            if start_line < scroll_position && end_line >= scroll_position {
                if let Some(line_content) = lines.get(start_line) {
                    context_nodes.push((start_line, end_line, line_content.to_string()));
                }
            }
        }
    }

    // Sort by start line (outermost scopes first)
    context_nodes.sort_by_key(|(start, _, _)| *start);

    // Remove duplicates (same start line) - prefer the first occurrence
    context_nodes.dedup_by_key(|(start, _, _)| *start);

    // Convert to ContextLine and limit to max_lines
    let result: Vec<ContextLine> = context_nodes
        .into_iter()
        .take(config.max_lines)
        .map(|(start_line, _, content)| ContextLine {
            line_number: start_line + 1, // 1-indexed for display
            content,
        })
        .collect();

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_source() {
        let config = ContextConfig::default();
        let result = compute_context_lines("", "test.rs", 5, &config);
        assert!(result.is_empty());
    }

    #[test]
    fn test_scroll_at_zero() {
        let config = ContextConfig::default();
        let source = "fn main() {\n    println!(\"hello\");\n}";
        let result = compute_context_lines(source, "test.rs", 0, &config);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rust_function_context() {
        let config = ContextConfig::default();
        let source = r#"fn main() {
    let x = 1;
    let y = 2;
    let z = 3;
    println!("{}", x + y + z);
}"#;
        let result = compute_context_lines(source, "test.rs", 3, &config);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "fn main() {");
        assert_eq!(result[0].line_number, 1);
    }

    #[test]
    fn test_nested_rust_contexts() {
        let config = ContextConfig::default();
        let source = r#"impl Foo {
    fn bar() {
        if true {
            let x = 1;
        }
    }
}"#;
        let result = compute_context_lines(source, "test.rs", 3, &config);
        // Should show impl, fn, and if
        assert!(result.len() >= 2);
        assert!(result[0].content.contains("impl Foo"));
    }

    #[test]
    fn test_unsupported_language() {
        let config = ContextConfig::default();
        let source = "some content\nmore content\neven more";
        let result = compute_context_lines(source, "test.xyz", 2, &config);
        assert!(result.is_empty());
    }

    #[test]
    fn test_disabled_config() {
        let config = ContextConfig {
            enabled: false,
            max_lines: 5,
        };
        let source = "fn main() {\n    let x = 1;\n}";
        let result = compute_context_lines(source, "test.rs", 1, &config);
        assert!(result.is_empty());
    }
}
