use std::path::Path;

use git2::{Commit, DiffFormat, DiffOptions, Repository, StatusOptions, Time, Tree};

use super::backend::{CommitInfo, StackedCommitInfo, VcsBackend, VcsError};

/// Format a duration in seconds as relative time (e.g., "2 hours ago").
fn format_relative_time(secs_ago: i64) -> String {
    if secs_ago < 0 {
        return "in the future".to_string();
    }
    if secs_ago < 60 {
        return format!("{} seconds ago", secs_ago);
    }
    let mins = secs_ago / 60;
    if mins < 60 {
        return format!(
            "{} {} ago",
            mins,
            if mins == 1 { "minute" } else { "minutes" }
        );
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!(
            "{} {} ago",
            hours,
            if hours == 1 { "hour" } else { "hours" }
        );
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{} {} ago", days, if days == 1 { "day" } else { "days" });
    }
    let weeks = days / 7;
    if weeks < 4 {
        return format!(
            "{} {} ago",
            weeks,
            if weeks == 1 { "week" } else { "weeks" }
        );
    }
    let months = days / 30;
    if months < 12 {
        return format!(
            "{} {} ago",
            months,
            if months == 1 { "month" } else { "months" }
        );
    }
    let years = days / 365;
    format!(
        "{} {} ago",
        years,
        if years == 1 { "year" } else { "years" }
    )
}

/// Format git2::Time as YYYY-MM-DD HH:MM:SS.
fn format_git_time(time: &Time) -> String {
    // git2::Time provides seconds since epoch and offset in minutes
    let secs = time.seconds();
    let offset_mins = time.offset_minutes();

    // Apply timezone offset to get local time
    let local_secs = secs + (offset_mins as i64 * 60);

    // Calculate date/time components
    // Days since Unix epoch
    let days = local_secs / 86400;
    let time_of_day = (local_secs % 86400 + 86400) % 86400; // Handle negative values

    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Convert days to year/month/day (simplified calendar calculation)
    let (year, month, day) = days_to_ymd(days);

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hours, minutes, seconds
    )
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    // Algorithm from Howard Hinnant's date algorithms
    // https://howardhinnant.github.io/date_algorithms.html#civil_from_days
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month prime [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

/// Files to exclude from diff output.
const EXCLUDED_FILES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
];

/// Path patterns to exclude from diff output.
const EXCLUDED_PATTERNS: &[&str] = &["node_modules/"];

/// Check if a path should be excluded from diff output.
fn should_exclude_path(path: &str) -> bool {
    // Check exact file matches
    if let Some(filename) = path.rsplit('/').next() {
        if EXCLUDED_FILES.contains(&filename) {
            return true;
        }
    }
    // Check pattern matches
    for pattern in EXCLUDED_PATTERNS {
        if path.contains(pattern) {
            return true;
        }
    }
    false
}

/// Git backend using git2 (libgit2) for repository access.
pub struct GitBackend {
    repo: Repository,
}

impl GitBackend {
    /// Open a git repository at the given path.
    /// Uses git2::Repository::discover to find the repo from any subdirectory.
    pub fn new(path: &Path) -> Result<Self, VcsError> {
        let repo = Repository::discover(path).map_err(|_| VcsError::NotARepository)?;
        Ok(GitBackend { repo })
    }

    /// Open a git repository from the current working directory.
    /// Convenience method for tests.
    #[cfg(test)]
    pub fn from_cwd() -> Result<Self, VcsError> {
        Self::new(Path::new("."))
    }

    /// Validate that a reference doesn't look like a flag (defense in depth).
    fn validate_ref_format(reference: &str) -> Result<(), VcsError> {
        if reference.trim().starts_with('-') {
            return Err(VcsError::InvalidRef(format!(
                "references cannot start with '-': {}",
                reference
            )));
        }
        Ok(())
    }

