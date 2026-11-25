//! Review threads: comments anchored to a file or line range, with agent replies.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::diff::{DiffPanelFocus, FileDiff};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnnotationTarget {
    File,
    LineRange {
        #[serde(rename = "side")]
        panel: DiffPanelFocus,
        start_line: usize,
        end_line: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Author {
    Human,
    Agent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: Author,
    pub author_name: String,
    pub body: String,
    pub created_at: u64,
    #[serde(default)]
    pub queued: bool,
}

impl Comment {
    pub fn human(body: String, queued: bool) -> Self {
        Self {
            id: new_id(),
            author: Author::Human,
            author_name: "you".to_string(),
            body,
            created_at: now_seconds(),
            queued,
        }
    }

    pub fn agent(body: String, author_name: String) -> Self {
        Self {
            id: new_id(),
            author: Author::Agent,
            author_name,
            body,
            created_at: now_seconds(),
            queued: false,
        }
    }

    #[cfg(feature = "jj")]
    pub fn format_time(&self) -> String {
        use chrono::{DateTime, Local};
        let created_at = UNIX_EPOCH + std::time::Duration::from_secs(self.created_at);
        DateTime::<Local>::from(created_at)
            .format("%H:%M")
            .to_string()
    }

    #[cfg(not(feature = "jj"))]
    pub fn format_time(&self) -> String {
        format!(
            "{:02}:{:02}",
            (self.created_at / 3600) % 24,
            (self.created_at / 60) % 60
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    #[serde(rename = "file")]
    pub filename: String,
    pub target: AnnotationTarget,
    pub anchor_text: Option<String>,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default)]
    pub outdated: bool,
    pub created_at: u64,
    pub comments: Vec<Comment>,
}

pub const EARLIER_COMMENTS_LABEL: &str = "earlier comments:";

impl Thread {
    pub fn new(
        filename: String,
        target: AnnotationTarget,
        anchor_text: Option<String>,
        first_comment: Comment,
    ) -> Self {
        Self {
            id: new_id(),
            filename,
            target,
            anchor_text,
            resolved: false,
            outdated: false,
            created_at: first_comment.created_at,
            comments: vec![first_comment],
        }
    }

    pub fn sent_comments(&self) -> impl Iterator<Item = &Comment> {
        self.comments.iter().filter(|comment| !comment.queued)
    }

    pub fn awaiting_agent(&self) -> bool {
        !self.resolved
            && self
                .sent_comments()
                .last()
                .is_some_and(|comment| comment.author == Author::Human)
    }

    pub fn draft(&self) -> Option<&Comment> {
        self.comments.last().filter(|comment| comment.queued)
    }

    pub fn queued_count(&self) -> usize {
        self.comments
            .iter()
            .filter(|comment| comment.queued)
            .count()
    }

    pub fn mark_sent(&mut self, only: Option<&str>) -> usize {
        let mut sent = 0;
        for comment in &mut self.comments {
            if comment.queued && only.is_none_or(|id| id == comment.id) {
                comment.queued = false;
                sent += 1;
            }
        }
        sent
    }

    /// Inserts an agent reply after already-sent comments so unsent drafts stay last.
    pub fn add_agent_reply(&mut self, reply: Comment) {
        let sent_count = self.sent_comments().count();
        self.comments.insert(sent_count, reply);
    }

    pub fn target_label(&self) -> &'static str {
        match &self.target {
            AnnotationTarget::File => "file",
            AnnotationTarget::LineRange { panel, .. } => match panel {
                DiffPanelFocus::Old => "old",
                DiffPanelFocus::New | DiffPanelFocus::None => "new",
            },
        }
    }

    pub fn line_range_display(&self) -> String {
        match &self.target {
            AnnotationTarget::File => String::new(),
            AnnotationTarget::LineRange {
                start_line,
                end_line,
                ..
            } => {
                if start_line == end_line {
                    format!("L{start_line}")
                } else {
                    format!("L{start_line}-{end_line}")
                }
            }
        }
    }

    pub fn line_reference(&self) -> Option<String> {
        let AnnotationTarget::LineRange {
            panel,
            start_line,
            end_line,
        } = &self.target
        else {
            return None;
        };
        let side = if is_old_side(*panel) { "LEFT" } else { "RIGHT" };
        Some(if start_line == end_line {
            format!("line {start_line} ({side})")
        } else {
            format!("lines {start_line}-{end_line} ({side})")
        })
    }

    /// Formats the thread for the coding agent. Omits anchored code when earlier
    /// comments were already sent so follow-up prompts stay short.
    pub fn describe<'a>(&self, comments: impl Iterator<Item = &'a Comment>) -> String {
        const MAX_CODE_LINES: usize = 40;

        let comments: Vec<&Comment> = comments.collect();
        let earlier_comments = self
            .sent_comments()
            .filter(|sent| comments.iter().all(|comment| comment.id != sent.id))
            .count();

        let mut text = format!("## Thread {} · {}", self.id, self.filename);
        if let Some(line_reference) = self.line_reference() {
            text.push_str(&format!(" {line_reference}"));
        }
        let states = [
            (self.resolved, "resolved"),
            (self.outdated, "outdated: the code under it changed"),
            (self.awaiting_agent(), "awaiting your reply"),
        ];
        for (_, state) in states.iter().filter(|(applies, _)| *applies) {
            text.push_str(&format!(" · {state}"));
        }
        if earlier_comments > 0 {
            text.push_str(&format!(" · {EARLIER_COMMENTS_LABEL} {earlier_comments}"));
        }

        if let Some(anchor_text) = self.anchor_text.as_ref().filter(|_| earlier_comments == 0) {
            let code_lines: Vec<&str> = anchor_text.split('\n').collect();
            let shown_lines = code_lines.len().min(MAX_CODE_LINES);
            text.push_str(&format!(
                "\n\n```\n{}\n```",
                code_lines[..shown_lines].join("\n")
            ));
            if shown_lines < code_lines.len() {
                text.push_str(&format!(
                    "\n({} more lines)",
                    code_lines.len() - shown_lines
                ));
            }
        }
        for comment in comments {
            text.push_str(&format!("\n\n{}: {}", comment.author_name, comment.body));
        }
        text
    }

    pub fn overlaps(&self, filename: &str, target: &AnnotationTarget) -> bool {
        if self.filename != filename {
            return false;
        }
        match (&self.target, target) {
            (AnnotationTarget::File, AnnotationTarget::File) => true,
            (
                AnnotationTarget::LineRange {
                    panel,
                    start_line,
                    end_line,
                },
                AnnotationTarget::LineRange {
                    panel: other_panel,
                    start_line: other_start,
                    end_line: other_end,
                },
            ) => {
                is_old_side(*panel) == is_old_side(*other_panel)
                    && start_line <= other_end
                    && other_start <= end_line
            }
            _ => false,
        }
    }

    /// Re-anchors the thread after a diff reload: stays put if unchanged, follows a
    /// unique exact match on the same side, and marks outdated otherwise.
    fn refresh_anchor(&mut self, file_diffs: &[FileDiff]) {
        let Some(diff) = file_diffs
            .iter()
            .find(|diff| diff.filename == self.filename)
        else {
            self.outdated = true;
            return;
        };
        let (
            AnnotationTarget::LineRange {
                panel,
                start_line,
                end_line,
            },
            Some(anchor_text),
        ) = (&mut self.target, &self.anchor_text)
        else {
            self.outdated = false;
            return;
        };

        let file_lines: Vec<&str> = side_content(diff, *panel).lines().collect();
        let anchor_lines: Vec<&str> = anchor_text.split('\n').collect();
        let anchored_at = |first_line: usize| {
            first_line >= 1
                && file_lines.get(first_line - 1..first_line - 1 + anchor_lines.len())
                    == Some(anchor_lines.as_slice())
        };
        if anchored_at(*start_line) {
            self.outdated = false;
            return;
        }

        let mut candidates = (1..=file_lines.len()).filter(|first_line| anchored_at(*first_line));
        match (candidates.next(), candidates.next()) {
            (Some(first_line), None) => {
                *start_line = first_line;
                *end_line = first_line + anchor_lines.len() - 1;
                self.outdated = false;
            }
            _ => self.outdated = true,
        }
    }
}

