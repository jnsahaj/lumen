use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};

/// Changed files from file system watcher
pub struct WatchEvent {
    pub changed_files: HashSet<String>,
}

/// Owns the debouncer; dropping it shuts down the watcher thread.
#[allow(dead_code)]
pub struct WatchHandle(notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>);

/// Watch the working tree at `root`. Reported paths are relative to `root`,
/// matching `FileDiff::filename` (e.g. "src/foo.rs").
pub fn setup_watcher(root: &Path) -> Option<(WatchHandle, Receiver<WatchEvent>)> {
    let (tx, rx) = mpsc::channel();
    let event_root = root.to_path_buf();

    let mut debouncer = new_debouncer(
        Duration::from_millis(500),
        move |res: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
            if let Ok(events) = res {
                let mut changed_files = HashSet::new();
                for event in events {
                    if event.kind == DebouncedEventKind::Any {
                        let relative_path = event.path.strip_prefix(&event_root).unwrap_or(&event.path);
                        changed_files.insert(relative_path.to_string_lossy().to_string());
                    }
                }
                if !changed_files.is_empty() {
                    let _ = tx.send(WatchEvent { changed_files });
                }
            }
        },
    )
    .ok()?;

    debouncer
        .watcher()
        .watch(root, notify::RecursiveMode::Recursive)
        .ok()?;

    Some((WatchHandle(debouncer), rx))
}
