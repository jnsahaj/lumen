//! Jujutsu (jj) backend implementation using jj-lib.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use chrono::Local;
use futures::StreamExt;
use jj_lib::backend::TreeValue;
use jj_lib::commit::Commit;
use jj_lib::config::StackedConfig;
use jj_lib::conflicts::{
    materialize_merge_result_to_bytes, try_materialize_file_conflict_value, ConflictMarkerStyle,
    ConflictMaterializeOptions,
};
use jj_lib::diff::{diff, DiffHunkKind};
use jj_lib::files::FileMergeHunkLevel;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::merge::{MergedTreeValue, SameChange};
use jj_lib::object_id::ObjectId;
use jj_lib::repo::{ReadonlyRepo, Repo, StoreFactories};
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathUiConverter;
use jj_lib::revset::{
    RevsetAliasesMap, RevsetDiagnostics, RevsetExtensions, RevsetParseContext,
    RevsetWorkspaceContext, SymbolResolver, SymbolResolverExtension,
};
use jj_lib::settings::UserSettings;
use jj_lib::time_util::DatePatternContext;
use jj_lib::tree_merge::MergeOptions;
use jj_lib::workspace::{default_working_copy_factories, Workspace};
use pollster::FutureExt;

use super::backend::{CommitInfo, VcsBackend, VcsError};

/// Files to exclude from diff output (same as GIT_DIFF_EXCLUSIONS in git_entity).
const DIFF_EXCLUDED_FILES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
];

/// Path patterns to exclude from diff output.
const DIFF_EXCLUDED_PATTERNS: &[&str] = &["node_modules/"];

/// Check if a path should be excluded from diff output.
fn should_exclude_path(path: &str) -> bool {
    // Check exact file matches
    if let Some(filename) = path.rsplit('/').next() {
        if DIFF_EXCLUDED_FILES.contains(&filename) {
            return true;
        }
    }
    // Check pattern matches
    for pattern in DIFF_EXCLUDED_PATTERNS {
        if path.contains(pattern) {
            return true;
        }
    }
    false
}

/// Jujutsu backend using jj-lib for native repo access.
pub struct JjBackend {
    workspace: Workspace,
    repo: Arc<ReadonlyRepo>,
    settings: UserSettings,
    workspace_path: std::path::PathBuf,
}

impl JjBackend {
    /// Load a jj workspace and repository from the given path.
    pub fn new(workspace_path: &Path) -> Result<Self, VcsError> {
        // Create minimal settings
        let config = StackedConfig::with_defaults();
        let settings = UserSettings::from_config(config)
            .map_err(|e| VcsError::Other(format!("Failed to create settings: {}", e)))?;

        // Load workspace
        let workspace = Workspace::load(
            &settings,
            workspace_path,
            &StoreFactories::default(),
            &default_working_copy_factories(),
        )
        .map_err(|e| VcsError::Other(format!("Failed to load workspace: {}", e)))?;

        // Load repo at HEAD
        let repo = workspace
            .repo_loader()
            .load_at_head()
            .map_err(|e| VcsError::Other(format!("Failed to load repo: {}", e)))?;

        Ok(JjBackend {
            workspace,
            repo,
            settings,
            workspace_path: workspace_path.to_path_buf(),
        })
    }

    /// Resolve a revset expression to a single commit.
    fn resolve_single_commit(&self, revset_str: &str) -> Result<Commit, VcsError> {
        let repo = self.repo.as_ref();

        // Set up path converter for workspace context
        let path_converter = RepoPathUiConverter::Fs {
            cwd: self.workspace_path.clone(),
            base: self.workspace_path.clone(),
        };
        let workspace_ctx = RevsetWorkspaceContext {
            path_converter: &path_converter,
            workspace_name: self.workspace.workspace_name(),
        };

        // Create parse context with workspace context for @ resolution
        let context = RevsetParseContext {
            aliases_map: &RevsetAliasesMap::default(),
            local_variables: HashMap::new(),
            user_email: self.settings.user_email(),
            date_pattern_context: DatePatternContext::from(Local::now()),
            default_ignored_remote: None,
            use_glob_by_default: true,
            extensions: &RevsetExtensions::default(),
            workspace: Some(workspace_ctx),
        };

        // Parse the revset expression
        let mut diagnostics = RevsetDiagnostics::new();
        let expression = jj_lib::revset::parse(&mut diagnostics, revset_str, &context)
            .map_err(|e| VcsError::InvalidRef(format!("Parse error: {}", e)))?;

        // Create symbol resolver
        let symbol_resolver =
            SymbolResolver::new(repo, &([] as [&Box<dyn SymbolResolverExtension>; 0]));

        // Resolve and evaluate
        let resolved = expression
            .resolve_user_expression(repo, &symbol_resolver)
            .map_err(|e| VcsError::InvalidRef(format!("Resolution error: {}", e)))?;

        let revset = resolved
            .evaluate(repo)
            .map_err(|e| VcsError::Other(format!("Evaluation error: {}", e)))?;

        // Get the first commit
        let mut iter = revset.iter();
        let commit_id = iter
            .next()
            .ok_or_else(|| VcsError::InvalidRef(revset_str.to_string()))?
            .map_err(|e| VcsError::Other(format!("Iterator error: {}", e)))?;

        // Load the commit
        let commit = repo
            .store()
            .get_commit(&commit_id)
            .map_err(|e| VcsError::Other(format!("Failed to load commit: {}", e)))?;

        Ok(commit)
    }

