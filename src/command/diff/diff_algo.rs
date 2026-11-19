use similar::{ChangeTag, TextDiff};

use super::types::{
    expand_tabs, ChangeType, DiffFullscreen, DiffLine, DiffPanelFocus, InlineSegment,
};

/// Check if a string contains meaningful (non-whitespace) content.
fn has_meaningful_content(s: &str) -> bool {
    s.chars().any(|c| !c.is_whitespace())
}

/// Compute word-level diff segments for a pair of modified lines.
/// Returns Some((old_segments, new_segments)) if word-level highlighting is useful,
/// or None if the lines are too different to benefit from word-level highlighting.
///
/// Following Git's diff-highlight approach, word-level highlighting is only shown
/// when a significant portion of the line is unchanged. This prevents noisy
/// highlighting when unrelated lines are paired together.
fn compute_word_diff(
    old_text: &str,
    new_text: &str,
) -> Option<(Vec<InlineSegment>, Vec<InlineSegment>)> {
    let diff = TextDiff::configure().diff_unicode_words(old_text, new_text);

    let mut old_segments = Vec::new();
    let mut new_segments = Vec::new();

    // Track lengths for ratio calculation
    let mut unchanged_len = 0usize;

    for change in diff.iter_all_changes() {
        let text = change.value().to_string();
        match change.tag() {
            ChangeTag::Equal => {
                // Track meaningful (non-whitespace) unchanged content length
                if has_meaningful_content(&text) {
                    unchanged_len += text.trim().len();
                }
                // Unchanged text goes to both sides, not emphasized
                old_segments.push(InlineSegment {
                    text: text.clone(),
                    emphasized: false,
                });
                new_segments.push(InlineSegment {
                    text,
                    emphasized: false,
                });
            }
            ChangeTag::Delete => {
                // Deleted text only goes to old side, emphasized
                old_segments.push(InlineSegment {
                    text,
                    emphasized: true,
                });
            }
            ChangeTag::Insert => {
                // Inserted text only goes to new side, emphasized
                new_segments.push(InlineSegment {
                    text,
                    emphasized: true,
                });
            }
        }
    }

    // Calculate ratio of unchanged content vs total content
    // Use the longer line as the baseline to be conservative
    let old_trimmed_len = old_text.trim().len();
    let new_trimmed_len = new_text.trim().len();
    let total_len = old_trimmed_len.max(new_trimmed_len);

    // Only show word-level diff if at least 20% of content is unchanged
    // This prevents noisy highlighting when unrelated lines are paired
    const MIN_UNCHANGED_RATIO: f64 = 0.20;
    if total_len == 0 || (unchanged_len as f64 / total_len as f64) < MIN_UNCHANGED_RATIO {
        return None;
    }

    Some((old_segments, new_segments))
}

/// Counts inserted and deleted lines for a file pair without building the
/// full side-by-side structure. Cheaper than `compute_side_by_side` when only
/// the totals are needed (e.g. sidebar aggregate stats).
pub fn count_added_removed(old: &str, new: &str) -> (usize, usize) {
    let diff = TextDiff::from_lines(old, new);
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += 1,
            ChangeTag::Delete => removed += 1,
            ChangeTag::Equal => {}
        }
    }
    (added, removed)
}

