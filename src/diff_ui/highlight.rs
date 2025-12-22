use std::path::Path;

use once_cell::sync::Lazy;
use ratatui::prelude::*;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

pub const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "function",
    "function.builtin",
    "function.method",
    "keyword",
    "label",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "string",
    "string.special",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

pub fn highlight_color(index: usize) -> Color {
    match HIGHLIGHT_NAMES.get(index) {
        Some(&"comment") => Color::Rgb(106, 115, 125),
        Some(&"keyword") => Color::Rgb(255, 123, 114),
        Some(&"string" | &"string.special") => Color::Rgb(165, 214, 255),
        Some(&"number" | &"constant" | &"constant.builtin") => Color::Rgb(121, 192, 255),
        Some(&"function" | &"function.builtin" | &"function.method") => Color::Rgb(210, 168, 255),
        Some(&"type" | &"type.builtin" | &"constructor") => Color::Rgb(255, 203, 107),
        Some(&"variable.builtin") => Color::Rgb(255, 123, 114),
        Some(&"property") => Color::Rgb(121, 192, 255),
        Some(&"operator") => Color::Rgb(255, 123, 114),
        Some(&"tag") => Color::Rgb(126, 231, 135),
        Some(&"attribute") => Color::Rgb(121, 192, 255),
        Some(&"punctuation" | &"punctuation.bracket" | &"punctuation.delimiter") => {
            Color::Rgb(200, 200, 200)
        }
        _ => Color::Rgb(230, 230, 230),
    }
}

const TS_HIGHLIGHTS: &str = r#"
(comment) @comment
(string) @string
(template_string) @string
(number) @number
(true) @constant.builtin
(false) @constant.builtin
(null) @constant.builtin
(undefined) @constant.builtin
(regex) @string.special

["const" "let" "var" "function" "class" "interface" "type" "enum" "namespace" "module" "declare" "implements" "extends" "public" "private" "protected" "readonly" "static" "abstract" "async" "await" "return" "if" "else" "for" "while" "do" "switch" "case" "default" "break" "continue" "try" "catch" "finally" "throw" "new" "delete" "typeof" "instanceof" "in" "of" "as" "is" "import" "export" "from" "default" "void"] @keyword

(type_identifier) @type
(predefined_type) @type.builtin

(function_declaration name: (identifier) @function)
(method_definition name: (property_identifier) @function.method)
(call_expression function: (identifier) @function)
(call_expression function: (member_expression property: (property_identifier) @function.method))
(arrow_function) @function

(property_identifier) @property
(shorthand_property_identifier) @property
(shorthand_property_identifier_pattern) @property

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["." "," ";" ":"] @punctuation.delimiter
["=" "+" "-" "*" "/" "%" "!" "?" "<" ">" "&" "|" "^" "~" "==" "===" "!=" "!==" "<=" ">=" "&&" "||" "++" "--" "+=" "-=" "=>" "?."] @operator
"#;

const TSX_HIGHLIGHTS: &str = r#"
(comment) @comment
(string) @string
(template_string) @string
(number) @number
(true) @constant.builtin
(false) @constant.builtin
(null) @constant.builtin
(undefined) @constant.builtin
(regex) @string.special

["const" "let" "var" "function" "class" "interface" "type" "enum" "namespace" "module" "declare" "implements" "extends" "public" "private" "protected" "readonly" "static" "abstract" "async" "await" "return" "if" "else" "for" "while" "do" "switch" "case" "default" "break" "continue" "try" "catch" "finally" "throw" "new" "delete" "typeof" "instanceof" "in" "of" "as" "is" "import" "export" "from" "default" "void"] @keyword

(type_identifier) @type
(predefined_type) @type.builtin

(function_declaration name: (identifier) @function)
(method_definition name: (property_identifier) @function.method)
(call_expression function: (identifier) @function)
(call_expression function: (member_expression property: (property_identifier) @function.method))
(arrow_function) @function

(property_identifier) @property
(shorthand_property_identifier) @property
(shorthand_property_identifier_pattern) @property