    /// Generate a unified diff for a commit (comparing to its first parent).
    fn generate_diff(&self, commit: &Commit) -> Result<String, VcsError> {
        let repo = self.repo.as_ref();

        // Get parent tree (or empty tree for root commits)
        let parent_tree = if commit.parent_ids().is_empty() {
            repo.store().empty_merged_tree()
        } else {
            let parent_id = &commit.parent_ids()[0];
            let parent = repo
                .store()
                .get_commit(parent_id)
                .map_err(|e| VcsError::Other(format!("Failed to get parent: {}", e)))?;
            parent.tree()
        };

        // Get commit tree
        let commit_tree = commit.tree();

        // Generate diff output
        let mut diff_output = String::new();

        // Use the tree diff stream to iterate over changes
        let diff_stream = parent_tree.diff_stream(&commit_tree, &EverythingMatcher);
        let entries: Vec<_> = async { diff_stream.collect().await }.block_on();

        for entry in entries {
            let diff = entry
                .values
                .map_err(|e| VcsError::Other(format!("Diff iteration error: {}", e)))?;

            let path_str = entry.path.as_internal_file_string();

            // Skip excluded files (same as GIT_DIFF_EXCLUSIONS)
            if should_exclude_path(path_str) {
                continue;
            }

            // Get content from before/after values
            let old_content = self.get_content_from_value(repo, &entry.path, &diff.before)?;
            let new_content = self.get_content_from_value(repo, &entry.path, &diff.after)?;

            self.format_diff_entry(&mut diff_output, path_str, &old_content, &new_content);
        }

        Ok(diff_output)
    }

    /// Format a single diff entry
    fn format_diff_entry(
        &self,
        output: &mut String,
        path_str: &str,
        old_content: &Option<String>,
        new_content: &Option<String>,
    ) {
        if old_content.is_none() && new_content.is_some() {
            // Added file
            output.push_str(&format!("diff --git a/{} b/{}\n", path_str, path_str));
            output.push_str("new file mode 100644\n");
            output.push_str("--- /dev/null\n");
            output.push_str(&format!("+++ b/{}\n", path_str));
            if let Some(content) = new_content {
                self.format_hunk(output, "", content);
            }
        } else if old_content.is_some() && new_content.is_none() {
            // Deleted file
            output.push_str(&format!("diff --git a/{} b/{}\n", path_str, path_str));
            output.push_str("deleted file mode 100644\n");
            output.push_str(&format!("--- a/{}\n", path_str));
            output.push_str("+++ /dev/null\n");
            if let Some(content) = old_content {
                self.format_hunk(output, content, "");
            }
        } else if let (Some(old), Some(new)) = (old_content, new_content) {
            if old != new {
                // Modified file
                output.push_str(&format!("diff --git a/{} b/{}\n", path_str, path_str));
                output.push_str(&format!("--- a/{}\n", path_str));
                output.push_str(&format!("+++ b/{}\n", path_str));
                self.format_hunk(output, old, new);
            }
        }
    }

