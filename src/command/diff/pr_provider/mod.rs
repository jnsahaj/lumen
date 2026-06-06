//! Pull-request hosting provider abstraction.
//!
//! `lumen diff --pr` originally only understood GitHub (it shelled out to the
//! `gh` CLI everywhere). This module introduces a [`PrProvider`] trait so other
//! forges can be supported, with [`github`] for GitHub and [`azure`] for Azure
//! DevOps. Adding a forge (e.g. `glab` for GitLab) is one new module plus one
//! entry in [`PROVIDERS`].

mod azure;
mod github;

use std::collections::HashSet;
use std::process::Command;
use std::thread;

use super::types::FileDiff;
use super::PrInfo;

use azure::AzureProvider;
use github::GitHubProvider;

/// An error from a PR-provider operation. The variant is the kind, so the diff
/// UI can react to it (e.g. prompt for credentials on [`PrError::Auth`]).
/// [`PrError::Other`] is the catch-all for CLI/transport failures, and the
/// `From<String>` impl lets internal string errors fall through to it.
#[derive(Debug, thiserror::Error)]
pub enum PrError {
    /// Authentication or authorization failed (missing/insufficient token).
    #[error("authentication failed: {0}")]
    Auth(String),
    /// The PR (or the current branch's PR) could not be found.
    #[error("not found: {0}")]
    NotFound(String),
    /// The input couldn't be parsed as a PR reference for this provider.
    #[error("invalid PR reference: {0}")]
    InvalidRef(String),
    /// Anything else: CLI invocation failure, transport error, bad output.
    #[error("{0}")]
    Other(String),
}

impl From<String> for PrError {
    fn from(s: String) -> Self {
        PrError::Other(s)
    }
}

impl From<&str> for PrError {
    fn from(s: &str) -> Self {
        PrError::Other(s.to_string())
    }
}

/// Provider-specific data carried on [`PrInfo`]. Each variant holds exactly the
/// fields its forge needs, so adding a forge is a new variant rather than more
/// `Option`s smeared across a shared struct.
#[derive(Clone, Debug)]
pub enum ProviderData {
    GitHub {
        /// PR node id, used by the viewed-file GraphQL mutations.
        node_id: String,
    },
    Azure {
        /// Organisation base URL, e.g. `https://dev.azure.com/org`.
        org_url: String,
        project: String,
    },
}

/// A pull-request hosting provider. Each method maps to one capability the diff
/// UI needs; the viewed-file sync methods default to no-ops so providers without
/// that concept (Azure DevOps) don't have to implement them.
///
/// `Sync` is required so a `&'static dyn PrProvider` (stored on [`PrInfo`]) can
/// be moved into the background threads that sync viewed-file state.
pub trait PrProvider: Sync {
    /// Does this provider recognise `input` as one of its PR URLs?
    fn matches_url(&self, input: &str) -> bool;

    /// Does this provider recognise `origin` (a git remote URL) as one of its
    /// repositories? Used to pick a provider for bare PR numbers.
    fn matches_origin(&self, origin: &str) -> bool;

    /// Resolve a PR number/URL into full metadata.
    fn fetch_pr_info(&self, input: &str, repo_override: Option<&str>) -> Result<PrInfo, PrError>;

    /// Find the PR associated with the current branch.
    fn detect_current_branch_pr(&self, repo_override: Option<&str>) -> Result<String, PrError>;

    /// Load the file diffs for a PR.
    fn load_pr_file_diffs(&self, pr: &PrInfo) -> Result<Vec<FileDiff>, PrError>;

    /// Build a browser URL for `filename` within the PR.
    fn file_web_url(&self, pr: &PrInfo, filename: &str) -> Option<String>;

    /// Per-file "viewed" state sync, if this provider supports it. Returning
    /// `Some` *is* the capability — there's no separate boolean flag that can
    /// drift out of step with the implementation.
    fn viewed_sync(&self) -> Option<&dyn ViewedSync> {
        None
    }
}

