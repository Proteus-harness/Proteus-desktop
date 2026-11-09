use super::*;

fn cache(pool: &Rc<Budget>) -> HeightCache {
    HeightCache::in_budget(100, 100., pool.clone())
}

fn idle(cache: &HeightCache) {
    cache.0.touched.set(Instant::now() - IDLE * 2);
}

fn measure(cache: &HeightCache, rows: Range<usize>) {
    let reserve = rows.len() * 2 + 2;
    cache.mutate(rows.clone(), reserve, |map, allowed| {
        for row in rows.clone() {
            map.set_limited(row, 200. + row as f64, allowed, rows.clone());
        }
    });
}

fn assert_accounted(pool: &Budget, caches: &[&HeightCache]) {
    let actual: usize = caches.iter().map(|cache| cache.read(charged)).sum();
    assert_eq!(pool.registry.borrow().bytes, actual);
    assert!(actual <= pool.limit);
}

#[::core::prelude::v1::test]
fn shared_pressure_compacts_idle_maps_without_changing_their_height() {
    let pool = Rc::new(Budget::new(10 * SPAN_BYTES, 64));
    let a = cache(&pool);
    let b = cache(&pool);
    let c = cache(&pool);
    measure(&a, 20..23);
    measure(&b, 40..43);
    measure(&c, 60..63);
    assert_accounted(&pool, &[&a, &b, &c]);
    let before = a.read(|map| (map.total_height(), map.revision()));
    idle(&a);
    measure(&c, 60..64);
    a.read(|map| {
        assert_eq!(map.span_count(), 1);
        assert_eq!(map.total_height(), before.0);
        assert!(map.revision() > before.1);
    });
    assert_accounted(&pool, &[&a, &b, &c]);
}

#[::core::prelude::v1::test]
fn last_owner_releases_its_shared_charge() {
    let pool = Rc::new(Budget::new(10 * SPAN_BYTES, 64));
    let a = cache(&pool);
    measure(&a, 20..23);
    let clone = a.clone();
    let bytes = pool.registry.borrow().bytes;
    assert!(bytes > 0);
    drop(a);
    assert_eq!(pool.registry.borrow().bytes, bytes);
    drop(clone);
    assert_eq!(pool.registry.borrow().bytes, 0);
    assert!(pool.registry.borrow().entries.is_empty());
}

#[::core::prelude::v1::test]
fn per_map_pressure_keeps_the_current_window_and_both_prefix_sums() {
    let pool = Rc::new(Budget::new(100 * SPAN_BYTES, 16));
    let a = cache(&pool);
    a.mutate(0..100, 14, |map, allowed| {
        for row in (0..14).step_by(2) {
            map.set_limited(row, 200. + row as f64, allowed, 0..100);
        }
    });
    let protected = 4..8;
    let before = a.read(|map| {
        (
            map.total_height(),
            map.prefix(protected.start),
            map.prefix(protected.end),
            protected
                .clone()
                .map(|row| map.lookup(row))
                .collect::<Vec<_>>(),
            map.span_count(),
        )
    });
    a.mutate(protected.clone(), 6, |_, _| {});
    a.read(|map| {
        assert!(map.span_count() < before.4);
        assert_eq!(map.total_height(), before.0);
        assert_eq!(map.prefix(protected.start), before.1);
        assert_eq!(map.prefix(protected.end), before.2);
        assert_eq!(
            protected
                .clone()
                .map(|row| map.lookup(row))
                .collect::<Vec<_>>(),
            before.3
        );
    });
    assert_accounted(&pool, &[&a]);
}

#[::core::prelude::v1::test]
fn an_in_flight_batch_keeps_its_reservation_during_nested_cache_work() {
    let pool = Rc::new(Budget::new(6 * SPAN_BYTES, 64));
    let a = cache(&pool);
    let b = cache(&pool);
    a.mutate(10..21, 4, |map, allowed| {
        assert_eq!(allowed, 5);
        b.mutate(40..41, 4, |map, other_allowed| {
            assert_eq!(other_allowed, 3);
            map.set_limited(40, 500., other_allowed, 40..41);
        });
        assert_eq!(pool.registry.borrow().bytes, pool.limit);
        map.set_limited(10, 200., allowed, 10..21);
        map.set_limited(20, 300., allowed, 10..21);
    });
    assert_accounted(&pool, &[&a, &b]);
    assert_eq!(a.read(|map| map.lookup(20)), Some((300., true)));
    assert_eq!(b.read(|map| map.lookup(40)), Some((500., true)));
}

#[::core::prelude::v1::test]
fn structural_growth_in_a_full_active_pool_keeps_the_new_total() {
    let pool = Rc::new(Budget::new(2 * SPAN_BYTES, 64));
    let a = cache(&pool);
    let b = cache(&pool);
    a.mutate(10..11, 2, |map, allowed| {
        map.set_limited(10, 1000., allowed, 10..11);
    });
    let before = a.read(HeightMap::total_height);
    idle(&a);
    b.mutate(40..41, 2, |map, allowed| {
        map.set_limited(40, 500., allowed, 40..41);
    });
    assert_eq!(a.read(HeightMap::span_count), 1);
    a.mutate(0..0, 2, |map, allowed| {
        assert_eq!(allowed, 1);
        map.resize(120);
    });
    a.read(|map| {
        assert_eq!(map.total_rows(), 120);
        assert_eq!(map.total_height(), before + 2000.);
        assert_eq!(map.span_count(), 1);
    });
    assert_accounted(&pool, &[&a, &b]);
}
