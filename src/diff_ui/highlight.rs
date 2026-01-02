use std::path::Path;

use once_cell::sync::Lazy;
use ratatui::prelude::*;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use super::theme;

pub const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "function",
    "function.builtin",
    "function.method",
    "function.macro",
    "keyword",
    "label",
    "module",
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
    "variable.member",
];

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
"#;

const RUST_HIGHLIGHTS: &str = r#"
; Comments
(line_comment) @comment
(block_comment) @comment

; Strings and literals
(string_literal) @string
(raw_string_literal) @string
(char_literal) @string
(integer_literal) @number
(float_literal) @number
(boolean_literal) @constant.builtin

; Types
(type_identifier) @type
(primitive_type) @type.builtin

; Functions
(function_item (identifier) @function)
(function_signature_item (identifier) @function)
(call_expression function: (identifier) @function)
(call_expression function: (field_expression field: (field_identifier) @function.method))
(call_expression function: (scoped_identifier name: (identifier) @function))
(generic_function function: (identifier) @function)
(generic_function function: (scoped_identifier name: (identifier) @function))

; Macros
(macro_invocation macro: (identifier) @function.macro "!" @function.macro)
(macro_definition "macro_rules!" @function.macro)

; Fields and properties
(field_identifier) @variable.member
(shorthand_field_identifier) @variable.member

; Labels and lifetimes
(lifetime (identifier) @label)

; Parameters
(parameter (identifier) @variable.parameter)

; Modules
(mod_item name: (identifier) @module)
(scoped_identifier path: (identifier) @module)

; Self, crate, and special
(self) @variable.builtin
(crate) @keyword
(super) @keyword
(mutable_specifier) @keyword

; Keywords
"as" @keyword
"async" @keyword
"await" @keyword
"break" @keyword
"const" @keyword
"continue" @keyword
"dyn" @keyword
"else" @keyword
"enum" @keyword
"extern" @keyword
"fn" @keyword
"for" @keyword
"if" @keyword
"impl" @keyword
"in" @keyword
"let" @keyword
"loop" @keyword
"match" @keyword
"mod" @keyword
"move" @keyword
"pub" @keyword
"ref" @keyword
"return" @keyword
"static" @keyword
"struct" @keyword
"trait" @keyword
"type" @keyword
"unsafe" @keyword
"use" @keyword
"where" @keyword
"while" @keyword

; Operators
"*" @operator
"&" @operator
"=" @operator
"+" @operator
"-" @operator
"/" @operator
"%" @operator
"!" @operator
"<" @operator
">" @operator
"==" @operator
"!=" @operator
"<=" @operator
">=" @operator
"&&" @operator
"||" @operator
"+=" @operator
"-=" @operator
"*=" @operator
"/=" @operator
".." @operator
"..=" @operator
"=>" @operator
"->" @operator
"?" @operator

; Punctuation
"(" @punctuation.bracket
")" @punctuation.bracket
"[" @punctuation.bracket
"]" @punctuation.bracket
"{" @punctuation.bracket
"}" @punctuation.bracket
"::" @punctuation.delimiter
":" @punctuation.delimiter
"." @punctuation.delimiter
"," @punctuation.delimiter
";" @punctuation.delimiter

; Attributes
(attribute_item) @attribute
(inner_attribute_item) @attribute
"#;

const JSON_HIGHLIGHTS: &str = r#"
(string) @string
(number) @number
(true) @constant.builtin
(false) @constant.builtin
(null) @constant.builtin
(pair key: (string) @property)

"[" @punctuation.bracket
"]" @punctuation.bracket
"{" @punctuation.bracket
"}" @punctuation.bracket
":" @punctuation.delimiter
"," @punctuation.delimiter
"#;

const PYTHON_HIGHLIGHTS: &str = r#"
; Comments and strings
(comment) @comment
(string) @string
(escape_sequence) @string.special

; Literals
(integer) @number
(float) @number
(none) @constant.builtin
(true) @constant.builtin
(false) @constant.builtin

; Types and attributes
(type (identifier) @type)
(attribute attribute: (identifier) @property)

