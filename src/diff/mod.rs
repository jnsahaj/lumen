pub mod algo;
pub mod context;
pub mod github;
pub mod highlight;
pub mod watcher;

use std::collections::HashSet;
use std::fs;
use std::ops::RangeInclusive;
use std::path::Path;

use crate::vcs::{CommitReference, VcsBackend};

pub use algo::{
    compute_line_stats, compute_side_by_side, count_added_removed, find_hunk_starts,
    hide_unchanged_rows, hunk_comment_lines, hunk_lines_on, HUNK_CONTEXT_ROWS,
};
pub use context::ContextConfig;
pub use github::PrInfo;

#[derive(Default)]
pub struct DiffOptions {
    pub reference: Option<CommitReference>,
    pub pr: Option<String>,
    pub detect_pr: bool,
    pub file: Option<Vec<String>>,
    pub watch: bool,
    pub theme: Option<String>,
    pub stacked: bool,
    pub focus: Option<String>,
    pub origin: Option<String>,
    pub wrap: bool,
    pub hunks_only: bool,
    pub agent_block: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
}

impl FileStatus {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
        }
    }

    pub fn from_contents(old: &str, new: &str) -> Self {
        if old.is_empty() && !new.is_empty() {
            Self::Added
        } else if !old.is_empty() && new.is_empty() {
            Self::Deleted
        } else {
            Self::Modified
        }
    }
}

pub struct FileDiff {
    pub filename: String,
    pub old_content: String,
    pub new_content: String,
    pub status: FileStatus,
    pub is_binary: bool,
}

impl FileDiff {
    pub fn from_contents(filename: String, old_content: String, new_content: String) -> Self {
        let status = FileStatus::from_contents(&old_content, &new_content);
        let is_binary = is_binary_content(&old_content) || is_binary_content(&new_content);
        Self {
            filename,
            old_content,
            new_content,
            status,
            is_binary,
        }
    }

    pub fn is_new_file(&self) -> bool {
        self.old_content.is_empty() && !self.new_content.is_empty()
    }

    pub fn is_deleted_file(&self) -> bool {
        !self.old_content.is_empty() && self.new_content.is_empty()
    }

    pub fn effective_fullscreen(&self, requested: DiffFullscreen) -> DiffFullscreen {
        if self.is_new_file() {
            DiffFullscreen::NewOnly
        } else if self.is_deleted_file() {
            DiffFullscreen::OldOnly
        } else {
            requested
        }
    }
}

pub fn is_binary_content(content: &str) -> bool {
    content.bytes().take(8192).any(|b| b == 0)
}

pub fn expand_tabs(s: &str, tab_width: usize) -> String {
    if !s.contains('\t') {
        return s.to_string();
    }
    if tab_width == 0 {
        return s.replace('\t', "");
    }
    let mut result = String::with_capacity(s.len());
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' {
            let spaces = tab_width - (col % tab_width);
            for _ in 0..spaces {
                result.push(' ');
            }
            col += spaces;
        } else {
            result.push(c);
            col += 1;
        }
    }
    result
}

#[derive(Clone)]
pub struct DiffViewSettings {
    pub context: ContextConfig,
    pub tab_width: usize,
    pub wrap: bool,
    pub hunks_only: bool,
}