    /// Get content from a MergedTreeValue
    fn get_content_from_value(
        &self,
        repo: &dyn Repo,
        path: &RepoPath,
        value: &MergedTreeValue,
    ) -> Result<Option<String>, VcsError> {
        // Check if resolved (no conflict)
        if let Some(resolved) = value.as_resolved() {
            match resolved {
                Some(TreeValue::File { id, .. }) => {
                    // Read file content using async API with pollster
                    let mut content = Vec::new();
                    let mut reader = repo
                        .store()
                        .read_file(path, id)
                        .block_on()
                        .map_err(|e| VcsError::Other(format!("Failed to read file: {}", e)))?;

                    // Use tokio async read with pollster
                    async { tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut content).await }
                        .block_on()
                        .map_err(|e| VcsError::Other(format!("Failed to read content: {}", e)))?;

                    Ok(Some(String::from_utf8_lossy(&content).into_owned()))
                }
                None => Ok(None),
                _ => Ok(None), // Symlink, Tree, etc.
            }
        } else {
            // Conflict - materialize with proper conflict markers using jj-lib
            self.materialize_conflict(repo, path, value)
        }
    }

    /// Materialize a conflict value into a string with conflict markers.
    fn materialize_conflict(
        &self,
        repo: &dyn Repo,
        path: &RepoPath,
        value: &MergedTreeValue,
    ) -> Result<Option<String>, VcsError> {
        // Try to materialize as a file conflict
        let file_conflict = try_materialize_file_conflict_value(repo.store(), path, value)
            .block_on()
            .map_err(|e| VcsError::Other(format!("Failed to materialize conflict: {}", e)))?;

        match file_conflict {
            Some(file) => {
                // Use Git-style conflict markers for compatibility
                let options = ConflictMaterializeOptions {
                    marker_style: ConflictMarkerStyle::Git,
                    marker_len: None,
                    merge: MergeOptions {
                        hunk_level: FileMergeHunkLevel::Line,
                        same_change: SameChange::Accept,
                    },
                };

                let content = materialize_merge_result_to_bytes(&file.contents, &options);
                Ok(Some(String::from_utf8_lossy(&content).into_owned()))
            }
            None => {
                // Non-file conflict (e.g., file vs symlink) - use placeholder
                Ok(Some(
                    "<<<<<<< Conflict (non-file)\n(complex conflict - file vs non-file)\n>>>>>>>\n"
                        .to_string(),
                ))
            }
        }
    }

    /// Format a unified diff using jj-lib's proper diff algorithm.
    /// Produces hunks with context lines for better readability.
    fn format_hunk(&self, output: &mut String, old: &str, new: &str) {
        const CONTEXT_LINES: usize = 3;

        // Use jj-lib's diff algorithm
        let hunks = diff([old.as_bytes(), new.as_bytes()]);

        // Collect lines with their types for unified diff output
        let old_lines: Vec<&str> = old.lines().collect();

        // Track position in each file
        let mut old_pos = 0usize;
        let mut new_pos = 0usize;

        // Accumulate changes with context
        let mut pending_output = String::new();
        let mut hunk_old_start = 0usize;
        let mut hunk_new_start = 0usize;
        let mut hunk_old_count = 0usize;
        let mut hunk_new_count = 0usize;
        let mut in_hunk = false;

        // Helper to count lines in byte slice
        fn count_lines(content: &[u8]) -> usize {
            if content.is_empty() {
                0
            } else {
                String::from_utf8_lossy(content).lines().count()
            }
        }

        // Helper to iterate lines in byte slice
        fn iter_lines(content: &[u8]) -> impl Iterator<Item = String> + '_ {
            String::from_utf8_lossy(content)
                .lines()
                .map(|s| s.to_owned())
                .collect::<Vec<_>>()
                .into_iter()
        }

        for hunk in &hunks {
            match hunk.kind {
                DiffHunkKind::Matching => {
                    let content = &hunk.contents[0];
                    let content_str = String::from_utf8_lossy(content);
                    let lines: Vec<&str> = content_str.lines().collect();
                    let line_count = lines.len();

                    if in_hunk {
                        // Add context lines after a change
                        for line in lines.iter().take(CONTEXT_LINES) {
                            pending_output.push_str(&format!(" {}\n", line));
                            hunk_old_count += 1;
                            hunk_new_count += 1;
                        }

                        // If we have more than CONTEXT_LINES, end this hunk
                        if line_count > CONTEXT_LINES * 2 {
                            // Flush current hunk
                            output.push_str(&format!(
                                "@@ -{},{} +{},{} @@\n",
                                if hunk_old_start == 0 {
                                    0
                                } else {
                                    hunk_old_start
                                },
                                hunk_old_count,
                                if hunk_new_start == 0 {
                                    0
                                } else {
                                    hunk_new_start
                                },
                                hunk_new_count
                            ));
                            output.push_str(&pending_output);
                            pending_output.clear();
                            in_hunk = false;
                        }
                    }

                    old_pos += line_count;
                    new_pos += line_count;
                }
                DiffHunkKind::Different => {
                    let old_content = &hunk.contents[0];
                    let new_content = &hunk.contents[1];
                    let old_content_lines = count_lines(old_content);
                    let new_content_lines = count_lines(new_content);

                    if !in_hunk {
                        // Start new hunk with context
                        in_hunk = true;
                        hunk_old_start = old_pos.saturating_sub(CONTEXT_LINES) + 1;
                        hunk_new_start = new_pos.saturating_sub(CONTEXT_LINES) + 1;
                        hunk_old_count = 0;
                        hunk_new_count = 0;

                        // Add leading context from old file
                        let context_start = old_pos.saturating_sub(CONTEXT_LINES);
                        for i in context_start..old_pos {
                            if i < old_lines.len() {
                                pending_output.push_str(&format!(" {}\n", old_lines[i]));
                                hunk_old_count += 1;
                                hunk_new_count += 1;
                            }
                        }
                    }

                    // Add removed lines
                    for line in iter_lines(old_content) {
                        pending_output.push_str(&format!("-{}\n", line));
                        hunk_old_count += 1;
                    }

                    // Add added lines
                    for line in iter_lines(new_content) {
                        pending_output.push_str(&format!("+{}\n", line));
                        hunk_new_count += 1;
                    }

                    old_pos += old_content_lines;
                    new_pos += new_content_lines;
                }
            }
        }

        // Flush any remaining hunk
        if in_hunk {
            output.push_str(&format!(
                "@@ -{},{} +{},{} @@\n",
                if hunk_old_start == 0 {
                    0
                } else {
                    hunk_old_start
                },
                hunk_old_count,
                if hunk_new_start == 0 {
                    0
                } else {
                    hunk_new_start
                },
                hunk_new_count
            ));
            output.push_str(&pending_output);
        }

        // Handle edge case: empty old or new content
        if hunks.is_empty() || (old.is_empty() && new.is_empty()) {
            return;
        }

        // If no hunks were generated but content differs, fall back to simple diff
        if output.is_empty() && old != new {
            let old_line_count = old.lines().count();
            let new_line_count = new.lines().count();

            output.push_str(&format!(
                "@@ -{},{} +{},{} @@\n",
                if old_line_count == 0 { 0 } else { 1 },
                old_line_count,
                if new_line_count == 0 { 0 } else { 1 },
                new_line_count
            ));

            for line in old.lines() {
                output.push_str(&format!("-{}\n", line));
            }
            for line in new.lines() {
                output.push_str(&format!("+{}\n", line));
            }
        }
    }
}

