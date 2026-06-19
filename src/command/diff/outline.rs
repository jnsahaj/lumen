//! Structural outline extraction for the diff viewer's "outline" view mode.
//!
//! Walks a file's tree-sitter syntax tree and returns the declaration
//! "skeleton" (classes, functions, fields, etc.) as a flat, depth-tagged list
//! of signature lines. This is the same machinery [`super::context`] uses to
//! find enclosing scopes, generalized to collect *all* declarations and to also
//! include member declarations (struct/class fields, properties, enum members).
//!
//! Languages without an outline query fall through to an empty result, which
//! the renderer surfaces as a "no outline for this file type" hint.

use std::path::Path;

use once_cell::sync::Lazy;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Parser, Query, QueryCursor};

/// A single declaration in a file's outline. The renderer folds the diff so
/// only these declarations' signature lines stay visible, so an entry just
/// needs to locate the declaration's line span; the visible text/colors come
/// from the real diff line at that position.
#[derive(Clone, Debug)]
pub struct OutlineEntry {
    /// 1-based line number of the declaration's first line (the side being shown).
    pub line_number: usize,
    /// 0-based start row of the declaration node.
    pub start_row: usize,
    /// 0-based end row of the declaration node.
    pub end_row: usize,
    /// Whether this declaration's line span intersects a changed hunk.
    /// Filled in by the caller (the diff view state), `false` from extraction.
    pub changed: bool,
}

// Outline queries per language. These capture whole declaration nodes; the
// renderer shows only each node's first line. Compared to the context queries
// in `context.rs`, these drop control-flow nodes (if/for/while/...) and add
// member declarations — most importantly fields/properties.
//
// Each language lists a primary (richer) query and a known-good fallback. If
// the primary fails to compile against the bundled grammar (e.g. a node name
// differs), the fallback keeps the outline working with fewer node kinds.

const RUST_OUTLINE: &str = r#"
(function_item) @item
(function_signature_item) @item
(impl_item) @item
(struct_item) @item
(enum_item) @item
(union_item) @item
(trait_item) @item
(mod_item) @item
(macro_definition) @item
(const_item) @item
(static_item) @item
(type_item) @item
(field_declaration) @item
(enum_variant) @item
(if_expression) @block
(match_expression) @block
(for_expression) @block
(while_expression) @block
(loop_expression) @block
"#;
const RUST_OUTLINE_FALLBACK: &str = r#"
(function_item) @item
(impl_item) @item
(struct_item) @item
(enum_item) @item
(trait_item) @item
(mod_item) @item
"#;

// Real TS/React structure is dominated by `const X = <expr>` at module scope —
// components (arrows, `memo(...)`, `styled(...)`), hooks, plus schemas, column
// defs, configs (object/array/call values). So capture *every* top-level
// const/let binding regardless of its value, not just function-valued ones.
// Nested bindings are only captured when function-valued (handlers), so local
// vars like `const [x, setX] = useState()` and inline callbacks stay out.
const TYPESCRIPT_OUTLINE: &str = r#"
(function_declaration) @item
(class_body (method_definition) @item)
(class_declaration) @item
(abstract_class_declaration) @item
(interface_declaration) @item
(type_alias_declaration) @item
(enum_declaration) @item
(internal_module) @item
(public_field_definition) @item
(program (lexical_declaration (variable_declarator) @item))
(program (variable_declaration (variable_declarator) @item))
(program (export_statement (lexical_declaration (variable_declarator) @item)))
(program (export_statement (variable_declaration (variable_declarator) @item)))
(variable_declarator value: (arrow_function)) @item
(variable_declarator value: (function_expression)) @item
(if_statement) @block
(for_statement) @block
(for_in_statement) @block
(while_statement) @block
(do_statement) @block
(switch_statement) @block
(try_statement) @block
"#;
const TYPESCRIPT_OUTLINE_FALLBACK: &str = r#"
(function_declaration) @item
(class_body (method_definition) @item)
(class_declaration) @item
(interface_declaration) @item
(variable_declarator value: (arrow_function)) @item
"#;

