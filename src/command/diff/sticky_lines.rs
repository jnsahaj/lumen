/// Configuration for sticky lines feature
#[allow(dead_code)]
#[derive(Clone)]
pub struct StickyLinesConfig {
    pub enabled: bool,
    pub max_lines: usize,
}

impl Default for StickyLinesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_lines: 5,
        }
    }
}

/// Represents a sticky line with its content and original line number
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct StickyLine {
    pub line_number: usize,
    pub content: String,
    pub indentation: usize,
}

/// Computes the indentation level (number of leading spaces/tabs) of a line
#[allow(dead_code)]
fn get_indentation(line: &str) -> usize {
    let mut indent = 0;
    for ch in line.chars() {
        match ch {
            ' ' => indent += 1,
            '\t' => indent += 4,
            _ => break,
        }
    }
    indent
}

/// Checks if a line opens a scope block.
/// Returns true for function/class/control flow definitions.
#[allow(dead_code)]
fn is_block_opener(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Skip comments
    if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with("/*") {
        return false;
    }

    // Must end with { or : (for Python)
    if !trimmed.ends_with('{') && !trimmed.ends_with(':') {
        return false;
    }

    // Skip standalone braces or simple blocks
    if trimmed == "{" || trimmed == ":{" {
        return false;
    }

    // Check for common block openers
    let lower = trimmed.to_lowercase();

    // Function/method definitions
    if lower.contains("fn ")
        || lower.contains("func ")
        || lower.contains("function ")
        || lower.contains("def ")
        || lower.starts_with("pub fn ")
        || lower.starts_with("async fn ")
        || lower.starts_with("pub async fn ")
    {
        return true;
    }

    // Class/struct/impl/trait definitions
    if lower.contains("impl ")
        || lower.contains("struct ")
        || lower.contains("enum ")
        || lower.contains("trait ")
        || lower.contains("class ")
        || lower.contains("interface ")
        || lower.starts_with("pub struct ")
        || lower.starts_with("pub enum ")
        || lower.starts_with("pub trait ")
    {
        return true;
    }

    // Control flow
    if lower.starts_with("if ")
        || lower.starts_with("} else if ")
        || lower.starts_with("else if ")
        || lower == "else {"
        || lower.starts_with("for ")
        || lower.starts_with("while ")
        || lower.starts_with("loop ")
        || lower.starts_with("match ")
        || lower.starts_with("switch ")
        || lower.starts_with("try ")
        || lower.starts_with("catch ")
        || lower.starts_with("finally ")
    {
        return true;
    }

    // Module/namespace
    if lower.starts_with("mod ")
        || lower.starts_with("pub mod ")
        || lower.starts_with("namespace ")
        || lower.starts_with("module ")
    {
        return true;
    }

    // Closures and lambdas (only multi-line ones)
    if (lower.contains("|") && lower.ends_with('{'))
        || lower.contains("=> {")
        || lower.contains("-> {")
    {
        return true;
    }

    false
}

