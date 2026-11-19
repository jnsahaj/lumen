//! Threads of one repository, kept in a file outside the repository so that the
//! review UI and `lumiere thread` can both read and change them.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::review_thread::Thread;

const SCHEMA_VERSION: u64 = 1;

#[derive(Serialize, Deserialize)]
struct ThreadFile {
    version: u64,
    repository: String,
    threads: Vec<Thread>,
}

/// Identifies one version of the thread file, to notice writes by another process.
#[derive(Clone, Debug, PartialEq)]
pub struct StoreStamp(SystemTime, u64);

pub struct ThreadStore {
    directory: PathBuf,
    repository_root: PathBuf,
}

impl ThreadStore {
    pub fn for_repository(repository_root: &Path) -> Result<Self, String> {
        let state_dir = default_state_dir()
            .ok_or_else(|| "no state directory is available on this system".to_string())?;
        Ok(Self::in_state_dir(&state_dir, repository_root))
    }

    pub fn in_state_dir(state_dir: &Path, repository_root: &Path) -> Self {
        let repository_root = repository_root
            .canonicalize()
            .unwrap_or_else(|_| repository_root.to_path_buf());
        let root_hash: String = Sha256::digest(repository_root.to_string_lossy().as_bytes())[..8]
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect();
        Self {
            directory: state_dir.join(root_hash),
            repository_root,
        }
    }

    fn file_path(&self) -> PathBuf {
        self.directory.join("threads.json")
    }

    pub fn stamp(&self) -> Option<StoreStamp> {
        let metadata = fs::metadata(self.file_path()).ok()?;
        Some(StoreStamp(metadata.modified().ok()?, metadata.len()))
    }

    /// The stored threads. A repository without a thread file has none.
    pub fn load(&self) -> Result<Vec<Thread>, String> {
        let path = self.file_path();
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("cannot read {}: {}", path.display(), error)),
        };
        let invalid = |error: serde_json::Error| format!("{} is not valid: {}", path.display(), error);
        let json: serde_json::Value = serde_json::from_str(&content).map_err(invalid)?;
        let version = json.get("version").and_then(serde_json::Value::as_u64);
        if version != Some(SCHEMA_VERSION) {
            return Err(format!(
                "{} has schema version {}, but this lumiere reads version {}",
                path.display(),
                version.map_or("unknown".to_string(), |version| version.to_string()),
                SCHEMA_VERSION
            ));
        }
        let file: ThreadFile = serde_json::from_value(json).map_err(invalid)?;
        Ok(file.threads)
    }

    /// Applies `change` to the stored threads while holding the store's lock, so that
    /// two processes changing threads at once do not lose each other's work.
    /// Returns the threads after the change and what `change` returned.
    pub fn update<T>(
        &self,
        change: impl FnOnce(&mut Vec<Thread>) -> T,
    ) -> Result<(Vec<Thread>, T), String> {
        let failed = |action: &str, error: std::io::Error| {
            format!("cannot {} in {}: {}", action, self.directory.display(), error)
        };
        fs::create_dir_all(&self.directory).map_err(|error| failed("create the folder", error))?;
        let lock = File::create(self.directory.join("threads.lock"))
            .map_err(|error| failed("create the lock", error))?;
        lock.lock().map_err(|error| failed("take the lock", error))?;

        let stored_threads = self.load()?;
        let mut threads = stored_threads.clone();
        let outcome = change(&mut threads);
        if threads != stored_threads {
            let file = ThreadFile {
                version: SCHEMA_VERSION,
                repository: self.repository_root.to_string_lossy().into_owned(),
                threads: threads.clone(),
            };
            let json = serde_json::to_string_pretty(&file).map_err(|error| error.to_string())?;
            let temporary_path = self
                .directory
                .join(format!("threads.json.{}.tmp", std::process::id()));
            fs::write(&temporary_path, json).map_err(|error| failed("write threads", error))?;
            fs::rename(&temporary_path, self.file_path())
                .map_err(|error| failed("replace threads", error))?;
        }
        Ok((threads, outcome))
    }
}

/// Tests keep their thread files out of the user's state directory.
#[cfg(test)]
fn default_state_dir() -> Option<PathBuf> {
    Some(std::env::temp_dir().join(format!("lumiere-test-state-{}", std::process::id())))
}

#[cfg(not(test))]
fn default_state_dir() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|state_dir| state_dir.join("lumiere"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::diff::review_thread::{AnnotationTarget, Comment};
    use crate::vcs::test_utils::make_temp_dir;

    fn store_in(state_dir: &Path) -> ThreadStore {
        ThreadStore::in_state_dir(state_dir, Path::new("/work/api"))
    }

    fn file_thread(body: &str) -> Thread {
        Thread::new(
            "src/api.rs".to_string(),
            AnnotationTarget::File,
            None,
            Comment::human(body.to_string(), true),
        )
    }

    #[test]
    fn test_threads_survive_a_round_trip_and_a_missing_file_is_empty() {
        let state_dir = make_temp_dir("thread-store-round-trip");
        let store = store_in(&state_dir);
        assert_eq!(store.load(), Ok(Vec::new()));

        let thread = file_thread("rename this");
        let (stored, ()) = store.update(|threads| threads.push(thread.clone())).expect("update");

        assert_eq!(stored, vec![thread.clone()]);
        assert_eq!(store_in(&state_dir).load(), Ok(vec![thread]));
        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_two_writers_appending_at_once_keep_every_comment() {
        let state_dir = make_temp_dir("thread-store-writers");
        let thread = file_thread("first");
        let thread_id = thread.id.clone();
        store_in(&state_dir).update(|threads| threads.push(thread)).expect("create thread");

        let writers: Vec<_> = (0..2)
            .map(|writer| {
                let state_dir = state_dir.clone();
                let thread_id = thread_id.clone();
                std::thread::spawn(move || {
                    let store = store_in(&state_dir);
                    for reply in 0..20 {
                        store
                            .update(|threads| {
                                let thread = threads.iter_mut().find(|t| t.id == thread_id).unwrap();
                                let body = format!("writer {} reply {}", writer, reply);
                                thread.comments.push(Comment::agent(body, "pi".to_string()));
                            })
                            .expect("append");
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().expect("writer thread");
        }

        let threads = store_in(&state_dir).load().expect("load");
        assert_eq!(threads[0].comments.len(), 41);
        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn test_unknown_schema_version_is_rejected_and_left_untouched() {
        let state_dir = make_temp_dir("thread-store-version");
        let store = store_in(&state_dir);
        store.update(|threads| threads.push(file_thread("first"))).expect("create file");
        let newer = fs::read_to_string(store.file_path())
            .expect("read")
            .replace("\"version\": 1", "\"version\": 2");
        fs::write(store.file_path(), &newer).expect("write newer version");

        let load_error = store.load().expect_err("newer version");
        let update_error = store.update(|threads| threads.clear()).expect_err("newer version");

        assert!(load_error.contains("has schema version 2, but this lumiere reads version 1"), "{}", load_error);
        assert_eq!(update_error, load_error);
        assert_eq!(fs::read_to_string(store.file_path()).expect("read"), newer);
        let _ = fs::remove_dir_all(&state_dir);
    }
}
