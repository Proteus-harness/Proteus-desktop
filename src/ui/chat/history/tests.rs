use super::{CACHE_ROWS, Direction, History};
use crate::transcript::{Change, Transcript};
use proteus_contracts::app_protocol::AppTranscriptBase;
use proteus_contracts::app_protocol::{
    AppTranscriptCursor, AppTranscriptMessage, AppTranscriptPage,
};

fn rows(start: usize, end: usize) -> Vec<AppTranscriptMessage> {
    (start..end)
        .map(|i| AppTranscriptMessage {
            message_id: Some(uuid::Uuid::from_u128(i as u128 + 1)),
            phase: None,
            role: "user".into(),
            text: format!("row {i}"),
            images: vec![],
            tool: None,
            subagent: None,
            streaming: false,
        })
        .collect()
}
fn cursor(revision: &str, before: usize) -> AppTranscriptCursor {
    AppTranscriptCursor {
        revision: revision.into(),
        before,
        base: None,
    }
}
fn page(start: usize, end: usize) -> AppTranscriptPage {
    AppTranscriptPage {
        revision: "first".into(),
        start,
        total: 1000,
        messages: rows(start, end),
        base: None,
    }
}
fn fixture() -> (History, Transcript) {
    let mut history = History::default();
    let mut live = Transcript::default();
    history.apply(&mut live, rows(950, 1000), Some(cursor("first", 950)));
    (history, live)
}

#[test]
fn pages_move_both_ways_without_retaining_the_entire_visited_history() {
    let (mut history, live) = fixture();
    // Visit the whole fixture upward; all fetched data remains recoverable on disk.
    for end in (50..=950).rev().step_by(50) {
        let requested = history.request(Direction::Older, &live).unwrap();
        assert_eq!(requested.before, end);
        let displayed = history.displayed(&live);
        let anchor = displayed.entries()[3].key;
        history
            .accept_page(
                &live,
                Direction::Older,
                &requested,
                page(end - 50, end),
                3..6,
            )
            .unwrap();
        let displayed = history.displayed(&live);
        assert_eq!(displayed.entries()[53].key, anchor);
        assert!(displayed.entries().len() <= CACHE_ROWS);
        assert_eq!(live.entries().len(), 50);
    }
    assert!(history.request(Direction::Older, &live).is_none());
    assert_eq!(history.archive.as_ref().unwrap().start, 0);
    // The newer direction reloads evicted pages in their original order.
    while history.archive.as_ref().unwrap().end() < 950 {
        let requested = history.request(Direction::Newer, &live).unwrap();
        let start = history.archive.as_ref().unwrap().end();
        let len = history.displayed(&live).entries().len();
        let anchor = history.displayed(&live).entries()[len - 5].key;
        history
            .accept_page(
                &live,
                Direction::Newer,
                &requested,
                page(start, requested.before),
                len - 5..len - 2,
            )
            .unwrap();
        let displayed = history.displayed(&live);
        assert!(displayed.entries().iter().any(|row| row.key == anchor));
        assert!(displayed.entries().len() <= CACHE_ROWS);
        let start = history.archive.as_ref().unwrap().start;
        for (offset, row) in displayed.entries().iter().enumerate() {
            assert_eq!(&*row.text, format!("row {}", start + offset));
        }
    }
    let len = history.displayed(&live).entries().len();
    history.merge_tail(&live, len - 5..len);
    let archived = history.displayed(&live).entries();
    assert_eq!(&*archived.last().unwrap().text, "row 999");
    let anchor = archived.last().unwrap().key;
    let changes = history.latest(&live);
    assert!(!history.archived());
    assert_eq!(live.entries().last().unwrap().key, anchor);
    assert!(
        changes
            .iter()
            .all(|change| !matches!(change, Change::Spliced { removed: 200, .. }))
    );
}