/// Checks if a line starts a multi-line function/method definition.
/// These are lines like `fn foo(`, `async function bar(`, `private async method(` etc.
#[allow(dead_code)]
fn is_multiline_fn_start(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Must end with ( to indicate params continue on next line
    if !trimmed.ends_with('(') {
        return false;
    }

    let lower = trimmed.to_lowercase();

    // Rust functions
    if lower.contains("fn ")
        || lower.starts_with("pub fn ")
        || lower.starts_with("async fn ")
        || lower.starts_with("pub async fn ")
    {
        return true;
    }

    // JS/TS functions and methods
    if lower.contains("function ") || lower.contains("function(") {
        return true;
    }

    // Class methods with modifiers (private, public, async, static, etc.)
    // Pattern: optional modifiers followed by method name and (
    if (lower.contains("private ")
        || lower.contains("public ")
        || lower.contains("protected ")
        || lower.contains("static ")
        || lower.contains("async "))
        && trimmed.ends_with('(')
    {
        return true;
    }

    // Python def
    if lower.starts_with("def ") || lower.contains(" def ") {
        return true;
    }

    false
}

/// Checks if a line ends a multi-line function signature and opens a block.
/// These are lines like `): ReturnType {` or `) {`
#[allow(dead_code)]
fn is_multiline_fn_end(line: &str) -> bool {
    let trimmed = line.trim();

    // Must end with {
    if !trimmed.ends_with('{') {
        return false;
    }

    // Pattern: starts with ) or contains ) followed by type annotation and {
    // Examples: `) {`, `): void {`, `): Promise<T> {`, `) => {`
    trimmed.starts_with(')')
        || trimmed.starts_with("):")
        || trimmed.starts_with(") {")
        || trimmed.starts_with(") =")
        || trimmed.contains(") {")
        || trimmed.contains("): ")
        || trimmed.contains(") =>")
}

/// Track which block openers are still "open" (not closed) at a given position.
/// This uses brace counting to determine which blocks contain the scroll position.
#[allow(dead_code)]
pub fn compute_sticky_lines(
    lines: &[(usize, String)],
    scroll_position: usize,
    config: &StickyLinesConfig,
) -> Vec<StickyLine> {
    if !config.enabled || lines.is_empty() || scroll_position == 0 {
        return Vec::new();
    }

    // Stack to track open blocks: (line_index, line_number, content, indent)
    let mut open_blocks: Vec<(usize, usize, String, usize)> = Vec::new();

    // Track pending multi-line function definition: (line_index, line_number, content, indent)
    let mut pending_multiline_fn: Option<(usize, usize, String, usize)> = None;

    // Process lines from start to scroll position
    for (idx, (line_num, content)) in lines.iter().enumerate() {
        if idx >= scroll_position {
            break;
        }

        let indent = get_indentation(content);
        let close_braces = content.matches('}').count();

        // Check for multi-line function start
        if is_multiline_fn_start(content) {
            pending_multiline_fn = Some((idx, *line_num, content.clone(), indent));
        }
        // Check for multi-line function end - use the start line as the block opener
        else if is_multiline_fn_end(content) {
            if let Some((start_idx, start_line_num, start_content, start_indent)) =
                pending_multiline_fn.take()
            {
                open_blocks.push((start_idx, start_line_num, start_content, start_indent));
            } else {
                // No pending start, might be single-line or arrow function
                open_blocks.push((idx, *line_num, content.clone(), indent));
            }
        }
        // Regular single-line block opener
        else if is_block_opener(content) {
            pending_multiline_fn = None; // Clear any pending (might be false positive)
            open_blocks.push((idx, *line_num, content.clone(), indent));
        }

        // Remove blocks that have been closed based on indentation
        // A block is closed when we see a closing brace at its indent level or less
        if close_braces > 0 && !is_multiline_fn_end(content) {
            while let Some((_, _, _, block_indent)) = open_blocks.last() {
                if indent <= *block_indent && !is_block_opener(content) {
                    open_blocks.pop();
                } else {
                    break;
                }
            }
        }
    }

    // Filter to only blocks that are genuinely still open at scroll position
    // by checking indentation relative to current view
    let current_indent = lines
        .get(scroll_position)
        .map(|(_, c)| get_indentation(c))
        .unwrap_or(0);

    let sticky_lines: Vec<StickyLine> = open_blocks
        .into_iter()
        .filter(|(_, _, _, indent)| *indent < current_indent || current_indent == 0)
        .map(|(_, line_num, content, indent)| StickyLine {
            line_number: line_num,
            content,
            indentation: indent,
        })
        .collect();

    // Limit to max_lines, keeping outermost scopes
    if sticky_lines.len() > config.max_lines {
        sticky_lines.into_iter().take(config.max_lines).collect()
    } else {
        sticky_lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_lines() {
        let config = StickyLinesConfig::default();
        let lines: Vec<(usize, String)> = vec![];
        assert!(compute_sticky_lines(&lines, 0, &config).is_empty());
    }

    #[test]
    fn test_basic_function_scope() {
        let config = StickyLinesConfig::default();
        let lines: Vec<(usize, String)> = vec![
            (1, "fn main() {".to_string()),
            (2, "    let x = 1;".to_string()),
            (3, "    let y = 2;".to_string()),
            (4, "    println!(\"hello\");".to_string()),
            (5, "}".to_string()),
        ];

        let sticky = compute_sticky_lines(&lines, 3, &config);
        assert_eq!(sticky.len(), 1);
        assert_eq!(sticky[0].content, "fn main() {");
    }

    #[test]
    fn test_nested_scopes() {
        let config = StickyLinesConfig::default();
        let lines: Vec<(usize, String)> = vec![
            (1, "impl Foo {".to_string()),
            (2, "    fn bar() {".to_string()),
            (3, "        if true {".to_string()),
            (4, "            let x = 1;".to_string()),
            (5, "        }".to_string()),
            (6, "    }".to_string()),
            (7, "}".to_string()),
        ];

        let sticky = compute_sticky_lines(&lines, 3, &config);
        assert_eq!(sticky.len(), 3);
        assert_eq!(sticky[0].content, "impl Foo {");
        assert_eq!(sticky[1].content, "    fn bar() {");
        assert_eq!(sticky[2].content, "        if true {");
    }

    #[test]
    fn test_closed_block_not_shown() {
        let config = StickyLinesConfig::default();
        let lines: Vec<(usize, String)> = vec![
            (1, "fn main() {".to_string()),
            (2, "    if true {".to_string()),
            (3, "        let x = 1;".to_string()),
            (4, "    }".to_string()),
            (5, "    let y = 2;".to_string()),
            (6, "}".to_string()),
        ];

        // Scroll position 5 is after the if block closed
        let sticky = compute_sticky_lines(&lines, 4, &config);
        // Should only show main(), not the closed if block
        assert_eq!(sticky.len(), 1);
        assert_eq!(sticky[0].content, "fn main() {");
    }

    #[test]
    fn test_multiline_function_definition() {
        let config = StickyLinesConfig::default();
        let lines: Vec<(usize, String)> = vec![
            (1, "class MyClass {".to_string()),
            (2, "    private async syncFilesFromGitPayload(".to_string()),
            (3, "        payload: VMPayloadWithGit,".to_string()),
            (4, "        env: string | undefined,".to_string()),
            (
                5,
                "        options?: { skipSyncAndRuntime?: boolean },".to_string(),
            ),
            (6, "    ): Promise<Map<string, Buffer>> {".to_string()),
            (7, "        const { git } = payload;".to_string()),
            (8, "        return new Map();".to_string()),
            (9, "    }".to_string()),
            (10, "}".to_string()),
        ];

        // Scroll to line 7 (inside the function body)
        let sticky = compute_sticky_lines(&lines, 6, &config);
        assert_eq!(sticky.len(), 2);
        assert_eq!(sticky[0].content, "class MyClass {");
        // Should show the first line of the function, not the line with {
        assert_eq!(
            sticky[1].content,
            "    private async syncFilesFromGitPayload("
        );
    }
}
