//! Cache of per-file highlighters, filled by a background worker.
//!
//! Tree-sitter needs the whole file to highlight a single line correctly, and
//! parsing costs roughly 20ms per 1000 lines. Callers on the render path ask
//! for a highlighter with `get`, draw plain text on a miss, and `request` a
//! parse that lands in the cache a few frames later.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use super::FileHighlighter;

/// (file index, is new side). Old and new content are independent files from
/// tree-sitter's perspective, so each side gets its own highlighter.
pub type HighlightKey = (usize, bool);

struct Job {
    generation: u64,
    key: HighlightKey,
    content: Arc<str>,
    filename: String,
}

/// Highlighters kept before the least recently used one is evicted.
const MAX_CACHED: usize = 64;

type Done = (u64, HighlightKey, FileHighlighter);

#[derive(Default)]
struct QueueState {
    jobs: Vec<Job>,
    closed: bool,
}

#[derive(Default)]
struct JobQueue {
    state: Mutex<QueueState>,
    wake: Condvar,
}

pub struct HighlightCache {
    /// Finished highlighters with the `clock` reading of their last use.
    ready: RefCell<HashMap<HighlightKey, (Arc<FileHighlighter>, u64)>>,
    clock: Cell<u64>,
    /// Queued or currently being parsed.
    pending: RefCell<HashSet<HighlightKey>>,
    /// Bumped by `clear`. File indices are only meaningful within one
    /// generation, so results from an older one are dropped on arrival.
    generation: Cell<u64>,
    queue: Arc<JobQueue>,
    results_tx: Sender<Done>,
    results_rx: Receiver<Done>,
    worker_started: Cell<bool>,
}

impl Default for HighlightCache {
    fn default() -> Self {
        let (results_tx, results_rx) = mpsc::channel();
        Self {
            ready: RefCell::new(HashMap::new()),
            clock: Cell::new(0),
            pending: RefCell::new(HashSet::new()),
            generation: Cell::new(0),
            queue: Arc::new(JobQueue::default()),
            results_tx,
            results_rx,
            worker_started: Cell::new(false),
        }
    }
}

impl HighlightCache {
    pub fn generation(&self) -> u64 {
        self.generation.get()
    }

    /// Look up a finished highlighter. Never blocks.
    pub fn get(&self, key: HighlightKey) -> Option<Arc<FileHighlighter>> {
        self.drain();
        let now = self.tick();
        let mut ready = self.ready.borrow_mut();
        let (highlighter, last_used) = ready.get_mut(&key)?;
        *last_used = now;
        Some(Arc::clone(highlighter))
    }

    /// Store a highlighter the caller built itself.
    pub fn insert(&self, key: HighlightKey, highlighter: Arc<FileHighlighter>) {
        let now = self.tick();
        let mut ready = self.ready.borrow_mut();
        if ready.len() >= MAX_CACHED && !ready.contains_key(&key) {
            let oldest = ready.iter().min_by_key(|(_, (_, used))| *used).map(|(k, _)| *k);
            if let Some(oldest) = oldest {
                ready.remove(&oldest);
            }
        }
        ready.insert(key, (highlighter, now));
    }

    fn tick(&self) -> u64 {
        self.clock.set(self.clock.get() + 1);
        self.clock.get()
    }

    /// Queue a background parse for `key` unless it is already cached or on
    /// its way. Latest file wins: queued parses for other files are dropped,
    /// so skimming through many files doesn't leave a backlog behind the one
    /// the user stopped on. `content` is only called if a parse is queued.
    pub fn request(
        &self,
        key: HighlightKey,
        content: impl FnOnce() -> Arc<str>,
        filename: &str,
    ) {
        if self.ready.borrow().contains_key(&key) || !self.pending.borrow_mut().insert(key) {
            return;
        }

        {
            let mut queue = self.queue.state.lock().unwrap();
            let mut pending = self.pending.borrow_mut();
            queue.jobs.retain(|job| {
                let keep = job.key.0 == key.0;
                if !keep {
                    pending.remove(&job.key);
                }
                keep
            });
            queue.jobs.push(Job {
                generation: self.generation.get(),
                key,
                content: content(),
                filename: filename.to_string(),
            });
        }
        self.queue.wake.notify_one();

        if !self.worker_started.replace(true) {
            let queue = Arc::clone(&self.queue);
            let results = self.results_tx.clone();
            thread::spawn(move || run_worker(queue, results));
        }
    }

    /// True while a parse is queued or running. The event loop polls faster
    /// while this holds so the result is drawn as soon as it lands.
    pub fn has_pending(&self) -> bool {
        self.drain();
        !self.pending.borrow().is_empty()
    }

    /// Forget everything. Call when the set of files changes.
    pub fn clear(&self) {
        self.generation.set(self.generation.get() + 1);
        self.queue.state.lock().unwrap().jobs.clear();
        self.pending.borrow_mut().clear();
        self.ready.borrow_mut().clear();
    }