(jsx_element open_tag: (jsx_opening_element name: (identifier) @tag))
(jsx_element close_tag: (jsx_closing_element name: (identifier) @tag))
(jsx_self_closing_element name: (identifier) @tag)
(jsx_attribute (property_identifier) @attribute)

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["." "," ";" ":"] @punctuation.delimiter
["=" "+" "-" "*" "/" "%" "!" "?" "<" ">" "&" "|" "^" "~" "==" "===" "!=" "!==" "<=" ">=" "&&" "||" "++" "--" "+=" "-=" "=>" "?."] @operator
"#;

const JS_HIGHLIGHTS: &str = r#"
(comment) @comment
(string) @string
(template_string) @string
(number) @number
(true) @constant.builtin
(false) @constant.builtin
(null) @constant.builtin
(undefined) @constant.builtin
(regex) @string.special

["const" "let" "var" "function" "class" "extends" "async" "await" "return" "if" "else" "for" "while" "do" "switch" "case" "default" "break" "continue" "try" "catch" "finally" "throw" "new" "delete" "typeof" "instanceof" "in" "of" "import" "export" "from" "default" "void"] @keyword

(function_declaration name: (identifier) @function)
(method_definition name: (property_identifier) @function.method)
(call_expression function: (identifier) @function)
(call_expression function: (member_expression property: (property_identifier) @function.method))
(arrow_function) @function

(property_identifier) @property
(shorthand_property_identifier) @property

(jsx_element open_tag: (jsx_opening_element name: (identifier) @tag))
(jsx_element close_tag: (jsx_closing_element name: (identifier) @tag))
(jsx_self_closing_element name: (identifier) @tag)
(jsx_attribute (property_identifier) @attribute)

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["." "," ";" ":"] @punctuation.delimiter
["=" "+" "-" "*" "/" "%" "!" "?" "<" ">" "&" "|" "^" "~" "==" "===" "!=" "!==" "<=" ">=" "&&" "||" "++" "--" "+=" "-=" "=>" "?."] @operator
"#;

const RUST_HIGHLIGHTS: &str = r#"
(line_comment) @comment
(block_comment) @comment
(string_literal) @string
(raw_string_literal) @string
(char_literal) @string
(integer_literal) @number
(float_literal) @number
(boolean_literal) @constant.builtin

[
  "let" "mut" "const" "static" "fn" "struct" "enum" "impl" "trait" 
  "type" "mod" "use" "pub" "crate" "self" "super" "as" "where" 
  "async" "await" "move" "ref" "if" "else" "match" "for" "while" 
  "loop" "break" "continue" "return" "unsafe" "extern" "dyn"
] @keyword

(type_identifier) @type
(primitive_type) @type.builtin

(function_item name: (identifier) @function)
(call_expression function: (identifier) @function)
(macro_invocation macro: (identifier) @function)

(field_identifier) @property
"#;

const JSON_HIGHLIGHTS: &str = r#"
(string) @string
(number) @number
(true) @constant.builtin
(false) @constant.builtin
(null) @constant.builtin
(pair key: (string) @property)
["[" "]" "{" "}"] @punctuation.bracket
[":" ","] @punctuation.delimiter
"#;

const PYTHON_HIGHLIGHTS: &str = r#"
(comment) @comment
(string) @string
(integer) @number
(float) @number
(true) @constant.builtin
(false) @constant.builtin
(none) @constant.builtin

["def" "class" "lambda" "if" "elif" "else" "for" "while" "try" "except" "finally" "with" "as" "import" "from" "return" "yield" "raise" "pass" "break" "continue" "global" "nonlocal" "assert" "async" "await" "and" "or" "not" "in" "is"] @keyword

(function_definition name: (identifier) @function)
(call function: (identifier) @function)
(call function: (attribute attribute: (identifier) @function.method))

