use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};

/// Changed files from file system watcher
pub struct WatchEvent {
    pub changed_files: HashSet<String>,
}

pub fn setup_watcher() -> Option<Receiver<WatchEvent>> {
    let (tx, rx) = mpsc::channel();

    // Get current directory for making paths relative
    let cwd = std::env::current_dir().ok();

    let mut debouncer = new_debouncer(
        Duration::from_millis(500),
        move |res: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
            if let Ok(events) = res {
                let mut changed_files = HashSet::new();
                for event in events {
                    if event.kind == DebouncedEventKind::Any {
                        // Normalize path to be relative from current directory
                        // This matches the format used in file_diffs.filename (e.g., "src/foo.rs")
                        let path_str = if let Some(ref cwd) = cwd {
                            event
                                .path
                                .strip_prefix(cwd)
                                .unwrap_or(&event.path)
                                .to_string_lossy()
                                .to_string()
                        } else {
                            event.path.to_string_lossy().to_string()
                        };
                        // Also strip leading "./" if present
                        let normalized = path_str.strip_prefix("./").unwrap_or(&path_str);
                        changed_files.insert(normalized.to_string());
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
        .watch(Path::new("."), notify::RecursiveMode::Recursive)
        .ok()?;

    std::mem::forget(debouncer);

    Some(rx)
}
