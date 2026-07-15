use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use super::{PrError, PrInfo, ViewedFileProvider};

struct ViewedUpdate {
    sequence: u64,
    path: String,
    viewed: bool,
}

fn coalesce_updates(
    first: ViewedUpdate,
    queued: impl Iterator<Item = ViewedUpdate>,
) -> Vec<ViewedUpdate> {
    let mut batch = vec![first];
    for queued in queued {
        if let Some(update) = batch.iter_mut().find(|update| update.path == queued.path) {
            *update = queued;
        } else {
            batch.push(queued);
        }
    }
    batch
}

pub(crate) struct ViewedCompletion {
    pub path: String,
    pub viewed: bool,
    pub result: Result<(), PrError>,
}

pub(crate) struct ViewedFileSync {
    provider: ViewedFileProvider,
    updates: Option<Sender<ViewedUpdate>>,
    completions: Receiver<(u64, ViewedCompletion)>,
    pending: HashMap<String, (u64, bool)>,
    next_sequence: u64,
    local_completions: VecDeque<ViewedCompletion>,
    worker: Option<JoinHandle<()>>,
}

impl ViewedFileSync {
    pub fn new(pr: &PrInfo) -> Option<Self> {
        let provider = pr.viewed_file_provider()?;
        let worker_provider = provider.clone();
        let (update_tx, update_rx) = mpsc::channel::<ViewedUpdate>();
        let (completion_tx, completion_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            while let Ok(first) = update_rx.recv() {
                let batch = coalesce_updates(first, update_rx.try_iter());
                for update in batch {
                    let result = worker_provider.set(&update.path, update.viewed);
                    let completion = ViewedCompletion {
                        path: update.path,
                        viewed: update.viewed,
                        result,
                    };
                    let _ = completion_tx.send((update.sequence, completion));
                }
            }
        });
        Some(Self {
            provider,
            updates: Some(update_tx),
            completions: completion_rx,
            pending: HashMap::new(),
            next_sequence: 0,
            local_completions: VecDeque::new(),
            worker: Some(worker),
        })
    }

    pub fn set(&mut self, path: &str, viewed: bool) {
        let path = path.to_string();
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.pending.insert(path.clone(), (sequence, viewed));
        let send_failed = self.updates.as_ref().is_none_or(|updates| {
            updates
                .send(ViewedUpdate {
                    sequence,
                    path: path.clone(),
                    viewed,
                })
                .is_err()
        });
        if send_failed {
            self.pending.remove(&path);
            self.local_completions.push_back(ViewedCompletion {
                path,
                viewed,
                result: Err(PrError::Other(
                    "viewed status synchronization worker stopped".to_string(),
                )),
            });
        }
    }

    pub fn viewed_paths(&self) -> Result<HashSet<String>, PrError> {
        let mut paths = self.provider.fetch()?;
        for (path, (_, viewed)) in &self.pending {
            if *viewed {
                paths.insert(path.clone());
            } else {
                paths.remove(path);
            }
        }
        Ok(paths)
    }

    pub fn drain(&mut self) -> Vec<ViewedCompletion> {
        let mut current = self.local_completions.drain(..).collect::<Vec<_>>();
        while let Ok((sequence, completion)) = self.completions.try_recv() {
            if self.pending.get(&completion.path) == Some(&(sequence, completion.viewed)) {
                self.pending.remove(&completion.path);
                current.push(completion);
            }
        }
        current
    }
}

impl Drop for ViewedFileSync {
    fn drop(&mut self) {
        self.updates.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(sequence: u64, path: &str, viewed: bool) -> ViewedUpdate {
        ViewedUpdate {
            sequence,
            path: path.to_string(),
            viewed,
        }
    }

    #[test]
    fn coalesces_queued_updates_to_latest_value_per_path() {
        let batch = coalesce_updates(
            update(1, "a.rs", true),
            [
                update(2, "b.rs", true),
                update(3, "a.rs", false),
                update(4, "a.rs", true),
            ]
            .into_iter(),
        );

        assert_eq!(batch.len(), 2);
        assert_eq!((batch[0].sequence, batch[0].viewed), (4, true));
        assert_eq!((batch[1].sequence, batch[1].path.as_str()), (2, "b.rs"));
    }
}