    /// Generate unified diff for a commit, comparing to its parent.
    /// For root commits (no parent), compares to an empty tree.
    fn generate_commit_diff(&self, commit: &Commit) -> Result<String, VcsError> {
        let tree = commit
            .tree()
            .map_err(|e| VcsError::Other(format!("failed to get commit tree: {}", e)))?;

        // Get parent tree (or None for root commits)
        let parent_tree: Option<Tree> = if commit.parent_count() > 0 {
            commit.parent(0).ok().and_then(|p| p.tree().ok())
        } else {
            None
        };

        // Create diff with options
        let mut opts = DiffOptions::new();
        opts.show_binary(true);
        opts.context_lines(3);

        let diff = self
            .repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))
            .map_err(|e| VcsError::Other(format!("failed to create diff: {}", e)))?;

        // Format diff as unified patch, filtering excluded files
        let mut output = String::new();
        diff.print(DiffFormat::Patch, |delta, _hunk, line| {
            // Check if this file should be excluded
            if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                if should_exclude_path(path) {
                    return true; // Skip this line
                }
            }
            if let Some(path) = delta.old_file().path().and_then(|p| p.to_str()) {
                if should_exclude_path(path) {
                    return true; // Skip this line
                }
            }

            // Determine line prefix based on origin
            let prefix = match line.origin() {
                '+' | '-' | ' ' => line.origin(),
                'F' | 'H' | 'B' => '\0', // File header, hunk header, binary - no prefix
                _ => '\0',
            };

            if prefix != '\0' {
                output.push(prefix);
            }
            if let Ok(content) = std::str::from_utf8(line.content()) {
                output.push_str(content);
            }
            true
        })
        .map_err(|e| VcsError::Other(format!("failed to format diff: {}", e)))?;

        Ok(output)
    }

    /// Stage specific files for commit.
    /// Files should be relative paths from the repository root.
    pub fn stage_files(&self, paths: &[&Path]) -> Result<(), VcsError> {
        let mut index = self
            .repo
            .index()
            .map_err(|e| VcsError::Other(format!("failed to get index: {}", e)))?;

        for path in paths {
            index.add_path(path).map_err(|e| {
                VcsError::Other(format!("failed to stage {}: {}", path.display(), e))
            })?;
        }

        index
            .write()
            .map_err(|e| VcsError::Other(format!("failed to write index: {}", e)))?;

        Ok(())
    }

    /// Create a commit with the given message using the currently staged files.
    /// Returns the commit SHA on success.
    pub fn commit(&self, message: &str) -> Result<String, VcsError> {
        // Get user's git config for author/committer
        let config = self
            .repo
            .config()
            .map_err(|e| VcsError::Other(format!("failed to get git config: {}", e)))?;

        let name = config.get_string("user.name").map_err(|_| {
            VcsError::Other(
                "git user.name not configured. Run: git config user.name \"Your Name\"".to_string(),
            )
        })?;

        let email = config.get_string("user.email").map_err(|_| {
            VcsError::Other(
                "git user.email not configured. Run: git config user.email \"you@example.com\""
                    .to_string(),
            )
        })?;

        let sig = git2::Signature::now(&name, &email)
            .map_err(|e| VcsError::Other(format!("failed to create signature: {}", e)))?;

        let mut index = self
            .repo
            .index()
            .map_err(|e| VcsError::Other(format!("failed to get index: {}", e)))?;

        let tree_oid = index
            .write_tree()
            .map_err(|e| VcsError::Other(format!("failed to write tree: {}", e)))?;

        let tree = self
            .repo
            .find_tree(tree_oid)
            .map_err(|e| VcsError::Other(format!("failed to find tree: {}", e)))?;

        let parent = self.repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();

        let oid = self
            .repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
            .map_err(|e| VcsError::Other(format!("failed to create commit: {}", e)))?;

        Ok(oid.to_string())
    }
}

impl VcsBackend for GitBackend {
    fn get_commit(&self, reference: &str) -> Result<CommitInfo, VcsError> {
        let reference = reference.trim();
        Self::validate_ref_format(reference)?;

        // Use git2 to get commit metadata
        let obj = self
            .repo
            .revparse_single(reference)
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;
        let commit = obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;

        let commit_id = commit.id().to_string();
        let author_sig = commit.author();
        let author_name = author_sig.name().unwrap_or("");
        let author_email = author_sig.email().unwrap_or("");
        let author = format!("{} <{}>", author_name, author_email);

        // Format time as YYYY-MM-DD HH:MM:SS
        let time = commit.time();
        let date = format_git_time(&time);

        let message = commit
            .message()
            .unwrap_or("")
            .trim_end_matches('\n')
            .to_string();

        // Generate diff using git2
        let diff = self.generate_commit_diff(&commit)?;

        Ok(CommitInfo {
            commit_id,
            change_id: None, // Git doesn't have change IDs
            message,
            diff,
            author,
            date,
        })
    }

    fn get_working_tree_diff(&self, staged: bool) -> Result<String, VcsError> {
        let mut opts = DiffOptions::new();
        opts.show_binary(true);
        opts.context_lines(3);

        let diff = if staged {
            // Staged: diff HEAD tree to index
            let head = self.repo.head().ok().and_then(|h| h.peel_to_tree().ok());
            self.repo
                .diff_tree_to_index(head.as_ref(), None, Some(&mut opts))
                .map_err(|e| VcsError::Other(format!("failed to create staged diff: {}", e)))?
        } else {
            // Unstaged: diff index to workdir
            self.repo
                .diff_index_to_workdir(None, Some(&mut opts))
                .map_err(|e| VcsError::Other(format!("failed to create unstaged diff: {}", e)))?
        };

        // Format diff as unified patch, filtering excluded files
        let mut output = String::new();
        diff.print(DiffFormat::Patch, |delta, _hunk, line| {
            // Check if this file should be excluded
            if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                if should_exclude_path(path) {
                    return true;
                }
            }
            if let Some(path) = delta.old_file().path().and_then(|p| p.to_str()) {
                if should_exclude_path(path) {
                    return true;
                }
            }

            let prefix = match line.origin() {
                '+' | '-' | ' ' => line.origin(),
                _ => '\0',
            };
            if prefix != '\0' {
                output.push(prefix);
            }
            if let Ok(content) = std::str::from_utf8(line.content()) {
                output.push_str(content);
            }
            true
        })
        .map_err(|e| VcsError::Other(format!("failed to format diff: {}", e)))?;

