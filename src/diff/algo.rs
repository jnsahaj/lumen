use similar::{ChangeTag, TextDiff};

use super::{
    expand_tabs, ChangeType, DiffFullscreen, DiffLine, DiffPanelFocus, InlineSegment, LineStats,
};

fn has_meaningful_content(s: &str) -> bool {
    s.chars().any(|c| !c.is_whitespace())
}

/// Word-level diff segments for a modified line pair, suppressed when less than
/// 20% of non-whitespace content is unchanged (Git's diff-highlight heuristic).
fn compute_word_diff(
    old_text: &str,
    new_text: &str,
) -> Option<(Vec<InlineSegment>, Vec<InlineSegment>)> {
    let diff = TextDiff::configure().diff_unicode_words(old_text, new_text);

    let mut old_segments = Vec::new();
    let mut new_segments = Vec::new();
    let mut unchanged_len = 0usize;

    for change in diff.iter_all_changes() {
        let text = change.value().to_string();
        match change.tag() {
            ChangeTag::Equal => {
                if has_meaningful_content(&text) {
                    unchanged_len += text.trim().len();
                }
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
                old_segments.push(InlineSegment {
                    text,
                    emphasized: true,
                });
            }
            ChangeTag::Insert => {
                new_segments.push(InlineSegment {
                    text,
                    emphasized: true,
                });
            }
        }
    }

    let total_len = old_text.trim().len().max(new_text.trim().len());
    const MIN_UNCHANGED_RATIO: f64 = 0.20;
    if total_len == 0 || (unchanged_len as f64 / total_len as f64) < MIN_UNCHANGED_RATIO {
        return None;
    }

    Some((old_segments, new_segments))
}

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

pub fn compute_line_stats(side_by_side: &[DiffLine]) -> LineStats {
    let mut added = 0;
    let mut removed = 0;
    for line in side_by_side {
        match line.change_type {
            ChangeType::Insert => added += 1,
            ChangeType::Delete => removed += 1,
            ChangeType::Modified => {
                added += 1;
                removed += 1;
            }
            ChangeType::Equal => {}
        }
    }
    LineStats { added, removed }
}

/// Computes a side-by-side diff pairing consecutive deletions with following insertions.
pub fn compute_side_by_side(old: &str, new: &str, tab_width: usize) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(old, new);
    let mut lines = Vec::new();
    let mut old_num = 1usize;
    let mut new_num = 1usize;

    let changes: Vec<_> = diff.iter_all_changes().collect();
    let mut i = 0;

    while i < changes.len() {
        match changes[i].tag() {
            ChangeTag::Equal => {
                let text = expand_tabs(changes[i].value().trim_end(), tab_width);
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
                let mut deletions = Vec::new();
                while i < changes.len() && changes[i].tag() == ChangeTag::Delete {
                    deletions.push((
                        old_num,
                        expand_tabs(changes[i].value().trim_end(), tab_width),
                    ));
                    old_num += 1;
                    i += 1;
                }

                let mut insertions = Vec::new();
                while i < changes.len() && changes[i].tag() == ChangeTag::Insert {
                    insertions.push((
                        new_num,
                        expand_tabs(changes[i].value().trim_end(), tab_width),
                    ));
                    new_num += 1;
                    i += 1;
                }

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

                    let (old_segments, new_segments) =
                        if matches!(change_type, ChangeType::Modified) {
                            let old_text = old_line.as_ref().map(|(_, t)| t.as_str()).unwrap_or("");
                            let new_text = new_line.as_ref().map(|(_, t)| t.as_str()).unwrap_or("");
                            match compute_word_diff(old_text, new_text) {
                                Some((old_segs, new_segs)) => (Some(old_segs), Some(new_segs)),
                                None => (None, None),
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
                lines.push(DiffLine {
                    old_line: None,
                    new_line: Some((
                        new_num,
                        expand_tabs(changes[i].value().trim_end(), tab_width),
                    )),
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

pub const HUNK_CONTEXT_ROWS: usize = 3;

/// Hides unchanged rows further than `context` rows from any change unless `always_shown`
/// keeps them. A run of one hidden row is shown directly since a separator takes the same space.
pub fn hide_unchanged_rows(
    rows: Vec<DiffLine>,
    context: usize,
    always_shown: impl Fn(&DiffLine) -> bool,
) -> Vec<DiffLine> {
    if rows.is_empty() {
        return rows;
    }
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

/// Side and line range a hunk comment attaches to: the visible reading side
/// (`Old` in `OldOnly`, otherwise `New`), falling back to the opposite side when empty.
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
            .map(|line| {
                if changed.contains(&line) {
                    format!("changed {line}\n")
                } else {
                    format!("line {line}\n")
                }
            })
            .collect()
    }

    fn hunks_only_rows(
        line_count: usize,
        changed: &[usize],
        always_shown: &[usize],
    ) -> Vec<String> {
        let rows = compute_side_by_side(
            &numbered_lines(line_count, &[]),
            &numbered_lines(line_count, changed),
            4,
        );
        let kept = |row: &DiffLine| {
            row.new_line
                .as_ref()
                .is_some_and(|(n, _)| always_shown.contains(n))
        };
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
        assert_eq!(hunks_only_rows(8, &[1], &[]), ["1", "2", "3", "4", "~4"]);
        assert_eq!(hunks_only_rows(8, &[8], &[]), ["~4", "5", "6", "7", "8"]);
    }

    #[test]
    fn test_changes_close_to_each_other_are_shown_without_a_separator() {
        let between = |second_change: usize| {
            let rows = hunks_only_rows(40, &[10, second_change], &[]);
            let from = rows.iter().position(|row| row == "13").unwrap();
            let to = rows
                .iter()
                .position(|row| row == &(second_change - 3).to_string())
                .unwrap();
            rows[from + 1..to].to_vec()
        };

        assert!(between(17).is_empty());
        assert_eq!(between(18), ["14"]);
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
