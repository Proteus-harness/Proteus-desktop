use super::*;

fn check(map: &HeightMap, oracle: &[Option<(f64, bool)>]) {
    assert_eq!(map.total_rows(), oracle.len());
    assert_eq!(map.measured_rows(), oracle.iter().flatten().count());
    let mut prefix = 0.;
    for (row, expected) in oracle.iter().enumerate() {
        assert!((map.prefix(row) - prefix).abs() < 0.00001);
        assert_eq!(map.lookup(row), *expected);
        let height = expected.map_or(map.prior, |(height, _)| height);
        assert_eq!(map.estimate(row), Some(height));
        if height > 0. {
            let pixel = prefix + height * 0.37;
            let (target, fraction) = map.target(pixel).unwrap();
            assert_eq!(target, row, "inverse at pixel {pixel}");
            assert!((fraction - 0.37).abs() < 0.00001);
        }
        prefix += height;
    }
    assert!((map.total_height() - prefix).abs() < 0.00001);
    assert!((map.prefix(usize::MAX) - prefix).abs() < 0.00001);
    assert_eq!(map.lookup(oracle.len()), None);
    assert_eq!(map.estimate(oracle.len()), None);
    if !oracle.is_empty() {
        assert_eq!(map.target(prefix), Some((oracle.len(), 0.)));
    }
    let spans: Vec<_> = map.tree.iter().copied().collect();
    assert!(spans.iter().all(|span| span.rows > 0));
    assert!(spans.windows(2).all(|pair| !pair[0].joins(pair[1])));
    assert_eq!(map.span_count(), spans.len());
}

#[test]
fn sparse_updates_match_a_dense_layout_through_rebases_and_invalidations() {
    let mut map = HeightMap::with_limit(64, 100., 4096);
    let mut oracle = vec![None; 64];
    let mut random = 0x8a37_241d_u64;
    for _ in 0..3000 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let value = (random >> 24) as usize;
        match value % 5 {
            0 | 1 if !oracle.is_empty() => {
                let row = (value / 5) % oracle.len();
                let height = [0., 1.25, 36., 100., 5000.][(value / 31) % 5];
                let expected = Some((height, true));
                assert_eq!(map.set(row, height), oracle[row] != expected);
                oracle[row] = expected;
            }
            2 => {
                let a = (value / 5) % (oracle.len() + 1);
                let b = (value / 137) % (oracle.len() + 1);
                let range = a.min(b)..a.max(b);
                map.invalidate(range.clone());
                for (_, fresh) in oracle[range].iter_mut().flatten() {
                    *fresh = false;
                }
            }
            3 => {
                let total = value / 5 % 128;
                map.resize(total);
                oracle.resize(total, None);
            }
            _ => {
                let total = value / 5 % 128;
                let prefix = value / 127 % (oracle.len() + 1);
                map.rebase(prefix, total);
                oracle.truncate(prefix.min(total));
                oracle.resize(total, None);
            }
        }
        check(&map, &oracle);
    }
}

#[test]
fn a_gigantic_unknown_history_needs_one_span_and_measuring_does_not_reestimate_neighbors() {
    let total = 1_000_000_000;
    let mut map = HeightMap::new(total, 80.);
    assert_eq!(map.span_count(), 1);
    assert_eq!(map.accounted_bytes(), SPAN_BYTES);
    let before = map.prefix(700_000_000);
    assert!(map.set(20, 50_000.));
    assert_eq!(map.span_count(), 3);
    assert_eq!(map.estimate(19), Some(80.));
    assert_eq!(map.estimate(21), Some(80.));
    assert_eq!(map.prefix(700_000_000) - before, 50_000. - 80.);
    assert_eq!(map.prefix(20), 20. * 80.);
    assert_eq!(map.target(20. * 80. + 25_000.), Some((20, 0.5)));
    map.resize(total + 1000);
    assert_eq!(map.lookup(20), Some((50_000., true)));
    assert_eq!(map.span_count(), 3);
}

#[test]
fn folded_runs_coalesce_and_inverse_skips_zero_height_members() {
    let mut map = HeightMap::new(10_000, 100.);
    for row in 0..10_000 {
        assert!(map.set(row, 0.));
        assert!(map.span_count() <= 2);
    }
    assert_eq!(map.span_count(), 1);
    assert_eq!(map.target(0.), Some((10_000, 0.)));
    assert!(map.set(5000, 40.));
    assert_eq!(map.target(0.), Some((5000, 0.)));
    assert_eq!(map.target(20.), Some((5000, 0.5)));
    assert_eq!(map.target(40.), Some((10_000, 0.)));
    map.invalidate(2000..7000);
    assert_eq!(map.total_height(), 40.);
    assert_eq!(map.lookup(5000), Some((40., false)));
    assert_eq!(map.lookup(1000), Some((0., true)));
}

#[test]
fn admission_does_not_discard_existing_estimates_or_allocate_the_new_fact() {
    let mut map = HeightMap::with_limit(10, 100., 3);
    assert!(map.set(5, 40.));
    assert_eq!(map.span_count(), 3);
    assert!(map.set(6, 40.));
    assert_eq!(map.span_count(), 3);
    let height = map.total_height();
    assert!(!map.set_limited(1, 5000., 3, 0..10));
    assert_eq!(map.lookup(1), None);
    assert_eq!(map.lookup(5), Some((40., true)));
    assert_eq!(map.total_height(), height);
    assert_eq!(map.accounted_bytes(), 3 * SPAN_BYTES);
}

