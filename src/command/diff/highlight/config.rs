use once_cell::sync::Lazy;
use tree_sitter_highlight::HighlightConfiguration;

use super::queries::*;

// Elixir uses the bundled highlight queries from tree-sitter-elixir
const ELIXIR_HIGHLIGHTS: &str = tree_sitter_elixir::HIGHLIGHTS_QUERY;

// Java uses the bundled highlight queries from tree-sitter-java
const JAVA_HIGHLIGHTS: &str = tree_sitter_java::HIGHLIGHTS_QUERY;

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

/// One grammar and the file extensions it covers. Compiling a highlight
/// query takes tens of milliseconds, so each config is built the first time
/// a file of that language is highlighted rather than all at startup.
pub struct Language {
    pub extensions: &'static [&'static str],
    config: Lazy<Option<LanguageConfig>>,
}

impl Language {
    pub fn config(&self) -> Option<&LanguageConfig> {
        self.config.as_ref()
    }
}

const fn language(
    extensions: &'static [&'static str],
    load: fn() -> Option<LanguageConfig>,
) -> Language {
    Language {
        extensions,
        config: Lazy::new(load),
    }
}

fn load_config(
    language: tree_sitter::Language,
    name: &str,
    highlights: &str,
) -> Option<LanguageConfig> {
    match HighlightConfiguration::new(language, name, highlights, "", "") {
        Ok(mut config) => {
            config.configure(HIGHLIGHT_NAMES);
            Some(LanguageConfig { config })
        }
        Err(_e) => {
            #[cfg(debug_assertions)]
            eprintln!("[WARN] Failed to load {} highlight config: {:?}", name, _e);
            None
        }
    }
}

pub static LANGUAGES: [Language; 19] = [
    language(&["ts"], || {
        load_config(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), "typescript", TS_HIGHLIGHTS)
    }),
    language(&["tsx"], || {
        load_config(tree_sitter_typescript::LANGUAGE_TSX.into(), "tsx", TSX_HIGHLIGHTS)
    }),
    language(&["js", "jsx"], || {
        load_config(tree_sitter_javascript::LANGUAGE.into(), "javascript", JS_HIGHLIGHTS)
    }),
    language(&["rs"], || {
        load_config(tree_sitter_rust::LANGUAGE.into(), "rust", RUST_HIGHLIGHTS)
    }),
    language(&["json"], || {
        load_config(tree_sitter_json::LANGUAGE.into(), "json", JSON_HIGHLIGHTS)
    }),
    language(&["py"], || {
        load_config(tree_sitter_python::LANGUAGE.into(), "python", PYTHON_HIGHLIGHTS)
    }),
    language(&["go"], || {
        load_config(tree_sitter_go::LANGUAGE.into(), "go", GO_HIGHLIGHTS)
    }),
    language(&["css"], || {
        load_config(tree_sitter_css::LANGUAGE.into(), "css", CSS_HIGHLIGHTS)
    }),
    language(&["html"], || {
        load_config(tree_sitter_html::LANGUAGE.into(), "html", HTML_HIGHLIGHTS)
    }),
    language(&["toml"], || {
        load_config(tree_sitter_toml_ng::LANGUAGE.into(), "toml", TOML_HIGHLIGHTS)
    }),
    language(&["sh", "bash"], || {
        load_config(tree_sitter_bash::LANGUAGE.into(), "bash", BASH_HIGHLIGHTS)
    }),
    language(&["md", "mdx"], || {
        load_config(tree_sitter_md::LANGUAGE.into(), "markdown", MD_HIGHLIGHTS)
    }),
    language(&["cs"], || {
        load_config(tree_sitter_c_sharp::LANGUAGE.into(), "c_sharp", CSHARP_HIGHLIGHTS)
    }),
    language(&["rb"], || {
        load_config(tree_sitter_ruby::LANGUAGE.into(), "ruby", RUBY_HIGHLIGHTS)
    }),
    language(&["ex", "exs"], || {
        load_config(tree_sitter_elixir::LANGUAGE.into(), "elixir", ELIXIR_HIGHLIGHTS)
    }),
    language(&["java"], || {
        load_config(tree_sitter_java::LANGUAGE.into(), "java", JAVA_HIGHLIGHTS)
    }),
    language(&["zig"], || {
        load_config(tree_sitter_zig::LANGUAGE.into(), "zig", ZIG_HIGHLIGHTS)
    }),
    language(&["c", "h"], || {
        load_config(tree_sitter_c::LANGUAGE.into(), "c", C_HIGHLIGHTS)
    }),
    language(&["cpp", "cc", "cxx", "hpp", "hh", "hxx"], || {
        load_config(tree_sitter_cpp::LANGUAGE.into(), "cpp", CPP_HIGHLIGHTS)
    }),
];