#[test]
fn overlapping_live_tail_keeps_older_visible_rows_until_the_anchor_reaches_live() {
    let (mut history, live) = fixture();
    let requested = history.request(Direction::Older, &live).unwrap();
    history
        .accept_page(&live, Direction::Older, &requested, page(900, 950), 0..2)
        .unwrap();
    assert!(history.request(Direction::Newer, &live).is_none());
    let list = gpui_kit::ListState::new(100, gpui_kit::ListAlignment::Top, gpui_kit::px(400.));
    // The live rows already overlap, but row 949 is still on screen above a
    // short folded tail. A near-bottom prefetch must not evict that row.
    list.scroll_to(gpui_kit::ListOffset {
        item_ix: 49,
        offset_in_item: gpui_kit::px(19.),
    });
    let anchor = history.displayed(&live).entries()[49].key;
    assert!(history.merge_tail(&live, 49..100).is_empty());
    assert!(history.archived());
    assert_eq!(history.displayed(&live).entries()[49].key, anchor);
    assert_eq!(history.displayed(&live).entries().len(), 100);

    // Once the first visible row belongs to live, only the off-screen prefix
    // is removed. The native logical offset must keep its key and pixels.
    list.scroll_to(gpui_kit::ListOffset {
        item_ix: 50,
        offset_in_item: gpui_kit::px(7.),
    });
    let anchor = history.displayed(&live).entries()[50].key;
    for change in history.merge_tail(&live, 50..100) {
        if let Change::Spliced {
            start,
            removed,
            inserted,
        } = change
        {
            list.splice(start..start + removed, inserted);
        }
    }
    assert!(!history.archived());
    let offset = list.logical_scroll_top();
    assert_eq!(offset.item_ix, 0);
    assert_eq!(offset.offset_in_item, gpui_kit::px(7.));
    assert_eq!(live.entries()[offset.item_ix].key, anchor);
    assert_eq!(list.item_count(), live.entries().len());
}

#[test]
fn byte_budget_counts_tool_payloads_and_preserves_visible_rows() {
    let (mut history, live) = fixture();
    let requested = history.request(Direction::Older, &live).unwrap();
    let mut huge = page(900, 950);
    for row in &mut huge.messages {
        row.text = "a".repeat(256 * 1024);
    }
    history
        .accept_page(&live, Direction::Older, &requested, huge, 0..2)
        .unwrap();
    let window = history.archive.as_ref().unwrap();
    // This prefetched page itself exceeds the budget, so retain it plus the viewport.
    assert_eq!(window.transcript.entries().len(), 52);
    assert_eq!(&*window.transcript.entries()[50].text, "row 950");
}

#[test]
fn streaming_tail_stays_separate_and_verified_snapshots_preserve_archive() {
    let (mut history, mut live) = fixture();
    let requested = history.request(Direction::Older, &live).unwrap();
    history
        .accept_page(&live, Direction::Older, &requested, page(900, 950), 0..2)
        .unwrap();
    let anchor = history.displayed(&live).entries()[0].key;
    live.push_user(None, "new live message", vec![]);
    assert_eq!(history.displayed(&live).entries().len(), 100);
    assert_eq!(
        live.entries().last().unwrap().text.as_ref(),
        "new live message"
    );
    let mut next = cursor("second", 952);
    next.base = Some(AppTranscriptBase {
        revision: "first".into(),
        common_prefix: 1000,
    });
    let (_, changes) = history.apply(&mut live, rows(952, 1002), Some(next));
    assert!(changes.is_empty());
    assert_eq!(history.displayed(&live).entries()[0].key, anchor);
    assert_eq!(live.entries().len(), 50);
    assert!(
        history
            .accept_page(&live, Direction::Older, &requested, page(850, 900), 0..2)
            .is_none()
    );
    let mut rewrite = cursor("rewrite", 950);
    rewrite.base = Some(AppTranscriptBase {
        revision: "second".into(),
        common_prefix: 2,
    });
    let (_, changes) = history.apply(&mut live, rows(950, 1000), Some(rewrite));
    assert!(!changes.is_empty());
    assert!(!history.archived());
}

#[test]
fn returning_to_live_tail_keeps_gpui_logical_anchor_and_unique_row_keys() {
    let (mut history, mut live) = fixture();
    let requested = history.request(Direction::Older, &live).unwrap();
    history
        .accept_page(&live, Direction::Older, &requested, page(900, 950), 0..2)
        .unwrap();
    live.push_user(None, "new during browsing", vec![]);
    let old = history.displayed(&live).entries();
    assert!(
        !old.iter()
            .any(|row| row.key == live.entries().last().unwrap().key)
    );
    let list =
        gpui_kit::ListState::new(old.len(), gpui_kit::ListAlignment::Top, gpui_kit::px(400.));
    list.scroll_to(gpui_kit::ListOffset {
        item_ix: 75,
        offset_in_item: gpui_kit::px(19.),
    });
    let anchor = old[75].key;
    for change in history.latest(&live) {
        if let Change::Spliced {
            start,
            removed,
            inserted,
        } = change
        {
            list.splice(start..start + removed, inserted);
        }
    }
    let offset = list.logical_scroll_top();
    assert_eq!(offset.item_ix, 25);
    assert_eq!(offset.offset_in_item, gpui_kit::px(19.));
    assert_eq!(live.entries()[offset.item_ix].key, anchor);
    assert_eq!(list.item_count(), live.entries().len());
}

