use super::{ChatScroller, HistoryRange};
use gpui_kit::component::scroll::ScrollbarHandle as _;
use gpui_kit::{ListOffset, point, px};
use std::time::Instant;

fn folded_tail(autoscroll: bool) -> ChatScroller {
    let state = ChatScroller::new(autoscroll);
    state.list.splice(0..0, 50);
    state.list.scroll_to(ListOffset {
        item_ix: 0,
        offset_in_item: px(0.),
    });
    state.scrollbar.set_range(
        Some(HistoryRange {
            start: 166,
            rows: 50,
            total: 216,
        }),
        false,
    );
    state
}

#[test]
fn wheel_up_releases_a_short_resident_tail_without_any_pixel_movement() {
    let mut state = folded_tail(true);
    // Unmeasured/folded rows provide no local scroll room. Older history must
    // still become reachable, even on the first event after a page splice.
    assert_eq!(state.gap(), px(0.));
    assert!(state.last_offset.is_none());
    state.user_scroll(point(px(0.), px(3.)));
    for _ in 0..3 {
        state.step(Instant::now());
    }
    assert!(!state.following);
    assert!(state.visible_rows().start < 8);
}

#[test]
fn downward_wheel_at_the_end_resumes_following_after_manual_browsing() {
    let mut state = folded_tail(true);
    state.user_scroll(point(px(0.), px(3.)));
    state.step(Instant::now());
    assert!(!state.following);
    state.user_scroll(point(px(0.), px(-3.)));
    state.step(Instant::now());
    assert!(state.following);
}

#[test]
fn wheel_after_a_seek_cancels_positioning_but_horizontal_child_input_does_not() {
    let mut state = folded_tail(true);
    state.scrollbar.set_offset(point(px(0.), px(-30.)));
    let target = state.scrollbar.take_target().unwrap();
    assert!(!state.scrollbar.scroll_to(target));
    assert_eq!(state.user_scroll(point(px(20.), px(1.))), None);
    assert_eq!(state.user_scroll(point(px(0.), px(3.))), Some(true));
    assert_eq!(state.user_scroll(point(px(0.), px(3.))), Some(false));
    state.step(Instant::now());
    assert!(!state.following);
}

#[test]
fn another_upward_wheel_can_retry_an_older_page_at_the_same_edge() {
    let mut state = folded_tail(true);
    state.history_edges = (true, false);
    state.user_scroll(point(px(0.), px(3.)));
    assert!(!state.history_edges.0);
    assert!(!state.following);
}

#[test]
fn wheel_down_respects_disabled_automatic_following() {
    let mut state = folded_tail(false);
    state.user_scroll(point(px(0.), px(3.)));
    state.user_scroll(point(px(0.), px(-3.)));
    state.step(Instant::now());
    assert!(!state.following);
}
