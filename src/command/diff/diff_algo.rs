use similar::{ChangeTag, TextDiff};

use super::types::{expand_tabs, ChangeType, DiffLine, InlineSegment};

/// Minimum similarity ratio (0.0-1.0) required to show word-level diff.
/// If lines are less similar than this threshold, we skip word-level highlighting
/// and just show the whole line as changed (like GitHub does for very different lines).
const WORD_DIFF_SIMILARITY_THRESHOLD: f32 = 0.35;

/// Compute word-level diff segments for a pair of modified lines.
/// Returns Some((old_segments, new_segments)) if lines are similar enough for word-level diff,
/// or None if the lines are too different (below similarity threshold).
fn compute_word_diff(
    old_text: &str,
    new_text: &str,
) -> Option<(Vec<InlineSegment>, Vec<InlineSegment>)> {
    let diff = TextDiff::from_words(old_text, new_text);

    // Calculate similarity ratio - if too low, skip word-level diff
    let ratio = diff.ratio();
    if ratio < WORD_DIFF_SIMILARITY_THRESHOLD {
        return None;
    }

    let mut old_segments = Vec::new();
    let mut new_segments = Vec::new();

    // Track how much is emphasized vs total
    let mut old_emphasized_len = 0usize;
    let mut new_emphasized_len = 0usize;

    for change in diff.iter_all_changes() {
        let text = change.value().to_string();
        match change.tag() {
            ChangeTag::Equal => {
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
                old_emphasized_len += text.len();
                old_segments.push(InlineSegment {
                    text,
                    emphasized: true,
                });
            }
            ChangeTag::Insert => {
                // Inserted text only goes to new side, emphasized
                new_emphasized_len += text.len();
                new_segments.push(InlineSegment {
                    text,
                    emphasized: true,
                });
            }
        }
    }

    // If almost everything is emphasized, skip word-level diff
    let old_total: usize = old_segments.iter().map(|s| s.text.len()).sum();
    let new_total: usize = new_segments.iter().map(|s| s.text.len()).sum();

    let old_emphasis_ratio = if old_total > 0 {
        old_emphasized_len as f32 / old_total as f32
    } else {
        1.0
    };
    let new_emphasis_ratio = if new_total > 0 {
        new_emphasized_len as f32 / new_total as f32
    } else {
        1.0
    };

    // If more than 80% of either line is emphasized, skip word-level diff
    if old_emphasis_ratio > 0.80 && new_emphasis_ratio > 0.80 {
        return None;
    }

    Some((old_segments, new_segments))
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
                            if let Some((old_segs, new_segs)) = compute_word_diff(old_text, new_text)
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
                });
                new_num += 1;
                i += 1;
            }
        }
    }
    lines
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