const JAVASCRIPT_OUTLINE: &str = r#"
(function_declaration) @item
(class_body (method_definition) @item)
(class_declaration) @item
(field_definition) @item
(program (lexical_declaration (variable_declarator) @item))
(program (variable_declaration (variable_declarator) @item))
(program (export_statement (lexical_declaration (variable_declarator) @item)))
(program (export_statement (variable_declaration (variable_declarator) @item)))
(variable_declarator value: (arrow_function)) @item
(variable_declarator value: (function_expression)) @item
(if_statement) @block
(for_statement) @block
(for_in_statement) @block
(while_statement) @block
(do_statement) @block
(switch_statement) @block
(try_statement) @block
"#;
const JAVASCRIPT_OUTLINE_FALLBACK: &str = r#"
(function_declaration) @item
(class_body (method_definition) @item)
(class_declaration) @item
(variable_declarator value: (arrow_function)) @item
"#;

const PYTHON_OUTLINE: &str = r#"
(function_definition) @item
(class_definition) @item
(if_statement) @block
(for_statement) @block
(while_statement) @block
(try_statement) @block
(with_statement) @block
(match_statement) @block
"#;

const GO_OUTLINE: &str = r#"
(function_declaration) @item
(method_declaration) @item
(type_declaration) @item
(const_declaration) @item
(var_declaration) @item
(field_declaration) @item
(if_statement) @block
(for_statement) @block
(expression_switch_statement) @block
(type_switch_statement) @block
(select_statement) @block
"#;
const GO_OUTLINE_FALLBACK: &str = r#"
(function_declaration) @item
(method_declaration) @item
(type_declaration) @item
"#;

const CSHARP_OUTLINE: &str = r#"
(namespace_declaration) @item
(class_declaration) @item
(struct_declaration) @item
(interface_declaration) @item
(enum_declaration) @item
(record_declaration) @item
(method_declaration) @item
(constructor_declaration) @item
(destructor_declaration) @item
(property_declaration) @item
(field_declaration) @item
(event_declaration) @item
(delegate_declaration) @item
(enum_member_declaration) @item
(if_statement) @block
(for_statement) @block
(foreach_statement) @block
(while_statement) @block
(switch_statement) @block
(try_statement) @block
"#;
const CSHARP_OUTLINE_FALLBACK: &str = r#"
(namespace_declaration) @item
(class_declaration) @item
(struct_declaration) @item
(interface_declaration) @item
(enum_declaration) @item
(method_declaration) @item
(property_declaration) @item
"#;

struct LanguageOutline {
    language: Language,
    query: Query,
}

/// Build a query from the first source that compiles against `lang`.
fn build_query(lang: &Language, sources: &[&str]) -> Option<Query> {
    sources.iter().find_map(|src| Query::new(lang, src).ok())
}

/// Register a language's outline query if it compiles against the grammar.
fn register(
    outlines: &mut Vec<(&'static str, LanguageOutline)>,
    ext: &'static str,
    lang: Language,
    sources: &[&str],
) {
    if let Some(query) = build_query(&lang, sources) {
        outlines.push((ext, LanguageOutline { language: lang, query }));
    }
}

static LANGUAGE_OUTLINES: Lazy<Vec<(&'static str, LanguageOutline)>> = Lazy::new(|| {
    let mut outlines = Vec::new();

    register(
        &mut outlines,
        "rs",
        tree_sitter_rust::LANGUAGE.into(),
        &[RUST_OUTLINE, RUST_OUTLINE_FALLBACK],
    );
    register(
        &mut outlines,
        "ts",
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        &[TYPESCRIPT_OUTLINE, TYPESCRIPT_OUTLINE_FALLBACK],
    );
    register(
        &mut outlines,
        "tsx",
        tree_sitter_typescript::LANGUAGE_TSX.into(),
        &[TYPESCRIPT_OUTLINE, TYPESCRIPT_OUTLINE_FALLBACK],
    );
    register(
        &mut outlines,
        "js",
        tree_sitter_javascript::LANGUAGE.into(),
        &[JAVASCRIPT_OUTLINE, JAVASCRIPT_OUTLINE_FALLBACK],
    );
    register(
        &mut outlines,
        "jsx",
        tree_sitter_javascript::LANGUAGE.into(),
        &[JAVASCRIPT_OUTLINE, JAVASCRIPT_OUTLINE_FALLBACK],
    );
    register(
        &mut outlines,
        "py",
        tree_sitter_python::LANGUAGE.into(),
        &[PYTHON_OUTLINE],
    );
    register(
        &mut outlines,
        "go",
        tree_sitter_go::LANGUAGE.into(),
        &[GO_OUTLINE, GO_OUTLINE_FALLBACK],
    );
    register(
        &mut outlines,
        "cs",
        tree_sitter_c_sharp::LANGUAGE.into(),
        &[CSHARP_OUTLINE, CSHARP_OUTLINE_FALLBACK],
    );

    outlines
});

