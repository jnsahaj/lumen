use once_cell::sync::OnceCell;
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

/// A language Lumiere can colour. Its highlighting rules take from under a
/// millisecond to over 100 ms to compile, so that is done when a file needs them.
struct Language {
    extensions: &'static [&'static str],
    name: &'static str,
    grammar: fn() -> tree_sitter::Language,
    highlights: &'static str,
    config: OnceCell<Option<LanguageConfig>>,
}

impl Language {
    const fn new(
        extensions: &'static [&'static str],
        name: &'static str,
        grammar: fn() -> tree_sitter::Language,
        highlights: &'static str,
    ) -> Self {
        Self { extensions, name, grammar, highlights, config: OnceCell::new() }
    }

    fn config(&self) -> Option<&LanguageConfig> {
        self.config
            .get_or_init(|| {
                match HighlightConfiguration::new((self.grammar)(), self.name, self.highlights, "", "") {
                    Ok(mut config) => {
                        config.configure(HIGHLIGHT_NAMES);
                        Some(LanguageConfig { config })
                    }
                    Err(_e) => {
                        #[cfg(debug_assertions)]
                        eprintln!("[WARN] Failed to load {} highlight config: {:?}", self.name, _e);
                        None
                    }
                }
            })
            .as_ref()
    }
}

static LANGUAGES: [Language; 19] = [
    Language::new(&["ts"], "typescript", || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), TS_HIGHLIGHTS),
    Language::new(&["tsx"], "tsx", || tree_sitter_typescript::LANGUAGE_TSX.into(), TSX_HIGHLIGHTS),
    Language::new(&["js", "jsx"], "javascript", || tree_sitter_javascript::LANGUAGE.into(), JS_HIGHLIGHTS),
    Language::new(&["rs"], "rust", || tree_sitter_rust::LANGUAGE.into(), RUST_HIGHLIGHTS),
    Language::new(&["json"], "json", || tree_sitter_json::LANGUAGE.into(), JSON_HIGHLIGHTS),
    Language::new(&["py"], "python", || tree_sitter_python::LANGUAGE.into(), PYTHON_HIGHLIGHTS),
    Language::new(&["go"], "go", || tree_sitter_go::LANGUAGE.into(), GO_HIGHLIGHTS),
    Language::new(&["css"], "css", || tree_sitter_css::LANGUAGE.into(), CSS_HIGHLIGHTS),
    Language::new(&["html"], "html", || tree_sitter_html::LANGUAGE.into(), HTML_HIGHLIGHTS),
    Language::new(&["toml"], "toml", || tree_sitter_toml_ng::LANGUAGE.into(), TOML_HIGHLIGHTS),
    Language::new(&["sh", "bash"], "bash", || tree_sitter_bash::LANGUAGE.into(), BASH_HIGHLIGHTS),
    Language::new(&["md", "mdx"], "markdown", || tree_sitter_md::LANGUAGE.into(), MD_HIGHLIGHTS),
    Language::new(&["cs"], "c_sharp", || tree_sitter_c_sharp::LANGUAGE.into(), CSHARP_HIGHLIGHTS),
    Language::new(&["rb"], "ruby", || tree_sitter_ruby::LANGUAGE.into(), RUBY_HIGHLIGHTS),
    Language::new(&["ex", "exs"], "elixir", || tree_sitter_elixir::LANGUAGE.into(), ELIXIR_HIGHLIGHTS),
    Language::new(&["java"], "java", || tree_sitter_java::LANGUAGE.into(), JAVA_HIGHLIGHTS),
    Language::new(&["zig"], "zig", || tree_sitter_zig::LANGUAGE.into(), ZIG_HIGHLIGHTS),
    Language::new(&["c", "h"], "c", || tree_sitter_c::LANGUAGE.into(), C_HIGHLIGHTS),
    Language::new(&["cpp", "cc", "cxx", "hpp", "hh", "hxx"], "cpp", || tree_sitter_cpp::LANGUAGE.into(), CPP_HIGHLIGHTS),
];

/// The highlighting rules for files ending in `extension`, compiled on first use.
pub fn config_for_extension(extension: &str) -> Option<&'static LanguageConfig> {
    LANGUAGES
        .iter()
        .find(|language| language.extensions.contains(&extension))
        .and_then(Language::config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_every_language_has_rules_that_compile() {
        for language in &LANGUAGES {
            for extension in language.extensions {
                assert!(
                    config_for_extension(extension).is_some(),
                    "no highlighting for .{} ({})",
                    extension,
                    language.name
                );
            }
        }
    }
}