#[test]
fn snapshot_keeps_archived_prefix_when_live_progress_is_replaced() {
    let (mut history, mut live) = fixture();
    live.push_user(None, "pending progress", vec![]);
    let requested = history.request(Direction::Older, &live).unwrap();
    history
        .accept_page(&live, Direction::Older, &requested, page(900, 950), 0..2)
        .unwrap();
    let key = history.displayed(&live).entries()[0].key;
    let mut next = cursor("second", 952);
    next.base = Some(AppTranscriptBase {
        revision: "first".into(),
        common_prefix: 1000,
    });
    let (_, changes) = history.apply(&mut live, rows(952, 1002), Some(next));
    assert_eq!(
        changes,
        vec![Change::Spliced {
            start: 100,
            removed: 1,
            inserted: 0
        }]
    );
    assert_eq!(history.displayed(&live).entries()[0].key, key);
    assert_eq!(history.archive.as_ref().unwrap().end(), 1000);
}

#[test]
fn scrollbar_range_counts_unloaded_rows_and_live_progress() {
    let (mut history, mut live) = fixture();
    let initial = history.range(&live).unwrap();
    assert_eq!(
        (initial.start, initial.rows, initial.total),
        (950, 50, 1000)
    );
    let requested = history.request(Direction::Older, &live).unwrap();
    history
        .accept_page(&live, Direction::Older, &requested, page(900, 950), 0..2)
        .unwrap();
    let loaded = history.range(&live).unwrap();
    assert_eq!((loaded.start, loaded.rows, loaded.total), (900, 100, 1000));
    live.push_user(None, "new while browsing", vec![]);
    assert_eq!(history.range(&live).unwrap().total, 1001);
    assert_eq!(history.displayed(&live).entries().len(), 100);
}

#[test]
fn scrollbar_seek_loads_only_one_page_around_the_requested_message() {
    let (mut history, live) = fixture();
    history.seek = Some(super::ScrollTarget {
        row: 500,
        fraction: 0.25,
    });
    let requested = history.seek_cursor().unwrap();
    assert_eq!(requested.before, 525);
    let (_, target) = history
        .accept_seek(&live, &requested, page(475, 525))
        .unwrap();
    assert_eq!(target.row, 500);
    assert_eq!(target.fraction, 0.25);
    let range = history.range(&live).unwrap();
    assert_eq!((range.start, range.rows, range.total), (475, 50, 1000));
    assert_eq!(&*history.displayed(&live).entries()[25].text, "row 500");
    assert_eq!(live.entries().len(), 50);
    assert!(history.seek.is_none());
}

#[test]
fn in_flight_seek_can_satisfy_a_newer_pointer_position_only_inside_its_page() {
    let (mut history, live) = fixture();
    history.seek = Some(super::ScrollTarget {
        row: 500,
        fraction: 0.,
    });
    let requested = history.seek_cursor().unwrap();
    history.seek = Some(super::ScrollTarget {
        row: 100,
        fraction: 0.,
    });
    assert!(
        history
            .accept_seek(&live, &requested, page(475, 525))
            .is_none()
    );
    assert!(!history.archived());
    assert_eq!(history.seek.unwrap().row, 100);
    history.seek = Some(super::ScrollTarget {
        row: 510,
        fraction: 0.5,
    });
    let (_, target) = history
        .accept_seek(&live, &requested, page(475, 525))
        .unwrap();
    assert_eq!(target.row, 510);
    assert_eq!(target.fraction, 0.5);
}

#[test]
fn snapshot_invalidates_an_in_flight_scrollbar_seek() {
    let (mut history, mut live) = fixture();
    history.seek = Some(super::ScrollTarget {
        row: 500,
        fraction: 0.,
    });
    let requested = history.seek_cursor().unwrap();
    history.apply(&mut live, rows(951, 1001), Some(cursor("second", 951)));
    assert!(
        history
            .accept_seek(&live, &requested, page(475, 525))
            .is_none()
    );
    assert!(!history.archived());
    assert!(history.seek.is_none());
}

#[test]
fn returning_to_resident_data_invalidates_an_older_in_flight_seek() {
    let (mut history, live) = fixture();
    history.seek = Some(super::ScrollTarget {
        row: 500,
        fraction: 0.,
    });
    history.loading = true;
    let epoch = history.epoch;
    let requested = history.seek_cursor().unwrap();
    assert!(history.cancel_seek());
    assert_ne!(history.epoch, epoch);
    assert!(!history.loading);
    assert!(
        history
            .accept_seek(&live, &requested, page(475, 525))
            .is_none()
    );
    assert!(!history.archived());
}
