use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// How long changes are gathered before they are reported as one event.
const GATHERING_TIME: Duration = Duration::from_millis(500);

/// Changed files from file system watcher
pub struct WatchEvent {
    pub changed_files: HashSet<String>,
}

/// Owns the watcher; dropping it stops the watching.
#[allow(dead_code)]
pub struct WatchHandle(RecommendedWatcher);

/// Watch the working tree at `root`. Reported paths are relative to `root`,
/// matching `FileDiff::filename` (e.g. "src/foo.rs").
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
    watcher.watch(root, RecursiveMode::Recursive).ok()?;

    let (event_tx, event_rx) = mpsc::channel();
    let root = root.to_path_buf();
    let relative_to_root = move |path: PathBuf| {
        path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().to_string()
    };
    // Ends when the watcher is dropped, or when nobody listens any more.
    thread::spawn(move || {
        while let Ok(first_path) = changed_path_rx.recv() {
            let mut changed_files = HashSet::from([relative_to_root(first_path)]);
            let gathered_until = Instant::now() + GATHERING_TIME;
            while let Ok(path) = changed_path_rx
                .recv_timeout(gathered_until.saturating_duration_since(Instant::now()))
            {
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
}
