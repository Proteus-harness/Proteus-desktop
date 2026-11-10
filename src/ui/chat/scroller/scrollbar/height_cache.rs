//! Shared scalar-cache admission. Cold maps keep their summed height when
//! compacted; cache pressure must never turn a measured history back into a
//! differently sized unknown history.
use std::{
    cell::{Cell, RefCell, RefMut},
    collections::BTreeMap,
    ops::Range,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use super::height_map::{DEFAULT_MAX_SPANS, HeightMap, SPAN_BYTES};
use super::height_store::{self, Key, Snapshot};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const IDLE: Duration = Duration::from_secs(1);

struct Entry {
    owner: Weak<Owner>,
    bytes: usize,
}

#[derive(Default)]
struct Registry {
    next_id: u64,
    bytes: usize,
    entries: BTreeMap<u64, Entry>,
}

struct Budget {
    limit: usize,
    map_spans: usize,
    registry: RefCell<Registry>,
}

impl Budget {
    fn new(limit: usize, map_spans: usize) -> Self {
        Self {
            limit,
            map_spans,
            registry: RefCell::default(),
        }
    }
}

thread_local! {
    static SHARED: Rc<Budget> = Rc::new(Budget::new(MAX_BYTES, DEFAULT_MAX_SPANS));
}

struct Owner {
    id: u64,
    map: RefCell<HeightMap>,
    budget: Rc<Budget>,
    touched: Cell<Instant>,
    persistence: RefCell<Option<Persisted>>,
}

struct Persisted {
    key: Key,
    exported: u64,
    last_export: Instant,
}

impl Owner {
    fn export(&self, map: &HeightMap, force: bool) {
        let mut persistence = self.persistence.borrow_mut();
        let Some(state) = persistence.as_mut() else {
            return;
        };
        if state.exported == map.revision()
            || (!force && state.last_export.elapsed() < height_store::THROTTLE)
        {
            return;
        }
        let Some(store) = height_store::global() else {
            return;
        };
        let Some(permit) = store.reserve() else {
            return;
        };
        let mut key = state.key.clone();
        key.prefix = key.prefix.min(map.total_rows());
        let spans = map.cold_spans(key.prefix);
        state.exported = map.revision();
        state.last_export = Instant::now();
        if spans.iter().all(|span| span.kind == 0) {
            return;
        }
        store.submit(Snapshot::new(key, spans, permit));
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.export(&self.map.borrow(), true);
        if let Ok(mut registry) = self.budget.registry.try_borrow_mut() {
            registry.remove(self.id);
        }
    }
}

/// One coarse summary is fixed overhead of each open view. All additional
/// spans share the application-wide logical budget. This accounts retained
/// scalar metadata, not tree allocator overhead, transient edits, or RSS.
fn charged(map: &HeightMap) -> usize {
    map.accounted_bytes().saturating_sub(SPAN_BYTES)
}

impl Registry {
    fn remove(&mut self, id: u64) {
        if let Some(entry) = self.entries.remove(&id) {
            self.bytes -= entry.bytes;
        }
    }

    fn account(&mut self, id: u64, bytes: usize) {
        if let Some(entry) = self.entries.get_mut(&id) {
            self.bytes = self.bytes - entry.bytes + bytes;
            entry.bytes = bytes;
        }
    }

    fn make_room(&mut self, current: u64, additional: usize, limit: usize, now: Instant) {
        if self.bytes.saturating_add(additional) <= limit {
            return;
        }
        // Only pressure walks the weak registry. Ordinary offset/prefix reads
        // merely touch their owner's timestamp.
        let mut cold = Vec::new();
        let mut dead = Vec::new();
        for (&id, entry) in &self.entries {
            let Some(owner) = entry.owner.upgrade() else {
                dead.push(id);
                continue;
            };
            if id != current
                && entry.bytes > 0
                && now.saturating_duration_since(owner.touched.get()) >= IDLE
            {
                cold.push((owner.touched.get(), id));
            }
        }
        for id in dead {
            self.remove(id);
        }
        cold.sort_unstable();
        for (_, id) in cold {
            if self.bytes.saturating_add(additional) <= limit {
                break;
            }
            let Some(owner) = self.entries[&id].owner.upgrade() else {
                self.remove(id);
                continue;
            };
            let Ok(mut map) = owner.map.try_borrow_mut() else {
                continue;
            };
            map.compact(0..0, 1);
            self.account(id, charged(&map));
        }
    }
}

#[derive(Clone)]
pub(super) struct HeightCache(Rc<Owner>);

impl Default for HeightCache {
    fn default() -> Self {
        Self::new(0, f64::from(f32::from(super::UNKNOWN_ROW_HEIGHT)))
    }
}

impl HeightCache {
    pub fn new(total: usize, prior: f64) -> Self {
        SHARED.with(|budget| Self::in_budget(total, prior, budget.clone()))
    }

    fn in_budget(total: usize, prior: f64, budget: Rc<Budget>) -> Self {
        let id = {
            let mut registry = budget.registry.borrow_mut();
            registry.next_id += 1;
            registry.next_id
        };
        let owner = Rc::new(Owner {
            id,
            map: RefCell::new(HeightMap::with_limit(total, prior, budget.map_spans)),
            budget: budget.clone(),
            touched: Cell::new(Instant::now()),
            persistence: RefCell::default(),
        });
        budget.registry.borrow_mut().entries.insert(
            id,
            Entry {
                owner: Rc::downgrade(&owner),
                bytes: 0,
            },
        );
        Self(owner)
    }

    pub fn read<R>(&self, read: impl FnOnce(&HeightMap) -> R) -> R {
        self.0.touched.set(Instant::now());
        read(&self.0.map.borrow())
    }

    pub fn revision(&self) -> u64 {
        self.read(HeightMap::revision)
    }

    pub fn bind(&self, key: Key) -> Option<async_channel::Receiver<Option<Snapshot>>> {
        if self
            .0
            .persistence
            .borrow()
            .as_ref()
            .is_some_and(|state| state.key == key)
        {
            return None;
        }
        // Changing document/layout identity must never label the previous
        // geometry as belonging to a new revision. Invalidation happens first.
        *self.0.persistence.borrow_mut() = Some(Persisted {
            key: key.clone(),
            exported: u64::MAX,
            last_export: Instant::now(),
        });
        height_store::global()?.load(key)
    }

    pub fn unbind(&self) {
        *self.0.persistence.borrow_mut() = None;
    }

    pub fn import(&self, snapshot: Snapshot, total: usize, protected: Range<usize>) -> bool {
        if !height_store::available() {
            return false;
        }
        let valid = self.0.persistence.borrow().as_ref().is_some_and(|state| {
            state.key.identity == snapshot.key.identity
                && state.key.revision == snapshot.key.revision
                && state.key.layout == snapshot.key.layout
                && state.key.prefix >= snapshot.key.prefix
        });
        if !valid {
            return false;
        }
        self.mutate(protected.clone(), snapshot.spans.len(), |map, allowed| {
            map.resize(total);
            map.merge_cold_protected(&snapshot.spans, snapshot.key.prefix, allowed, protected);
        });
        true
    }

    pub fn flush_all() {
        SHARED.with(|budget| {
            let mut owners: Vec<_> = budget
                .registry
                .borrow()
                .entries
                .values()
                .filter_map(|entry| entry.owner.upgrade())
                .collect();
            owners.sort_unstable_by_key(|owner| std::cmp::Reverse(owner.touched.get()));
            for owner in owners {
                owner.export(&owner.map.borrow(), true);
            }
        });
    }

    /// Reserve a completed batch's worst-case growth before its tree edits.
    /// Point writes must pass `allowed_spans` to HeightMap::set_limited. Native
    /// resident measurements remain authoritative when detail cannot fit.
    pub fn mutate<R>(
        &self,
        protected: Range<usize>,
        reserve_spans: usize,
        update: impl FnOnce(&mut HeightMap, usize) -> R,
    ) -> R {
        let now = Instant::now();
        self.0.touched.set(now);
        let mut map = self.0.map.borrow_mut();
        let map_limit = self.0.budget.map_spans;
        let reserve = reserve_spans.min(map_limit - 1);
        if map.span_count().saturating_add(reserve) > map_limit {
            map.compact(protected.clone(), map_limit - reserve);
        }
        let allowed = {
            let mut registry = self.0.budget.registry.borrow_mut();
            registry.account(self.0.id, charged(&map));
            let wanted = map.span_count().saturating_add(reserve).min(map_limit);
            let additional = wanted
                .saturating_sub(map.span_count())
                .saturating_mul(SPAN_BYTES);
            registry.make_room(self.0.id, additional, self.0.budget.limit, now);
            let spare = self.0.budget.limit.saturating_sub(registry.bytes) / SPAN_BYTES;
            if spare < reserve && reserve <= 2 {
                // Structural changes must still work when every other view
                // is active. Drop precision, never the summed pixel extent.
                map.compact(0..0, 1);
                registry.account(self.0.id, charged(&map));
            }
            let spare = self.0.budget.limit.saturating_sub(registry.bytes) / SPAN_BYTES;
            let allowed = map
                .span_count()
                .saturating_add(reserve.min(spare))
                .clamp(1, map_limit);
            // A reservation is charged throughout the closure, including
            // nested cache work. Another view cannot take its allowance.
            registry.account(self.0.id, (allowed - 1) * SPAN_BYTES);
            allowed
        };
        let mut mutation = Mutation {
            owner: &self.0,
            map,
            allowed,
            protected,
        };
        update(&mut mutation.map, allowed)
    }
}

struct Mutation<'a> {
    owner: &'a Owner,
    map: RefMut<'a, HeightMap>,
    allowed: usize,
    protected: Range<usize>,
}

impl Drop for Mutation<'_> {
    fn drop(&mut self) {
        // resize/rebase have a fixed two-span structural working allowance.
        // Coarsen before publishing if that allowance exceeded a full pool.
        if self.map.span_count() > self.allowed {
            self.map.compact(self.protected.clone(), self.allowed);
            if self.map.span_count() > self.allowed {
                self.map.compact(0..0, 1);
            }
        }
        self.owner
            .budget
            .registry
            .borrow_mut()
            .account(self.owner.id, charged(&self.map));
        self.owner.export(&self.map, false);
    }
}

#[cfg(test)]
#[path = "height_cache/tests.rs"]
mod tests;
