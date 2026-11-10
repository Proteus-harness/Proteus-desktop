//! Disposable scalar geometry: bounded snapshots, a single low-priority disk
//! worker, replaceable debounced writes, and atomic sibling-file replacement.
//! Like prefs::persistence and Zed's workspace serialization, disk work never
//! runs on the UI thread. This cache contains no transcript or widget state.
use super::height_map::{ColdSpan, DEFAULT_MAX_SPANS};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub(in super::super::super) type Hash = [u8; 32];
pub(super) const MAX_SPANS: usize = DEFAULT_MAX_SPANS;
const MAX_SNAPSHOTS: usize = 4;
const MAX_READS: usize = 16;
const DEBOUNCE: Duration = Duration::from_millis(750);
pub(super) const THROTTLE: Duration = Duration::from_secs(2);
const HEADER_BYTES: usize = 148;
const SPAN_BYTES: usize = 17;
const MAX_FILE_BYTES: usize = HEADER_BYTES + MAX_SPANS * SPAN_BYTES;
const MAX_DISK_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FILES: usize = 128;
const MAGIC: &[u8; 8] = b"PRHEIGHT";

#[cfg(test)]
fn hash(value: impl AsRef<[u8]>) -> Hash {
    Sha256::digest(value.as_ref()).into()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super::super) struct Key {
    pub identity: Hash,
    pub revision: Hash,
    pub layout: Hash,
    pub prefix: usize,
    pub base: Option<(Hash, usize)>,
}

/// At most four scalar vectors exist across UI exports, queued/running writes
/// and unread load results (<= 1.5 MiB logical scalar payload). Admission is
/// before export/decode allocation. A slow disk cannot grow a payload queue.
pub(super) struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(in super::super::super) struct Snapshot {
    pub(super) key: Key,
    pub(super) spans: Vec<ColdSpan>,
    _permit: Permit,
}

impl Snapshot {
    pub(super) fn new(key: Key, spans: Vec<ColdSpan>, permit: Permit) -> Self {
        Self {
            key,
            spans,
            _permit: permit,
        }
    }
}

struct Pending {
    writes: BTreeMap<Hash, (Instant, Snapshot)>,
    reads: VecDeque<(Key, async_channel::Sender<Option<Snapshot>>)>,
    closing: bool,
    finished: bool,
    invalidate: bool,
}

struct Shared {
    state: Mutex<Pending>,
    changed: Condvar,
    snapshots: Arc<AtomicUsize>,
    disabled: AtomicBool,
}

pub(super) struct Store {
    shared: Arc<Shared>,
}
static STORE: OnceLock<Option<Store>> = OnceLock::new();
static BYPASS: AtomicBool = AtomicBool::new(false);

fn cache_dir() -> Option<PathBuf> {
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(root.join(crate::APP_ID).join("row-heights-v1"))
}

fn preferences_dir() -> Option<PathBuf> {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(root.join(crate::APP_ID).join("extension-preferences"))
}

pub(super) fn global() -> Option<&'static Store> {
    if !process_available(&BYPASS, None) {
        return STORE.get().and_then(Option::as_ref);
    }
    STORE
        .get_or_init(|| cache_dir().and_then(|root| Store::start(root, preferences_dir()).ok()))
        .as_ref()
}

pub(super) fn available() -> bool {
    process_available(&BYPASS, STORE.get().and_then(Option::as_ref))
}

fn process_available(invalidated: &AtomicBool, store: Option<&Store>) -> bool {
    !invalidated.load(Ordering::Acquire) && store.is_none_or(|store| !store.disabled())
}

/// Explicit package invalidations can reflect private runtime inputs that the
/// client cannot version. Bypass disk for this process and clear it in worker
/// order; a later process also fingerprints durable package preferences.
pub(super) fn invalidate() {
    // An invalidation before the first chat also forbids later cache reuse.
    BYPASS.store(true, Ordering::Release);
    if let Some(store) = STORE.get().and_then(Option::as_ref) {
        store.invalidate();
    }
}