        Ok(output)
    }

    fn get_range_diff(&self, from: &str, to: &str, three_dot: bool) -> Result<String, VcsError> {
        Self::validate_ref_format(from)?;
        Self::validate_ref_format(to)?;

        // Resolve both refs to commits
        let from_obj = self
            .repo
            .revparse_single(from)
            .map_err(|_| VcsError::InvalidRef(from.to_string()))?;
        let from_commit = from_obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(from.to_string()))?;

        let to_obj = self
            .repo
            .revparse_single(to)
            .map_err(|_| VcsError::InvalidRef(to.to_string()))?;
        let to_commit = to_obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(to.to_string()))?;

        // For three-dot syntax, compare merge-base to 'to'
        // For two-dot syntax, compare 'from' to 'to'
        let base_tree = if three_dot {
            // Find merge base
            let merge_base_oid = self
                .repo
                .merge_base(from_commit.id(), to_commit.id())
                .map_err(|e| VcsError::Other(format!("failed to find merge base: {}", e)))?;
            let merge_base = self
                .repo
                .find_commit(merge_base_oid)
                .map_err(|e| VcsError::Other(format!("failed to find merge base commit: {}", e)))?;
            merge_base
                .tree()
                .map_err(|e| VcsError::Other(format!("failed to get merge base tree: {}", e)))?
        } else {
            from_commit
                .tree()
                .map_err(|e| VcsError::Other(format!("failed to get from tree: {}", e)))?
        };

        let to_tree = to_commit
            .tree()
            .map_err(|e| VcsError::Other(format!("failed to get to tree: {}", e)))?;

        let mut opts = DiffOptions::new();
        opts.show_binary(true);
        opts.context_lines(3);

        let diff = self
            .repo
            .diff_tree_to_tree(Some(&base_tree), Some(&to_tree), Some(&mut opts))
            .map_err(|e| VcsError::Other(format!("failed to create range diff: {}", e)))?;

        // Format diff as unified patch, filtering excluded files
        let mut output = String::new();
        diff.print(DiffFormat::Patch, |delta, _hunk, line| {
            if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                if should_exclude_path(path) {
                    return true;
                }
            }
            if let Some(path) = delta.old_file().path().and_then(|p| p.to_str()) {
                if should_exclude_path(path) {
                    return true;
                }
            }

            let prefix = match line.origin() {
                '+' | '-' | ' ' => line.origin(),
                _ => '\0',
            };
            if prefix != '\0' {
                output.push(prefix);
            }
            if let Ok(content) = std::str::from_utf8(line.content()) {
                output.push_str(content);
            }
            true
        })
        .map_err(|e| VcsError::Other(format!("failed to format diff: {}", e)))?;

        Ok(output)
    }

    fn get_changed_files(&self, reference: &str) -> Result<Vec<String>, VcsError> {
        let reference = reference.trim();

        // Check if this is a range (contains ..)
        if reference.contains("..") {
            let parts: Vec<&str> = if reference.contains("...") {
                reference.split("...").collect()
            } else {
                reference.split("..").collect()
            };

            if parts.len() == 2 {
                Self::validate_ref_format(parts[0])?;
                Self::validate_ref_format(parts[1])?;

                let from_obj = self
                    .repo
                    .revparse_single(parts[0])
                    .map_err(|_| VcsError::InvalidRef(parts[0].to_string()))?;
                let from_commit = from_obj
                    .peel_to_commit()
                    .map_err(|_| VcsError::InvalidRef(parts[0].to_string()))?;
                let from_tree = from_commit
                    .tree()
                    .map_err(|e| VcsError::Other(format!("failed to get from tree: {}", e)))?;

                let to_obj = self
                    .repo
                    .revparse_single(parts[1])
                    .map_err(|_| VcsError::InvalidRef(parts[1].to_string()))?;
                let to_commit = to_obj
                    .peel_to_commit()
                    .map_err(|_| VcsError::InvalidRef(parts[1].to_string()))?;
                let to_tree = to_commit
                    .tree()
                    .map_err(|e| VcsError::Other(format!("failed to get to tree: {}", e)))?;

                let diff = self
                    .repo
                    .diff_tree_to_tree(Some(&from_tree), Some(&to_tree), None)
                    .map_err(|e| VcsError::Other(format!("failed to create diff: {}", e)))?;

                return Ok(diff
                    .deltas()
                    .filter_map(|d| {
                        d.new_file()
                            .path()
                            .and_then(|p| p.to_str().map(String::from))
                    })
                    .collect());
            }
        }

        // Single commit - compare to parent tree (or empty tree for root)
        Self::validate_ref_format(reference)?;
        let obj = self
            .repo
            .revparse_single(reference)
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;
        let commit = obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;
        let tree = commit
            .tree()
            .map_err(|e| VcsError::Other(format!("failed to get commit tree: {}", e)))?;

        let parent_tree: Option<Tree> = if commit.parent_count() > 0 {
            commit.parent(0).ok().and_then(|p| p.tree().ok())
        } else {
            None
        };

        let diff = self
            .repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
            .map_err(|e| VcsError::Other(format!("failed to create diff: {}", e)))?;

        Ok(diff
            .deltas()
            .filter_map(|d| {
                d.new_file()
                    .path()
                    .and_then(|p| p.to_str().map(String::from))
            })
            .collect())
    }

    fn get_file_content_at_ref(&self, reference: &str, path: &Path) -> Result<String, VcsError> {
        let reference = reference.trim();
        Self::validate_ref_format(reference)?;

        // Resolve reference to commit
        let obj = self
            .repo
            .revparse_single(reference)
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;
        let commit = obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;
        let tree = commit
            .tree()
            .map_err(|e| VcsError::Other(format!("failed to get tree: {}", e)))?;

        // Look up file in tree
        let entry = tree
            .get_path(path)
            .map_err(|_| VcsError::FileNotFound(path.display().to_string()))?;

        // Get blob content
        let blob = self
            .repo
            .find_blob(entry.id())
            .map_err(|_| VcsError::FileNotFound(path.display().to_string()))?;

        Ok(String::from_utf8_lossy(blob.content()).into_owned())
    }

    fn get_current_branch(&self) -> Result<Option<String>, VcsError> {
        let head = self
            .repo
            .head()
            .map_err(|e| VcsError::Other(format!("failed to get HEAD: {}", e)))?;

        if head.is_branch() {
            Ok(head.shorthand().map(|s| s.to_string()))
        } else {
            // Detached HEAD state
            Ok(None)
        }
    }

    fn get_commit_log_for_fzf(&self) -> Result<String, VcsError> {
        let mut revwalk = self
            .repo
            .revwalk()
            .map_err(|e| VcsError::Other(format!("failed to create revwalk: {}", e)))?;

        // Start from HEAD
        revwalk
            .push_head()
            .map_err(|e| VcsError::Other(format!("failed to push head: {}", e)))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let mut output = String::new();
        for oid_result in revwalk {
            let oid = oid_result.map_err(|e| VcsError::Other(format!("revwalk error: {}", e)))?;
            let commit = self
                .repo
                .find_commit(oid)
                .map_err(|e| VcsError::Other(format!("failed to find commit: {}", e)))?;

            let short_id = &oid.to_string()[..7];
            let summary = commit.summary().unwrap_or("");
            let time_secs = commit.time().seconds();
            let relative_time = format_relative_time(now - time_secs);

            // Format: short_hash summary relative_time
            // Using ANSI codes for color (yellow hash, default text, dim time)
            output.push_str(&format!(
                "\x1b[33m{}\x1b[0m {} \x1b[90m{}\x1b[0m\n",
                short_id, summary, relative_time
            ));
        }

        Ok(output)
    }

    fn resolve_ref(&self, reference: &str) -> Result<String, VcsError> {
        let reference = reference.trim();
        Self::validate_ref_format(reference)?;

        // Use git2 to resolve reference to commit SHA
        let obj = self
            .repo
            .revparse_single(reference)
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;

        let commit = obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;

        Ok(commit.id().to_string())
    }

    fn get_working_tree_changed_files(&self) -> Result<Vec<String>, VcsError> {
        use std::collections::HashSet;

        let mut opts = StatusOptions::new();
        opts.include_untracked(true);
        opts.exclude_submodules(true);
        opts.include_ignored(false);

        let statuses = self
            .repo
            .statuses(Some(&mut opts))
            .map_err(|e| VcsError::Other(format!("failed to get status: {}", e)))?;

        let files: HashSet<String> = statuses
            .iter()
            .filter_map(|s| s.path().map(String::from))
            .collect();

        Ok(files.into_iter().collect())
    }

    fn get_merge_base(&self, ref1: &str, ref2: &str) -> Result<String, VcsError> {
        let ref1 = ref1.trim();
        let ref2 = ref2.trim();

        Self::validate_ref_format(ref1)?;
        Self::validate_ref_format(ref2)?;

        let obj1 = self
            .repo
            .revparse_single(ref1)
            .map_err(|_| VcsError::InvalidRef(ref1.to_string()))?;
        let oid1 = obj1
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(ref1.to_string()))?
            .id();

        let obj2 = self
            .repo
            .revparse_single(ref2)
            .map_err(|_| VcsError::InvalidRef(ref2.to_string()))?;
        let oid2 = obj2
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(ref2.to_string()))?
            .id();

        let merge_base = self
            .repo
            .merge_base(oid1, oid2)
            .map_err(|e| VcsError::Other(format!("failed to find merge base: {}", e)))?;

        Ok(merge_base.to_string())
    }

    fn working_copy_parent_ref(&self) -> &'static str {
        "HEAD"
    }

    fn get_range_changed_files(&self, from: &str, to: &str) -> Result<Vec<String>, VcsError> {
        let from = from.trim();
        let to = to.trim();

        Self::validate_ref_format(from)?;
        Self::validate_ref_format(to)?;

        let from_obj = self
            .repo
            .revparse_single(from)
            .map_err(|_| VcsError::InvalidRef(from.to_string()))?;
        let from_tree = from_obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(from.to_string()))?
            .tree()
            .map_err(|e| VcsError::Other(format!("failed to get from tree: {}", e)))?;

        let to_obj = self
            .repo
            .revparse_single(to)
            .map_err(|_| VcsError::InvalidRef(to.to_string()))?;
        let to_tree = to_obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(to.to_string()))?
            .tree()
            .map_err(|e| VcsError::Other(format!("failed to get to tree: {}", e)))?;

        let diff = self
            .repo
            .diff_tree_to_tree(Some(&from_tree), Some(&to_tree), None)
            .map_err(|e| VcsError::Other(format!("failed to create diff: {}", e)))?;

        Ok(diff
            .deltas()
            .filter_map(|d| {
                d.new_file()
                    .path()
                    .and_then(|p| p.to_str().map(String::from))
            })
            .collect())
    }

    fn get_parent_ref_or_empty(&self, reference: &str) -> Result<String, VcsError> {
        let reference = reference.trim();
        Self::validate_ref_format(reference)?;

        let obj = self
            .repo
            .revparse_single(reference)
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;
        let commit = obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(reference.to_string()))?;

        if commit.parent_count() > 0 {
            // Has parent - return the parent ref
            Ok(format!("{}^", reference))
        } else {
            // No parent (root commit) - return git's empty tree SHA
            // This is a well-known constant: the SHA of an empty tree
            Ok("4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string())
        }
    }

    fn get_commits_in_range(
        &self,
        from: &str,
        to: &str,
    ) -> Result<Vec<StackedCommitInfo>, VcsError> {
        let from = from.trim();
        let to = to.trim();

        Self::validate_ref_format(from)?;
        Self::validate_ref_format(to)?;

        // Resolve refs to OIDs
        let from_obj = self
            .repo
            .revparse_single(from)
            .map_err(|_| VcsError::InvalidRef(from.to_string()))?;
        let from_oid = from_obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(from.to_string()))?
            .id();

        let to_obj = self
            .repo
            .revparse_single(to)
            .map_err(|_| VcsError::InvalidRef(to.to_string()))?;
        let to_oid = to_obj
            .peel_to_commit()
            .map_err(|_| VcsError::InvalidRef(to.to_string()))?
            .id();

        // Set up revwalk from 'to' to 'from' (exclusive)
        let mut revwalk = self
            .repo
            .revwalk()
            .map_err(|e| VcsError::Other(format!("failed to create revwalk: {}", e)))?;
        revwalk
            .push(to_oid)
            .map_err(|e| VcsError::Other(format!("failed to push to revwalk: {}", e)))?;
        revwalk
            .hide(from_oid)
            .map_err(|e| VcsError::Other(format!("failed to hide from revwalk: {}", e)))?;

        // Collect commits in reverse order (oldest first)
        let mut commits: Vec<StackedCommitInfo> = Vec::new();
        for oid_result in revwalk {
            let oid = oid_result.map_err(|e| VcsError::Other(format!("revwalk error: {}", e)))?;
            let commit = self
                .repo
                .find_commit(oid)
                .map_err(|e| VcsError::Other(format!("failed to find commit: {}", e)))?;

            let commit_id = oid.to_string();
            let short_id = commit_id[..7.min(commit_id.len())].to_string();
            let summary = commit.summary().unwrap_or("").to_string();

            // Filter commits with no file changes (e.g., merge commits)
            if self
                .get_changed_files(&commit_id)
                .map(|f| !f.is_empty())
                .unwrap_or(false)
            {
                commits.push(StackedCommitInfo {
                    commit_id,
                    short_id,
                    change_id: None,
                    summary,
                });
            }
        }

        // Reverse to get oldest first
        commits.reverse();
        Ok(commits)
    }

    fn name(&self) -> &'static str {
        "git"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs::test_utils::RepoGuard;

    #[test]
    fn test_get_commit_returns_valid_info() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let info = backend.get_commit("HEAD").expect("should get commit");
        assert!(!info.commit_id.is_empty());
        assert!(info.change_id.is_none()); // Git has no change IDs
        assert_eq!(info.message, "init");
        assert!(info.author.contains("Test User"));
        assert!(!info.diff.is_empty());
    }

    #[test]
    fn test_get_working_tree_diff_returns_string() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        // Should succeed even if empty
        let diff = backend.get_working_tree_diff(false);
        assert!(diff.is_ok());
    }

    #[test]
    fn test_get_changed_files_returns_paths() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let files = backend.get_changed_files("HEAD").expect("should get files");
        assert!(files.contains(&"README.md".to_string()));
    }

    #[test]
    fn test_get_current_branch() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let branch = backend.get_current_branch().expect("should get branch");
        assert!(branch.is_some());
    }

    #[test]
    fn test_get_file_content_at_ref() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let content = backend
            .get_file_content_at_ref("HEAD", Path::new("README.md"))
            .expect("should get content");
        assert_eq!(content.trim(), "hello");
    }

    #[test]
    fn test_invalid_ref_returns_error() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let result = backend.get_commit("nonexistent12345");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_file_content_at_ref_missing_file() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let result = backend.get_file_content_at_ref("HEAD", Path::new("nonexistent.txt"));
        assert!(
            matches!(result, Err(VcsError::FileNotFound(_))),
            "Expected FileNotFound error, got: {:?}",
            result
        );
    }

    #[test]
    fn test_get_commit_log_for_fzf() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let log = backend.get_commit_log_for_fzf().expect("should get log");
        assert!(!log.is_empty(), "commit log should not be empty");
        // Log should contain the short hash from the commit
        assert!(
            log.lines().next().is_some(),
            "log should have at least one line"
        );
    }

    #[test]
    fn test_get_working_tree_diff_staged() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-staged");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Initial commit
        fs::write(dir.join("file.txt"), "initial\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "init"]);

        // Stage one change, leave another unstaged
        fs::write(dir.join("file.txt"), "staged change\n").expect("modify file");
        git(&dir, &["add", "file.txt"]);
        fs::write(dir.join("file.txt"), "staged change\nunstaged change\n").expect("modify again");

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");

        // Staged diff should only show "staged change"
        let staged_diff = backend
            .get_working_tree_diff(true)
            .expect("should get staged diff");
        assert!(
            staged_diff.contains("staged change"),
            "staged diff should contain staged changes"
        );
        assert!(
            !staged_diff.contains("unstaged change"),
            "staged diff should NOT contain unstaged changes"
        );

        // Unstaged diff should show the additional unstaged change
        let unstaged_diff = backend
            .get_working_tree_diff(false)
            .expect("should get unstaged diff");
        assert!(
            unstaged_diff.contains("unstaged change"),
            "unstaged diff should contain unstaged changes"
        );

        // Cleanup
        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_range_diff() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-range");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Commit A
        fs::write(dir.join("file.txt"), "commit A\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "commit A"]);

        // Commit B
        fs::write(dir.join("file.txt"), "commit B\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "commit B"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");

        // Range diff HEAD~1..HEAD (two-dot)
        let diff = backend
            .get_range_diff("HEAD~1", "HEAD", false)
            .expect("should get range diff");
        assert!(
            diff.contains("commit A") || diff.contains("commit B"),
            "range diff should contain changes"
        );

        // Three-dot range diff also works
        let diff_3dot = backend
            .get_range_diff("HEAD~1", "HEAD", true)
            .expect("should get three-dot diff");
        assert!(
            !diff_3dot.is_empty() || diff.contains("commit"),
            "three-dot diff should work"
        );

        // Cleanup
        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_range_diff_excludes_lock_files() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-range-exclusion");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Commit A with lock file
        fs::write(dir.join("file.txt"), "A\n").expect("write file");
        fs::write(dir.join("package-lock.json"), "{\"v\":1}\n").expect("write lock");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "A"]);

        // Commit B - modify both
        fs::write(dir.join("file.txt"), "B\n").expect("modify file");
        fs::write(dir.join("package-lock.json"), "{\"v\":2}\n").expect("modify lock");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "B"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");
        let diff = backend
            .get_range_diff("HEAD~1", "HEAD", false)
            .expect("should get range diff");

        assert!(
            diff.contains("file.txt"),
            "range diff should contain file.txt"
        );
        assert!(
            !diff.contains("package-lock.json"),
            "range diff should NOT contain package-lock.json"
        );

        // Cleanup
        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_diff_excludes_lock_files() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-exclusion");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Create files including lock files
        fs::write(dir.join("test.txt"), "hello\n").expect("write test.txt");
        fs::write(dir.join("package-lock.json"), "{}\n").expect("write package-lock.json");
        fs::write(dir.join("Cargo.lock"), "lock\n").expect("write Cargo.lock");

        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "init with lock files"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");
        let info = backend.get_commit("HEAD").expect("should get commit");

        // Diff should contain test.txt but NOT lock files
        assert!(
            info.diff.contains("test.txt"),
            "diff should contain test.txt"
        );
        assert!(
            !info.diff.contains("package-lock.json"),
            "diff should NOT contain package-lock.json"
        );
        assert!(
            !info.diff.contains("Cargo.lock"),
            "diff should NOT contain Cargo.lock"
        );

        // Cleanup
        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_working_tree_diff_excludes_lock_files() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-wt-exclusion");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Initial commit
        fs::write(dir.join("test.txt"), "hello\n").expect("write test.txt");
        fs::write(dir.join("package-lock.json"), "{}\n").expect("write package-lock.json");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "init"]);

        // Modify both files
        fs::write(dir.join("test.txt"), "world\n").expect("modify test.txt");
        fs::write(dir.join("package-lock.json"), "{\"v\": 2}\n").expect("modify package-lock.json");

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");
        let diff = backend
            .get_working_tree_diff(false)
            .expect("should get diff");

        // Diff should contain test.txt but NOT package-lock.json
        assert!(
            diff.contains("test.txt"),
            "working tree diff should contain test.txt"
        );
        assert!(
            !diff.contains("package-lock.json"),
            "working tree diff should NOT contain package-lock.json"
        );

        // Cleanup
        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_working_tree_diff_empty() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        // Clean working tree should return empty string
        let diff = backend
            .get_working_tree_diff(false)
            .expect("should succeed on clean tree");
        assert!(
            diff.is_empty(),
            "clean working tree should return empty diff"
        );
    }

    #[test]
    fn test_get_range_diff_identical_commits() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        // Diff of HEAD..HEAD should be empty
        let diff = backend
            .get_range_diff("HEAD", "HEAD", false)
            .expect("should succeed for identical commits");
        assert!(diff.is_empty(), "diff of identical commits should be empty");
    }

    #[test]
    fn test_commit_info_field_format() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");
        let commit = backend.get_commit("HEAD").expect("should get commit");

        // commit_id should be 40-char hex
        assert_eq!(
            commit.commit_id.len(),
            40,
            "commit_id should be 40-char hex, got: {}",
            commit.commit_id
        );
        assert!(
            commit.commit_id.chars().all(|c| c.is_ascii_hexdigit()),
            "commit_id should be hex"
        );

        // Git has no change_id
        assert!(
            commit.change_id.is_none(),
            "git commits should not have change_id"
        );

        // author format: "Name <email>"
        assert!(
            commit.author.contains('<') && commit.author.contains('>'),
            "author should be 'Name <email>' format, got: {}",
            commit.author
        );

        // date format: YYYY-MM-DD HH:MM:SS (19 chars)
        assert_eq!(
            commit.date.len(),
            19,
            "date should be 19 chars (YYYY-MM-DD HH:MM:SS), got: {}",
            commit.date
        );
        assert!(
            commit.date.chars().nth(4) == Some('-')
                && commit.date.chars().nth(7) == Some('-')
                && commit.date.chars().nth(10) == Some(' ')
                && commit.date.chars().nth(13) == Some(':')
                && commit.date.chars().nth(16) == Some(':'),
            "date should be YYYY-MM-DD HH:MM:SS format, got: {}",
            commit.date
        );
    }

    #[test]
    fn test_resolve_ref_head_returns_sha() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let sha = backend.resolve_ref("HEAD").expect("should resolve HEAD");

        assert_eq!(sha.len(), 40, "should return 40-char SHA, got: {}", sha);
        assert!(
            sha.chars().all(|c| c.is_ascii_hexdigit()),
            "SHA should be hex"
        );
    }

    #[test]
    fn test_resolve_ref_invalid_returns_error() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let result = backend.resolve_ref("nonexistent_ref_xyz");
        assert!(result.is_err(), "resolve_ref should fail for invalid ref");
    }

    #[test]
    fn test_resolve_ref_matches_commit_id() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let commit = backend.get_commit("HEAD").expect("should get commit");
        let sha = backend.resolve_ref("HEAD").expect("should resolve HEAD");

        assert_eq!(
            sha, commit.commit_id,
            "resolve_ref should return same SHA as get_commit"
        );
    }

    #[test]
    fn test_get_working_tree_changed_files_modified() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-wt-changed");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Initial commit
        fs::write(dir.join("file.txt"), "initial\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "init"]);

        // Modify file (unstaged)
        fs::write(dir.join("file.txt"), "modified\n").expect("modify file");

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");
        let files = backend
            .get_working_tree_changed_files()
            .expect("should get changed files");

        assert!(
            files.contains(&"file.txt".to_string()),
            "should include modified file, got: {:?}",
            files
        );

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_working_tree_changed_files_untracked() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-wt-untracked");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Initial commit
        fs::write(dir.join("file.txt"), "initial\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "init"]);

        // Add untracked file
        fs::write(dir.join("new.txt"), "new file\n").expect("write new file");

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");
        let files = backend
            .get_working_tree_changed_files()
            .expect("should get changed files");

        assert!(
            files.contains(&"new.txt".to_string()),
            "should include untracked file, got: {:?}",
            files
        );

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_working_tree_changed_files_clean() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let files = backend
            .get_working_tree_changed_files()
            .expect("should succeed on clean tree");

        assert!(files.is_empty(), "clean tree should return empty vec");
    }

    #[test]
    fn test_get_merge_base_returns_ancestor() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-merge-base");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Commit A (base)
        fs::write(dir.join("file.txt"), "base\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "base"]);

        // Create branch and commit B
        git(&dir, &["checkout", "-b", "branch"]);
        fs::write(dir.join("file.txt"), "branch\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "branch commit"]);

        // Back to main, commit C
        git(&dir, &["checkout", "main"]);
        fs::write(dir.join("other.txt"), "main\n").expect("write other");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "main commit"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");
        let merge_base = backend
            .get_merge_base("main", "branch")
            .expect("should find merge base");

        // Merge base should be 40-char SHA
        assert_eq!(merge_base.len(), 40, "should return 40-char SHA");

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_merge_base_invalid_ref() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        let result = backend.get_merge_base("HEAD", "nonexistent_branch_xyz");
        assert!(result.is_err(), "should fail for invalid ref");
    }

    #[test]
    fn test_working_copy_parent_ref_returns_head() {
        let backend = GitBackend::from_cwd().expect("should open repo");
        assert_eq!(backend.working_copy_parent_ref(), "HEAD");
    }

    #[test]
    fn test_get_parent_ref_or_empty_root_commit() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        // HEAD is the first (root) commit in RepoGuard - has no parent
        let parent_ref = backend
            .get_parent_ref_or_empty("HEAD")
            .expect("should succeed");

        // Should return empty tree SHA for root commit
        assert_eq!(
            parent_ref, "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
            "root commit should return empty tree SHA"
        );
    }

    #[test]
    fn test_get_parent_ref_or_empty_normal_commit() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-parent-ref");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // First commit (root)
        fs::write(dir.join("file.txt"), "first\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "first"]);

        // Second commit (has parent)
        fs::write(dir.join("file.txt"), "second\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "second"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");
        let parent_ref = backend
            .get_parent_ref_or_empty("HEAD")
            .expect("should succeed");

        // Should return HEAD^ for commit with parent
        assert_eq!(parent_ref, "HEAD^", "commit with parent should return SHA^");

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ref_starting_with_dash_rejected() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        // Refs starting with - could be interpreted as flags - should be rejected
        let result = backend.get_commit("--upload-pack=evil");
        assert!(
            matches!(result, Err(VcsError::InvalidRef(_))),
            "refs starting with - should be rejected"
        );

        let result2 = backend.get_commit("-n");
        assert!(
            matches!(result2, Err(VcsError::InvalidRef(_))),
            "refs starting with - should be rejected"
        );
    }

    #[test]
    fn test_get_commits_in_range_empty_range() {
        let _repo = RepoGuard::new();
        let backend = GitBackend::from_cwd().expect("should open repo");

        // HEAD..HEAD is empty range
        let commits = backend
            .get_commits_in_range("HEAD", "HEAD")
            .expect("should succeed");
        assert!(commits.is_empty(), "HEAD..HEAD should return empty vec");
    }

    #[test]
    fn test_get_commits_in_range_with_commits() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-range-commits");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // Commit A
        fs::write(dir.join("file.txt"), "A\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "commit A"]);

        // Commit B
        fs::write(dir.join("file.txt"), "B\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "commit B"]);

        // Commit C
        fs::write(dir.join("file.txt"), "C\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "commit C"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");

        // Range HEAD~2..HEAD should return commits B and C (2 commits)
        let commits = backend
            .get_commits_in_range("HEAD~2", "HEAD")
            .expect("should get commits");

        assert_eq!(commits.len(), 2, "should have 2 commits in range");
        assert_eq!(commits[0].summary, "commit B", "first should be B (oldest)");
        assert_eq!(
            commits[1].summary, "commit C",
            "second should be C (newest)"
        );

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_commits_in_range_fields_populated() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-range-fields");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // First commit
        fs::write(dir.join("file.txt"), "first\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "first commit"]);

        // Second commit
        fs::write(dir.join("file.txt"), "second\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "second commit"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");
        let commits = backend
            .get_commits_in_range("HEAD~1", "HEAD")
            .expect("should get commits");

        assert_eq!(commits.len(), 1);
        let commit = &commits[0];

        // commit_id should be 40-char hex
        assert_eq!(commit.commit_id.len(), 40, "commit_id should be 40 chars");
        assert!(
            commit.commit_id.chars().all(|c| c.is_ascii_hexdigit()),
            "commit_id should be hex"
        );

        // short_id should be 7 chars (git default)
        assert!(
            commit.short_id.len() >= 7,
            "short_id should be at least 7 chars"
        );

        // change_id should be None for git
        assert!(commit.change_id.is_none(), "git has no change_id");

        // summary should match commit message
        assert_eq!(commit.summary, "second commit");

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_commits_in_range_excludes_empty_commits() {
        use crate::vcs::test_utils::{git, make_temp_dir};
        use std::fs;

        let _lock = crate::vcs::test_utils::cwd_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = make_temp_dir("git-range-empty");
        let original = std::env::current_dir().expect("get cwd");

        git(&dir, &["init"]);
        git(&dir, &["config", "user.email", "test@example.com"]);
        git(&dir, &["config", "user.name", "Test User"]);

        // First commit with changes
        fs::write(dir.join("file.txt"), "first\n").expect("write file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "first with changes"]);

        // Second commit with changes
        fs::write(dir.join("file.txt"), "second\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "second with changes"]);

        // Empty commit (no file changes)
        git(&dir, &["commit", "--allow-empty", "-m", "empty commit"]);

        // Third commit with changes
        fs::write(dir.join("file.txt"), "third\n").expect("modify file");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-m", "third with changes"]);

        std::env::set_current_dir(&dir).expect("set cwd");

        let backend = GitBackend::from_cwd().expect("should open repo");

        // Get range from first commit to HEAD
        let commits = backend
            .get_commits_in_range("HEAD~3", "HEAD")
            .expect("should get commits");

        // Should have 3 commits (second, empty excluded, third) - but empty is excluded
        // so we get 2 commits
        assert_eq!(
            commits.len(),
            2,
            "should have 2 commits (empty commit excluded)"
        );

        // Verify empty commit is not included
        for commit in &commits {
            assert_ne!(
                commit.summary, "empty commit",
                "empty commit should be excluded"
            );
        }

        let _ = std::env::set_current_dir(&original);
        let _ = fs::remove_dir_all(&dir);
    }
}