/// Syncing per-file "viewed" state with the forge (e.g. GitHub's PR file
/// checkboxes). Providers without the concept simply don't return one from
/// [`PrProvider::viewed_sync`].
pub trait ViewedSync {
    /// Fetch the set of paths currently marked as viewed.
    fn fetch(&self, pr: &PrInfo) -> Result<HashSet<String>, PrError>;

    /// Mark/unmark a file as viewed (blocking).
    fn set(&self, pr: &PrInfo, path: &str, viewed: bool) -> Result<(), PrError>;
}

/// All compiled-in providers. Detection iterates this; adding a forge is one
/// new module plus one entry here. Both providers are zero-sized, so the
/// `&'static` references cost nothing.
static PROVIDERS: &[&dyn PrProvider] = &[&GitHubProvider, &AzureProvider];

/// Used when no provider matches a bare PR number's remote.
const DEFAULT_PROVIDER: &dyn PrProvider = &GitHubProvider;

/// True if `input` looks like a PR reference (a known PR URL or a bare number).
pub fn is_pr_reference(input: &str) -> bool {
    PROVIDERS.iter().any(|p| p.matches_url(input)) || input.parse::<u64>().is_ok()
}

fn read_origin_url() -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

/// Pick a provider from the git `origin` remote (and any `--origin` override),
/// defaulting to GitHub when nothing matches.
fn provider_for_origin(repo_override: Option<&str>) -> &'static dyn PrProvider {
    let candidates = [repo_override.map(|s| s.to_string()), read_origin_url()];
    for candidate in candidates.into_iter().flatten() {
        if let Some(p) = PROVIDERS
            .iter()
            .copied()
            .find(|p| p.matches_origin(&candidate))
        {
            return p;
        }
    }
    DEFAULT_PROVIDER
}

/// Pick a provider from a PR URL/number, falling back to origin detection.
fn provider_for_input(input: &str, repo_override: Option<&str>) -> &'static dyn PrProvider {
    if let Some(p) = PROVIDERS.iter().copied().find(|p| p.matches_url(input)) {
        return p;
    }
    provider_for_origin(repo_override)
}

pub fn fetch_pr_info(input: &str, repo_override: Option<&str>) -> Result<PrInfo, PrError> {
    provider_for_input(input, repo_override).fetch_pr_info(input, repo_override)
}

pub fn detect_current_branch_pr(repo_override: Option<&str>) -> Result<String, PrError> {
    provider_for_origin(repo_override).detect_current_branch_pr(repo_override)
}

pub fn load_pr_file_diffs(pr: &PrInfo) -> Result<Vec<FileDiff>, PrError> {
    pr.provider.load_pr_file_diffs(pr)
}

pub fn fetch_viewed_files(pr: &PrInfo) -> Result<HashSet<String>, PrError> {
    match pr.provider.viewed_sync() {
        Some(vs) => vs.fetch(pr),
        None => Ok(HashSet::new()),
    }
}

pub fn pr_file_web_url(pr: &PrInfo, filename: &str) -> Option<String> {
    pr.provider.file_web_url(pr, filename)
}

pub fn mark_file_as_viewed_async(pr: &PrInfo, file_path: &str) {
    set_file_viewed_async(pr, file_path, true);
}

pub fn unmark_file_as_viewed_async(pr: &PrInfo, file_path: &str) {
    set_file_viewed_async(pr, file_path, false);
}

fn set_file_viewed_async(pr: &PrInfo, file_path: &str, viewed: bool) {
    if pr.provider.viewed_sync().is_none() {
        return;
    }
    let pr = pr.clone();
    let path = file_path.to_string();
    thread::spawn(move || {
        if let Some(vs) = pr.provider.viewed_sync() {
            let _ = vs.set(&pr, &path, viewed);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_pr_reference_detects_forms() {
        assert!(is_pr_reference("123"));
        assert!(is_pr_reference("https://github.com/o/r/pull/1"));
        assert!(is_pr_reference(
            "https://dev.azure.com/o/p/_git/r/pullrequest/1"
        ));
        assert!(!is_pr_reference("main..feature"));
    }
}
