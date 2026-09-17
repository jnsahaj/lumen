use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::state::{Annotation, AnnotationTarget};
use super::types::DiffPanelFocus;

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AnnotationStore {
    #[serde(default)]
    pub schema: u32,
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub annotations: Vec<StoredAnnotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAnnotation {
    pub id: u64,
    pub filename: String,
    pub target: StoredTarget,
    pub content: String,
    pub created_at_unix: u64,
    pub anchor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StoredTarget {
    File,
    LineRange {
        panel: StoredPanel,
        start_line: usize,
        end_line: usize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StoredPanel {
    None,
    Old,
    New,
}

fn panel_to_stored(panel: DiffPanelFocus) -> StoredPanel {
    match panel {
        DiffPanelFocus::None => StoredPanel::None,
        DiffPanelFocus::Old => StoredPanel::Old,
        DiffPanelFocus::New => StoredPanel::New,
    }
}

fn panel_from_stored(panel: &StoredPanel) -> DiffPanelFocus {
    match panel {
        StoredPanel::None => DiffPanelFocus::None,
        StoredPanel::Old => DiffPanelFocus::Old,
        StoredPanel::New => DiffPanelFocus::New,
    }
}

fn target_to_stored(target: &AnnotationTarget) -> StoredTarget {
    match target {
        AnnotationTarget::File => StoredTarget::File,
        AnnotationTarget::LineRange {
            panel,
            start_line,
            end_line,
        } => StoredTarget::LineRange {
            panel: panel_to_stored(*panel),
            start_line: *start_line,
            end_line: *end_line,
        },
    }
}

fn target_from_stored(target: &StoredTarget) -> AnnotationTarget {
    match target {
        StoredTarget::File => AnnotationTarget::File,
        StoredTarget::LineRange {
            panel,
            start_line,
            end_line,
        } => AnnotationTarget::LineRange {
            panel: panel_from_stored(panel),
            start_line: *start_line,
            end_line: *end_line,
        },
    }
}

/// Convert a live `Annotation` into its on-disk representation.
pub fn to_stored(ann: &Annotation) -> StoredAnnotation {
    let created_at_unix = ann
        .created_at
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    StoredAnnotation {
        id: ann.id,
        filename: ann.filename.clone(),
        target: target_to_stored(&ann.target),
        content: ann.content.clone(),
        created_at_unix,
        anchor: ann.anchor.clone(),
    }
}

/// Convert a stored annotation back into a live `Annotation`.
///
/// `stale` is always initialized to `false` here — the caller recomputes
/// staleness by comparing `anchor` against the current on-disk blob hash.
pub fn from_stored(stored: &StoredAnnotation) -> Annotation {
    Annotation {
        id: stored.id,
        filename: stored.filename.clone(),
        target: target_from_stored(&stored.target),
        content: stored.content.clone(),
        created_at: UNIX_EPOCH + Duration::from_secs(stored.created_at_unix),
        anchor: stored.anchor.clone(),
        stale: false,
    }
}

/// Pure path resolution — no I/O, no git shelling, easy to unit test.
/// `owner_repo` is e.g. "owner/repo" from `resolve_origin_repo()`, or `None` if unresolvable.
/// `fallback_slug` is used instead when `owner_repo` is `None` (e.g. a slugified cwd path).
/// `pr_number` takes priority as the session key when present (PR review resumes by PR, not branch,
/// since branches get force-pushed). Otherwise the session key is a sanitized `diff_reference`,
/// falling back to `current_branch`. If neither `pr_number`, `diff_reference`, nor `current_branch`
/// is available, returns `None` (persistence disabled for this session).
pub fn resolve_annotation_store_path(
    owner_repo: Option<&str>,
    fallback_slug: &str,
    pr_number: Option<u64>,
    diff_reference: Option<&str>,
    current_branch: Option<&str>,
) -> Option<PathBuf> {
    let state_base = state_base_dir()?;
    let repo_segment = owner_repo
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("local__{}", fallback_slug));
    let session_key = if let Some(n) = pr_number {
        format!("pr-{}", n)
    } else if let Some(r) = diff_reference {
        sanitize_key(r)
    } else if let Some(b) = current_branch {
        sanitize_key(b)
    } else {
        return None;
    };
    Some(
        state_base
            .join("lumen")
            .join("annotations")
            .join(repo_segment)
            .join(format!("{}.json", session_key)),
    )
}

fn state_base_dir() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("XDG_STATE_HOME") {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    dirs::home_dir().map(|h| h.join(".local").join("state"))
}

fn sanitize_key(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .to_lowercase()
}

/// Slugify an arbitrary path for the `local__<slug>` fallback repo segment.
pub fn slugify_path(path: &Path) -> String {
    sanitize_key(&path.to_string_lossy())
}

pub fn load(path: &Path) -> AnnotationStore {
    let Ok(contents) = fs::read_to_string(path) else {
        return AnnotationStore::default();
    };
    match serde_json::from_str::<AnnotationStore>(&contents) {
        Ok(store) if store.schema == SCHEMA_VERSION => store,
        _ => AnnotationStore::default(),
    }
}

pub fn save_merged(path: &Path, update: impl FnOnce(&mut AnnotationStore)) -> io::Result<()> {
    let mut store = load(path);
    store.schema = SCHEMA_VERSION;
    update(&mut store);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if store.annotations.is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    let json = serde_json::to_string_pretty(&store).unwrap_or_default();
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, json)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// git2::Oid::hash_object over the working-tree file's current bytes on disk.
/// Returns `None` on any I/O or git2 error — never panics.
pub fn blob_hash_for_path(filename: &str) -> Option<String> {
    let bytes = fs::read(filename).ok()?;
    git2::Oid::hash_object(git2::ObjectType::Blob, &bytes)
        .ok()
        .map(|oid| oid.to_string())
}

/// Best-effort deletion of `*.json` files under `repo_dir` whose mtime is older
/// than `max_age_days`. Swallows every error — must never fail the caller.
pub fn prune_stale(repo_dir: &Path, max_age_days: u64) {
    let Ok(entries) = fs::read_dir(repo_dir) else {
        return;
    };
    let cutoff = SystemTime::now().checked_sub(Duration::from_secs(max_age_days * 86_400));
    let Some(cutoff) = cutoff else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if modified < cutoff {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumen-annotation-store-test-{}", name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_path_prefers_pr_number_over_other_args() {
        let path = resolve_annotation_store_path(
            Some("owner/repo"),
            "fallback",
            Some(42),
            Some("main..feature"),
            Some("feature"),
        )
        .unwrap();

        assert_eq!(path.file_name().unwrap(), "pr-42.json");
    }

    #[test]
    fn resolve_path_uses_diff_reference_when_no_pr() {
        let path = resolve_annotation_store_path(
            Some("owner/repo"),
            "fallback",
            None,
            Some("main..feature"),
            Some("feature"),
        )
        .unwrap();

        assert_eq!(path.file_name().unwrap(), "main--feature.json");
    }

    #[test]
    fn resolve_path_uses_current_branch_when_no_pr_or_reference() {
        let path = resolve_annotation_store_path(
            Some("owner/repo"),
            "fallback",
            None,
            None,
            Some("my-feature"),
        )
        .unwrap();

        assert_eq!(path.file_name().unwrap(), "my-feature.json");
    }

    #[test]
    fn resolve_path_returns_none_when_no_session_key_available() {
        let path = resolve_annotation_store_path(Some("owner/repo"), "fallback", None, None, None);

        assert!(path.is_none());
    }

    #[test]
    fn resolve_path_falls_back_to_local_slug_when_owner_repo_unresolved() {
        let path =
            resolve_annotation_store_path(None, "my-slug", None, None, Some("main")).unwrap();

        assert!(path.to_string_lossy().contains("local__my-slug"));
    }

    #[test]
    fn load_returns_default_store_when_file_missing() {
        let dir = temp_dir("load-missing");
        let path = dir.join("does-not-exist.json");

        let store = load(&path);

        assert_eq!(store.schema, 0);
        assert!(store.annotations.is_empty());
    }

    #[test]
    fn load_returns_default_store_when_schema_mismatched() {
        let dir = temp_dir("load-schema-mismatch");
        let path = dir.join("annotations.json");
        fs::write(&path, r#"{"schema":999,"next_id":5,"annotations":[]}"#).unwrap();

        let store = load(&path);

        assert_eq!(store.schema, 0);
        assert_eq!(store.next_id, 0);
    }

    #[test]
    fn save_merged_round_trips_annotations() {
        let dir = temp_dir("save-round-trip");
        let path = dir.join("annotations.json");

        save_merged(&path, |store| {
            store.next_id = 3;
            store.annotations.push(StoredAnnotation {
                id: 1,
                filename: "src/main.rs".to_string(),
                target: StoredTarget::File,
                content: "looks good".to_string(),
                created_at_unix: 1000,
                anchor: Some("deadbeef".to_string()),
            });
        })
        .unwrap();

        let loaded = load(&path);
        assert_eq!(loaded.next_id, 3);
        assert_eq!(loaded.annotations.len(), 1);
        assert_eq!(loaded.annotations[0].filename, "src/main.rs");
        assert_eq!(loaded.annotations[0].content, "looks good");
    }

    #[test]
    fn save_merged_deletes_file_when_store_ends_up_empty() {
        let dir = temp_dir("save-empty-deletes");
        let path = dir.join("annotations.json");

        save_merged(&path, |store| {
            store.annotations.push(StoredAnnotation {
                id: 1,
                filename: "a.rs".to_string(),
                target: StoredTarget::File,
                content: "note".to_string(),
                created_at_unix: 1,
                anchor: None,
            });
        })
        .unwrap();
        assert!(path.exists());

        save_merged(&path, |store| {
            store.annotations.clear();
        })
        .unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn save_merged_leaves_no_leftover_tmp_file() {
        let dir = temp_dir("save-no-tmp-leftover");
        let path = dir.join("annotations.json");

        save_merged(&path, |store| {
            store.annotations.push(StoredAnnotation {
                id: 1,
                filename: "a.rs".to_string(),
                target: StoredTarget::File,
                content: "note".to_string(),
                created_at_unix: 1,
                anchor: None,
            });
        })
        .unwrap();

        let tmp_path = path.with_extension("json.tmp");
        assert!(!tmp_path.exists());
    }
}
