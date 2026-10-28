use super::{HistoryRange, HistoryScrollbar, ScrollTarget};
use gpui_kit::{
    ListAlignment, ListOffset, ListState, component::scroll::ScrollbarHandle, point, px,
};

fn fixture() -> HistoryScrollbar {
    let list = ListState::new(51, ListAlignment::Top, px(400.));
    let bar = HistoryScrollbar::new(list, px(0.));
    bar.set_range(
        Some(HistoryRange {
            start: 950,
            rows: 50,
            total: 1000,
        }),
        false,
    );
    let mut state = bar.state.get();
    state.unit = Some(px(100.));
    bar.state.set(state);
    bar
}

#[test]
fn loading_and_evicting_older_pages_preserves_thumb_extent_and_global_anchor() {
    let bar = fixture();
    let extent = bar.content_size();
    let mut start = 950;
    let mut rows = 50;
    while start > 0 {
        bar.list.scroll_to(ListOffset {
            item_ix: 3,
            offset_in_item: px(0.),
        });
        let position = bar.offset();
        bar.list.splice(0..0, 50);
        start -= 50;
        rows += 50;
        if rows > 200 {
            bar.list.splice(200..rows, 0);
            rows = 200;
        }
        bar.set_range(
            Some(HistoryRange {
                start,
                rows,
                total: 1000,
            }),
            false,
        );
        assert_eq!(bar.content_size(), extent);
        assert_eq!(bar.offset(), position, "page starting at {start}");
        assert!(bar.list.item_count() <= 201);
    }
}

#[test]
fn reloading_newer_page_and_evicting_prefix_keeps_the_same_global_anchor() {
    let bar = fixture();
    bar.list.splice(0..0, 150);
    bar.set_range(
        Some(HistoryRange {
            start: 100,
            rows: 200,
            total: 1000,
        }),
        false,
    );
    bar.list.scroll_to(ListOffset {
        item_ix: 195,
        offset_in_item: px(0.),
    });
    let position = bar.offset();
    let extent = bar.content_size();
    bar.list.splice(200..200, 50);
    bar.list.splice(0..50, 0);
    bar.set_range(
        Some(HistoryRange {
            start: 150,
            rows: 200,
            total: 1000,
        }),
        false,
    );
    assert_eq!(bar.offset(), position);
    assert_eq!(bar.content_size(), extent);
}

#[test]
fn drag_addresses_the_whole_history_and_holds_position_until_the_page_arrives() {
    let bar = fixture();
    bar.set_offset(point(px(0.), px(-50025.)));
    let target = bar.take_target().unwrap();
    assert_eq!(
        target,
        ScrollTarget {
            row: 500,
            fraction: 0.25
        }
    );
    assert!(!bar.scroll_to(target));
    assert_eq!(bar.offset().y, px(-50025.));
    assert!(bar.take_target().is_none());
    bar.set_range(
        Some(HistoryRange {
            start: 475,
            rows: 50,
            total: 1000,
        }),
        true,
    );
    assert!(bar.scroll_to(target));
    assert_eq!(bar.list.logical_scroll_top().item_ix, 25);
    // The fractional row is not measured yet. A newer remote position must
    // supersede that local positioning as well as the previous HTTP request.
    bar.set_offset(point(px(0.), px(-10000.)));
    let newer = bar.take_target().unwrap();
    assert!(!bar.scroll_to(newer));
    assert!(bar.state.get().local.is_none());
    assert_eq!(bar.offset().y, px(-10000.));
}

#[test]
fn drag_freezes_the_extent_even_when_live_messages_arrive() {
    let bar = fixture();
    bar.start_drag();
    let extent = bar.content_size();
    bar.set_range(
        Some(HistoryRange {
            start: 950,
            rows: 51,
            total: 1001,
        }),
        false,
    );
    assert_eq!(bar.content_size(), extent);
    bar.end_drag();
    assert_eq!(bar.content_size().height, extent.height + px(100.));
}

#[test]
fn bottom_of_track_addresses_the_live_tail_not_the_loaded_page_end() {
    let bar = fixture();
    bar.set_range(
        Some(HistoryRange {
            start: 100,
            rows: 200,
            total: 1000,
        }),
        false,
    );
    bar.set_offset(point(px(0.), px(-100000.)));
    assert_eq!(
        bar.take_target(),
        Some(ScrollTarget {
            row: 1000,
            fraction: 0.
        })
    );
}

#[test]
fn unpaged_chat_uses_the_original_pixel_scroll_handle() {
    let bar = fixture();
    bar.set_range(None, false);
    assert_eq!(bar.content_size(), bar.list.content_size());
    assert_eq!(bar.offset(), bar.list.offset());
    bar.set_offset(point(px(0.), px(-25.)));
    assert!(bar.take_target().is_none());
}

#[test]
fn wheel_input_cancels_a_pending_remote_position() {
    let bar = fixture();
    bar.set_offset(point(px(0.), px(-50000.)));
    let target = bar.take_target().unwrap();
    assert!(!bar.scroll_to(target));
    assert!(bar.cancel_target());
    assert!(bar.take_target().is_none());
    assert!(!bar.state.get().waiting);
    assert!(bar.state.get().displayed.is_none());
}