impl VcsBackend for JjBackend {
    fn get_commit(&self, reference: &str) -> Result<CommitInfo, VcsError> {
        let reference = reference.trim();

        // Resolve the commit using revset
        let commit = self.resolve_single_commit(reference)?;

        // Extract commit info
        let commit_id = commit.id().hex();
        let change_id = commit.change_id().hex();
        let message = commit.description().to_string();
        let author_sig = commit.author();
        let author = format!("{} <{}>", author_sig.name, author_sig.email);

        // Format date from author timestamp
        let date = chrono::DateTime::from_timestamp_millis(author_sig.timestamp.timestamp.0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_default();

        // Generate diff
        let diff = self.generate_diff(&commit)?;

        Ok(CommitInfo {
            commit_id,
            change_id: Some(change_id),
            message,
            diff,
            author,
            date,
        })
    }

    fn get_working_tree_diff(&self, _staged: bool) -> Result<String, VcsError> {
        // For jj, working tree changes are part of @ commit
        // Get diff of @ vs @-
        let wc_commit = self.resolve_single_commit("@")?;
        self.generate_diff(&wc_commit)
    }

    fn get_range_diff(&self, from: &str, to: &str, _three_dot: bool) -> Result<String, VcsError> {
        // Get diff between two commits
        let from_commit = self.resolve_single_commit(from)?;
        let to_commit = self.resolve_single_commit(to)?;

        let from_tree = from_commit.tree();
        let to_tree = to_commit.tree();

        let mut diff_output = String::new();
        let repo = self.repo.as_ref();

        let diff_stream = from_tree.diff_stream(&to_tree, &EverythingMatcher);
        let entries: Vec<_> = async { diff_stream.collect().await }.block_on();

        for entry in entries {
            let diff = entry
                .values
                .map_err(|e| VcsError::Other(format!("Diff iteration error: {}", e)))?;

            let path_str = entry.path.as_internal_file_string();

            // Skip excluded files (same as GIT_DIFF_EXCLUSIONS)
            if should_exclude_path(path_str) {
                continue;
            }

            let old_content = self.get_content_from_value(repo, &entry.path, &diff.before)?;
            let new_content = self.get_content_from_value(repo, &entry.path, &diff.after)?;

            self.format_diff_entry(&mut diff_output, path_str, &old_content, &new_content);
        }

        Ok(diff_output)
    }

    fn get_changed_files(&self, reference: &str) -> Result<Vec<String>, VcsError> {
        let commit = self.resolve_single_commit(reference)?;
        let repo = self.repo.as_ref();

        // Get parent tree (or empty tree for root commits)
        let parent_tree = if commit.parent_ids().is_empty() {
            repo.store().empty_merged_tree()
        } else {
            let parent_id = &commit.parent_ids()[0];
            let parent = repo
                .store()
                .get_commit(parent_id)
                .map_err(|e| VcsError::Other(format!("Failed to get parent: {}", e)))?;
            parent.tree()
        };

        let commit_tree = commit.tree();
        let diff_stream = parent_tree.diff_stream(&commit_tree, &EverythingMatcher);
        let entries: Vec<_> = async { diff_stream.collect().await }.block_on();

        let mut files = Vec::new();
        for entry in entries {
            let path_str = entry.path.as_internal_file_string();
            // Skip excluded files (same as GIT_DIFF_EXCLUSIONS)
            if !should_exclude_path(path_str) {
                files.push(path_str.to_string());
            }
        }

        Ok(files)
    }

    fn get_file_content_at_ref(&self, reference: &str, path: &Path) -> Result<String, VcsError> {
        let commit = self.resolve_single_commit(reference)?;
        let tree = commit.tree();

        // Convert path to RepoPath
        let path_str = path.to_string_lossy().into_owned();
        let repo_path = jj_lib::repo_path::RepoPathBuf::from_internal_string(path_str.clone())
            .map_err(|e| VcsError::InvalidRef(format!("Invalid path: {}", e)))?;

        // Get the value at the path
        let value = tree
            .path_value(&repo_path)
            .map_err(|e| VcsError::Other(format!("Failed to get path value: {}", e)))?;

        // Check if resolved (no conflict)
        if let Some(resolved) = value.as_resolved() {
            match resolved {
                Some(TreeValue::File { id, .. }) => {
                    // Read file content
                    let mut content = Vec::new();
                    let mut reader = self
                        .repo
                        .store()
                        .read_file(&repo_path, id)
                        .block_on()
                        .map_err(|e| VcsError::Other(format!("Failed to read file: {}", e)))?;

                    async { tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut content).await }
                        .block_on()
                        .map_err(|e| VcsError::Other(format!("Failed to read content: {}", e)))?;

                    Ok(String::from_utf8_lossy(&content).into_owned())
                }
                None => Err(VcsError::FileNotFound(path_str)),
                Some(TreeValue::Symlink(_)) => {
                    Err(VcsError::Other("Symlinks not supported".to_string()))
                }
                Some(TreeValue::Tree(_)) => Err(VcsError::Other("Path is a directory".to_string())),
                Some(TreeValue::GitSubmodule(_)) => {
                    Err(VcsError::Other("Git submodules not supported".to_string()))
                }
            }
        } else {
            // Conflicted file - materialize with proper conflict markers
            let repo = self.repo.as_ref();
            match self.materialize_conflict(repo, &repo_path, &value)? {
                Some(content) => Ok(content),
                None => Err(VcsError::FileNotFound(path_str)),
            }
        }
    }