(attribute attribute: (identifier) @property)
(type (identifier) @type)

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["." "," ":" ";"] @punctuation.delimiter
["=" "+" "-" "*" "/" "%" "!" "<" ">" "&" "|" "^" "~" "==" "!=" "<=" ">=" "+=" "-=" "*=" "/=" "**" "//"] @operator
"#;

const GO_HIGHLIGHTS: &str = r#"
(comment) @comment
(interpreted_string_literal) @string
(raw_string_literal) @string
(rune_literal) @string
(int_literal) @number
(float_literal) @number
(true) @constant.builtin
(false) @constant.builtin
(nil) @constant.builtin

["func" "var" "const" "type" "struct" "interface" "map" "chan" "package" "import" "if" "else" "for" "range" "switch" "case" "default" "select" "break" "continue" "return" "go" "defer" "fallthrough" "goto"] @keyword

(type_identifier) @type
(function_declaration name: (identifier) @function)
(call_expression function: (identifier) @function)
(call_expression function: (selector_expression field: (field_identifier) @function.method))

(field_identifier) @property

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["." "," ";" ":"] @punctuation.delimiter
["=" "+" "-" "*" "/" "%" "!" "<" ">" "&" "|" "^" ":=" "==" "!=" "<=" ">=" "&&" "||" "++" "--" "+=" "-=" "<-"] @operator
"#;

const CSS_HIGHLIGHTS: &str = r#"
(comment) @comment
(string_value) @string
(integer_value) @number
(float_value) @number
(color_value) @constant
(property_name) @property
(tag_name) @tag
(class_name) @type
(id_name) @constant
["@media" "@import" "@keyframes" "@font-face"] @keyword
["(" ")" "[" "]" "{" "}"] @punctuation.bracket
[":" ";" ","] @punctuation.delimiter
"#;

const HTML_HIGHLIGHTS: &str = r#"
(comment) @comment
(quoted_attribute_value) @string
(tag_name) @tag
(attribute_name) @attribute
(text) @string
["<" ">" "</" "/>"] @punctuation.bracket
["="] @operator
"#;

const TOML_HIGHLIGHTS: &str = r#"
(comment) @comment
(string) @string
(integer) @number
(float) @number
(boolean) @constant.builtin
(bare_key) @property
(dotted_key) @property
["[" "]" "{" "}"] @punctuation.bracket
["." "=" ","] @punctuation.delimiter
"#;

const BASH_HIGHLIGHTS: &str = r#"
(comment) @comment
(string) @string
(raw_string) @string
(number) @number
(command_name) @function
(variable_name) @variable
["if" "then" "else" "elif" "fi" "case" "esac" "for" "while" "do" "done" "in" "function" "return" "local" "export"] @keyword
["(" ")" "[" "]" "[[" "]]" "{" "}"] @punctuation.bracket
[";" "|" "||" "&&" "&" ">" "<" ">>" "<<"] @operator
"#;

pub struct LanguageConfig {
    pub config: HighlightConfiguration,
}