fn get_language_outline(filename: &str) -> Option<&'static LanguageOutline> {
    let ext = Path::new(filename).extension().and_then(|e| e.to_str())?;
    LANGUAGE_OUTLINES
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, ctx)| ctx)
}

/// Parse `source` and return its declaration outline in source order.
///
/// Returns an empty vec for unsupported languages, empty input, or parse failures.
pub fn compute_outline(source: &str, filename: &str) -> Vec<OutlineEntry> {
    if source.is_empty() {
        return Vec::new();
    }

    let Some(lang_outline) = get_language_outline(filename) else {
        return Vec::new();
    };

    let mut parser = Parser::new();
    if parser.set_language(&lang_outline.language).is_err() {
        return Vec::new();
    }

    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };

    // Control-flow nodes are captured as `@block` (vs `@item` for declarations).
    // Blocks only earn a row when they span enough lines, so trivial one-liner
    // guards don't flood the outline while real if/try/loop bodies still show.
    const MIN_BLOCK_SPAN: usize = 2; // node must cover >= 3 source lines
    let block_idx = lang_outline
        .query
        .capture_names()
        .iter()
        .position(|n| *n == "block");

    // Collect (start_row, end_row) for every captured node.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&lang_outline.query, tree.root_node(), source.as_bytes());
    while let Some(m) = matches.next() {
        for capture in m.captures {
            let node = capture.node;
            let (start, end) = (node.start_position().row, node.end_position().row);
            let is_block = block_idx == Some(capture.index as usize);
            if is_block && end.saturating_sub(start) < MIN_BLOCK_SPAN {
                continue;
            }
            spans.push((start, end));
        }
    }

    // Sort by start row; for ties keep the widest span (outermost) first so the
    // dedup below keeps the enclosing declaration when two share a start line.
    spans.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    spans.dedup_by_key(|(start, _)| *start);

    spans
        .iter()
        .map(|&(start, end)| OutlineEntry {
            line_number: start + 1,
            start_row: start,
            end_row: end,
            changed: false,
        })
        .collect()
}

/// One row of the rendered diff in any view mode: either a real side-by-side
/// line to render normally, or a collapsed region (a fold). This is the single
/// coordinate space the renderer and every primitive (scroll, selection,
/// annotations, hunk focus, mouse) operate on; a view mode is just an adapter
/// that produces a different list of these.
#[derive(Clone, Debug, PartialEq)]
pub enum DiffRow {
    /// Index into the side-by-side diff of a line to render normally.
    Code(usize),
    /// A collapsed region of `lines` source lines that hid `added` insertions
    /// and `removed` deletions. `at` is the side-by-side index where the region
    /// begins (used to map positions between view modes).
    Fold {
        at: usize,
        lines: usize,
        added: usize,
        removed: usize,
    },
}

impl DiffRow {
    /// The side-by-side index this row anchors to.
    pub fn sbs_index(&self) -> usize {
        match self {
            DiffRow::Code(i) => *i,
            DiffRow::Fold { at, .. } => *at,
        }
    }
}

