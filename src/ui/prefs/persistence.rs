//! Serial, replaceable snapshots: one write in flight and one latest pending.
//! Like Zed's settings-store file writer, disk work is separate from UI state;
//! unlike its edit queue, intermediate complete snapshots can be replaced.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub(super) struct Completion {
    pub revision: u64,
    pub result: Result<(), String>,
}

impl Completion {
    /// An old success must not clear the error of a newer edit (or vice versa).
    pub fn problem_for(self, revision: u64) -> Option<Option<String>> {
        (self.revision == revision).then(|| {
            self.result
                .err()
                .map(|error| format!("Не сохранено: {error}"))
        })
    }
}

struct State<T> {
    revision: u64,
    pending: Option<(u64, Arc<T>)>,
    completion: Option<Completion>,
    closing: bool,
    finished: bool,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
}

pub(super) struct Writer<T> {
    shared: Arc<Shared<T>>,
}

/// A separate handle lets the quit future wait without keeping UI state alive.
pub(super) struct Shutdown<T>(Arc<Shared<T>>);

impl<T: Send + Sync + 'static> Writer<T> {
    pub fn new(
        mut save: impl FnMut(&T) -> Result<(), String> + Send + 'static,
    ) -> std::io::Result<(Self, async_channel::Receiver<()>)> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                revision: 0,
                pending: None,
                completion: None,
                closing: false,
                finished: false,
            }),
            changed: Condvar::new(),
        });
        let worker = shared.clone();
        // Only a wakeup is queued, never a preferences payload or one per edit.
        let (notify, receiver) = async_channel::bounded(1);
        std::thread::Builder::new()
            .name("preferences-writer".into())
            .spawn(move || {
                loop {
                    let snapshot = {
                        let mut state = worker.state.lock().unwrap();
                        while state.pending.is_none() && !state.closing {
                            state = worker.changed.wait(state).unwrap();
                        }
                        match state.pending.take() {
                            Some(snapshot) => snapshot,
                            None => {
                                state.finished = true;
                                worker.changed.notify_all();
                                break;
                            }
                        }
                    };
                    let result = save(&snapshot.1);
                    let mut state = worker.state.lock().unwrap();
                    state.completion = Some(Completion {
                        revision: snapshot.0,
                        result,
                    });
                    let _ = notify.try_send(());
                }
            })?;
        Ok((Self { shared }, receiver))
    }

    pub fn submit(&self, value: Arc<T>) -> Result<u64, &'static str> {
        let mut state = self.shared.state.lock().unwrap();
        if state.closing {
            return Err("preferences writer is shutting down");
        }
        state.revision += 1;
        let revision = state.revision;
        state.pending = Some((revision, value));
        self.shared.changed.notify_one();
        Ok(revision)
    }

    pub fn completion(&self) -> Option<Completion> {
        self.shared.state.lock().unwrap().completion.take()
    }

    pub fn shutdown(&self) -> Shutdown<T> {
        let mut state = self.shared.state.lock().unwrap();
        state.closing = true;
        self.shared.changed.notify_one();
        Shutdown(self.shared.clone())
    }
}

impl<T> Drop for Writer<T> {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        state.closing = true;
        self.shared.changed.notify_one();
    }
}

impl<T> Shutdown<T> {
    /// Blocking quit boundary, never called for an ordinary preference edit.
    /// A hung filesystem cannot hold app shutdown indefinitely.
    pub fn wait(self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.0.state.lock().unwrap();
        while !state.finished {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, _) = self.0.changed.wait_timeout(state, remaining).unwrap();
            state = next;
        }
        true
    }
}

#[cfg(test)]
mod tests;