pub static CONFIGS: Lazy<Vec<(&'static str, LanguageConfig)>> = Lazy::new(|| {
    let mut configs = Vec::new();

    // TypeScript
    if let Ok(mut config) = HighlightConfiguration::new(
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "typescript",
        TS_HIGHLIGHTS,
        "",
        "",
    ) {
        config.configure(HIGHLIGHT_NAMES);
        configs.push(("ts", LanguageConfig { config }));
    }

    // TSX
    if let Ok(mut config) = HighlightConfiguration::new(
        tree_sitter_typescript::LANGUAGE_TSX.into(),
        "tsx",
        TSX_HIGHLIGHTS,
        "",
        "",
    ) {
        config.configure(HIGHLIGHT_NAMES);
        configs.push(("tsx", LanguageConfig { config }));
    }

    // JavaScript/JSX
    if let Ok(mut config) = HighlightConfiguration::new(
        tree_sitter_javascript::LANGUAGE.into(),
        "javascript",
        JS_HIGHLIGHTS,
        "",
        "",
    ) {
        config.configure(HIGHLIGHT_NAMES);
        configs.push(("js", LanguageConfig { config }));
        configs.push((
            "jsx",
            LanguageConfig {
                config: {
                    let mut c = HighlightConfiguration::new(
                        tree_sitter_javascript::LANGUAGE.into(),
                        "javascript",
                        JS_HIGHLIGHTS,
                        "",
                        "",
                    )
                    .unwrap();
                    c.configure(HIGHLIGHT_NAMES);
                    c
                },
            },
        ));
    }

    // Rust
    if let Ok(mut config) = HighlightConfiguration::new(
        tree_sitter_rust::LANGUAGE.into(),
        "rust",
        RUST_HIGHLIGHTS,
        "",
        "",
    ) {
        config.configure(HIGHLIGHT_NAMES);
        configs.push(("rs", LanguageConfig { config }));
    }

    // JSON
    if let Ok(mut config) = HighlightConfiguration::new(
        tree_sitter_json::LANGUAGE.into(),
        "json",
        JSON_HIGHLIGHTS,
        "",
        "",
    ) {
        config.configure(HIGHLIGHT_NAMES);
        configs.push(("json", LanguageConfig { config }));
    }

    // Python
    if let Ok(mut config) = HighlightConfiguration::new(
        tree_sitter_python::LANGUAGE.into(),
        "python",
        PYTHON_HIGHLIGHTS,
        "",
        "",
    ) {
        config.configure(HIGHLIGHT_NAMES);
        configs.push(("py", LanguageConfig { config }));
    }

    // Go
    if let Ok(mut config) = HighlightConfiguration::new(
        tree_sitter_go::LANGUAGE.into(),
        "go",
        GO_HIGHLIGHTS,
        "",
        "",
    ) {
        config.configure(HIGHLIGHT_NAMES);
        configs.push(("go", LanguageConfig { config }));
    }

    // CSS
    if let Ok(mut config) = HighlightConfiguration::new(
        tree_sitter_css::LANGUAGE.into(),
        "css",
        CSS_HIGHLIGHTS,
        "",
        "",
    ) {
        config.configure(HIGHLIGHT_NAMES);
        configs.push(("css", LanguageConfig { config }));
    }

    // HTML
    if let Ok(mut config) = HighlightConfiguration::new(
        tree_sitter_html::LANGUAGE.into(),
        "html",
        HTML_HIGHLIGHTS,
        "",
        "",
    ) {
        config.configure(HIGHLIGHT_NAMES);
        configs.push(("html", LanguageConfig { config }));
    }

    // TOML
    if let Ok(mut config) = HighlightConfiguration::new(
        tree_sitter_toml_ng::LANGUAGE.into(),
        "toml",
        TOML_HIGHLIGHTS,
        "",
        "",
    ) {
        config.configure(HIGHLIGHT_NAMES);
        configs.push(("toml", LanguageConfig { config }));
    }

    // Bash
    if let Ok(mut config) = HighlightConfiguration::new(
        tree_sitter_bash::LANGUAGE.into(),
        "bash",
        BASH_HIGHLIGHTS,
        "",
        "",
    ) {
        config.configure(HIGHLIGHT_NAMES);
        configs.push(("sh", LanguageConfig { config }));
        configs.push((
            "bash",
            LanguageConfig {
                config: {
                    let mut c = HighlightConfiguration::new(
                        tree_sitter_bash::LANGUAGE.into(),
                        "bash",
                        BASH_HIGHLIGHTS,
                        "",
                        "",
                    )
                    .unwrap();
                    c.configure(HIGHLIGHT_NAMES);
                    c
                },
            },
        ));
    }

    configs
});

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

    highlighted
        .into_iter()
        .map(|(text, highlight_idx)| {
            let fg = highlight_idx
                .map(highlight_color)
                .unwrap_or(Color::Rgb(230, 230, 230));
            Span::styled(text, Style::default().fg(fg).bg(bg_color))
        })
        .collect()
}

pub fn init() {
    let _ = &*CONFIGS;
}