#[test]
fn compaction_retains_extent_and_the_protected_reading_coordinates() {
    let mut map = HeightMap::new(300, 100.);
    for row in 0..300 {
        map.set(row, [0., 35., 5000., 80.][row % 4]);
    }
    let height = map.total_height();
    let prefix: Vec<_> = (120..=160).map(|row| map.prefix(row)).collect();
    let facts: Vec<_> = (120..160).map(|row| map.lookup(row)).collect();
    assert!(map.compact(120..160, 45));
    assert!(map.span_count() <= 42);
    assert_eq!(map.total_height(), height);
    assert_eq!(
        (120..=160).map(|row| map.prefix(row)).collect::<Vec<_>>(),
        prefix
    );
    assert_eq!(
        (120..160).map(|row| map.lookup(row)).collect::<Vec<_>>(),
        facts
    );
    assert_eq!(
        map.lookup(0),
        None,
        "a coarse span is not an exact row fact"
    );
    assert!(map.compact(0..0, 1));
    assert_eq!(map.span_count(), 1);
    assert_eq!(map.total_height(), height);
    let (row, fraction) = map.target(height * 0.37).unwrap();
    let projected = map.prefix(row) + map.estimate(row).unwrap() * fraction;
    assert!((projected - height * 0.37).abs() < 0.000001);
}

#[test]
fn a_bounded_sequential_walk_keeps_cold_aggregate_height_instead_of_forgetting_it() {
    let mut map = HeightMap::with_limit(1000, 100., 32);
    let mut expected = 1000. * 100.;
    for row in 0..1000 {
        let height = [0., 31., 81., 1000.][row % 4];
        expected += height - 100.;
        assert!(map.set_limited(row, height, 32, row.saturating_sub(10)..row + 1));
        assert!(map.span_count() <= 32);
        assert!((map.total_height() - expected).abs() < 0.000001);
        assert_eq!(map.lookup(row), Some((height, true)));
    }
}

#[test]
fn layout_reset_and_revision_rebase_drop_only_the_disallowed_facts() {
    let mut map = HeightMap::new(20, 100.);
    map.set(4, 5000.);
    map.set(10, 40.);
    map.set(19, 20.);
    map.rebase(10, 25);
    assert_eq!(map.lookup(4), Some((5000., true)));
    assert_eq!(map.lookup(10), None);
    assert_eq!(map.lookup(19), None);
    assert_eq!(map.estimate(24), Some(100.));
    let height = map.total_height();
    map.invalidate_all();
    assert_eq!(map.total_height(), height);
    assert_eq!(map.lookup(4), Some((5000., false)));
    map.reset(25, 60.);
    assert_eq!(map.span_count(), 1);
    assert_eq!(map.lookup(4), None);
    assert_eq!(map.total_height(), 25. * 60.);
}

#[test]
fn unchanged_facts_and_freshness_do_not_publish_a_new_revision() {
    let mut map = HeightMap::new(20, 100.);
    let empty = map.revision();
    map.resize(20);
    map.invalidate(0..20);
    map.invalidate_all();
    map.reset(20, 100.);
    assert_eq!(map.revision(), empty);
    assert!(map.set(4, 50.));
    let measured = map.revision();
    assert!(measured > empty);
    assert!(!map.set(4, 50.));
    map.invalidate(5..10);
    assert_eq!(map.revision(), measured);
    map.invalidate(4..5);
    let stale = map.revision();
    assert!(stale > measured);
    map.invalidate(4..5);
    assert_eq!(map.revision(), stale);
}

#[test]
fn warming_a_compacted_pair_preserves_mass_in_either_measurement_order() {
    for order in [[0, 1], [1, 0]] {
        let mut map = HeightMap::new(2, 100.);
        map.set(0, 20.);
        map.set(1, 80.);
        assert!(map.compact(0..0, 1));
        assert_eq!(map.total_height(), 100.);
        assert_eq!(map.estimate(0), Some(50.));
        for row in order {
            assert!(map.set(row, [20., 80.][row]));
            assert_eq!(map.total_height(), 100., "order={order:?}, row={row}");
        }
        assert_eq!(map.prefix(1), 20.);
        assert_eq!(map.target(60.), Some((1, 0.5)));
        assert_eq!(map.measured_rows(), 2);
    }
}

#[test]
fn refining_an_interior_coarse_row_distributes_only_the_retained_remainder() {
    let mut map = HeightMap::new(5, 100.);
    for (row, height) in [10., 20., 40., 50., 80.].into_iter().enumerate() {
        map.set(row, height);
    }
    map.compact(0..0, 1);
    assert!(map.set(2, 40.));
    assert_eq!(map.total_height(), 200.);
    assert_eq!(map.lookup(2), Some((40., true)));
    for row in [0, 1, 3, 4] {
        assert_eq!(map.estimate(row), Some(40.));
        assert_eq!(map.lookup(row), None);
    }
    assert_eq!(map.span_count(), 3);
}

#[test]
fn the_last_approximate_row_and_facts_exceeding_the_aggregate_can_change_extent() {
    let mut map = HeightMap::new(2, 100.);
    map.set(0, 20.);
    map.set(1, 80.);
    map.compact(0..0, 1);
    map.set(0, 20.);
    assert_eq!(map.total_height(), 100.);
    assert_eq!(map.estimate(1), Some(80.));
    map.set(1, 90.);
    assert_eq!(
        map.total_height(),
        110.,
        "a genuine change to the last row is observable"
    );
    map.compact(0..0, 1);
    map.set(0, 150.);
    assert_eq!(
        map.total_height(),
        150.,
        "the new fact exceeds all retained mass"
    );
    assert_eq!(map.estimate(1), Some(0.));
    map.set(1, 90.);
    assert_eq!(map.total_height(), 240.);
}
