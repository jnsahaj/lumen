use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

use super::queries::*;

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

    load_config(
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "typescript",
        TS_HIGHLIGHTS,
        "ts",
        &mut configs,
    );

    load_config(
        tree_sitter_typescript::LANGUAGE_TSX.into(),
        "tsx",
        TSX_HIGHLIGHTS,
        "tsx",
        &mut configs,
    );

    load_config(
        tree_sitter_javascript::LANGUAGE.into(),
        "javascript",
        JS_HIGHLIGHTS,
        "js",
        &mut configs,
    );

    load_config(
        tree_sitter_javascript::LANGUAGE.into(),
        "javascript",
        JS_HIGHLIGHTS,
        "jsx",
        &mut configs,
    );

    load_config(
        tree_sitter_rust::LANGUAGE.into(),
        "rust",
        RUST_HIGHLIGHTS,
        "rs",
        &mut configs,
    );

    load_config(
        tree_sitter_json::LANGUAGE.into(),
        "json",
        JSON_HIGHLIGHTS,
        "json",
        &mut configs,
    );

    load_config(
        tree_sitter_python::LANGUAGE.into(),
        "python",
        PYTHON_HIGHLIGHTS,
        "py",
        &mut configs,
    );

    load_config(
        tree_sitter_go::LANGUAGE.into(),
        "go",
        GO_HIGHLIGHTS,
        "go",
        &mut configs,
    );

    load_config(
        tree_sitter_css::LANGUAGE.into(),
        "css",
        CSS_HIGHLIGHTS,
        "css",
        &mut configs,
    );

    load_config(
        tree_sitter_html::LANGUAGE.into(),
        "html",
        HTML_HIGHLIGHTS,
        "html",
        &mut configs,
    );

    load_config(
        tree_sitter_toml_ng::LANGUAGE.into(),
        "toml",
        TOML_HIGHLIGHTS,
        "toml",
        &mut configs,
    );

    load_config(
        tree_sitter_bash::LANGUAGE.into(),
        "bash",
        BASH_HIGHLIGHTS,
        "sh",
        &mut configs,
    );

    load_config(
        tree_sitter_bash::LANGUAGE.into(),
        "bash",
        BASH_HIGHLIGHTS,
        "bash",
        &mut configs,
    );

    load_config(
        tree_sitter_md::LANGUAGE.into(),
        "markdown",
        MD_HIGHLIGHTS,
        "md",
        &mut configs,
    );

    load_config(
        tree_sitter_md::LANGUAGE.into(),
        "markdown",
        MD_HIGHLIGHTS,
        "mdx",
        &mut configs,
    );

    load_config(
        tree_sitter_c_sharp::LANGUAGE.into(),
        "c_sharp",
        CSHARP_HIGHLIGHTS,
        "cs",
        &mut configs,
    );

    load_config(
        tree_sitter_ruby::LANGUAGE.into(),
        "ruby",
        RUBY_HIGHLIGHTS,
        "rb",
        &mut configs,
    );

    configs
});