; Functions
(function_definition name: (identifier) @function)
(call function: (identifier) @function)
(call function: (attribute attribute: (identifier) @function.method))
(decorator) @function
(decorator (identifier) @function)

; Keywords
"as" @keyword
"assert" @keyword
"async" @keyword
"await" @keyword
"break" @keyword
"class" @keyword
"continue" @keyword
"def" @keyword
"del" @keyword
"elif" @keyword
"else" @keyword
"except" @keyword
"finally" @keyword
"for" @keyword
"from" @keyword
"global" @keyword
"if" @keyword
"import" @keyword
"lambda" @keyword
"nonlocal" @keyword
"pass" @keyword
"raise" @keyword
"return" @keyword
"try" @keyword
"while" @keyword
"with" @keyword
"yield" @keyword
"match" @keyword
"case" @keyword
"and" @operator
"or" @operator
"not" @operator
"in" @operator
"is" @operator

"#;

const GO_HIGHLIGHTS: &str = r#"
; Comments and strings
(comment) @comment
(interpreted_string_literal) @string
(raw_string_literal) @string
(rune_literal) @string

; Literals
(int_literal) @number
(float_literal) @number
(true) @constant.builtin
(false) @constant.builtin
(nil) @constant.builtin

; Types
(type_identifier) @type
(type_spec name: (type_identifier) @type)

; Functions
(function_declaration name: (identifier) @function)
(method_declaration name: (field_identifier) @function.method)
(call_expression function: (identifier) @function)
(call_expression function: (selector_expression field: (field_identifier) @function.method))

; Fields
(field_identifier) @property

; Package
(package_identifier) @module

; Keywords
"break" @keyword
"case" @keyword
"chan" @keyword
"const" @keyword
"continue" @keyword
"default" @keyword
"defer" @keyword
"else" @keyword
"fallthrough" @keyword
"for" @keyword
"func" @keyword
"go" @keyword
"goto" @keyword
"if" @keyword
"import" @keyword
"interface" @keyword
"map" @keyword
"package" @keyword
"range" @keyword
"return" @keyword
"select" @keyword
"struct" @keyword
"switch" @keyword
"type" @keyword
"var" @keyword

; Operators
"=" @operator
"+" @operator
"-" @operator
"*" @operator
"/" @operator
"%" @operator
"!" @operator
"<" @operator
">" @operator
"&" @operator
"|" @operator
"^" @operator
":=" @operator
"==" @operator
"!=" @operator
"<=" @operator
">=" @operator
"&&" @operator
"||" @operator
"++" @operator
"--" @operator
"+=" @operator
"-=" @operator
"<-" @operator

; Punctuation
"(" @punctuation.bracket
")" @punctuation.bracket
"[" @punctuation.bracket
"]" @punctuation.bracket
"{" @punctuation.bracket
"}" @punctuation.bracket
"." @punctuation.delimiter
"," @punctuation.delimiter
";" @punctuation.delimiter
":" @punctuation.delimiter
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
(at_keyword) @keyword
"#;

const HTML_HIGHLIGHTS: &str = r#"
(comment) @comment
(quoted_attribute_value) @string
(tag_name) @tag
(attribute_name) @attribute
"#;

const TOML_HIGHLIGHTS: &str = r#"
(comment) @comment
(string) @string
(integer) @number
(float) @number
(boolean) @constant.builtin
(bare_key) @property
(dotted_key) @property
"#;

const BASH_HIGHLIGHTS: &str = r#"
(comment) @comment
(string) @string
(raw_string) @string
(number) @number
(command_name) @function
(variable_name) @variable
"#;

pub struct LanguageConfig {
    pub config: HighlightConfiguration,
}

