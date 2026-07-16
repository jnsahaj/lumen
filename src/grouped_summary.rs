use std::collections::HashSet;

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, Deserialize)]
pub struct GroupedSummary {
    pub groups: Vec<DiffGroup>,
    #[serde(default)]
    pub overall_summary: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiffGroup {
    pub title: String,
    #[serde(default)]
    pub files: Vec<String>,
    pub summary: String,
}

#[derive(Error, Debug)]
pub enum GroupedSummaryError {
    #[error("could not parse grouped summary as JSON: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("model returned an empty grouped summary")]
    Empty,
}

#[derive(Debug, Default)]
pub struct ReconcileReport {
    /// Referenced by a group but not in ground truth.
    pub unknown_files: Vec<String>,
    /// In ground truth but appear in no group.
    pub ungrouped_files: Vec<String>,
}

impl ReconcileReport {
    pub fn is_clean(&self) -> bool {
        self.unknown_files.is_empty() && self.ungrouped_files.is_empty()
    }
}

/// Strip a markdown code fence around a JSON payload, with a fallback that
/// slices out the outermost `{...}` if stray prose surrounds the JSON.
fn strip_json_fence(raw: &str) -> &str {
    let trimmed = raw.trim();

    let unfenced = if trimmed.starts_with("```") {
        let without_leading_fence = trimmed
            .strip_prefix("```")
            .and_then(|rest| rest.split_once('\n'))
            .map(|(_, rest)| rest)
            .unwrap_or(trimmed);

        without_leading_fence
            .trim_end()
            .strip_suffix("```")
            .unwrap_or(without_leading_fence)
            .trim()
    } else {
        trimmed
    };

    if unfenced.starts_with('{') {
        return unfenced;
    }

    match (unfenced.find('{'), unfenced.rfind('}')) {
        (Some(start), Some(end)) if start <= end => &unfenced[start..=end],
        _ => trimmed,
    }
}

/// Parse a model response into a [`GroupedSummary`], tolerating markdown
/// fences and stray prose around the JSON payload.
pub fn parse_grouped_summary(raw: &str) -> Result<GroupedSummary, GroupedSummaryError> {
    let json = strip_json_fence(raw);
    let summary: GroupedSummary = serde_json::from_str(json)?;

    if summary.groups.is_empty() {
        return Err(GroupedSummaryError::Empty);
    }

    Ok(summary)
}

/// Extract the ordered, deduplicated list of file paths changed in a unified
/// diff, straight from its `+++`/`---` file headers.
pub fn files_from_unified_diff(diff: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();

    let lines: Vec<&str> = diff.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let path = if let Some(new_path) = line.strip_prefix("+++ b/") {
            Some(new_path.to_string())
        } else if *line == "+++ /dev/null" {
            lines
                .get(idx.wrapping_sub(1))
                .and_then(|prev| prev.strip_prefix("--- a/"))
                .map(str::to_string)
        } else {
            None
        };

        if let Some(path) = path {
            if seen.insert(path.clone()) {
                files.push(path);
            }
        }
    }

    files
}

/// Cross-check the model's grouping against the diff's ground-truth file
/// list, dropping hallucinated files and collecting anything left out into
/// an `Ungrouped` catch-all group.
pub fn reconcile_groups(summary: &mut GroupedSummary, ground_truth: &[String]) -> ReconcileReport {
    let known: HashSet<&str> = ground_truth.iter().map(String::as_str).collect();
    let mut report = ReconcileReport::default();
    let mut seen_unknown = HashSet::new();
    let mut grouped_files: HashSet<String> = HashSet::new();

    for group in &mut summary.groups {
        let mut kept = Vec::with_capacity(group.files.len());
        for file in group.files.drain(..) {
            if known.contains(file.as_str()) {
                grouped_files.insert(file.clone());
                kept.push(file);
            } else if seen_unknown.insert(file.clone()) {
                report.unknown_files.push(file);
            }
        }
        group.files = kept;
    }

    let ungrouped: Vec<String> = ground_truth
        .iter()
        .filter(|file| !grouped_files.contains(file.as_str()))
        .cloned()
        .collect();

    if !ungrouped.is_empty() {
        report.ungrouped_files = ungrouped.clone();
        summary.groups.push(DiffGroup {
            title: "Ungrouped".to_string(),
            files: ungrouped,
            summary: "Files not confidently assigned to a group.".to_string(),
        });
    }

    report
}