impl Store {
    fn start(root: PathBuf, preferences: Option<PathBuf>) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(Pending {
                writes: BTreeMap::new(),
                reads: VecDeque::new(),
                closing: false,
                finished: false,
                invalidate: false,
            }),
            changed: Condvar::new(),
            snapshots: Arc::new(AtomicUsize::new(0)),
            disabled: AtomicBool::new(false),
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("row-height-cache".into())
            .spawn(move || {
                // Set only this worker's niceness on Linux; no application priority change.
                #[cfg(target_os = "linux")]
                unsafe {
                    libc::setpriority(libc::PRIO_PROCESS, 0, 10);
                }
                worker_loop(root, preferences, worker);
            })?;
        Ok(Self { shared })
    }

    pub fn reserve(&self) -> Option<Permit> {
        if self.disabled() {
            return None;
        }
        reserve(&self.shared.snapshots)
    }

    fn disabled(&self) -> bool {
        self.shared.disabled.load(Ordering::Acquire)
    }

    fn invalidate(&self) {
        self.shared.disabled.store(true, Ordering::Release);
        let mut state = self.shared.state.lock().unwrap();
        state.reads.clear();
        state.writes.clear();
        state.invalidate = true;
        self.shared.changed.notify_one();
    }

    pub fn load(&self, key: Key) -> Option<async_channel::Receiver<Option<Snapshot>>> {
        let mut state = self.shared.state.lock().unwrap();
        if self.disabled() || state.closing || state.reads.len() >= MAX_READS {
            return None;
        }
        let (send, receive) = async_channel::bounded(1);
        state.reads.push_back((key, send));
        self.shared.changed.notify_one();
        Some(receive)
    }

    pub fn submit(&self, snapshot: Snapshot) {
        let mut state = self.shared.state.lock().unwrap();
        if self.disabled() || state.closing {
            return;
        }
        state
            .writes
            .insert(snapshot.key.identity, (Instant::now(), snapshot));
        self.shared.changed.notify_one();
    }

    pub fn shutdown(&self, timeout: Duration) {
        let mut state = self.shared.state.lock().unwrap();
        state.closing = true;
        state.reads.clear();
        self.shared.changed.notify_one();
        let deadline = Instant::now() + timeout;
        while !state.finished {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap()
                .0;
        }
    }
}

fn reserve(pool: &Arc<AtomicUsize>) -> Option<Permit> {
    pool.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
        (n < MAX_SNAPSHOTS).then_some(n + 1)
    })
    .ok()
    .map(|_| Permit(pool.clone()))
}

fn worker_loop(root: PathBuf, preferences: Option<PathBuf>, shared: Arc<Shared>) {
    enum Work {
        Read(Key, async_channel::Sender<Option<Snapshot>>),
        Write(Snapshot),
        Invalidate,
    }
    let mut settings = None;
    loop {
        let work = {
            let mut state = shared.state.lock().unwrap();
            loop {
                if std::mem::take(&mut state.invalidate) {
                    break Some(Work::Invalidate);
                }
                if let Some((key, _)) = state.reads.front() {
                    let id = key.identity;
                    // Closing and immediately reopening a tab must see its
                    // queued latest geometry even before debounce expires.
                    if let Some((_, snapshot)) = state.writes.remove(&id) {
                        break Some(Work::Write(snapshot));
                    }
                }
                if let Some((key, reply)) = state.reads.pop_front() {
                    break Some(Work::Read(key, reply));
                }
                if let Some((&id, (submitted, _))) =
                    state.writes.iter().min_by_key(|(_, (time, _))| *time)
                {
                    let remaining = DEBOUNCE.saturating_sub(submitted.elapsed());
                    if state.closing || remaining.is_zero() {
                        break state
                            .writes
                            .remove(&id)
                            .map(|(_, snapshot)| Work::Write(snapshot));
                    }
                    state = shared.changed.wait_timeout(state, remaining).unwrap().0;
                } else if state.closing {
                    state.finished = true;
                    shared.changed.notify_all();
                    break None;
                } else {
                    state = shared.changed.wait(state).unwrap();
                }
            }
        };
        match work {
            None => break,
            Some(Work::Invalidate) => {
                let _ = disk::clear(&root);
            }
            Some(Work::Read(key, reply)) => {
                if reply.is_closed() {
                    continue;
                }
                if shared.disabled.load(Ordering::Acquire) {
                    let _ = reply.try_send(None);
                    continue;
                }
                let settings = *settings.get_or_insert_with(|| {
                    disk::preferences_fingerprint(preferences.as_deref()).ok()
                });
                if settings.is_none() {
                    shared.disabled.store(true, Ordering::Release);
                }
                let snapshot = settings.and_then(|settings| {
                    reserve(&shared.snapshots).and_then(|permit| {
                        let original = key.layout;
                        let mut key = key;
                        key.layout = disk_layout(key.layout, settings);
                        let mut snapshot = disk::load(&root, key, permit).ok().flatten()?;
                        snapshot.key.layout = original;
                        Some(snapshot)
                    })
                });
                if shared.disabled.load(Ordering::Acquire) {
                    let _ = reply.try_send(None);
                    continue;
                }
                let _ = reply.try_send(snapshot);
            }
            Some(Work::Write(mut snapshot)) => {
                if shared.disabled.load(Ordering::Acquire) {
                    continue;
                }
                let settings = *settings.get_or_insert_with(|| {
                    disk::preferences_fingerprint(preferences.as_deref()).ok()
                });
                let Some(settings) = settings else {
                    shared.disabled.store(true, Ordering::Release);
                    continue;
                };
                snapshot.key.layout = disk_layout(snapshot.key.layout, settings);
                let _ = disk::save(&root, &snapshot, MAX_DISK_BYTES, MAX_FILES);
            }
        }
    }
}

fn disk_layout(layout: Hash, settings: Hash) -> Hash {
    let mut hash = Sha256::new();
    hash.update(layout);
    hash.update(settings);
    hash.finalize().into()
}

#[path = "height_store/disk.rs"]
mod disk;
#[cfg(test)]
#[path = "height_store/tests.rs"]
mod tests;
