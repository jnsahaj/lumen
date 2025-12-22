use similar::{ChangeTag, TextDiff};

use crate::diff_ui::types::{ChangeType, DiffLine};

pub fn compute_side_by_side(old: &str, new: &str) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(old, new);
    let mut lines = Vec::new();
    let mut old_num = 1usize;
    let mut new_num = 1usize;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                lines.push(DiffLine {
                    old_line: Some((old_num, change.value().trim_end().to_string())),
                    new_line: Some((new_num, change.value().trim_end().to_string())),
                    change_type: ChangeType::Equal,
                });
                old_num += 1;
                new_num += 1;
            }
            ChangeTag::Delete => {
                lines.push(DiffLine {
                    old_line: Some((old_num, change.value().trim_end().to_string())),
                    new_line: None,
                    change_type: ChangeType::Delete,
                });
                old_num += 1;
            }
            ChangeTag::Insert => {
                lines.push(DiffLine {
                    old_line: None,
                    new_line: Some((new_num, change.value().trim_end().to_string())),
                    change_type: ChangeType::Insert,
                });
                new_num += 1;
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
