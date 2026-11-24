use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use git2::Repository;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// How long changes are gathered before they are reported as one event.
const GATHERING_TIME: Duration = Duration::from_millis(500);

/// Changed files from file system watcher
pub struct WatchEvent {
    pub changed_files: HashSet<String>,
}

/// Owns the watcher; dropping it stops the watching.
#[allow(dead_code)]
pub struct WatchHandle(Arc<Mutex<RecommendedWatcher>>);

/// The folders of a Git working tree worth watching. Every folder is registered on
/// its own, and a dependency folder such as `.venv` holds thousands of them.
struct WatchedTree {
    root: PathBuf,
    repository: Repository,
}

impl WatchedTree {
    fn is_reviewed_folder(&self, folder: &Path) -> bool {
        let Ok(relative) = folder.strip_prefix(&self.root) else {
            return false;
        };
        folder.file_name().is_some_and(|name| name != ".git")
            && !self.repository.is_path_ignored(relative).unwrap_or(false)
    }

    /// Watches `folder` and the folders under it that Git does not ignore. Fails
    /// only when `folder` itself cannot be watched.
    fn watch(&self, watcher: &mut RecommendedWatcher, folder: &Path) -> notify::Result<()> {
        watcher.watch(folder, RecursiveMode::NonRecursive)?;
        for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) && self.is_reviewed_folder(&path) {
                let _ = self.watch(watcher, &path);
            }
        }
        Ok(())
    }
}

