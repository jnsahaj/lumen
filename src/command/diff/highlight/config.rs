use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

use super::queries::*;

// Elixir uses the bundled highlight queries from tree-sitter-elixir
const ELIXIR_HIGHLIGHTS: &str = tree_sitter_elixir::HIGHLIGHTS_QUERY;

// Java uses the bundled highlight queries from tree-sitter-java
const JAVA_HIGHLIGHTS: &str = tree_sitter_java::HIGHLIGHTS_QUERY;

// tree-sitter-kotlin-sg bundles a legacy nvim-treesitter query. Several of its
// capture names are not present in HIGHLIGHT_NAMES, so tree-sitter-highlight
// would silently leave them unstyled. Some dotted names are also ambiguous:
// for example, `keyword.function` matches Lumen's earlier `function` entry
// before `keyword`. Rewriting the captures keeps Kotlin on Lumen's existing
// palette without adding language-specific aliases to every theme.
//
// Each tuple is `(capture emitted by the upstream query, capture recognized by
// Lumen)`.
const KOTLIN_HIGHLIGHT_MAPPINGS: &[(&str, &str)] = &[
    // Declarations and imports.
    ("@namespace", "@module"),
    ("@include", "@keyword"),
    ("@parameter", "@variable.parameter"),
    // Literals.
    ("@float", "@number"),
    ("@boolean", "@constant.builtin"),
    ("@character", "@string"),
    ("@string.escape", "@string.special"),
    ("@string.regex", "@string.special"),
    // Keyword subcategories used by the legacy query.
    ("@keyword.function", "@keyword"),
    ("@keyword.return", "@keyword"),
    ("@conditional", "@keyword"),
    ("@repeat", "@keyword"),
    ("@exception", "@keyword"),
    // Lumen uses one punctuation style for special interpolation delimiters.
    ("@punctuation.special", "@punctuation"),
];

fn kotlin_highlights() -> String {
    KOTLIN_HIGHLIGHT_MAPPINGS.iter().fold(
        tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY.to_owned(),
        |query, (from, to)| query.replace(from, to),
    )
}

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
    let kotlin_highlights = kotlin_highlights();

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

    load_config(
        tree_sitter_elixir::LANGUAGE.into(),
        "elixir",
        ELIXIR_HIGHLIGHTS,
        "ex",
        &mut configs,
    );

    load_config(
        tree_sitter_elixir::LANGUAGE.into(),
        "elixir",
        ELIXIR_HIGHLIGHTS,
        "exs",
        &mut configs,
    );

    load_config(
        tree_sitter_java::LANGUAGE.into(),
        "java",
        JAVA_HIGHLIGHTS,
        "java",
        &mut configs,
    );

    load_config(
        tree_sitter_kotlin_sg::LANGUAGE.into(),
        "kotlin",
        &kotlin_highlights,
        "kt",
        &mut configs,
    );

    load_config(
        tree_sitter_kotlin_sg::LANGUAGE.into(),
        "kotlin",
        &kotlin_highlights,
        "kts",
        &mut configs,
    );

    load_config(
        tree_sitter_zig::LANGUAGE.into(),
        "zig",
        ZIG_HIGHLIGHTS,
        "zig",
        &mut configs,
    );

    load_config(
        tree_sitter_c::LANGUAGE.into(),
        "c",
        C_HIGHLIGHTS,
        "c",
        &mut configs,
    );

    load_config(
        tree_sitter_c::LANGUAGE.into(),
        "c",
        C_HIGHLIGHTS,
        "h",
        &mut configs,
    );

    load_config(
        tree_sitter_cpp::LANGUAGE.into(),
        "cpp",
        CPP_HIGHLIGHTS,
        "cpp",
        &mut configs,
    );

    load_config(
        tree_sitter_cpp::LANGUAGE.into(),
        "cpp",
        CPP_HIGHLIGHTS,
        "cc",
        &mut configs,
    );

    load_config(
        tree_sitter_cpp::LANGUAGE.into(),
        "cpp",
        CPP_HIGHLIGHTS,
        "cxx",
        &mut configs,
    );

    load_config(
        tree_sitter_cpp::LANGUAGE.into(),
        "cpp",
        CPP_HIGHLIGHTS,
        "hpp",
        &mut configs,
    );

    load_config(
        tree_sitter_cpp::LANGUAGE.into(),
        "cpp",
        CPP_HIGHLIGHTS,
        "hh",
        &mut configs,
    );

    load_config(
        tree_sitter_cpp::LANGUAGE.into(),
        "cpp",
        CPP_HIGHLIGHTS,
        "hxx",
        &mut configs,
    );

    configs
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kotlin_query_only_uses_known_public_captures() {
        // Comparing the normalized query with HIGHLIGHT_NAMES keeps the mapping
        // exhaustive when upstream adds or changes captures.
        let highlights = kotlin_highlights();
        let config = HighlightConfiguration::new(
            tree_sitter_kotlin_sg::LANGUAGE.into(),
            "kotlin",
            &highlights,
            "",
            "",
        )
        .expect("Kotlin highlight query should compile");

        let unsupported: Vec<&str> = config
            .names()
            .iter()
            .filter(|name| {
                // `_...` captures are query-internal; `none` is the upstream
                // query's deliberate opt-out capture for interpolation content.
                !name.starts_with('_') && **name != "none" && !HIGHLIGHT_NAMES.contains(name)
            })
            .copied()
            .collect();

        assert!(
            unsupported.is_empty(),
            "Kotlin query has unmapped public captures: {unsupported:?}"
        );
    }
}
