//! Local, PR-independent persistence of viewed-file state for `lumen diff --watch`.
//!
//! State is keyed by `host/owner/repo/branch` (or a path-derived slug when there's
//! no parseable origin remote) and content-hashed per file, so it survives
//! quitting `--watch`, rebases, and force-pushes as long as the file's blob
//! contents are unchanged. This module never talks to GitHub/`gh` — it is a
//! purely local cache, opt-in via `--save-viewed`.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

const SCHEMA_VERSION: u32 = 1;
const DELETED_SENTINEL: &str = "__deleted__";

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
pub struct HunkState {
    pub hash: String,
    pub hunks: Vec<usize>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
pub struct ViewedState {
    pub schema: u32,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    /// Reserved for future hunk-level persistence. Unused for now.
    #[serde(default)]
    pub hunks: BTreeMap<String, HunkState>,
}

impl ViewedState {
    fn new() -> Self {
        Self {
            schema: SCHEMA_VERSION,
            files: BTreeMap::new(),
            hunks: BTreeMap::new(),
        }
    }
}

/// Compute the git blob hash of a file's current working-tree contents.
/// Returns `DELETED_SENTINEL` if the file can't be read or hashed, so a
/// deleted file never spuriously matches a previously-saved hash.
pub fn blob_hash_for_working_tree(path: &Path) -> String {
    match fs::read(path) {
        Ok(bytes) => match git2::Oid::hash_object(git2::ObjectType::Blob, &bytes) {
            Ok(oid) => oid.to_string(),
            Err(_) => DELETED_SENTINEL.to_string(),
        },
        Err(_) => DELETED_SENTINEL.to_string(),
    }
}

/// Parse a git remote URL into `(host, owner, repo)`. Handles the three
/// common forms: `git@host:owner/repo.git`, `ssh://git@host/owner/repo.git`,
/// and `https://host/owner/repo.git` (with or without a trailing `.git`).
pub fn parse_remote_url(url: &str) -> Option<(String, String, String)> {
    let url = url.trim();
    let url = url.strip_suffix(".git").unwrap_or(url);

    let path_part = if let Some(rest) = url.strip_prefix("ssh://") {
        // ssh://[user@]host[:port]/owner/repo
        let rest = rest.split_once('@').map(|(_, r)| r).unwrap_or(rest);
        let (host_and_port, path) = rest.split_once('/')?;
        let host = host_and_port.split(':').next()?;
        return split_owner_repo(path).map(|(owner, repo)| (host.to_string(), owner, repo));
    } else if let Some(rest) = url.strip_prefix("https://") {
        rest
    } else if let Some(rest) = url.strip_prefix("http://") {
        rest
    } else if let Some(at_pos) = url.find('@') {
        // scp-like: git@host:owner/repo(.git)
        let rest = &url[at_pos + 1..];
        let (host, path) = rest.split_once(':')?;
        return split_owner_repo(path).map(|(owner, repo)| (host.to_string(), owner, repo));
    } else {
        return None;
    };

    let (host, path) = path_part.split_once('/')?;
    split_owner_repo(path).map(|(owner, repo)| (host.to_string(), owner, repo))
}

fn split_owner_repo(path: &str) -> Option<(String, String)> {
    let path = path.trim_matches('/');
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
        return None;
    }
    Some((parts[0].to_string(), parts[1].to_string()))
}

/// Sanitize a branch name for use as a filesystem path component.
/// Returns `None` for names containing `..` path traversal segments.
pub fn sanitize_branch_name(branch: &str) -> Option<String> {
    if branch.is_empty() {
        return None;
    }
    if branch.split('/').any(|segment| segment == "..") {
        return None;
    }
    Some(branch.replace('/', "__"))
}

/// Reverse of `sanitize_branch_name`, for matching saved state files back to
/// real branch names during pruning.
fn desanitize_branch_name(sanitized: &str) -> String {
    sanitized.replace("__", "/")
}

/// Build the `local__<sanitized-path>` fallback slug used when there's no
/// parseable origin remote.
fn local_fallback_slug(worktree_root: &Path) -> String {
    let raw = worktree_root.to_string_lossy();
    let sanitized = raw.replace(['/', '\\'], "__");
    format!("local__{}", sanitized.trim_start_matches("__"))
}