fn load_config(
    language: tree_sitter::Language,
    name: &str,
    highlights: &str,
    ext: &'static str,
    configs: &mut Vec<(&'static str, LanguageConfig)>,
) {
    match HighlightConfiguration::new(language, name, highlights, "", "") {
        Ok(mut config) => {
            config.configure(HIGHLIGHT_NAMES);
            configs.push((ext, LanguageConfig { config }));
        }
        Err(_e) => {
            #[cfg(debug_assertions)]
            eprintln!("[WARN] Failed to load {} highlight config: {:?}", name, _e);
        }
    }
}

pub static CONFIGS: Lazy<Vec<(&'static str, LanguageConfig)>> = Lazy::new(|| {
    let mut configs = Vec::new();

    // TypeScript
    load_config(
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "typescript",
        TS_HIGHLIGHTS,
        "ts",
        &mut configs,
    );

    // TSX
    load_config(
        tree_sitter_typescript::LANGUAGE_TSX.into(),
        "tsx",
        TSX_HIGHLIGHTS,
        "tsx",
        &mut configs,
    );

    // JavaScript
    load_config(
        tree_sitter_javascript::LANGUAGE.into(),
        "javascript",
        JS_HIGHLIGHTS,
        "js",
        &mut configs,
    );

    // JSX (same as JS but different extension)
    load_config(
        tree_sitter_javascript::LANGUAGE.into(),
        "javascript",
        JS_HIGHLIGHTS,
        "jsx",
        &mut configs,
    );

    // Rust
    load_config(
        tree_sitter_rust::LANGUAGE.into(),
        "rust",
        RUST_HIGHLIGHTS,
        "rs",
        &mut configs,
    );

    // JSON
    load_config(
        tree_sitter_json::LANGUAGE.into(),
        "json",
        JSON_HIGHLIGHTS,
        "json",
        &mut configs,
    );

    // Python
    load_config(
        tree_sitter_python::LANGUAGE.into(),
        "python",
        PYTHON_HIGHLIGHTS,
        "py",
        &mut configs,
    );

    // Go
    load_config(
        tree_sitter_go::LANGUAGE.into(),
        "go",
        GO_HIGHLIGHTS,
        "go",
        &mut configs,
    );

    // CSS
    load_config(
        tree_sitter_css::LANGUAGE.into(),
        "css",
        CSS_HIGHLIGHTS,
        "css",
        &mut configs,
    );

    // HTML
    load_config(
        tree_sitter_html::LANGUAGE.into(),
        "html",
        HTML_HIGHLIGHTS,
        "html",
        &mut configs,
    );

    // TOML
    load_config(
        tree_sitter_toml_ng::LANGUAGE.into(),
        "toml",
        TOML_HIGHLIGHTS,
        "toml",
        &mut configs,
    );

    // Bash (sh extension)
    load_config(
        tree_sitter_bash::LANGUAGE.into(),
        "bash",
        BASH_HIGHLIGHTS,
        "sh",
        &mut configs,
    );

    // Bash (bash extension)
    load_config(
        tree_sitter_bash::LANGUAGE.into(),
        "bash",
        BASH_HIGHLIGHTS,
        "bash",
        &mut configs,
    );

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
    let default_fg = theme::get().syntax.default_text;

    highlighted
        .into_iter()
        .map(|(text, highlight_idx)| {
            let fg = highlight_idx
                .map(highlight_color)
                .unwrap_or(default_fg);
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
        // Should have all expected languages
        assert!(extensions.contains(&"rs"), "Rust config should be loaded");
        assert!(extensions.contains(&"ts"), "TypeScript config should be loaded");
        assert!(extensions.contains(&"tsx"), "TSX config should be loaded");
        assert!(extensions.contains(&"js"), "JavaScript config should be loaded");
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
        // Should have multiple highlighted segments
        assert!(!result.is_empty(), "Rust highlighting should produce output");
        // Should have some highlights (not all None)
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
        assert!(!result.is_empty(), "TypeScript highlighting should produce output");
        let has_highlights = result.iter().any(|(_, h)| h.is_some());
        assert!(has_highlights, "TypeScript code should have syntax highlights");
    }

    #[test]
    fn test_python_highlighting() {
        let code = r#"def hello():
    x = 42
    return "world"
"#;
        let result = highlight_code(code, "test.py");
        assert!(!result.is_empty(), "Python highlighting should produce output");
        let has_highlights = result.iter().any(|(_, h)| h.is_some());
        assert!(has_highlights, "Python code should have syntax highlights");
    }
}