    fn drain(&self) {
        while let Ok((generation, key, highlighter)) = self.results_rx.try_recv() {
            if generation != self.generation.get() {
                continue;
            }
            self.pending.borrow_mut().remove(&key);
            self.insert(key, Arc::new(highlighter));
        }
    }
}

impl Drop for HighlightCache {
    fn drop(&mut self) {
        self.queue.state.lock().unwrap().closed = true;
        self.queue.wake.notify_one();
    }
}

fn run_worker(queue: Arc<JobQueue>, results: Sender<Done>) {
    loop {
        let job = {
            let mut state = queue.state.lock().unwrap();
            loop {
                if state.closed {
                    return;
                }
                if let Some(job) = state.jobs.pop() {
                    break job;
                }
                state = queue.wake.wait(state).unwrap();
            }
        };
        let highlighter = FileHighlighter::from_shared(job.content, &job.filename);
        if results.send((job.generation, job.key, highlighter)).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const RUST: &str = "fn main() {\n    let x = 42;\n}\n";

    /// Compiling a highlight query is slow in a debug build; do it up front
    /// so the deadlines below only cover the parse itself.
    fn new_cache() -> HighlightCache {
        crate::command::diff::highlight::preload("a.rs");
        HighlightCache::default()
    }

    fn wait_for(cache: &HighlightCache, key: HighlightKey) -> Arc<FileHighlighter> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(h) = cache.get(key) {
                return h;
            }
            assert!(Instant::now() < deadline, "highlighter never arrived");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn wait_until_idle(cache: &HighlightCache) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while cache.has_pending() {
            assert!(Instant::now() < deadline, "worker never went idle");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn miss_then_highlighted_after_request() {
        let cache = new_cache();
        assert!(cache.get((0, true)).is_none());
        assert!(!cache.has_pending());

        cache.request((0, true), || Arc::from(RUST), "a.rs");
        let highlighter = wait_for(&cache, (0, true));

        assert!(!highlighter.get_line_spans(2, None).is_empty());
        assert!(!cache.has_pending());
    }

    #[test]
    fn sides_are_cached_independently() {
        let cache = new_cache();
        cache.request((0, true), || Arc::from(RUST), "a.rs");
        cache.request((0, false), || Arc::from("struct Old;\n"), "a.rs");

        let new = wait_for(&cache, (0, true));
        let old = wait_for(&cache, (0, false));

        assert_eq!(new.get_line_spans(1, None)[0].content, "fn");
        assert_eq!(old.get_line_spans(1, None)[0].content, "struct");
    }

    #[test]
    fn clear_drops_cached_and_in_flight_results() {
        let cache = new_cache();
        cache.request((0, true), || Arc::from(RUST), "a.rs");
        wait_for(&cache, (0, true));

        cache.request((1, true), || Arc::from(RUST), "b.rs");
        let before = cache.generation();
        cache.clear();

        assert_ne!(cache.generation(), before);
        assert!(!cache.has_pending());
        // Give a parse that was already running time to report back.
        cache.request((2, true), || Arc::from(RUST), "c.rs");
        wait_for(&cache, (2, true));
        assert!(cache.get((0, true)).is_none());
        assert!(cache.get((1, true)).is_none());
    }

    #[test]
    fn inserted_highlighter_is_returned_without_a_parse() {
        let cache = new_cache();
        let highlighter = Arc::new(FileHighlighter::new(RUST, "a.rs"));
        cache.insert((3, false), Arc::clone(&highlighter));

        cache.request((3, false), || Arc::from(RUST), "a.rs");

        assert!(!cache.has_pending());
        assert!(Arc::ptr_eq(&cache.get((3, false)).unwrap(), &highlighter));
    }

    #[test]
    fn least_recently_used_highlighter_is_evicted() {
        let cache = HighlightCache::default();
        for file in 0..MAX_CACHED {
            cache.insert((file, true), Arc::default());
        }
        // Touch the oldest so the second oldest becomes the eviction victim.
        assert!(cache.get((0, true)).is_some());

        cache.insert((MAX_CACHED, true), Arc::default());

        assert!(cache.get((0, true)).is_some());
        assert!(cache.get((1, true)).is_none());
        assert!(cache.get((MAX_CACHED, true)).is_some());
    }

    #[test]
    fn dropped_jobs_can_be_requested_again() {
        let cache = new_cache();
        // Whichever of these the worker hasn't picked up yet gets dropped by
        // the next request for a different file.
        for file in 0..20 {
            cache.request((file, true), || Arc::from(RUST), "a.rs");
        }
        wait_until_idle(&cache);

        cache.request((0, true), || Arc::from(RUST), "a.rs");
        wait_for(&cache, (0, true));
        wait_for(&cache, (19, true));
    }
}