    fn get_current_branch(&self) -> Result<Option<String>, VcsError> {
        // jj uses "bookmarks" instead of "branches"
        // Find a bookmark that points to the working copy commit (@)
        let wc_commit = self.resolve_single_commit("@")?;
        let wc_commit_id = wc_commit.id();

        // Iterate local bookmarks to find one pointing to @
        for (name, target) in self.repo.view().local_bookmarks() {
            if let Some(commit_id) = target.as_normal() {
                if commit_id == wc_commit_id {
                    return Ok(Some(name.as_str().to_string()));
                }
            }
        }

        // No bookmark points to @
        Ok(None)
    }

    fn resolve_ref(&self, reference: &str) -> Result<String, VcsError> {
        let reference = reference.trim();
        let commit = self.resolve_single_commit(reference)?;
        Ok(commit.id().hex())
    }

    fn get_commit_log_for_fzf(&self) -> Result<String, VcsError> {
        // Get visible commits using "all()" revset, limited
        let repo = self.repo.as_ref();

        // Set up path converter for workspace context
        let path_converter = RepoPathUiConverter::Fs {
            cwd: self.workspace_path.clone(),
            base: self.workspace_path.clone(),
        };
        let workspace_ctx = RevsetWorkspaceContext {
            path_converter: &path_converter,
            workspace_name: self.workspace.workspace_name(),
        };

        let context = RevsetParseContext {
            aliases_map: &RevsetAliasesMap::default(),
            local_variables: HashMap::new(),
            user_email: self.settings.user_email(),
            date_pattern_context: DatePatternContext::from(Local::now()),
            default_ignored_remote: None,
            use_glob_by_default: true,
            extensions: &RevsetExtensions::default(),
            workspace: Some(workspace_ctx),
        };

        // Parse revset for recent commits (limit to ~100)
        let mut diagnostics = RevsetDiagnostics::new();
        let expression = jj_lib::revset::parse(&mut diagnostics, "all()", &context)
            .map_err(|e| VcsError::Other(format!("Parse error: {}", e)))?;

        let symbol_resolver =
            SymbolResolver::new(repo, &([] as [&Box<dyn SymbolResolverExtension>; 0]));

        let resolved = expression
            .resolve_user_expression(repo, &symbol_resolver)
            .map_err(|e| VcsError::Other(format!("Resolution error: {}", e)))?;

        let revset = resolved
            .evaluate(repo)
            .map_err(|e| VcsError::Other(format!("Evaluation error: {}", e)))?;

        let mut output = String::new();
        const MAX_COMMITS: usize = 100;

        for (count, commit_id_result) in revset.iter().enumerate() {
            if count >= MAX_COMMITS {
                break;
            }

            let commit_id =
                commit_id_result.map_err(|e| VcsError::Other(format!("Iterator error: {}", e)))?;

            let commit = repo
                .store()
                .get_commit(&commit_id)
                .map_err(|e| VcsError::Other(format!("Failed to load commit: {}", e)))?;

            let change_id = commit.change_id().hex();
            let commit_hash = commit.id().hex();
            let description = commit.description().lines().next().unwrap_or("");

            // Format: change_id commit_id description (compatible with fzf)
            output.push_str(&format!(
                "{} {} {}\n",
                &change_id[..12.min(change_id.len())],
                &commit_hash[..12.min(commit_hash.len())],
                description
            ));
        }

        Ok(output)
    }

    fn get_working_tree_changed_files(&self) -> Result<Vec<String>, VcsError> {
        // For jj, working tree changes are in @ vs @-
        // This is the same as get_changed_files("@") but we implement directly
        // to avoid the overhead of re-resolving the commit
        let wc_commit = self.resolve_single_commit("@")?;
        let repo = self.repo.as_ref();

        // Get parent tree (or empty tree for root commits)
        let parent_tree = if wc_commit.parent_ids().is_empty() {
            repo.store().empty_merged_tree()
        } else {
            let parent_id = &wc_commit.parent_ids()[0];
            let parent = repo
                .store()
                .get_commit(parent_id)
                .map_err(|e| VcsError::Other(format!("Failed to get parent: {}", e)))?;
            parent.tree()
        };

        let wc_tree = wc_commit.tree();
        let diff_stream = parent_tree.diff_stream(&wc_tree, &EverythingMatcher);
        let entries: Vec<_> = async { diff_stream.collect().await }.block_on();

        let mut files = Vec::new();
        for entry in entries {
            let path_str = entry.path.as_internal_file_string();
            // Skip excluded files (same as GIT_DIFF_EXCLUSIONS)
            if !should_exclude_path(path_str) {
                files.push(path_str.to_string());
            }
        }

        Ok(files)
    }