/// Computes a side-by-side diff using GitHub-style pairing.
///
/// This algorithm pairs consecutive deletions with consecutive insertions,
/// showing them on the same row. This avoids the visual offset where a modified
pub fn compute_side_by_side(old: &str, new: &str, tab_width: usize) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(old, new);
    let mut lines = Vec::new();
    let mut old_num = 1usize;
    let mut new_num = 1usize;

    // Collect all changes first
    let changes: Vec<_> = diff.iter_all_changes().collect();
    let mut i = 0;

    while i < changes.len() {
        let change = &changes[i];

        match change.tag() {
            ChangeTag::Equal => {
                let text = expand_tabs(change.value().trim_end(), tab_width);
                lines.push(DiffLine {
                    old_line: Some((old_num, text.clone())),
                    new_line: Some((new_num, text)),
                    change_type: ChangeType::Equal,
                    old_segments: None,
                    new_segments: None,
                    hidden_lines: 0,
                });
                old_num += 1;
                new_num += 1;
                i += 1;
            }
            ChangeTag::Delete => {
                // Collect consecutive deletions
                let mut deletions = Vec::new();
                while i < changes.len() && changes[i].tag() == ChangeTag::Delete {
                    deletions.push((
                        old_num,
                        expand_tabs(changes[i].value().trim_end(), tab_width),
                    ));
                    old_num += 1;
                    i += 1;
                }

                // Collect consecutive insertions that follow
                let mut insertions = Vec::new();
                while i < changes.len() && changes[i].tag() == ChangeTag::Insert {
                    insertions.push((
                        new_num,
                        expand_tabs(changes[i].value().trim_end(), tab_width),
                    ));
                    new_num += 1;
                    i += 1;
                }

                // Pair deletions with insertions
                let max_len = deletions.len().max(insertions.len());
                for j in 0..max_len {
                    let old_line = deletions.get(j).cloned();
                    let new_line = insertions.get(j).cloned();

                    let change_type = match (&old_line, &new_line) {
                        (Some(_), Some(_)) => ChangeType::Modified,
                        (Some(_), None) => ChangeType::Delete,
                        (None, Some(_)) => ChangeType::Insert,
                        (None, None) => unreachable!(),
                    };

                    // Compute word-level diff for modified lines (if similar enough)
                    let (old_segments, new_segments) =
                        if matches!(change_type, ChangeType::Modified) {
                            let old_text = old_line.as_ref().map(|(_, t)| t.as_str()).unwrap_or("");
                            let new_text = new_line.as_ref().map(|(_, t)| t.as_str()).unwrap_or("");
                            // compute_word_diff returns None if lines are too different
                            if let Some((old_segs, new_segs)) =
                                compute_word_diff(old_text, new_text)
                            {
                                (Some(old_segs), Some(new_segs))
                            } else {
                                (None, None)
                            }
                        } else {
                            (None, None)
                        };

                    lines.push(DiffLine {
                        old_line,
                        new_line,
                        change_type,
                        old_segments,
                        new_segments,
                        hidden_lines: 0,
                    });
                }
            }
            ChangeTag::Insert => {
                // Handle insertions that aren't preceded by deletions
                lines.push(DiffLine {
                    old_line: None,
                    new_line: Some((new_num, expand_tabs(change.value().trim_end(), tab_width))),
                    change_type: ChangeType::Insert,
                    old_segments: None,
                    new_segments: None,
                    hidden_lines: 0,
                });
                new_num += 1;
                i += 1;
            }
        }
    }
    lines
}

/// Unchanged rows shown before and after each change in hunks-only view.
pub const HUNK_CONTEXT_ROWS: usize = 3;

/// Hides the unchanged rows further than `context` rows from a change, except those
/// `always_shown` picks. Each run of hidden rows becomes one separator row. A run of
/// a single row is shown instead: a separator would take the same room.
pub fn hide_unchanged_rows(
    rows: Vec<DiffLine>,
    context: usize,
    always_shown: impl Fn(&DiffLine) -> bool,
) -> Vec<DiffLine> {
    let mut shown: Vec<bool> = rows.iter().map(&always_shown).collect();
    for (index, row) in rows.iter().enumerate() {
        if !matches!(row.change_type, ChangeType::Equal) {
            let last = (index + context).min(rows.len() - 1);
            shown[index.saturating_sub(context)..=last].fill(true);
        }
    }

    let mut result = Vec::new();
    let mut hidden_run: Vec<DiffLine> = Vec::new();
    for (row, is_shown) in rows.into_iter().zip(shown) {
        if is_shown {
            push_hidden_run(&mut result, &mut hidden_run);
            result.push(row);
        } else {
            hidden_run.push(row);
        }
    }
    push_hidden_run(&mut result, &mut hidden_run);
    result
}

fn push_hidden_run(result: &mut Vec<DiffLine>, hidden_run: &mut Vec<DiffLine>) {
    if hidden_run.len() > 1 {
        result.push(DiffLine::separator(hidden_run.len()));
        hidden_run.clear();
    } else {
        result.append(hidden_run);
    }
}

/// The first and last line that the hunk starting at row `hunk_start` has on one side.
pub fn hunk_lines_on(
    rows: &[DiffLine],
    hunk_start: usize,
    panel: DiffPanelFocus,
) -> Option<(usize, usize)> {
    let mut lines = rows
        .iter()
        .skip(hunk_start)
        .take_while(|row| !matches!(row.change_type, ChangeType::Equal))
        .filter_map(|row| row.line_number(panel));
    let start_line = lines.next()?;
    Some((start_line, lines.last().unwrap_or(start_line)))
}

