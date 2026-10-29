//! One UI-thread LRU owns bitmap Arcs; view-local maps only hold Weak values.
use std::collections::VecDeque;
use std::sync::{Arc, Weak};

pub(super) const MAX_CACHE_PIXELS: u64 = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 64;
const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;

struct Entry<K, V> {
    key: K,
    value: V,
    pixels: u64,
    source_bytes: usize,
    owners: Vec<Weak<()>>,
}

pub(super) struct Cache<K, V> {
    entries: VecDeque<Entry<K, V>>,
    pixels: u64,
    source_bytes: usize,
}

impl<K, V> Default for Cache<K, V> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            pixels: 0,
            source_bytes: 0,
        }
    }
}

impl<K: Eq, V: Clone> Cache<K, V> {
    fn prune(&mut self) {
        for entry in &mut self.entries {
            entry.owners.retain(|owner| owner.strong_count() > 0);
        }
        self.entries.retain(|entry| !entry.owners.is_empty());
        self.recount();
    }

    fn recount(&mut self) {
        self.pixels = self.entries.iter().map(|entry| entry.pixels).sum();
        self.source_bytes = self.entries.iter().map(|entry| entry.source_bytes).sum();
    }

    pub fn get(&mut self, key: &K, owner: &Arc<()>) -> Option<V> {
        self.prune();
        let position = self.entries.iter().position(|entry| &entry.key == key)?;
        let mut entry = self.entries.remove(position)?;
        if !entry
            .owners
            .iter()
            .any(|previous| previous.as_ptr() == Arc::as_ptr(owner))
        {
            entry.owners.push(Arc::downgrade(owner));
        }
        let value = entry.value.clone();
        self.entries.push_back(entry);
        Some(value)
    }

    /// Evict BEFORE the renderer allocates a new bitmap, not after insertion.
    pub fn reserve(&mut self, pixels: u64, source_bytes: usize) {
        self.prune();
        while !self.entries.is_empty()
            && (self.pixels + pixels > MAX_CACHE_PIXELS
                || self.source_bytes + source_bytes > MAX_SOURCE_BYTES
                || self.entries.len() >= MAX_ENTRIES)
        {
            let old = self.entries.pop_front().unwrap();
            self.pixels -= old.pixels;
            self.source_bytes -= old.source_bytes;
        }
    }

    pub fn insert(&mut self, key: K, value: V, pixels: u64, source_bytes: usize, owner: &Arc<()>) {
        if pixels > MAX_CACHE_PIXELS || source_bytes > MAX_SOURCE_BYTES {
            return;
        }
        self.reserve(pixels, source_bytes);
        self.entries.push_back(Entry {
            key,
            value,
            pixels,
            source_bytes,
            owners: vec![Arc::downgrade(owner)],
        });
        self.pixels += pixels;
        self.source_bytes += source_bytes;
    }

    pub fn retain_owner(&mut self, owner: &Arc<()>, keep: impl Fn(&K) -> bool) {
        for entry in &mut self.entries {
            if !keep(&entry.key) {
                entry
                    .owners
                    .retain(|previous| previous.as_ptr() != Arc::as_ptr(owner));
            }
        }
        self.prune();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lru_eviction_releases_the_old_bitmap_before_allocation() {
        let owner = Arc::new(());
        let mut cache = Cache::default();
        let images: Vec<_> = (0..4).map(|_| Arc::new(())).collect();
        for (index, image) in images.iter().enumerate() {
            cache.insert(index, image.clone(), 2 * 1024 * 1024, 1, &owner);
        }
        assert!(cache.get(&0, &owner).is_some()); // 1 is now least recently used.
        cache.reserve(2 * 1024 * 1024, 1);
        assert_eq!(Arc::strong_count(&images[1]), 1);
        assert_eq!(Arc::strong_count(&images[0]), 2);
        assert!(cache.pixels + 2 * 1024 * 1024 <= MAX_CACHE_PIXELS);
    }

    #[test]
    fn dpi_prune_keeps_other_view_owners_and_releases_last_owner() {
        let first = Arc::new(());
        let second = Arc::new(());
        let mut cache = Cache::default();
        cache.insert("old-dpi", 1, 100, 1, &first);
        assert_eq!(cache.get(&"old-dpi", &second), Some(1));
        cache.retain_owner(&first, |_| false);
        assert_eq!(cache.entries.len(), 1);
        cache.retain_owner(&second, |_| false);
        assert_eq!(cache.pixels, 0);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn tiny_bitmaps_cannot_grow_key_memory_or_entry_count_without_bound() {
        let owner = Arc::new(());
        let mut cache = Cache::default();
        for index in 0..200 {
            cache.insert(index, index, 1, 100_000, &owner);
        }
        assert!(cache.entries.len() <= MAX_ENTRIES);
        assert!(cache.source_bytes <= MAX_SOURCE_BYTES);
        drop(owner);
        cache.reserve(1, 1);
        assert!(cache.entries.is_empty());
    }
}
