use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Point, Query, QueryCursor, Tree};

use super::expand_tabs;

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

#[derive(Clone, Debug)]
pub struct ContextLine {
    pub line_number: usize,
    pub content: String,
}

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

const CSHARP_CONTEXT_QUERY: &str = r#"
(namespace_declaration) @context
(class_declaration) @context
(struct_declaration) @context
(interface_declaration) @context
(enum_declaration) @context
(record_declaration) @context
(method_declaration) @context
(constructor_declaration) @context
(property_declaration) @context
(for_statement) @context
(foreach_statement) @context
(while_statement) @context
(do_statement) @context
(if_statement) @context
(switch_statement) @context
(try_statement) @context
(using_statement) @context
(lock_statement) @context
"#;

pub struct LanguageContext {
    pub language: Language,
    query: Query,
}

struct ContextRule {
    extensions: &'static [&'static str],
    grammar: fn() -> Language,
    query_src: &'static str,
    compiled: OnceLock<Option<LanguageContext>>,
}

impl ContextRule {
    const fn new(
        extensions: &'static [&'static str],
        grammar: fn() -> Language,
        query_src: &'static str,
    ) -> Self {
        Self {
            extensions,
            grammar,
            query_src,
            compiled: OnceLock::new(),
        }
    }

    fn get(&self) -> Option<&LanguageContext> {
        self.compiled
            .get_or_init(|| {
                let language = (self.grammar)();
                Query::new(&language, self.query_src)
                    .ok()
                    .map(|query| LanguageContext { language, query })
            })
            .as_ref()
    }
}

/// Compiled lazily per language so opening a Python file does not compile
/// Rust/TypeScript/C# tree-sitter queries.
static CONTEXT_RULES: [ContextRule; 7] = [
    ContextRule::new(
        &["rs"],
        || tree_sitter_rust::LANGUAGE.into(),
        RUST_CONTEXT_QUERY,
    ),
    ContextRule::new(
        &["ts"],
        || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        TYPESCRIPT_CONTEXT_QUERY,
    ),
    ContextRule::new(
        &["tsx"],
        || tree_sitter_typescript::LANGUAGE_TSX.into(),
        TYPESCRIPT_CONTEXT_QUERY,
    ),
    ContextRule::new(
        &["js", "jsx"],
        || tree_sitter_javascript::LANGUAGE.into(),
        JAVASCRIPT_CONTEXT_QUERY,
    ),
    ContextRule::new(
        &["py"],
        || tree_sitter_python::LANGUAGE.into(),
        PYTHON_CONTEXT_QUERY,
    ),
    ContextRule::new(
        &["go"],
        || tree_sitter_go::LANGUAGE.into(),
        GO_CONTEXT_QUERY,
    ),
    ContextRule::new(
        &["cs"],
        || tree_sitter_c_sharp::LANGUAGE.into(),
        CSHARP_CONTEXT_QUERY,
    ),
];

pub fn get_language_context(filename: &str) -> Option<&'static LanguageContext> {
    let ext = Path::new(filename).extension().and_then(|e| e.to_str())?;
    CONTEXT_RULES
        .iter()
        .find(|rule| rule.extensions.contains(&ext))
        .and_then(ContextRule::get)
}

/// Returns the full source line containing `byte_offset` in O(line length).
fn line_at_byte(source: &str, byte_offset: usize) -> Option<&str> {
    if byte_offset > source.len() {
        return None;
    }
    let bytes = source.as_bytes();
    let start = bytes[..byte_offset]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |pos| pos + 1);
    let end = bytes[byte_offset..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(source.len(), |rel| byte_offset + rel);
    let line = &source[start..end];
    Some(line.strip_suffix('\r').unwrap_or(line))
}

/// Enclosing AST scope headers above `scroll_position` that still span across it.
pub fn compute_context_lines(
    source: &str,
    filename: &str,
    trees: &HashMap<String, Tree>,
    scroll_position: usize,
    config: &ContextConfig,
    tab_width: usize,
) -> Vec<ContextLine> {
    if !config.enabled || source.is_empty() || scroll_position == 0 {
        return Vec::new();
    }

    let Some(lang_ctx) = get_language_context(filename) else {
        return Vec::new();
    };

    let Some(tree) = trees.get(filename) else {
        return Vec::new();
    };

    let mut cursor = QueryCursor::new();
    cursor.set_point_range(Point::new(0, 0)..Point::new(scroll_position, 0));

    let mut context_nodes: Vec<(usize, String)> = Vec::new();
    let mut matches = cursor.matches(&lang_ctx.query, tree.root_node(), source.as_bytes());
    while let Some(m) = matches.next() {
        for capture in m.captures {
            let node = capture.node;
            let start_line = node.start_position().row;
            let end_line = node.end_position().row;

            if start_line < scroll_position && end_line >= scroll_position {
                if let Some(line_content) = line_at_byte(source, node.start_byte()) {
                    context_nodes.push((start_line, expand_tabs(line_content, tab_width)));
                }
            }
        }
    }

    context_nodes.sort_by_key(|(start, _)| *start);
    context_nodes.dedup_by_key(|(start, _)| *start);

    context_nodes
        .into_iter()
        .take(config.max_lines)
        .map(|(start_line, content)| ContextLine {
            line_number: start_line + 1,
            content,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use tree_sitter::Parser;

    use super::*;

    fn parse_tree(filename: &str, source: &str) -> HashMap<String, Tree> {
        let mut tree_cache = HashMap::new();
        let Some(lang_ctx) = get_language_context(filename) else {
            return tree_cache;
        };
        let mut parser = Parser::new();
        if parser.set_language(&lang_ctx.language).is_err() {
            return tree_cache;
        }
        if let Some(tree) = parser.parse(source, None) {
            tree_cache.insert(filename.to_string(), tree);
        }
        tree_cache
    }

    #[test]
    fn test_rust_function_and_nested_scopes() {
        let config = ContextConfig::default();
        let source = "impl Foo {\n    fn bar() {\n        if true {\n            let x = 1;\n        }\n    }\n}";
        let trees = parse_tree("test.rs", source);

        assert!(compute_context_lines(source, "test.rs", &trees, 0, &config, 4).is_empty());

        let result = compute_context_lines(source, "test.rs", &trees, 3, &config, 4);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].content, "impl Foo {");
        assert_eq!(result[0].line_number, 1);
        assert_eq!(result[1].content, "    fn bar() {");
        assert_eq!(result[2].content, "        if true {");

        let disabled = ContextConfig {
            enabled: false,
            max_lines: 5,
        };
        assert!(compute_context_lines(source, "test.rs", &trees, 3, &disabled, 4).is_empty());
    }

    #[test]
    fn test_unsupported_language_and_empty_source() {
        let config = ContextConfig::default();
        let trees = parse_tree("test.xyz", "some\ncontent\nhere");
        assert!(
            compute_context_lines("some\ncontent", "test.xyz", &trees, 1, &config, 4).is_empty()
        );

        let empty_trees = parse_tree("test.rs", "");
        assert!(compute_context_lines("", "test.rs", &empty_trees, 2, &config, 4).is_empty());
    }
}