/// The side and lines a comment on the hunk starting at row `hunk_start` is attached
/// to. It is the side being read: the old one when only the old panel is shown, the
/// new one otherwise. A hunk with no lines on that side, such as one that only
/// removes lines, is commented on the other side.
pub fn hunk_comment_lines(
    rows: &[DiffLine],
    hunk_start: usize,
    fullscreen: DiffFullscreen,
) -> Option<(DiffPanelFocus, usize, usize)> {
    let (shown_side, other_side) = match fullscreen {
        DiffFullscreen::OldOnly => (DiffPanelFocus::Old, DiffPanelFocus::New),
        _ => (DiffPanelFocus::New, DiffPanelFocus::Old),
    };
    [shown_side, other_side].into_iter().find_map(|panel| {
        let (start_line, end_line) = hunk_lines_on(rows, hunk_start, panel)?;
        Some((panel, start_line, end_line))
    })
}

pub fn find_hunk_starts(lines: &[DiffLine]) -> Vec<usize> {
    let mut hunks = Vec::new();
    let mut in_hunk = false;

    for (i, line) in lines.iter().enumerate() {
        let is_change = !matches!(line.change_type, ChangeType::Equal);
        if is_change && !in_hunk {
            hunks.push(i);
            in_hunk = true;
        } else if !is_change {
            in_hunk = false;
        }
    }
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered_lines(count: usize, changed: &[usize]) -> String {
        (1..=count)
            .map(|line| match changed.contains(&line) {
                true => format!("changed {}\n", line),
                false => format!("line {}\n", line),
            })
            .collect()
    }

    /// The new-side line number of each row, or `~N` for a separator hiding N lines.
    fn hunks_only_rows(line_count: usize, changed: &[usize], always_shown: &[usize]) -> Vec<String> {
        let rows = compute_side_by_side(
            &numbered_lines(line_count, &[]),
            &numbered_lines(line_count, changed),
            4,
        );
        let kept = |row: &DiffLine| row.new_line.as_ref().is_some_and(|(n, _)| always_shown.contains(n));
        hide_unchanged_rows(rows, HUNK_CONTEXT_ROWS, kept)
            .iter()
            .map(|row| match &row.new_line {
                Some((line, _)) => line.to_string(),
                None => format!("~{}", row.hidden_lines),
            })
            .collect()
    }

    #[test]
    fn test_a_change_is_shown_with_three_lines_around_it_and_the_rest_is_counted() {
        assert_eq!(
            hunks_only_rows(20, &[10], &[]),
            ["~6", "7", "8", "9", "10", "11", "12", "13", "~7"]
        );
        // At the edges of the file there is nothing to hide before or after.
        assert_eq!(hunks_only_rows(8, &[1], &[]), ["1", "2", "3", "4", "~4"]);
        assert_eq!(hunks_only_rows(8, &[8], &[]), ["~4", "5", "6", "7", "8"]);
    }

    #[test]
    fn test_changes_close_to_each_other_are_shown_without_a_separator() {
        let between = |second_change: usize| {
            let rows = hunks_only_rows(40, &[10, second_change], &[]);
            let from = rows.iter().position(|row| row == "13").expect("context after the first");
            let to = rows.iter().position(|row| row == &(second_change - 3).to_string()).expect("context before the second");
            rows[from + 1..to].to_vec()
        };

        assert!(between(17).is_empty(), "the two contexts touch");
        assert_eq!(between(18), ["14"], "one line is shown, not replaced by a separator");
        assert_eq!(between(19), ["~2"]);
    }

    #[test]
    fn test_a_kept_line_far_from_any_change_stays_between_two_separators() {
        assert_eq!(
            hunks_only_rows(30, &[5], &[20]),
            ["1", "2", "3", "4", "5", "6", "7", "8", "~11", "20", "~10"]
        );
    }

    #[test]
    fn test_hunks_start_at_the_same_changes_in_both_views() {
        let old = numbered_lines(60, &[]);
        let new = numbered_lines(60, &[10, 11, 30, 55]);
        let full_file = compute_side_by_side(&old, &new, 4);
        let hunks_in_full_file = find_hunk_starts(&full_file).len();

        let hunks_only = hide_unchanged_rows(full_file, HUNK_CONTEXT_ROWS, |_| false);

        assert_eq!(find_hunk_starts(&hunks_only).len(), hunks_in_full_file);
        assert_eq!(hunks_in_full_file, 3);
    }
}