/// Build the row plan for a view mode — the *only* place modes differ.
///
/// - Full diff: every side-by-side line as a `Code` row (plan index ==
///   side-by-side index), so full-diff rendering is unchanged.
/// - Outline: declaration signature lines stay visible; changed bodies collapse
///   to `Fold` markers and unchanged bodies vanish entirely.
/// - Outline+diff: declarations *and* changed lines stay visible; every hidden
///   gap (changed or not) becomes a `Fold`, so adjacent visible line numbers are
///   never misleadingly contiguous.
///
/// `entries` are the declarations from [`compute_outline`]; `use_old_side`
/// selects which side's line numbers identify declarations (old for deleted
/// files). `changed_only` keeps only declarations whose body/signature changed.
pub fn row_plan(
    view_mode: super::state::ViewMode,
    side_by_side: &[super::types::DiffLine],
    entries: &[OutlineEntry],
    use_old_side: bool,
    changed_only: bool,
) -> Vec<DiffRow> {
    use super::types::ChangeType;
    use std::collections::HashSet;

    if !view_mode.is_outline() {
        return (0..side_by_side.len()).map(DiffRow::Code).collect();
    }
    let expand = view_mode.expands();

    let header_lines: HashSet<usize> = entries
        .iter()
        .filter(|e| !changed_only || e.changed)
        .map(|e| e.line_number)
        .collect();

    let mut rows: Vec<DiffRow> = Vec::new();
    let (mut added, mut removed, mut lines) = (0usize, 0usize, 0usize);
    let mut region_start = 0usize;

    // Outline drops unchanged folds (declarations stack quietly); outline+diff
    // keeps every gap so line-number jumps are always marked.
    let flush = |rows: &mut Vec<DiffRow>,
                 added: &mut usize,
                 removed: &mut usize,
                 lines: &mut usize,
                 at: usize| {
        let changed = *added > 0 || *removed > 0;
        if *lines > 0 && (changed || expand) {
            rows.push(DiffRow::Fold {
                at,
                lines: *lines,
                added: *added,
                removed: *removed,
            });
        }
        *added = 0;
        *removed = 0;
        *lines = 0;
    };

    for (i, dl) in side_by_side.iter().enumerate() {
        let lnum = if use_old_side {
            dl.old_line.as_ref().map(|(n, _)| *n)
        } else {
            dl.new_line.as_ref().map(|(n, _)| *n)
        };
        let is_header = lnum.is_some_and(|n| header_lines.contains(&n));
        let is_change = !matches!(dl.change_type, ChangeType::Equal);
        if is_header || (expand && is_change) {
            // Show this line directly; flush any hidden run before it.
            flush(&mut rows, &mut added, &mut removed, &mut lines, region_start);
            region_start = i + 1;
            rows.push(DiffRow::Code(i));
        } else {
            lines += 1;
            match dl.change_type {
                ChangeType::Insert => added += 1,
                ChangeType::Delete => removed += 1,
                ChangeType::Modified => {
                    added += 1;
                    removed += 1;
                }
                ChangeType::Equal => {}
            }
        }
    }
    flush(&mut rows, &mut added, &mut removed, &mut lines, region_start);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reconstruct each entry's signature line from the source for assertions.
    fn sigs(source: &str, entries: &[OutlineEntry]) -> Vec<String> {
        let lines: Vec<&str> = source.lines().collect();
        entries
            .iter()
            .map(|e| lines.get(e.start_row).map(|l| l.trim_end().to_string()).unwrap_or_default())
            .collect()
    }

    #[test]
    fn test_unsupported_language_is_empty() {
        assert!(compute_outline("some text\nmore", "notes.txt").is_empty());
    }

    #[test]
    fn test_empty_source() {
        assert!(compute_outline("", "main.rs").is_empty());
    }

    #[test]
    fn test_rust_functions_and_impl_nesting() {
        let source = r#"struct Foo {
    bar: i32,
}

impl Foo {
    fn new() -> Self {
        Self { bar: 0 }
    }

    fn get(&self) -> i32 {
        self.bar
    }
}

fn free_fn() {}
"#;
        let outline = compute_outline(source, "lib.rs");
        let signatures = sigs(source, &outline);

        for expected in [
            "struct Foo {",
            "    bar: i32,",
            "impl Foo {",
            "    fn new() -> Self {",
            "    fn get(&self) -> i32 {",
            "fn free_fn() {}",
        ] {
            assert!(
                signatures.iter().any(|s| s == expected),
                "outline should contain {expected:?}, got {signatures:?}"
            );
        }

        // The impl encloses its methods (nesting reflected in the line spans).
        let impl_entry = outline
            .iter()
            .find(|e| e.start_row == 4)
            .expect("impl Foo on row 4");
        let new_method = outline
            .iter()
            .find(|e| e.start_row == 5)
            .expect("fn new on row 5");
        assert!(impl_entry.start_row < new_method.start_row && impl_entry.end_row >= new_method.end_row);

        // Entries are returned in source order.
        let line_numbers: Vec<usize> = outline.iter().map(|e| e.line_number).collect();
        let mut sorted = line_numbers.clone();
        sorted.sort_unstable();
        assert_eq!(line_numbers, sorted);
    }

    #[test]
    fn test_typescript_class_with_fields_and_getters() {
        let source = r#"export class Service {
    private count: number = 0;
    readonly name: string;

    constructor() {
        this.name = "x";
    }

    get total(): number {
        return this.count;
    }

    run(): void {}
}
"#;
        let outline = compute_outline(source, "service.ts");
        let signatures = sigs(source, &outline);

        assert!(signatures.iter().any(|s| s.contains("class Service")));
        assert!(signatures.iter().any(|s| s.contains("private count")));
        assert!(signatures.iter().any(|s| s.contains("readonly name")));
        assert!(signatures.iter().any(|s| s.contains("constructor(")));
        assert!(signatures.iter().any(|s| s.contains("get total(")));
        assert!(signatures.iter().any(|s| s.contains("run(): void")));
    }

    /// Dev utility: dump the outline for a real file.
    /// `OUTLINE_FILE=/abs/path.tsx cargo test dump_outline -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dump_outline() {
        let path = std::env::var("OUTLINE_FILE").expect("set OUTLINE_FILE");
        let src = std::fs::read_to_string(&path).expect("read file");
        let lines: Vec<&str> = src.lines().collect();
        let outline = compute_outline(&src, &path);
        println!("\n=== outline of {} ({} entries) ===", path, outline.len());
        for e in &outline {
            println!(
                "{:>5}  {}",
                e.line_number,
                lines.get(e.start_row).map(|l| l.trim_end()).unwrap_or("")
            );
        }
    }

    #[test]
    fn test_tsx_react_arrow_components_and_hooks() {
        let source = r#"import { useState } from "react";

type Props = { title: string };

export const Card = ({ title }: Props) => {
    const [open, setOpen] = useState(false);
    const toggle = () => setOpen((v) => !v);
    return <div onClick={() => toggle()}>{title}</div>;
};

export function useCounter() {
    return 0;
}

const helper = function () {
    return 1;
};
"#;
        let outline = compute_outline(source, "Card.tsx");
        let signatures = sigs(source, &outline);

        // Arrow-function component, its named handler, the function-decl hook,
        // a function-expression const, and the type alias are all captured.
        assert!(signatures.iter().any(|s| s.contains("export const Card")));
        assert!(signatures.iter().any(|s| s.contains("const toggle =")));
        assert!(signatures.iter().any(|s| s.contains("export function useCounter")));
        assert!(signatures.iter().any(|s| s.contains("const helper =")));
        assert!(signatures.iter().any(|s| s.contains("type Props")));
        // Inline JSX/callback arrows aren't bound to a declarator, so they're
        // excluded — the outline isn't flooded with anonymous callbacks.
        assert!(!signatures.iter().any(|s| s.contains("onClick")));
    }

    #[test]
    fn test_control_flow_blocks_with_one_liner_threshold() {
        let source = r#"export function handler(x: number) {
    if (x < 0) return -1;
    if (x > 100) {
        throw new Error("too big");
    }
    try {
        doThing();
    } catch (e) {
        log(e);
    }
}
"#;
        let outline = compute_outline(source, "h.ts");
        let signatures = sigs(source, &outline);

        // Multi-line control-flow earns a row...
        assert!(signatures.iter().any(|s| s.contains("if (x > 100)")));
        assert!(signatures.iter().any(|s| s.trim() == "try {"));
        // ...but a one-line guard does not (avoids flooding).
        assert!(!signatures.iter().any(|s| s.contains("if (x < 0)")));
        // Object-literal methods are not captured (only class methods): the
        // `fn` line never becomes its own entry, just the enclosing const.
        let obj_src = "const o = {\n  fn() {\n    return 1;\n  },\n};\n";
        let obj = compute_outline(obj_src, "o.ts");
        assert!(obj.iter().all(|e| e.start_row != 1), "object method should not be an entry");
    }

    #[test]
    fn test_python_functions_and_classes() {
        let source = r#"class Greeter:
    def __init__(self, name):
        self.name = name

    def greet(self):
        return self.name


def top_level():
    return 1
"#;
        let outline = compute_outline(source, "greet.py");
        let signatures = sigs(source, &outline);

        assert!(signatures.iter().any(|s| s.contains("class Greeter")));
        assert!(signatures.iter().any(|s| s.contains("def __init__")));
        assert!(signatures.iter().any(|s| s.contains("def greet")));
        assert!(signatures.iter().any(|s| s.contains("def top_level")));

        // The class encloses its methods.
        let cls = outline.iter().find(|e| e.start_row == 0).expect("class on row 0");
        let greet = outline
            .iter()
            .find(|e| signatures_at(source, e).contains("def greet"))
            .expect("def greet present");
        assert!(cls.start_row < greet.start_row && cls.end_row >= greet.end_row);
    }

    fn signatures_at(source: &str, e: &OutlineEntry) -> String {
        source.lines().nth(e.start_row).unwrap_or_default().to_string()
    }
}
