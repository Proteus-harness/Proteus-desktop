use super::super::height_map::HeightMap;
use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("build/verification/height-store")
                .join(uuid::Uuid::new_v4().to_string()),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn pool() -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(0))
}
fn key(id: &str) -> Key {
    Key {
        identity: hash(id),
        revision: hash("revision-1"),
        layout: hash("layout-1"),
        prefix: 10,
        base: None,
    }
}
fn snapshot(key: Key, pool: &Arc<AtomicUsize>) -> Snapshot {
    let mut map = HeightMap::new(12, 100.);
    for row in 0..10 {
        map.set(row, (row + 1) as f64 * 17.);
    }
    let prefix = key.prefix;
    Snapshot::new(key, map.cold_spans(prefix), reserve(pool).unwrap())
}

#[test]
fn real_disk_reopen_restores_heterogeneous_prefix_without_native_authority() {
    let fixture = Fixture::new();
    let pool = pool();
    let key = key("session");
    let source = snapshot(key.clone(), &pool);
    assert!(disk::save(&fixture.0, &source, MAX_DISK_BYTES, MAX_FILES).unwrap());
    let loaded = disk::load(&fixture.0, key, reserve(&pool).unwrap())
        .unwrap()
        .unwrap();
    let mut map = HeightMap::new(12, 100.);
    // A native measurement that completed while disk was busy wins.
    map.set(4, 800.);
    map.merge_cold(&loaded.spans, loaded.key.prefix, MAX_SPANS);
    assert_eq!(map.lookup(0), Some((17., false)));
    assert_eq!(map.lookup(4), Some((800., true)));
    assert_eq!(map.lookup(9), Some((170., false)));
    assert_eq!(map.lookup(10), None); // mutable tail was never persisted
    assert_eq!(map.prefix(4), 170.);
    assert_eq!(map.target(8.5), Some((0, 0.5)));
    assert_eq!(map.total_height(), 17. * 55. - 85. + 800. + 200.);
    drop(loaded);
    drop(source);
    assert_eq!(pool.load(Ordering::Relaxed), 0);
}

