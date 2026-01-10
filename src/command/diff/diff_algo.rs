use similar::{ChangeTag, TextDiff};

use super::types::{expand_tabs, ChangeType, DiffLine, InlineSegment};

/// Check if a string contains meaningful (non-whitespace) content.
fn has_meaningful_content(s: &str) -> bool {
    s.chars().any(|c| !c.is_whitespace())
}

/// Compute word-level diff segments for a pair of modified lines.
/// Returns Some((old_segments, new_segments)) if word-level highlighting is useful,
/// or None if the whole line would be highlighted (not useful).
///
/// Following Git's diff-highlight approach:
/// "Pairs are interesting to highlight only if we are going to end up
/// highlighting a subset (i.e., not the whole line)."
fn compute_word_diff(
    old_text: &str,
    new_text: &str,
) -> Option<(Vec<InlineSegment>, Vec<InlineSegment>)> {
    let diff = TextDiff::from_words(old_text, new_text);

    let mut old_segments = Vec::new();
    let mut new_segments = Vec::new();

    // Track if we have any meaningful equal (unchanged) content
    let mut has_meaningful_equal = false;

    for change in diff.iter_all_changes() {
        let text = change.value().to_string();
        match change.tag() {
            ChangeTag::Equal => {
                // Check if this equal segment is meaningful (not just whitespace)
                if has_meaningful_content(&text) {
                    has_meaningful_equal = true;
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

    // Following Git's diff-highlight: only show word-level diff if we have
    // meaningful unchanged content (i.e., we're highlighting a subset, not the whole line)
    if !has_meaningful_equal {
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