impl Default for DiffViewSettings {
    fn default() -> Self {
        Self {
            context: ContextConfig::default(),
            tab_width: 4,
            wrap: false,
            hunks_only: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct InlineSegment {
    pub text: String,
    pub emphasized: bool,
}

pub struct DiffLine {
    pub old_line: Option<(usize, String)>,
    pub new_line: Option<(usize, String)>,
    pub change_type: ChangeType,
    pub old_segments: Option<Vec<InlineSegment>>,
    pub new_segments: Option<Vec<InlineSegment>>,
    /// Above zero on a separator row standing for folded unchanged lines.
    pub hidden_lines: usize,
}

impl DiffLine {
    pub fn separator(hidden_lines: usize) -> Self {
        Self {
            old_line: None,
            new_line: None,
            change_type: ChangeType::Equal,
            old_segments: None,
            new_segments: None,
            hidden_lines,
        }
    }

    pub fn line_number(&self, panel: DiffPanelFocus) -> Option<usize> {
        match panel {
            DiffPanelFocus::Old => self.old_line.as_ref().map(|(n, _)| *n),
            DiffPanelFocus::New | DiffPanelFocus::None => self.new_line.as_ref().map(|(n, _)| *n),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeType {
    Equal,
    Delete,
    Insert,
    Modified,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineStats {
    pub added: usize,
    pub removed: usize,
}

/// Which panel side of the diff is targeted. Serialized in `threads.json`.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffPanelFocus {
    #[default]
    None,
    Old,
    New,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum DiffFullscreen {
    #[default]
    None,
    OldOnly,
    NewOnly,
}

/// Cached computed representation of one `FileDiff` for fast rendering, hunk navigation,
/// and horizontal scrolling without rescanning large files every frame.
pub struct ComputedDiff {
    pub file_index: usize,
    pub rows: Vec<DiffLine>,
    pub hunks: Vec<usize>,
    pub stats: LineStats,
    pub max_old_width: usize,
    pub max_new_width: usize,
    pub pinned_ranges: Vec<(DiffPanelFocus, RangeInclusive<usize>)>,
}

impl ComputedDiff {
    pub fn build(
        file_index: usize,
        diff: &FileDiff,
        settings: &DiffViewSettings,
        pinned_ranges: Vec<(DiffPanelFocus, RangeInclusive<usize>)>,
    ) -> Self {
        let mut rows =
            compute_side_by_side(&diff.old_content, &diff.new_content, settings.tab_width);
        let stats = compute_line_stats(&rows);
        if settings.hunks_only {
            rows = hide_unchanged_rows(rows, HUNK_CONTEXT_ROWS, |row| {
                pinned_ranges.iter().any(|(panel, lines)| {
                    row.line_number(*panel)
                        .is_some_and(|line| lines.contains(&line))
                })
            });
        }
        let hunks = find_hunk_starts(&rows);
        let mut max_old_width = 0usize;
        let mut max_new_width = 0usize;
        for row in &rows {
            if let Some((_, text)) = &row.old_line {
                let width = if text.is_ascii() {
                    text.len()
                } else {
                    text.chars().count()
                };
                max_old_width = max_old_width.max(width);
            }
            if let Some((_, text)) = &row.new_line {
                let width = if text.is_ascii() {
                    text.len()
                } else {
                    text.chars().count()
                };
                max_new_width = max_new_width.max(width);
            }
        }
        Self {
            file_index,
            rows,
            hunks,
            stats,
            max_old_width,
            max_new_width,
            pinned_ranges,
        }
    }
}

pub fn get_current_branch(backend: &dyn VcsBackend) -> String {
    backend
        .get_current_branch()
        .ok()
        .flatten()
        .unwrap_or_else(|| "unknown".to_string())
}

pub enum DiffRefs {
    WorkingTree,
    Single(String),
    Range { from: String, to: String },
    RangeToWorkingTree { from: String },
}

impl DiffRefs {
    pub fn from_options(options: &DiffOptions, backend: &dyn VcsBackend) -> Self {
        match &options.reference {
            None => Self::WorkingTree,
            Some(CommitReference::Single(sha)) => Self::Single(sha.clone()),
            Some(CommitReference::Range { from, to }) => Self::Range {
                from: from.clone(),
                to: to.clone(),
            },
            Some(CommitReference::TripleDots { from, to }) => {
                let merge_base = backend.get_merge_base(from, to).unwrap_or_else(|e| {
                    eprintln!(
                        "Warning: failed to find merge-base for {from}...{to}: {e}. Using '{from}' as base."
                    );
                    from.clone()
                });
                Self::Range {
                    from: merge_base,
                    to: to.clone(),
                }
            }
            Some(CommitReference::RangeToWorkingTree { from }) => {
                Self::RangeToWorkingTree { from: from.clone() }
            }
            Some(CommitReference::MergeBaseToWorkingTree { from }) => {
                let merge_base = backend
                    .get_merge_base(from, backend.working_copy_parent_ref())
                    .unwrap_or_else(|_| from.clone());
                Self::RangeToWorkingTree { from: merge_base }
            }
        }
    }
}

pub fn validate_reference(
    reference: Option<&CommitReference>,
    backend: &dyn VcsBackend,
) -> Result<(), String> {
    let revisions: Vec<&str> = match reference {
        None => Vec::new(),
        Some(CommitReference::Single(rev)) => vec![rev.as_str()],
        Some(CommitReference::Range { from, to } | CommitReference::TripleDots { from, to }) => {
            vec![from.as_str(), to.as_str()]
        }
        Some(
            CommitReference::RangeToWorkingTree { from }
            | CommitReference::MergeBaseToWorkingTree { from },
        ) => vec![from.as_str()],
    };
    for revision in revisions {
        backend
            .resolve_ref(revision)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn get_changed_files(options: &DiffOptions, backend: &dyn VcsBackend) -> Vec<String> {
    let refs = DiffRefs::from_options(options, backend);

    let files: Vec<String> = match refs {
        DiffRefs::Single(sha) => backend.get_changed_files(&sha).unwrap_or_default(),
        DiffRefs::Range { from, to } => backend
            .get_range_changed_files(&from, &to)
            .unwrap_or_default(),
        DiffRefs::WorkingTree => backend.get_working_tree_changed_files().unwrap_or_default(),
        DiffRefs::RangeToWorkingTree { from } => {
            let head_ref = backend.working_copy_parent_ref();
            let range_files = backend
                .get_range_changed_files(&from, head_ref)
                .unwrap_or_default();
            let wt_files = backend.get_working_tree_changed_files().unwrap_or_default();
            let mut seen = HashSet::new();
            range_files
                .into_iter()
                .chain(wt_files)
                .filter(|f| seen.insert(f.clone()))
                .collect()
        }
    };

    match &options.file {
        Some(filter) => files.into_iter().filter(|f| filter.contains(f)).collect(),
        None => files,
    }
}

pub fn load_file_diffs(options: &DiffOptions, backend: &dyn VcsBackend) -> Vec<FileDiff> {
    let refs = DiffRefs::from_options(options, backend);
    let files = get_changed_files(options, backend);
    let paths: Vec<&Path> = files.iter().map(Path::new).collect();

    let old_ref = match &refs {
        DiffRefs::Single(sha) => backend.get_parent_ref_or_empty(sha).unwrap_or_default(),
        DiffRefs::Range { from, .. } | DiffRefs::RangeToWorkingTree { from } => from.clone(),
        DiffRefs::WorkingTree => backend.working_copy_parent_ref().to_string(),
    };
    let old_contents: Vec<String> = if old_ref.is_empty() {
        vec![String::new(); files.len()]
    } else {
        backend
            .get_files_content_at_ref(&old_ref, &paths)
            .into_iter()
            .map(Option::unwrap_or_default)
            .collect()
    };

    let new_contents: Vec<String> = match &refs {
        DiffRefs::Single(sha) => backend
            .get_files_content_at_ref(sha, &paths)
            .into_iter()
            .map(Option::unwrap_or_default)
            .collect(),
        DiffRefs::Range { to, .. } => backend
            .get_files_content_at_ref(to, &paths)
            .into_iter()
            .map(Option::unwrap_or_default)
            .collect(),
        DiffRefs::WorkingTree | DiffRefs::RangeToWorkingTree { .. } => {
            let root = backend.working_tree_root();
            files
                .iter()
                .map(|filename| fs::read_to_string(root.join(filename)).unwrap_or_default())
                .collect()
        }
    };

    files
        .into_iter()
        .zip(old_contents)
        .zip(new_contents)
        .map(|((filename, old_content), new_content)| {
            FileDiff::from_contents(filename, old_content, new_content)
        })
        .collect()
}

pub fn load_single_commit_diffs(
    commit_id: &str,
    file_filter: &Option<Vec<String>>,
    backend: &dyn VcsBackend,
) -> Vec<FileDiff> {
    let files = backend.get_changed_files(commit_id).unwrap_or_default();
    let files: Vec<String> = match file_filter {
        Some(filter) => files.into_iter().filter(|f| filter.contains(f)).collect(),
        None => files,
    };
    let paths: Vec<&Path> = files.iter().map(Path::new).collect();
    let parent_ref = backend
        .get_parent_ref_or_empty(commit_id)
        .unwrap_or_default();

    let old_contents: Vec<String> = if parent_ref.is_empty() {
        vec![String::new(); files.len()]
    } else {
        backend
            .get_files_content_at_ref(&parent_ref, &paths)
            .into_iter()
            .map(Option::unwrap_or_default)
            .collect()
    };
    let new_contents: Vec<String> = backend
        .get_files_content_at_ref(commit_id, &paths)
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect();

    files
        .into_iter()
        .zip(old_contents)
        .zip(new_contents)
        .map(|((filename, old_content), new_content)| {
            FileDiff::from_contents(filename, old_content, new_content)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::vcs::test_utils::git;
    use crate::vcs::GitBackend;

    fn init_repo() -> TempDir {
        let dir = TempDir::new().expect("temp dir");
        git(dir.path(), &["init"]);
        git(dir.path(), &["config", "user.email", "test@example.com"]);
        git(dir.path(), &["config", "user.name", "Test User"]);
        dir
    }

    fn options_for(reference: Option<CommitReference>) -> DiffOptions {
        DiffOptions {
            reference,
            hunks_only: true,
            ..DiffOptions::default()
        }
    }

    #[test]
    fn test_load_file_diffs_working_tree_untracked_in_new_dir() {
        let dir = init_repo();
        let p = dir.path();
        fs::write(p.join("existing.txt"), "existing content\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "initial"]);

        fs::create_dir_all(p.join("new_dir")).unwrap();
        fs::write(p.join("new_dir/file1.txt"), "file 1\n").unwrap();
        fs::write(p.join("new_dir/file2.txt"), "file 2\n").unwrap();

        let backend = GitBackend::new(p).unwrap();
        let diffs = load_file_diffs(&options_for(None), &backend);

        let file1 = diffs
            .iter()
            .find(|d| d.filename == "new_dir/file1.txt")
            .expect("file1 included");
        assert!(diffs.iter().any(|d| d.filename == "new_dir/file2.txt"));
        assert_eq!(file1.status, FileStatus::Added);
        assert!(file1.old_content.is_empty());
        assert_eq!(file1.new_content, "file 1\n");
    }

    #[test]
    fn test_load_file_diffs_range_to_working_tree() {
        let dir = init_repo();
        let p = dir.path();
        fs::write(p.join("base.txt"), "base\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "base"]);

        fs::write(p.join("committed.txt"), "committed\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "add committed"]);

        fs::write(p.join("uncommitted.txt"), "uncommitted\n").unwrap();
        fs::write(p.join("base.txt"), "base modified\n").unwrap();

        let backend = GitBackend::new(p).unwrap();
        let diffs = load_file_diffs(
            &options_for(Some(CommitReference::RangeToWorkingTree {
                from: "HEAD~1".to_string(),
            })),
            &backend,
        );

        let base = diffs.iter().find(|d| d.filename == "base.txt").unwrap();
        assert_eq!(base.old_content, "base\n");
        assert_eq!(base.new_content, "base modified\n");

        let committed = diffs
            .iter()
            .find(|d| d.filename == "committed.txt")
            .unwrap();
        assert_eq!(committed.old_content, "");
        assert_eq!(committed.new_content, "committed\n");
        assert!(diffs.iter().any(|d| d.filename == "uncommitted.txt"));
    }

    #[test]
    fn test_load_single_commit_diffs_and_stacked_commits() {
        let dir = init_repo();
        let p = dir.path();
        fs::write(p.join("base.txt"), "base\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "base"]);

        fs::write(p.join("a.txt"), "commit A\n").unwrap();
        fs::write(p.join("skip.txt"), "skip\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "commit A"]);

        fs::write(p.join("a.txt"), "commit B\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "commit B"]);

        let backend = GitBackend::new(p).unwrap();
        let commits = backend.get_commits_in_range("HEAD~2", "HEAD").unwrap();
        assert_eq!(commits.len(), 2);

        let filtered = load_single_commit_diffs(
            &commits[0].commit_id,
            &Some(vec!["a.txt".to_string()]),
            &backend,
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].status, FileStatus::Added);
        assert_eq!(filtered[0].new_content, "commit A\n");

        let modified = load_single_commit_diffs(&commits[1].commit_id, &None, &backend);
        assert_eq!(modified.len(), 1);
        assert_eq!(modified[0].status, FileStatus::Modified);
        assert_eq!(modified[0].old_content, "commit A\n");
        assert_eq!(modified[0].new_content, "commit B\n");
    }

    #[test]
    fn test_merge_base_to_working_tree_ignores_commits_added_to_the_base_branch() {
        let dir = init_repo();
        let p = dir.path();
        fs::write(p.join("shared.txt"), "shared\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "shared"]);
        git(p, &["checkout", "-b", "trunk"]);

        git(p, &["checkout", "-b", "feature"]);
        fs::write(p.join("feature.txt"), "feature\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "feature"]);

        git(p, &["checkout", "trunk"]);
        fs::write(p.join("trunk-only.txt"), "trunk\n").unwrap();
        git(p, &["add", "."]);
        git(p, &["commit", "-m", "trunk moves ahead"]);

        git(p, &["checkout", "feature"]);
        fs::write(p.join("uncommitted.txt"), "uncommitted\n").unwrap();

        let backend = GitBackend::new(p).unwrap();
        let filenames_for = |reference: CommitReference| -> Vec<String> {
            let mut names: Vec<String> = load_file_diffs(&options_for(Some(reference)), &backend)
                .into_iter()
                .map(|d| d.filename)
                .collect();
            names.sort();
            names
        };

        assert_eq!(
            filenames_for(CommitReference::MergeBaseToWorkingTree {
                from: "trunk".to_string(),
            }),
            vec!["feature.txt", "uncommitted.txt"],
        );
        assert_eq!(
            filenames_for(CommitReference::RangeToWorkingTree {
                from: "trunk".to_string(),
            }),
            vec!["feature.txt", "trunk-only.txt", "uncommitted.txt"],
        );
        assert_eq!(
            validate_reference(
                Some(&CommitReference::MergeBaseToWorkingTree {
                    from: "missing".to_string(),
                }),
                &backend,
            ),
            Err("invalid reference: missing".to_string()),
        );
    }
}