#[test]
fn wrong_session_layout_or_unproven_revision_is_a_miss_and_base_keeps_only_proven_prefix() {
    let fixture = Fixture::new();
    let pool = pool();
    let original = key("session");
    disk::save(
        &fixture.0,
        &snapshot(original.clone(), &pool),
        MAX_DISK_BYTES,
        MAX_FILES,
    )
    .unwrap();
    let mut requested = original.clone();
    requested.identity = hash("other-session");
    assert!(
        disk::load(&fixture.0, requested, reserve(&pool).unwrap())
            .unwrap()
            .is_none()
    );
    let mut requested = original.clone();
    requested.layout = hash("new-width-font-or-renderer");
    assert!(
        disk::load(&fixture.0, requested, reserve(&pool).unwrap())
            .unwrap()
            .is_none()
    );
    let mut requested = original.clone();
    requested.revision = hash("revision-2");
    assert!(
        disk::load(&fixture.0, requested.clone(), reserve(&pool).unwrap())
            .unwrap()
            .is_none()
    );
    requested.base = Some((original.revision, 4));
    let loaded = disk::load(&fixture.0, requested, reserve(&pool).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(loaded.key.prefix, 4);
    let mut map = HeightMap::new(12, 100.);
    map.merge_cold(&loaded.spans, loaded.key.prefix, MAX_SPANS);
    assert_eq!(map.prefix(4), 170.);
    assert_eq!(map.lookup(4), None);
}

#[test]
fn freshness_only_warm_traversal_does_not_rewrite_disk() {
    let fixture = Fixture::new();
    let pool = pool();
    let key = key("session");
    let mut map = HeightMap::new(10, 100.);
    for row in 0..10 {
        map.set(row, 170.);
    }
    let before = map.cold_spans(10);
    let first = Snapshot::new(key.clone(), before.clone(), reserve(&pool).unwrap());
    assert!(disk::save(&fixture.0, &first, MAX_DISK_BYTES, MAX_FILES).unwrap());
    map.invalidate_all();
    for row in [1, 3, 5, 7, 9] {
        map.set(row, 170.);
    }
    // Native freshness may split runs. The persisted scalar geometry is identical.
    assert_eq!(map.cold_spans(10), before);
    let warm = Snapshot::new(key, map.cold_spans(10), reserve(&pool).unwrap());
    assert!(!disk::save(&fixture.0, &warm, MAX_DISK_BYTES, MAX_FILES).unwrap());
}

#[test]
fn corrupt_truncated_and_oversized_cache_cannot_restore_data() {
    let fixture = Fixture::new();
    let pool = pool();
    let key = key("session");
    disk::save(
        &fixture.0,
        &snapshot(key.clone(), &pool),
        MAX_DISK_BYTES,
        MAX_FILES,
    )
    .unwrap();
    let path = std::fs::read_dir(&fixture.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut bytes = std::fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&path, &bytes).unwrap();
    assert!(disk::load(&fixture.0, key.clone(), reserve(&pool).unwrap()).is_err());
    // Equal scalar output repairs corruption rather than trusting its header.
    assert!(
        disk::save(
            &fixture.0,
            &snapshot(key.clone(), &pool),
            MAX_DISK_BYTES,
            MAX_FILES
        )
        .unwrap()
    );
    assert!(
        disk::load(&fixture.0, key.clone(), reserve(&pool).unwrap())
            .unwrap()
            .is_some()
    );
    std::fs::write(&path, &bytes[..HEADER_BYTES - 1]).unwrap();
    assert!(disk::load(&fixture.0, key.clone(), reserve(&pool).unwrap()).is_err());
    std::fs::File::create(&path)
        .unwrap()
        .set_len(MAX_FILE_BYTES as u64 + 1)
        .unwrap();
    assert!(disk::load(&fixture.0, key, reserve(&pool).unwrap()).is_err());
}

#[test]
fn immediate_reopen_reads_the_pending_latest_snapshot_before_debounce() {
    let fixture = Fixture::new();
    let store = Store::start(fixture.0.clone(), None).unwrap();
    store.submit(snapshot(key("session"), &store.shared.snapshots));
    let receive = store.load(key("session")).unwrap();
    let loaded = receive.recv_blocking().unwrap().unwrap();
    assert_eq!(loaded.spans[9].pixels, 170.);
    store.shutdown(Duration::from_secs(3));
}

#[test]
fn local_disclosure_exclusions_survive_measurement_layout_reset_and_compaction() {
    let mut map = HeightMap::new(10, 100.);
    for row in 0..10 {
        map.set(row, (row + 1) as f64 * 17.);
    }
    map.exclude_disk(3..5);
    map.set(3, 900.);
    map.invalidate(3..5);
    map.set(4, 500.);
    assert_eq!(map.lookup(3), Some((900., false)));
    assert_eq!(map.lookup(4), Some((500., true)));
    let cold = map.cold_spans(10);
    let mut reopened = HeightMap::new(10, 100.);
    reopened.merge_cold(&cold, 10, MAX_SPANS);
    assert_eq!(reopened.lookup(2), Some((51., false)));
    assert_eq!(reopened.lookup(3), None);
    assert_eq!(reopened.estimate(3), Some(100.));
    assert_eq!(reopened.lookup(5), Some((102., false)));
    map.reset(10, 120.);
    map.set(3, 950.);
    let cold = map.cold_spans(10);
    reopened.reset(10, 120.);
    reopened.merge_cold(&cold, 10, MAX_SPANS);
    assert_eq!(reopened.lookup(3), None);
    assert_eq!(reopened.estimate(3), Some(120.));
    map.compact(0..0, 1);
    assert_eq!(map.span_count(), 1);
    assert!(map.cold_spans(10).iter().all(|span| span.kind == 0));
}

#[test]
fn cold_detail_can_replace_a_coarse_prior_under_pressure_without_losing_native_window() {
    let mut map = HeightMap::with_limit(100, 100., 8);
    map.set(4, 800.);
    map.compact(4..5, 1);
    let cold: Vec<_> = (0..8)
        .map(|row| ColdSpan {
            rows: 1,
            pixels: (row + 1) as f64 * 10.,
            kind: 1,
        })
        .collect();
    map.merge_cold_protected(&cold, 8, 8, 4..5);
    assert_eq!(map.lookup(4), Some((800., true)));
    assert_eq!(map.prefix(4), 100.);
    assert_eq!(map.total_height(), 360. - 50. + 800. + 9200.);
    assert!(map.span_count() <= 8);
}

#[test]
fn persisted_extension_preferences_change_the_layout_key_across_process_reopen() {
    let fixture = Fixture::new();
    let preferences = fixture.0.join("extension-preferences");
    std::fs::create_dir_all(&preferences).unwrap();
    let path = preferences.join("fixture-renderer.json");
    std::fs::write(&path, br#"{"wrap":true}"#).unwrap();
    let store = Store::start(fixture.0.clone(), Some(preferences.clone())).unwrap();
    store.submit(snapshot(key("session"), &store.shared.snapshots));
    assert!(
        store
            .load(key("session"))
            .unwrap()
            .recv_blocking()
            .unwrap()
            .is_some()
    );
    store.shutdown(Duration::from_secs(3));

    let unchanged = Store::start(fixture.0.clone(), Some(preferences.clone())).unwrap();
    assert!(
        unchanged
            .load(key("session"))
            .unwrap()
            .recv_blocking()
            .unwrap()
            .is_some()
    );
    unchanged.shutdown(Duration::from_secs(3));
    std::fs::write(&path, br#"{"wrap":false}"#).unwrap();
    let changed = Store::start(fixture.0.clone(), Some(preferences)).unwrap();
    assert!(
        changed
            .load(key("session"))
            .unwrap()
            .recv_blocking()
            .unwrap()
            .is_none()
    );
    changed.shutdown(Duration::from_secs(3));
}

#[test]
fn explicit_invalidation_bypasses_old_results_and_future_writes_then_clears_disk_in_order() {
    let fixture = Fixture::new();
    let store = Store::start(fixture.0.clone(), None).unwrap();
    store.submit(snapshot(key("session"), &store.shared.snapshots));
    let old = store
        .load(key("session"))
        .unwrap()
        .recv_blocking()
        .unwrap()
        .unwrap();
    store.invalidate();
    assert!(store.disabled()); // UI import rejects even already-delivered results
    assert!(store.load(key("session")).is_none());
    assert!(store.reserve().is_none());
    store.submit(snapshot(key("session"), &store.shared.snapshots));
    assert!(store.shared.state.lock().unwrap().writes.is_empty());
    store.shutdown(Duration::from_secs(3));
    assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 0);
    drop(old);
    let reopened = Store::start(fixture.0.clone(), None).unwrap();
    assert!(
        reopened
            .load(key("session"))
            .unwrap()
            .recv_blocking()
            .unwrap()
            .is_none()
    );
    reopened.shutdown(Duration::from_secs(3));
}

#[test]
fn unsafe_preference_byte_or_file_count_budget_bypasses_cache() {
    let fixture = Fixture::new();
    let preferences = fixture.0.join("extension-preferences");
    std::fs::create_dir_all(&preferences).unwrap();
    let oversized = preferences.join("large.json");
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(1024 * 1024 + 1)
        .unwrap();
    assert!(disk::preferences_fingerprint(Some(&preferences)).is_err());
    let store = Store::start(fixture.0.clone(), Some(preferences.clone())).unwrap();
    assert!(
        store
            .load(key("session"))
            .unwrap()
            .recv_blocking()
            .unwrap()
            .is_none()
    );
    assert!(store.disabled());
    store.shutdown(Duration::from_secs(3));
    std::fs::remove_file(oversized).unwrap();
    for index in 0..129 {
        std::fs::write(preferences.join(format!("{index}.json")), b"{}").unwrap();
    }
    assert!(disk::preferences_fingerprint(Some(&preferences)).is_err());
    let many = fixture.0.join("too-many-entries");
    std::fs::create_dir_all(&many).unwrap();
    for index in 0..513 {
        std::fs::write(many.join(format!("{index}.pending")), b"").unwrap();
    }
    assert!(disk::preferences_fingerprint(Some(&many)).is_err());
}

#[test]
fn explicit_invalidation_before_store_initialization_forbids_later_initialization_and_import() {
    // The same gate as global()/available(), without singleton test pollution.
    let invalidated = AtomicBool::new(false);
    assert!(process_available(&invalidated, None));
    invalidated.store(true, Ordering::Release);
    assert!(!process_available(&invalidated, None));
    let fixture = Fixture::new();
    let store = Store::start(fixture.0.clone(), None).unwrap();
    assert!(!process_available(&invalidated, Some(&store)));
    store.shutdown(Duration::from_secs(3));
}

#[test]
fn disk_budget_and_file_count_evict_before_allocation_and_large_entries_bypass() {
    let fixture = Fixture::new();
    let pool = pool();
    let entry_size = (HEADER_BYTES + 10 * SPAN_BYTES) as u64;
    let budget = entry_size * 3;
    for id in 0..20 {
        assert!(
            disk::save(
                &fixture.0,
                &snapshot(key(&id.to_string()), &pool),
                budget,
                3
            )
            .unwrap()
        );
        let metadata: Vec<_> = std::fs::read_dir(&fixture.0)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap())
            .collect();
        assert!(metadata.len() <= 3);
        assert!(metadata.iter().map(|entry| entry.len()).sum::<u64>() <= budget);
    }
    assert!(!disk::save(&fixture.0, &snapshot(key("too-large"), &pool), 1, 3).unwrap());
    let requested = key("19");
    assert!(
        disk::load(&fixture.0, requested, reserve(&pool).unwrap())
            .unwrap()
            .is_some()
    );
}

#[test]
fn queued_running_and_unread_snapshots_share_one_admission_limit() {
    let fixture = Fixture::new();
    let pool = pool();
    let leases: Vec<_> = (0..MAX_SNAPSHOTS)
        .map(|_| reserve(&pool).unwrap())
        .collect();
    assert!(reserve(&pool).is_none());
    drop(leases);
    assert!(reserve(&pool).is_some());
    let store = Store::start(fixture.0.clone(), None).unwrap();
    for count in 0..20 {
        let mut value = snapshot(key("session"), &store.shared.snapshots);
        value.spans[0].pixels = 17. + count as f64;
        store.submit(value);
        assert!(store.shared.state.lock().unwrap().writes.len() <= 1);
        assert!(store.shared.snapshots.load(Ordering::Relaxed) <= MAX_SNAPSHOTS);
    }
    store.shutdown(Duration::from_secs(3));
    let mut disk_key = key("session");
    disk_key.layout = disk_layout(
        disk_key.layout,
        disk::preferences_fingerprint(None).unwrap(),
    );
    let loaded = disk::load(&fixture.0, disk_key, reserve(&pool).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(loaded.spans[0].pixels, 36.);
    assert_eq!(store.shared.snapshots.load(Ordering::Relaxed), 0);
}