/// Render a [`GroupedSummary`] as markdown for terminal display.
pub fn render_markdown(summary: &GroupedSummary) -> String {
    let mut out = String::new();

    for group in &summary.groups {
        out.push_str(&format!("## {}\n\n", group.title));
        for file in &group.files {
            out.push_str(&format!("- `{}`\n", file));
        }
        out.push('\n');
        out.push_str(&group.summary);
        out.push_str("\n\n");
    }

    if let Some(overall) = &summary.overall_summary {
        if !overall.is_empty() {
            out.push_str("## Overall Summary\n\n");
            out.push_str(overall);
            out.push('\n');
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_json_fence_passes_through_plain_json() {
        let raw = r#"{"groups":[]}"#;
        assert_eq!(strip_json_fence(raw), r#"{"groups":[]}"#);
    }

    #[test]
    fn strip_json_fence_strips_json_language_tag() {
        let raw = "```json\n{\"groups\":[]}\n```";
        assert_eq!(strip_json_fence(raw), "{\"groups\":[]}");
    }

    #[test]
    fn strip_json_fence_strips_bare_fence() {
        let raw = "```\n{\"groups\":[]}\n```";
        assert_eq!(strip_json_fence(raw), "{\"groups\":[]}");
    }

    #[test]
    fn strip_json_fence_extracts_json_from_stray_prose() {
        let raw = "Here you go:\n{\"groups\":[]}\nHope that helps!";
        assert_eq!(strip_json_fence(raw), "{\"groups\":[]}");
    }

    #[test]
    fn parse_grouped_summary_parses_groups_and_overall_summary() {
        let raw = r#"{"groups":[{"title":"Feature","files":["src/a.rs"],"summary":"Adds a."}],"overall_summary":"Adds feature a."}"#;
        let summary = parse_grouped_summary(raw).unwrap();
        assert_eq!(summary.groups.len(), 1);
        assert_eq!(summary.groups[0].title, "Feature");
        assert_eq!(summary.overall_summary, Some("Adds feature a.".to_string()));
    }

    #[test]
    fn parse_grouped_summary_defaults_overall_summary_to_none() {
        let raw = r#"{"groups":[{"title":"Feature","files":["src/a.rs"],"summary":"Adds a."}]}"#;
        let summary = parse_grouped_summary(raw).unwrap();
        assert_eq!(summary.overall_summary, None);
    }

    #[test]
    fn parse_grouped_summary_rejects_empty_groups() {
        let raw = r#"{"groups":[]}"#;
        assert!(matches!(
            parse_grouped_summary(raw),
            Err(GroupedSummaryError::Empty)
        ));
    }

    #[test]
    fn parse_grouped_summary_rejects_garbage() {
        let raw = "not json at all";
        assert!(matches!(
            parse_grouped_summary(raw),
            Err(GroupedSummaryError::Parse(_))
        ));
    }

    #[test]
    fn files_from_unified_diff_returns_ordered_deduped_paths() {
        let diff = indoc::indoc! {"
            diff --git a/src/a.rs b/src/a.rs
            index 1111111..2222222 100644
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,1 +1,1 @@
            -old
            +new
            diff --git a/src/b.rs b/src/b.rs
            index 3333333..4444444 100644
            --- a/src/b.rs
            +++ b/src/b.rs
            @@ -1,1 +1,1 @@
            -old
            +new
        "};

        assert_eq!(
            files_from_unified_diff(diff),
            vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
        );
    }

    #[test]
    fn files_from_unified_diff_falls_back_to_old_path_for_deletions() {
        let diff = indoc::indoc! {"
            diff --git a/src/deleted.rs b/src/deleted.rs
            deleted file mode 100644
            index 1111111..0000000
            --- a/src/deleted.rs
            +++ /dev/null
            @@ -1,1 +0,0 @@
            -gone
        "};

        assert_eq!(
            files_from_unified_diff(diff),
            vec!["src/deleted.rs".to_string()]
        );
    }

    #[test]
    fn reconcile_groups_is_clean_when_all_files_accounted_for() {
        let mut summary = GroupedSummary {
            groups: vec![DiffGroup {
                title: "Feature".to_string(),
                files: vec!["src/a.rs".to_string()],
                summary: "Adds a.".to_string(),
            }],
            overall_summary: None,
        };
        let ground_truth = vec!["src/a.rs".to_string()];

        let report = reconcile_groups(&mut summary, &ground_truth);

        assert!(report.is_clean());
        assert_eq!(summary.groups.len(), 1);
    }

    #[test]
    fn reconcile_groups_drops_unknown_files_and_reports_them() {
        let mut summary = GroupedSummary {
            groups: vec![DiffGroup {
                title: "Feature".to_string(),
                files: vec!["src/a.rs".to_string(), "src/hallucinated.rs".to_string()],
                summary: "Adds a.".to_string(),
            }],
            overall_summary: None,
        };
        let ground_truth = vec!["src/a.rs".to_string()];

        let report = reconcile_groups(&mut summary, &ground_truth);

        assert_eq!(summary.groups[0].files, vec!["src/a.rs".to_string()]);
        assert_eq!(
            report.unknown_files,
            vec!["src/hallucinated.rs".to_string()]
        );
        assert!(!report.is_clean());
    }

    #[test]
    fn reconcile_groups_appends_ungrouped_group_for_missing_files() {
        let mut summary = GroupedSummary {
            groups: vec![DiffGroup {
                title: "Feature".to_string(),
                files: vec!["src/a.rs".to_string()],
                summary: "Adds a.".to_string(),
            }],
            overall_summary: None,
        };
        let ground_truth = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];

        let report = reconcile_groups(&mut summary, &ground_truth);

        assert_eq!(report.ungrouped_files, vec!["src/b.rs".to_string()]);
        assert!(!report.is_clean());
        assert_eq!(summary.groups.len(), 2);
        let ungrouped = &summary.groups[1];
        assert_eq!(ungrouped.title, "Ungrouped");
        assert_eq!(ungrouped.files, vec!["src/b.rs".to_string()]);
    }

    #[test]
    fn render_markdown_renders_groups_and_overall_summary() {
        let summary = GroupedSummary {
            groups: vec![
                DiffGroup {
                    title: "Feature".to_string(),
                    files: vec!["src/a.rs".to_string()],
                    summary: "Adds a.".to_string(),
                },
                DiffGroup {
                    title: "Tests".to_string(),
                    files: vec!["tests/a_test.rs".to_string()],
                    summary: "Covers a.".to_string(),
                },
            ],
            overall_summary: Some("Adds feature a with tests.".to_string()),
        };

        let expected = "## Feature\n\n\
            - `src/a.rs`\n\
            \n\
            Adds a.\n\n\
            ## Tests\n\n\
            - `tests/a_test.rs`\n\
            \n\
            Covers a.\n\n\
            ## Overall Summary\n\n\
            Adds feature a with tests.\n";

        assert_eq!(render_markdown(&summary), expected);
    }

    #[test]
    fn render_markdown_omits_overall_summary_section_when_none() {
        let summary = GroupedSummary {
            groups: vec![DiffGroup {
                title: "Feature".to_string(),
                files: vec!["src/a.rs".to_string()],
                summary: "Adds a.".to_string(),
            }],
            overall_summary: None,
        };

        let expected = "## Feature\n\n- `src/a.rs`\n\nAdds a.\n\n";

        assert_eq!(render_markdown(&summary), expected);
    }
}