/// Base directory for lumen's state files, respecting `$XDG_STATE_HOME` when set.
pub fn state_base_dir() -> PathBuf {
    let xdg = std::env::var("XDG_STATE_HOME").ok();
    state_base_dir_from(xdg.as_deref(), dirs::home_dir())
}

fn state_base_dir_from(xdg_state_home: Option<&str>, home_dir: Option<PathBuf>) -> PathBuf {
    match xdg_state_home {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => home_dir.unwrap_or_default().join(".local/state"),
    }
}

/// Resolve the on-disk storage path for viewed state, given already-resolved
/// origin info (or `None` for the local-slug fallback), the worktree root,
/// and the current branch. Returns `None` if there's no branch to key on
/// (e.g. detached HEAD) — callers should skip persistence entirely.
pub fn resolve_storage_path(
    origin: Option<(&str, &str, &str)>,
    worktree_root: &Path,
    branch: Option<&str>,
) -> Option<PathBuf> {
    let branch = branch?;
    let sanitized_branch = sanitize_branch_name(branch)?;

    let (host, owner, repo) = match origin {
        Some((host, owner, repo)) => (host.to_string(), owner.to_string(), repo.to_string()),
        None => (
            "local".to_string(),
            String::new(),
            local_fallback_slug(worktree_root),
        ),
    };

    let mut path = state_base_dir().join("lumen").join("viewed").join(host);
    if !owner.is_empty() {
        path = path.join(owner);
    }
    path = path.join(repo).join(format!("{}.json", sanitized_branch));
    Some(path)
}

/// Shell out to `git remote get-url origin` and parse it into `(host, owner, repo)`.
/// Returns `None` when there's no origin remote or it isn't parseable.
pub fn resolve_origin_host_owner_repo() -> Option<(String, String, String)> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_remote_url(&url)
}