/// Watch the working tree at `root`. Reported paths are relative to `root`,
/// matching `FileDiff::filename` (e.g. "src/foo.rs").
///
/// On Linux, where every folder is registered on its own, the folders Git ignores
/// are left out of a Git repository. Of Git's own folder only the top is watched,
/// where a commit, a checkout, staging and a fetch write a file. Elsewhere one
/// registration covers the whole tree, and registering folders one by one is slower.
pub fn setup_watcher(root: &Path) -> Option<(WatchHandle, Receiver<WatchEvent>)> {
    let (changed_path_tx, changed_path_rx) = mpsc::channel::<PathBuf>();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            return;
        };
        // Opening and reading a file is reported as well. Reloading the diff reads
        // files, so counting those as changes makes every reload cause the next one.
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        for path in event.paths {
            let _ = changed_path_tx.send(path);
        }
    })
    .ok()?;
    let repository = cfg!(target_os = "linux").then(|| Repository::open(root).ok()).flatten();
    let tree = repository.map(|repository| WatchedTree {
        root: root.to_path_buf(),
        repository,
    });
    match &tree {
        Some(tree) => {
            tree.watch(&mut watcher, root).ok()?;
            let _ = watcher.watch(tree.repository.path(), RecursiveMode::NonRecursive);
        }
        None => watcher.watch(root, RecursiveMode::Recursive).ok()?,
    }
    let watcher = Arc::new(Mutex::new(watcher));
    // The thread below must not keep the watcher alive: it ends when the watcher does.
    let watcher_of_new_folders = Arc::downgrade(&watcher);
    let watch_if_new_folder = move |path: &Path| {
        let Some(tree) = &tree else {
            return;
        };
        if path.is_dir() && tree.is_reviewed_folder(path) {
            if let Some(watcher) = watcher_of_new_folders.upgrade() {
                let _ = tree.watch(&mut watcher.lock().unwrap(), path);
            }
        }
    };

    let (event_tx, event_rx) = mpsc::channel();
    let root = root.to_path_buf();
    let relative_to_root = move |path: PathBuf| {
        path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().to_string()
    };
    // Ends when the watcher is dropped, or when nobody listens any more.
    thread::spawn(move || {
        while let Ok(first_path) = changed_path_rx.recv() {
            watch_if_new_folder(&first_path);
            let mut changed_files = HashSet::from([relative_to_root(first_path)]);
            let gathered_until = Instant::now() + GATHERING_TIME;
            while let Ok(path) = changed_path_rx
                .recv_timeout(gathered_until.saturating_duration_since(Instant::now()))
            {
                watch_if_new_folder(&path);
                changed_files.insert(relative_to_root(path));
            }
            if event_tx.send(WatchEvent { changed_files }).is_err() {
                break;
            }
        }
    });

    Some((WatchHandle(watcher), event_rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_writing_a_file_is_reported_and_reading_it_is_not() {
        let root = tempfile::tempdir().expect("temp dir");
        let root_path = root.path().canonicalize().expect("canonical temp dir");
        fs::create_dir(root_path.join("src")).expect("create src");
        let file = root_path.join("src").join("a.rs");
        fs::write(&file, "one\n").expect("write the file");
        let (_handle, events) = setup_watcher(&root_path).expect("watcher");

        fs::read_to_string(&file).expect("read the file");
        fs::read_dir(root_path.join("src")).expect("list the directory").count();
        assert!(events.recv_timeout(GATHERING_TIME * 3).is_err(), "a read was reported");

        fs::write(&file, "two\n").expect("change the file");
        let event = events.recv_timeout(Duration::from_secs(5)).expect("the change is reported");
        assert_eq!(event.changed_files, HashSet::from(["src/a.rs".to_string()]));
    }

    /// A Git repository with `.venv/` ignored, holding `src/` and `.venv/lib/`.
    fn repository_with_an_ignored_folder() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().expect("temp dir");
        let root_path = root.path().canonicalize().expect("canonical temp dir");
        Repository::init(&root_path).expect("git init");
        fs::write(root_path.join(".gitignore"), ".venv/\n").expect("write .gitignore");
        fs::create_dir(root_path.join("src")).expect("create src");
        fs::create_dir_all(root_path.join(".venv/lib")).expect("create .venv/lib");
        (root, root_path)
    }

    fn changed_files(events: &Receiver<WatchEvent>) -> Option<HashSet<String>> {
        events.recv_timeout(GATHERING_TIME * 4).ok().map(|event| event.changed_files)
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_changes_in_a_folder_git_ignores_are_not_reported() {
        let (_root, root_path) = repository_with_an_ignored_folder();
        let (_handle, events) = setup_watcher(&root_path).expect("watcher");

        fs::write(root_path.join(".venv/lib/module.py"), "x = 1\n").expect("write ignored file");
        fs::create_dir(root_path.join(".venv/lib/package")).expect("create ignored folder");
        assert_eq!(changed_files(&events), None);

        fs::write(root_path.join("src/a.rs"), "one\n").expect("write the file");
        assert_eq!(changed_files(&events), Some(HashSet::from(["src/a.rs".to_string()])));
    }

    #[test]
    fn test_a_change_in_a_folder_created_during_the_review_is_reported() {
        let (_root, root_path) = repository_with_an_ignored_folder();
        let (_handle, events) = setup_watcher(&root_path).expect("watcher");

        fs::create_dir_all(root_path.join("src/new/deeper")).expect("create folders");
        assert!(changed_files(&events).is_some(), "the new folder was not reported");

        fs::write(root_path.join("src/new/deeper/b.rs"), "one\n").expect("write the file");
        assert_eq!(
            changed_files(&events),
            Some(HashSet::from(["src/new/deeper/b.rs".to_string()]))
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_what_git_writes_on_a_commit_or_checkout_is_reported() {
        let (_root, root_path) = repository_with_an_ignored_folder();
        let (_handle, events) = setup_watcher(&root_path).expect("watcher");

        fs::write(root_path.join(".git/index"), "").expect("write the index");
        assert_eq!(changed_files(&events), Some(HashSet::from([".git/index".to_string()])));

        fs::create_dir_all(root_path.join(".git/objects/ab")).expect("create an object folder");
        fs::write(root_path.join(".git/objects/ab/cdef"), "").expect("write an object");
        assert_eq!(changed_files(&events), None);
    }
}