    fn get_merge_base(&self, ref1: &str, ref2: &str) -> Result<String, VcsError> {
        // For jj, find common ancestor using revset: heads(::ref1 & ::ref2)
        // This finds the greatest common ancestor(s)
        let revset_str = format!("heads(::({}) & ::({}))", ref1.trim(), ref2.trim());
        let commit = self.resolve_single_commit(&revset_str)?;
        Ok(commit.id().hex())
    }

    fn working_copy_parent_ref(&self) -> &'static str {
        "@-"
    }

    fn get_range_changed_files(&self, from: &str, to: &str) -> Result<Vec<String>, VcsError> {
        let from_commit = self.resolve_single_commit(from)?;
        let to_commit = self.resolve_single_commit(to)?;

        let from_tree = from_commit.tree();
        let to_tree = to_commit.tree();

        let diff_stream = from_tree.diff_stream(&to_tree, &EverythingMatcher);
        let entries: Vec<_> = async { diff_stream.collect().await }.block_on();

        let mut files = Vec::new();
        for entry in entries {
            let path_str = entry.path.as_internal_file_string();
            // Skip excluded files (same as GIT_DIFF_EXCLUSIONS)
            if !should_exclude_path(path_str) {
                files.push(path_str.to_string());
            }
        }

        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcs::test_utils::JjRepoGuard;

    #[test]
    fn test_jj_backend_new_succeeds_on_jj_repo() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir);
        assert!(
            backend.is_ok(),
            "JjBackend::new should succeed on jj repo: {:?}",
            backend.err()
        );
    }

    #[test]
    fn test_jj_backend_new_fails_on_non_jj_dir() {
        let temp = tempfile::TempDir::new().unwrap();
        let result = JjBackend::new(temp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_get_commit_at_working_copy() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let commit = backend.get_commit("@");
        assert!(
            commit.is_ok(),
            "get_commit(@) should succeed: {:?}",
            commit.err()
        );

        let commit = commit.unwrap();
        assert!(!commit.commit_id.is_empty());
        assert!(commit.change_id.is_some());
    }

    #[test]
    fn test_get_commit_at_parent() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        // @- is parent of working copy - might be root commit
        let commit = backend.get_commit("@-");
        assert!(
            commit.is_ok(),
            "get_commit(@-) should succeed: {:?}",
            commit.err()
        );
    }

    #[test]
    fn test_get_commit_invalid_ref() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let commit = backend.get_commit("nonexistent_ref_xyz");
        assert!(commit.is_err());
    }

    #[test]
    fn test_diff_of_added_file_contains_plus_header() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        // We already wrote README.md in JjRepoGuard::new()
        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let commit = backend.get_commit("@").expect("should get commit");

        // Working copy should show README.md as added
        assert!(
            commit.diff.contains("+++ b/README.md"),
            "diff should contain +++ b/README.md header for added file, got: {}",
            commit.diff
        );
    }

    #[test]
    fn test_diff_contains_hunk_headers() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let commit = backend.get_commit("@").expect("should get commit");

        // If there's content, there should be @@ hunk markers
        if !commit.diff.is_empty() {
            assert!(
                commit.diff.contains("@@"),
                "diff with content should contain @@ hunk headers"
            );
        }
    }

    #[test]
    fn test_get_file_content_at_ref() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let content = backend.get_file_content_at_ref("@", Path::new("README.md"));

        assert!(
            content.is_ok(),
            "get_file_content_at_ref should succeed: {:?}",
            content.err()
        );
        assert_eq!(content.unwrap(), "hello\n");
    }

    #[test]
    fn test_get_file_content_at_ref_missing_file() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let content = backend.get_file_content_at_ref("@", Path::new("nonexistent.txt"));

        assert!(
            matches!(content, Err(VcsError::FileNotFound(_))),
            "Expected FileNotFound error, got: {:?}",
            content
        );
    }

    #[test]
    fn test_get_file_content_multiple_files_returns_correct_content() {
        use std::fs;

        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        // Create additional files with distinct content
        fs::write(repo.dir.join("file1.txt"), "content of file1\n").expect("write file1");
        fs::write(repo.dir.join("file2.txt"), "content of file2\n").expect("write file2");

        // Force jj to snapshot (JjRepoGuard runs jj status, but we added files after)
        crate::vcs::test_utils::jj(&repo.dir, &["status"]);

        // Reload backend to get updated tree
        let backend = JjBackend::new(&repo.dir).expect("should load backend");

        // Verify each file returns its correct content
        let content1 = backend
            .get_file_content_at_ref("@", Path::new("file1.txt"))
            .expect("should get file1");
        assert_eq!(content1, "content of file1\n", "file1 content mismatch");

        let content2 = backend
            .get_file_content_at_ref("@", Path::new("file2.txt"))
            .expect("should get file2");
        assert_eq!(content2, "content of file2\n", "file2 content mismatch");

        let readme = backend
            .get_file_content_at_ref("@", Path::new("README.md"))
            .expect("should get README");
        assert_eq!(readme, "hello\n", "README content mismatch");
    }

    #[test]
    fn test_get_changed_files() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let files = backend.get_changed_files("@");

        assert!(
            files.is_ok(),
            "get_changed_files should succeed: {:?}",
            files.err()
        );

        let files = files.unwrap();
        assert!(
            files.contains(&"README.md".to_string()),
            "changed files should contain README.md, got: {:?}",
            files
        );
    }

    #[test]
    fn test_get_commit_log_for_fzf() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let log = backend.get_commit_log_for_fzf();

        assert!(
            log.is_ok(),
            "get_commit_log_for_fzf should succeed: {:?}",
            log.err()
        );

        let log = log.unwrap();
        assert!(!log.is_empty(), "commit log should not be empty");
        // Log should have at least one line with change_id commit_id
        assert!(
            log.lines().next().is_some(),
            "log should have at least one line"
        );
    }

    #[test]
    fn test_get_working_tree_diff() {
        use std::fs;

        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        // Modify the README to create a working tree change
        fs::write(repo.dir.join("README.md"), "modified content\n").expect("modify file");
        crate::vcs::test_utils::jj(&repo.dir, &["status"]); // Snapshot

        // Reload backend to get updated state
        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let diff = backend.get_working_tree_diff(false);

        assert!(
            diff.is_ok(),
            "get_working_tree_diff should succeed: {:?}",
            diff.err()
        );

        let diff = diff.unwrap();
        assert!(
            diff.contains("modified content") || diff.contains("README.md"),
            "working tree diff should contain changes, got: {}",
            diff
        );

        // staged param is ignored (jj has no staging) - same result for true/false
        let diff_staged = backend.get_working_tree_diff(true);
        assert!(
            diff_staged.is_ok(),
            "staged param should be ignored (jj has no staging)"
        );
    }

    #[test]
    fn test_get_range_diff() {
        use std::fs;

        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        // Create a new commit by describing @ and making new change
        crate::vcs::test_utils::jj(&repo.dir, &["describe", "-m", "first commit"]);
        crate::vcs::test_utils::jj(&repo.dir, &["new"]);

        fs::write(repo.dir.join("file2.txt"), "second commit\n").expect("write file2");
        crate::vcs::test_utils::jj(&repo.dir, &["status"]); // Snapshot

        // Reload backend
        let backend = JjBackend::new(&repo.dir).expect("should load backend");

        // Range diff @-..@ (parent to working copy)
        let diff = backend.get_range_diff("@-", "@", false);
        assert!(
            diff.is_ok(),
            "get_range_diff should succeed: {:?}",
            diff.err()
        );

        let diff = diff.unwrap();
        assert!(
            diff.contains("file2.txt"),
            "range diff should contain changes, got: {}",
            diff
        );

        // three_dot param is ignored in jj - same behavior
        let diff_3dot = backend.get_range_diff("@-", "@", true);
        assert!(
            diff_3dot.is_ok(),
            "three_dot param should be accepted (behavior same as two-dot in jj)"
        );
    }

    #[test]
    fn test_get_current_branch_returns_none_without_bookmark() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let branch = backend.get_current_branch();

        assert!(
            branch.is_ok(),
            "get_current_branch should succeed: {:?}",
            branch.err()
        );
        // Default jj repo has no bookmarks
        assert!(
            branch.unwrap().is_none(),
            "should return None when no bookmark points to @"
        );
    }

    #[test]
    fn test_diff_excludes_lock_files() {
        use std::fs;

        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        // Create files including lock files
        fs::write(repo.dir.join("test.txt"), "hello\n").expect("write test.txt");
        fs::write(repo.dir.join("package-lock.json"), "{}\n").expect("write package-lock.json");
        fs::write(repo.dir.join("Cargo.lock"), "lock\n").expect("write Cargo.lock");
        crate::vcs::test_utils::jj(&repo.dir, &["status"]); // Snapshot

        // Reload backend
        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let commit = backend.get_commit("@").expect("should get commit");

        // Diff should contain test.txt but NOT lock files
        assert!(
            commit.diff.contains("test.txt"),
            "diff should contain test.txt, got: {}",
            commit.diff
        );
        assert!(
            !commit.diff.contains("package-lock.json"),
            "diff should NOT contain package-lock.json"
        );
        assert!(
            !commit.diff.contains("Cargo.lock"),
            "diff should NOT contain Cargo.lock"
        );
    }

    #[test]
    fn test_commit_info_field_format() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let commit = backend.get_commit("@").expect("should get commit");

        // commit_id should be 40-char hex (standard git SHA)
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

        // change_id should be present for jj
        assert!(
            commit.change_id.is_some(),
            "jj commits should have change_id"
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
    fn test_get_range_diff_identical_commits() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");

        // Diff of @..@ should be empty
        let diff = backend
            .get_range_diff("@", "@", false)
            .expect("should succeed for identical commits");
        assert!(
            diff.is_empty(),
            "diff of identical commits should be empty, got: {}",
            diff
        );
    }

    #[test]
    fn test_resolve_ref_at_returns_sha() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let sha = backend.resolve_ref("@");

        assert!(
            sha.is_ok(),
            "resolve_ref(@) should succeed: {:?}",
            sha.err()
        );

        let sha = sha.unwrap();
        assert_eq!(sha.len(), 40, "should return 40-char SHA, got: {}", sha);
        assert!(
            sha.chars().all(|c| c.is_ascii_hexdigit()),
            "SHA should be hex"
        );
    }

    #[test]
    fn test_resolve_ref_at_parent_returns_sha() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let sha = backend.resolve_ref("@-");

        assert!(
            sha.is_ok(),
            "resolve_ref(@-) should succeed: {:?}",
            sha.err()
        );

        let sha = sha.unwrap();
        assert_eq!(sha.len(), 40, "should return 40-char SHA, got: {}", sha);
    }

    #[test]
    fn test_resolve_ref_invalid_returns_error() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let result = backend.resolve_ref("nonexistent_ref_xyz");

        assert!(result.is_err(), "resolve_ref should fail for invalid ref");
    }

    #[test]
    fn test_resolve_ref_commit_sha_prefix() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");

        // Get the commit SHA of @ to use as a prefix
        let commit = backend.get_commit("@").expect("should get commit");
        let commit_id = &commit.commit_id;

        // Use first 8 chars as prefix (jj allows short prefixes for commit IDs)
        let prefix = &commit_id[..8.min(commit_id.len())];
        let sha = backend.resolve_ref(prefix);

        assert!(
            sha.is_ok(),
            "resolve_ref with commit SHA prefix should succeed: {:?}",
            sha.err()
        );

        let sha = sha.unwrap();
        assert_eq!(sha.len(), 40, "should return 40-char SHA");
        assert_eq!(sha, *commit_id, "resolved SHA should match original commit");
    }

    #[test]
    fn test_get_working_tree_changed_files_includes_modified() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        // JjRepoGuard::new() creates README.md - this should show as changed
        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let files = backend
            .get_working_tree_changed_files()
            .expect("should get changed files");

        assert!(
            files.contains(&"README.md".to_string()),
            "should include README.md, got: {:?}",
            files
        );
    }

    #[test]
    fn test_get_working_tree_changed_files_includes_new_file() {
        use std::fs;

        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        // Add a new file
        fs::write(repo.dir.join("newfile.txt"), "new content\n").expect("write new file");
        crate::vcs::test_utils::jj(&repo.dir, &["status"]); // Snapshot

        // Reload backend to get updated state
        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let files = backend
            .get_working_tree_changed_files()
            .expect("should get changed files");

        assert!(
            files.contains(&"newfile.txt".to_string()),
            "should include newfile.txt, got: {:?}",
            files
        );
    }

    #[test]
    fn test_get_working_tree_changed_files_empty_after_commit() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        // Describe current commit and create new empty working copy
        crate::vcs::test_utils::jj(&repo.dir, &["describe", "-m", "initial"]);
        crate::vcs::test_utils::jj(&repo.dir, &["new"]);

        // Reload backend
        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let files = backend
            .get_working_tree_changed_files()
            .expect("should get changed files");

        assert!(
            files.is_empty(),
            "empty working copy should return empty vec, got: {:?}",
            files
        );
    }

    #[test]
    fn test_get_merge_base_returns_ancestor() {
        use std::fs;

        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        // Commit A (base) - describe @ and create new
        crate::vcs::test_utils::jj(&repo.dir, &["describe", "-m", "base"]);
        crate::vcs::test_utils::jj(&repo.dir, &["new"]);

        // Commit B - modify and describe
        fs::write(repo.dir.join("branch.txt"), "branch\n").expect("write branch file");
        crate::vcs::test_utils::jj(&repo.dir, &["status"]); // Snapshot
        crate::vcs::test_utils::jj(&repo.dir, &["describe", "-m", "branch commit"]);

        // Create bookmark at current commit
        crate::vcs::test_utils::jj(&repo.dir, &["bookmark", "create", "feature"]);

        // Go back to base and create divergent commit
        crate::vcs::test_utils::jj(&repo.dir, &["new", "@--"]); // Back to base
        fs::write(repo.dir.join("main.txt"), "main\n").expect("write main file");
        crate::vcs::test_utils::jj(&repo.dir, &["status"]); // Snapshot
        crate::vcs::test_utils::jj(&repo.dir, &["describe", "-m", "main commit"]);

        // Reload backend
        let backend = JjBackend::new(&repo.dir).expect("should load backend");

        // Find merge base between @ and feature bookmark
        let merge_base = backend
            .get_merge_base("@", "feature")
            .expect("should find merge base");

        // Merge base should be 40-char SHA
        assert_eq!(
            merge_base.len(),
            40,
            "should return 40-char SHA, got: {}",
            merge_base
        );
    }

    #[test]
    fn test_get_merge_base_invalid_ref() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        let result = backend.get_merge_base("@", "nonexistent_ref_xyz");
        assert!(result.is_err(), "should fail for invalid ref");
    }

    #[test]
    fn test_working_copy_parent_ref_returns_at_minus() {
        let Some(repo) = JjRepoGuard::new() else {
            eprintln!("Skipping test: jj not available");
            return;
        };

        let backend = JjBackend::new(&repo.dir).expect("should load backend");
        assert_eq!(backend.working_copy_parent_ref(), "@-");
    }
}