pub fn refresh_anchors(threads: &mut [Thread], file_diffs: &[FileDiff]) {
    for thread in threads {
        thread.refresh_anchor(file_diffs);
    }
}

pub fn anchor_text_for(
    file_diffs: &[FileDiff],
    filename: &str,
    target: &AnnotationTarget,
) -> Option<String> {
    let AnnotationTarget::LineRange {
        panel,
        start_line,
        end_line,
    } = target
    else {
        return None;
    };
    let diff = file_diffs.iter().find(|diff| diff.filename == filename)?;
    let file_lines: Vec<&str> = side_content(diff, *panel).lines().collect();
    let anchored_lines = file_lines.get(start_line.checked_sub(1)?..*end_line)?;
    Some(anchored_lines.join("\n"))
}

fn is_old_side(panel: DiffPanelFocus) -> bool {
    panel == DiffPanelFocus::Old
}

fn side_content(diff: &FileDiff, panel: DiffPanelFocus) -> &str {
    if is_old_side(panel) {
        &diff.old_content
    } else {
        &diff.new_content
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn new_id() -> String {
    static CREATED_IN_THIS_PROCESS: AtomicU64 = AtomicU64::new(0);
    let nanoseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut hasher = Sha256::new();
    hasher.update(nanoseconds.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(
        CREATED_IN_THIS_PROCESS
            .fetch_add(1, Ordering::Relaxed)
            .to_le_bytes(),
    );
    hasher.finalize()[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_diff(filename: &str, new_content: &str) -> FileDiff {
        FileDiff::from_contents(filename.to_string(), String::new(), new_content.to_string())
    }

    fn thread_on(content: &str, start_line: usize, end_line: usize) -> Thread {
        let target = AnnotationTarget::LineRange {
            panel: DiffPanelFocus::New,
            start_line,
            end_line,
        };
        let anchor_text = anchor_text_for(&[file_diff("a.rs", content)], "a.rs", &target);
        Thread::new(
            "a.rs".to_string(),
            target,
            anchor_text,
            Comment::human("check this".to_string(), false),
        )
    }

    fn lines_and_outdated(thread: &Thread) -> (usize, usize, bool) {
        match thread.target {
            AnnotationTarget::LineRange {
                start_line,
                end_line,
                ..
            } => (start_line, end_line, thread.outdated),
            AnnotationTarget::File => panic!("expected a line range"),
        }
    }

    fn after_change(mut thread: Thread, new_content: &str) -> (usize, usize, bool) {
        refresh_anchors(
            std::slice::from_mut(&mut thread),
            &[file_diff("a.rs", new_content)],
        );
        lines_and_outdated(&thread)
    }

    const ORIGINAL: &str = "fn a() {\n    one();\n    two();\n}\n";

    #[test]
    fn test_thread_anchor_tracking_and_outdated_recovery() {
        let moved = after_change(
            thread_on(ORIGINAL, 2, 3),
            "// new\n// header\nfn a() {\n    one();\n    two();\n}\n",
        );
        assert_eq!(moved, (4, 5, false));

        assert_eq!(
            after_change(
                thread_on(ORIGINAL, 2, 2),
                "fn a() {\n    uno();\n    two();\n}\n"
            ),
            (2, 2, true)
        );
        assert_eq!(
            after_change(thread_on(ORIGINAL, 2, 2), "fn a() {\n    two();\n}\n"),
            (2, 2, true)
        );
        assert_eq!(
            after_change(
                thread_on(ORIGINAL, 2, 2),
                "    one();\nfn a() {\n    two();\n    one();\n}\n"
            ),
            (2, 2, true)
        );

        let mut thread = thread_on(ORIGINAL, 2, 2);
        refresh_anchors(
            std::slice::from_mut(&mut thread),
            &[file_diff("other.rs", ORIGINAL)],
        );
        assert!(thread.outdated);
        refresh_anchors(
            std::slice::from_mut(&mut thread),
            &[file_diff("a.rs", ORIGINAL)],
        );
        assert_eq!(lines_and_outdated(&thread), (2, 2, false));
    }

    #[test]
    fn test_thread_awaiting_and_agent_reply_ordering() {
        let mut thread = thread_on(ORIGINAL, 2, 2);
        assert!(thread.awaiting_agent());

        thread
            .comments
            .push(Comment::human("follow-up".to_string(), true));
        thread.add_agent_reply(Comment::agent("reply".to_string(), "pi".to_string()));

        let bodies: Vec<&str> = thread.comments.iter().map(|c| c.body.as_str()).collect();
        assert_eq!(bodies, vec!["check this", "reply", "follow-up"]);
        assert_eq!(thread.draft().map(|c| c.body.as_str()), Some("follow-up"));
        assert!(!thread.awaiting_agent());

        thread.mark_sent(None);
        assert!(thread.awaiting_agent());

        thread.resolved = true;
        assert!(!thread.awaiting_agent());
    }
}
