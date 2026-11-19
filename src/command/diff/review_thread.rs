//! Review threads: a comment left on a file or a line range, and the replies to it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::types::{DiffPanelFocus, FileDiff};

/// What a thread is attached to: a whole file or a line range on one panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Author {
    Human,
    Agent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: Author,
    pub author_name: String,
    pub body: String,
    /// Seconds since the Unix epoch.
    pub created_at: u64,
    /// Written by the reviewer but not sent to the agent yet.
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

    /// Creation time as HH:MM in local time.
    #[cfg(feature = "jj")]
    pub fn format_time(&self) -> String {
        use chrono::{DateTime, Local};
        let created_at = UNIX_EPOCH + std::time::Duration::from_secs(self.created_at);
        DateTime::<Local>::from(created_at).format("%H:%M").to_string()
    }

    /// Creation time as HH:MM in UTC, when chrono is not compiled in.
    #[cfg(not(feature = "jj"))]
    pub fn format_time(&self) -> String {
        format!(
            "{:02}:{:02}",
            (self.created_at / 3600) % 24,
            (self.created_at / 60) % 60
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    #[serde(rename = "file")]
    pub filename: String,
    pub target: AnnotationTarget,
    /// Text of the anchored lines when the thread was created.
    pub anchor_text: Option<String>,
    #[serde(default)]
    pub resolved: bool,
    /// The anchored lines changed or left the diff since the thread was created.
    #[serde(default)]
    pub outdated: bool,
    pub created_at: u64,
    pub comments: Vec<Comment>,
}

/// In a thread's heading, before the number of comments left out of the text.
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

    /// Comments the agent can see: everything except what the reviewer has not sent yet.
    pub fn sent_comments(&self) -> impl Iterator<Item = &Comment> {
        self.comments.iter().filter(|comment| !comment.queued)
    }

    /// The agent owes a reply: the thread is open and the last sent comment is the reviewer's.
    pub fn awaiting_agent(&self) -> bool {
        !self.resolved
            && self
                .sent_comments()
                .last()
                .is_some_and(|comment| comment.author == Author::Human)
    }

    /// The reviewer's latest comment while it is still unsent, and so still editable.
    pub fn draft(&self) -> Option<&Comment> {
        self.comments.last().filter(|comment| comment.queued)
    }

    pub fn queued_count(&self) -> usize {
        self.comments.iter().filter(|comment| comment.queued).count()
    }

    /// Marks queued comments as sent: all of them, or just the comment `only`.
    /// Returns how many were sent.
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

    /// Adds the agent's reply after the comments it has seen, so that comments the
    /// reviewer has not sent yet stay last.
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
                    format!("L{}", start_line)
                } else {
                    format!("L{}-{}", start_line, end_line)
                }
            }
        }
    }

    /// Where a line-range thread sits, GitHub review style: "line 3 (RIGHT)".
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
            format!("line {} ({})", start_line, side)
        } else {
            format!("lines {}-{} ({})", start_line, end_line, side)
        })
    }

    /// The thread as text for the agent: its id and place, the code it is anchored
    /// to, then `comments`. Long code is cut; the agent can read the file. The
    /// heading says how many comments the agent already has that are not in `comments`.
    pub fn describe<'a>(&self, comments: impl Iterator<Item = &'a Comment>) -> String {
        const MAX_CODE_LINES: usize = 40;

        let comments: Vec<&Comment> = comments.collect();
        let earlier_comments = self
            .sent_comments()
            .filter(|sent| comments.iter().all(|comment| comment.id != sent.id))
            .count();

        let mut text = format!("## Thread {} · {}", self.id, self.filename);
        if let Some(line_reference) = self.line_reference() {
            text.push_str(&format!(" {}", line_reference));
        }
        let states = [
            (self.resolved, "resolved"),
            (self.outdated, "outdated: the code under it changed"),
            (self.awaiting_agent(), "awaiting your reply"),
        ];
        for (_, state) in states.iter().filter(|(applies, _)| *applies) {
            text.push_str(&format!(" · {}", state));
        }
        if earlier_comments > 0 {
            text.push_str(&format!(" · {} {}", EARLIER_COMMENTS_LABEL, earlier_comments));
        }

        if let Some(anchor_text) = &self.anchor_text {
            let code_lines: Vec<&str> = anchor_text.split('\n').collect();
            let shown_lines = code_lines.len().min(MAX_CODE_LINES);
            text.push_str(&format!("\n\n```\n{}\n```", code_lines[..shown_lines].join("\n")));
            if shown_lines < code_lines.len() {
                text.push_str(&format!("\n({} more lines)", code_lines.len() - shown_lines));
            }
        }
        for comment in comments {
            text.push_str(&format!("\n\n{}: {}", comment.author_name, comment.body));
        }
        text
    }

    /// Whether this thread sits on lines that `target` also covers.
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

    /// Follows the anchored lines after the diff changed: keeps the thread where its
    /// text still is, moves it when the text is found unchanged at exactly one other
    /// place on the same side, and marks it outdated otherwise.
    fn refresh_anchor(&mut self, file_diffs: &[FileDiff]) {
        let Some(diff) = file_diffs.iter().find(|diff| diff.filename == self.filename) else {
            self.outdated = true;
            return;
        };
        let (AnnotationTarget::LineRange { panel, start_line, end_line }, Some(anchor_text)) =
            (&mut self.target, &self.anchor_text)
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

/// Text of the lines a new thread on `target` is anchored to.
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

/// A short id that stays unique when several processes create threads in one repository.
fn new_id() -> String {
    static CREATED_IN_THIS_PROCESS: AtomicU64 = AtomicU64::new(0);
    let nanoseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut hasher = Sha256::new();
    hasher.update(nanoseconds.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(CREATED_IN_THIS_PROCESS.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    hasher.finalize()[..4]
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::diff::types::FileStatus;

    fn file_diff(filename: &str, new_content: &str) -> FileDiff {
        FileDiff {
            filename: filename.to_string(),
            old_content: String::new(),
            new_content: new_content.to_string(),
            status: FileStatus::Added,
            is_binary: false,
        }
    }

    /// A thread on lines `start_line..=end_line` of the new side of `content`.
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
    fn test_thread_follows_its_lines_when_lines_are_inserted_above() {
        let thread = thread_on(ORIGINAL, 2, 3);

        let moved = after_change(thread, "// new\n// header\nfn a() {\n    one();\n    two();\n}\n");

        assert_eq!(moved, (4, 5, false));
    }

    #[test]
    fn test_thread_is_outdated_when_its_line_is_edited_or_deleted() {
        let edited = after_change(thread_on(ORIGINAL, 2, 2), "fn a() {\n    uno();\n    two();\n}\n");
        let deleted = after_change(thread_on(ORIGINAL, 2, 2), "fn a() {\n    two();\n}\n");

        assert_eq!(edited, (2, 2, true));
        assert_eq!(deleted, (2, 2, true));
    }

    #[test]
    fn test_thread_is_outdated_when_its_text_moved_and_appears_twice() {
        let thread = thread_on(ORIGINAL, 2, 2);

        let ambiguous = after_change(thread, "    one();\nfn a() {\n    two();\n    one();\n}\n");

        assert_eq!(ambiguous, (2, 2, true));
    }

    #[test]
    fn test_thread_is_outdated_when_its_file_leaves_the_diff_and_recovers_when_it_returns() {
        let mut thread = thread_on(ORIGINAL, 2, 2);

        refresh_anchors(std::slice::from_mut(&mut thread), &[file_diff("other.rs", ORIGINAL)]);
        assert!(thread.outdated);

        refresh_anchors(std::slice::from_mut(&mut thread), &[file_diff("a.rs", ORIGINAL)]);
        assert_eq!(lines_and_outdated(&thread), (2, 2, false));
    }

    #[test]
    fn test_thread_awaits_the_agent_until_it_replies_or_the_thread_is_resolved() {
        let mut thread = thread_on(ORIGINAL, 2, 2);
        assert!(thread.awaiting_agent());

        thread.add_agent_reply(Comment::agent("agreed".to_string(), "pi".to_string()));
        assert!(!thread.awaiting_agent());

        thread.comments.push(Comment::human("then do it".to_string(), true));
        assert!(!thread.awaiting_agent(), "a queued comment is not sent yet");

        thread.mark_sent(None);
        assert!(thread.awaiting_agent());

        thread.resolved = true;
        assert!(!thread.awaiting_agent());
    }

    #[test]
    fn test_agent_reply_goes_before_a_comment_that_is_still_queued() {
        let mut thread = thread_on(ORIGINAL, 2, 2);
        thread.comments.push(Comment::human("follow-up".to_string(), true));

        thread.add_agent_reply(Comment::agent("reply to the first".to_string(), "pi".to_string()));

        let bodies: Vec<&str> = thread.comments.iter().map(|c| c.body.as_str()).collect();
        assert_eq!(bodies, vec!["check this", "reply to the first", "follow-up"]);
        assert_eq!(thread.draft().map(|c| c.body.as_str()), Some("follow-up"));
        assert!(!thread.awaiting_agent());
    }
}