/// Shell out to `git rev-parse --show-toplevel` to find the worktree root,
/// falling back to the current directory if that fails.
pub fn git_worktree_root() -> PathBuf {
    Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

/// List local branch names via `git for-each-ref`. Returns `None` (rather
/// than an empty list) on any failure, so callers can distinguish "no
/// branches" from "couldn't ask git" and skip pruning in the latter case —
/// pruning off an empty-by-error list would wipe every saved branch.
pub fn list_local_branches() -> Option<Vec<String>> {
    let output = Command::new("git")
        .args(["for-each-ref", "--format=%(refname:short)", "refs/heads"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
    )
}

/// Full resolution glue for the current repo: resolves origin remote and
/// worktree root, then delegates to `resolve_storage_path`.
pub fn resolve_path_for_current_repo(branch: Option<&str>) -> Option<PathBuf> {
    let origin = resolve_origin_host_owner_repo();
    let worktree_root = git_worktree_root();
    resolve_storage_path(
        origin
            .as_ref()
            .map(|(h, o, r)| (h.as_str(), o.as_str(), r.as_str())),
        &worktree_root,
        branch,
    )
}

/// Load viewed state from disk. Never fails: any read/parse error, missing
/// file, or schema mismatch yields a fresh default state.
pub fn load(path: &Path) -> ViewedState {
    match fs::read_to_string(path) {
        Ok(contents) => match serde_json::from_str::<ViewedState>(&contents) {
            Ok(state) if state.schema == SCHEMA_VERSION => state,
            _ => ViewedState::new(),
        },
        Err(_) => ViewedState::new(),
    }
}

/// Re-read the on-disk state, apply `updates`, and atomically write the
/// result back. Re-reading fresh (rather than trusting an in-memory copy)
/// means concurrent `lumen diff --watch` sessions merge rather than clobber
/// each other's saved entries.
pub fn save_merged(path: &Path, updates: impl FnOnce(&mut ViewedState)) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    fs::create_dir_all(parent)?;

    let mut state = load(path);
    updates(&mut state);

    let json = serde_json::to_string_pretty(&state).map_err(io::Error::other)?;

    let tmp_name = format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("viewed"),
        std::process::id()
    );
    let tmp_path = parent.join(tmp_name);
    fs::write(&tmp_path, json)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Best-effort pruning of state files for branches that no longer exist.
/// Swallows all I/O errors — pruning failures must never fail the command.
pub fn prune_stale_branches(repo_viewed_dir: &Path, existing_branches: &[String]) {
    let Ok(entries) = fs::read_dir(repo_viewed_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let branch = desanitize_branch_name(stem);
        if !existing_branches.contains(&branch) {
            let _ = fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn blob_hash_is_stable_for_same_content() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, b"hello world").unwrap();

        let first = blob_hash_for_working_tree(&file);
        let second = blob_hash_for_working_tree(&file);

        assert_eq!(first, second);
        assert_ne!(first, DELETED_SENTINEL);
    }

    #[test]
    fn blob_hash_differs_for_different_content() {
        let dir = TempDir::new().unwrap();
        let file_a = dir.path().join("a.txt");
        let file_b = dir.path().join("b.txt");
        fs::write(&file_a, b"hello world").unwrap();
        fs::write(&file_b, b"goodbye world").unwrap();

        assert_ne!(
            blob_hash_for_working_tree(&file_a),
            blob_hash_for_working_tree(&file_b)
        );
    }

    #[test]
    fn blob_hash_returns_sentinel_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist.txt");

        assert_eq!(blob_hash_for_working_tree(&missing), DELETED_SENTINEL);
    }

    #[test]
    fn parse_remote_url_handles_https() {
        let parsed = parse_remote_url("https://github.com/jnsahaj/lumen.git");
        assert_eq!(
            parsed,
            Some((
                "github.com".to_string(),
                "jnsahaj".to_string(),
                "lumen".to_string()
            ))
        );
    }

    #[test]
    fn parse_remote_url_handles_https_without_git_suffix() {
        let parsed = parse_remote_url("https://gitlab.example.com/owner/repo");
        assert_eq!(
            parsed,
            Some((
                "gitlab.example.com".to_string(),
                "owner".to_string(),
                "repo".to_string()
            ))
        );
    }

    #[test]
    fn parse_remote_url_handles_scp_like_ssh() {
        let parsed = parse_remote_url("git@github.com:jnsahaj/lumen.git");
        assert_eq!(
            parsed,
            Some((
                "github.com".to_string(),
                "jnsahaj".to_string(),
                "lumen".to_string()
            ))
        );
    }

    #[test]
    fn parse_remote_url_handles_ssh_url_form() {
        let parsed = parse_remote_url("ssh://git@github.com/jnsahaj/lumen.git");
        assert_eq!(
            parsed,
            Some((
                "github.com".to_string(),
                "jnsahaj".to_string(),
                "lumen".to_string()
            ))
        );
    }

    #[test]
    fn parse_remote_url_handles_ssh_url_with_port() {
        let parsed = parse_remote_url("ssh://git@example.com:2222/owner/repo.git");
        assert_eq!(
            parsed,
            Some((
                "example.com".to_string(),
                "owner".to_string(),
                "repo".to_string()
            ))
        );
    }

    #[test]
    fn parse_remote_url_rejects_unparseable_input() {
        assert_eq!(parse_remote_url("not a url at all"), None);
        assert_eq!(parse_remote_url("https://github.com/onlyowner"), None);
    }

    #[test]
    fn sanitize_branch_name_replaces_slashes() {
        assert_eq!(
            sanitize_branch_name("feature/foo-bar"),
            Some("feature__foo-bar".to_string())
        );
    }

    #[test]
    fn sanitize_branch_name_rejects_path_traversal() {
        assert_eq!(sanitize_branch_name("../etc/passwd"), None);
        assert_eq!(sanitize_branch_name("feature/../evil"), None);
    }

    #[test]
    fn sanitize_branch_name_rejects_empty() {
        assert_eq!(sanitize_branch_name(""), None);
    }

    #[test]
    fn resolve_storage_path_uses_origin_when_available() {
        let worktree = PathBuf::from("/home/user/code/lumen");
        let path = resolve_storage_path(
            Some(("github.com", "jnsahaj", "lumen")),
            &worktree,
            Some("feature/foo"),
        )
        .unwrap();

        assert!(path.ends_with("lumen/viewed/github.com/jnsahaj/lumen/feature__foo.json"));
    }

    #[test]
    fn resolve_storage_path_falls_back_to_local_slug_without_origin() {
        let worktree = PathBuf::from("/home/user/code/scratch");
        let path = resolve_storage_path(None, &worktree, Some("main")).unwrap();

        let path_str = path.to_string_lossy();
        assert!(path_str.contains("local__"));
        assert!(path_str.contains("home__user__code__scratch"));
        assert!(path_str.ends_with("main.json"));
    }

    #[test]
    fn resolve_storage_path_returns_none_without_branch() {
        let worktree = PathBuf::from("/home/user/code/lumen");
        let path = resolve_storage_path(Some(("github.com", "a", "b")), &worktree, None);

        assert_eq!(path, None);
    }

    #[test]
    fn state_base_dir_prefers_xdg_state_home_when_set() {
        let base = state_base_dir_from(Some("/custom/state"), Some(PathBuf::from("/home/user")));
        assert_eq!(base, PathBuf::from("/custom/state"));
    }

    #[test]
    fn state_base_dir_falls_back_to_home_local_state() {
        let base = state_base_dir_from(None, Some(PathBuf::from("/home/user")));
        assert_eq!(base, PathBuf::from("/home/user/.local/state"));
    }

    #[test]
    fn state_base_dir_ignores_empty_xdg_state_home() {
        let base = state_base_dir_from(Some(""), Some(PathBuf::from("/home/user")));
        assert_eq!(base, PathBuf::from("/home/user/.local/state"));
    }

    #[test]
    fn load_missing_file_returns_default_state() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nope.json");

        let state = load(&path);

        assert_eq!(state.schema, SCHEMA_VERSION);
        assert!(state.files.is_empty());
    }

    #[test]
    fn load_schema_mismatch_returns_default_state() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, r#"{"schema":999,"files":{"a.txt":"deadbeef"}}"#).unwrap();

        let state = load(&path);

        assert_eq!(state.schema, SCHEMA_VERSION);
        assert!(state.files.is_empty());
    }

    #[test]
    fn load_malformed_json_returns_default_state() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, "{ not json").unwrap();

        let state = load(&path);

        assert_eq!(state, ViewedState::new());
    }

    #[test]
    fn save_merged_roundtrips_through_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("branch.json");

        save_merged(&path, |vs| {
            vs.files.insert("a.txt".to_string(), "hash-a".to_string());
        })
        .unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.files.get("a.txt"), Some(&"hash-a".to_string()));
    }

    #[test]
    fn save_merged_merges_instead_of_replacing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("branch.json");

        save_merged(&path, |vs| {
            vs.files.insert("a.txt".to_string(), "hash-a".to_string());
        })
        .unwrap();

        save_merged(&path, |vs| {
            vs.files.insert("b.txt".to_string(), "hash-b".to_string());
        })
        .unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.files.get("a.txt"), Some(&"hash-a".to_string()));
        assert_eq!(loaded.files.get("b.txt"), Some(&"hash-b".to_string()));
        assert_eq!(loaded.files.len(), 2);
    }

    #[test]
    fn save_merged_creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested").join("dirs").join("branch.json");

        save_merged(&path, |vs| {
            vs.files.insert("a.txt".to_string(), "hash-a".to_string());
        })
        .unwrap();

        assert!(path.exists());
    }

    #[test]
    fn save_merged_leaves_no_tmp_files_behind() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("branch.json");

        save_merged(&path, |vs| {
            vs.files.insert("a.txt".to_string(), "hash-a".to_string());
        })
        .unwrap();

        let leftover_tmp = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".tmp."));
        assert!(!leftover_tmp);
    }

    #[test]
    fn prune_stale_branches_removes_files_for_deleted_branches() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("main.json"), "{}").unwrap();
        fs::write(dir.path().join("feature__gone.json"), "{}").unwrap();

        prune_stale_branches(dir.path(), &["main".to_string()]);

        assert!(dir.path().join("main.json").exists());
        assert!(!dir.path().join("feature__gone.json").exists());
    }

    #[test]
    fn prune_stale_branches_keeps_existing_branches() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("feature__foo.json"), "{}").unwrap();

        prune_stale_branches(dir.path(), &["feature/foo".to_string()]);

        assert!(dir.path().join("feature__foo.json").exists());
    }

    #[test]
    fn prune_stale_branches_swallows_missing_dir_error() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");

        // Should not panic even though the directory doesn't exist.
        prune_stale_branches(&missing, &[]);
    }
}
