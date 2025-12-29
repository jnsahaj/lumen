use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};

pub fn setup_watcher() -> Option<Receiver<()>> {
    let (tx, rx) = mpsc::channel();

    let mut debouncer = new_debouncer(
        Duration::from_millis(500),
        move |res: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
            if let Ok(events) = res {
                for event in events {
                    if event.kind == DebouncedEventKind::Any {
                        let _ = tx.send(());
                        break;
                    }
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
