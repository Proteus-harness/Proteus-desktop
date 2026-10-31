use super::{ChatScroller, FooterState, HistoryRange, Hold};
use gpui_kit::component::scroll::ScrollbarHandle as _;
use gpui_kit::{ListOffset, point, px, size};
use std::time::{Duration, Instant};

use super::jump::JumpStatus;

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

#[test]
fn wheel_down_releases_disclosure_room_at_the_natural_content_end() {
    let mut state = folded_tail(true);
    state.tail.hold(px(600.));
    state.hold = Some(Hold {
        rows: 0..1,
        until: Instant::now() + Duration::from_secs(1),
    });
    state.user_scroll(point(px(0.), px(30.)));
    assert!(state.hold.is_none());
    assert_eq!(state.tail.height(), px(600.));
    assert!(!state.following);
    state.user_scroll(point(px(0.), px(-30.)));
    assert_eq!(state.tail.height(), px(0.));
}

#[test]
fn feedback_updates_only_the_final_row_and_preserves_browsing_and_disclosure_state() {
    let mut state = folded_tail(true);
    state.user_scroll(point(px(0.), px(30.)));
    state.tail.hold(px(600.));
    let rows = state.row_count();
    let thinking = FooterState {
        thinking: true,
        receipt: None,
    };
    assert!(state.update_footer(thinking.clone()));
    assert_eq!(state.row_count(), rows);
    assert_eq!(state.list.item_count(), rows + 1);
    assert_eq!(state.tail.height(), px(600.));
    assert!(!state.following);
    assert!(state.wheel_up);
    // Animation redraws do not dirty list measurements again.
    state.dirty = false;
    assert!(!state.update_footer(thinking));
    assert!(!state.dirty);
    state.last_offset = Some(px(180.));
    assert!(state.measure_footer(px(32.)));
    assert!(state.last_offset.is_none());
    state.dirty = false;
    assert!(!state.measure_footer(px(32.)));
    assert!(!state.dirty);
    assert!(state.update_footer(FooterState::default()));
    assert!(state.measure_footer(px(0.)));
    assert_eq!(state.footer_height, px(0.));
    assert_eq!(state.tail.height(), px(600.));
    assert_eq!(state.row_count(), rows);
}

#[test]
fn receipt_identity_and_status_invalidate_the_local_tail_without_transcript_splices() {
    let mut state = folded_tail(true);
    let receipt = |id: &str, status| FooterState {
        thinking: false,
        receipt: Some((id.into(), status, 0)),
    };
    assert!(state.update_footer(receipt("send-1", "Отправляю")));
    assert!(state.update_footer(receipt("send-1", "Отправлено")));
    assert!(state.update_footer(receipt("send-2", "Отправляю")));
    assert!(!state.update_footer(receipt("send-2", "Отправляю")));
    assert_eq!(state.row_count(), 50);
    assert!(state.following);
    assert_eq!(state.tail.height(), px(0.));
}

#[test]
fn natural_footer_consumes_disclosure_floor() {
    let mut state = folded_tail(true);
    assert!(state.update_footer(FooterState {
        thinking: true,
        receipt: None,
    }));
    assert!(state.measure_footer(px(32.)));
    state.tail.hold(px(32.));
    state.following = false;
    state.wheel_up = false;
    let rows = state.row_count();
    // A layout with the same 32px footer fully covers the retained minimum.
    // No content disappears and no extra empty room remains at the bottom.
    assert!(state.measure_footer(px(32.)));
    assert_eq!(state.tail.height(), px(0.));
    assert!(!state.following);
    state.step(Instant::now());
    assert!(state.following);
    assert_eq!(state.row_count(), rows);
    assert_eq!(state.footer_height, px(32.));
}

#[test]
fn footer_state_changes_cannot_consume_the_floor_using_a_stale_measurement() {
    let mut state = folded_tail(true);
    assert!(state.measure_footer(px(64.)));
    state.tail.hold(px(32.));
    assert!(state.update_footer(FooterState {
        thinking: true,
        receipt: None,
    }));
    assert_eq!(state.tail.height(), px(32.));
    // Only a fresh measurement is authoritative. The replacement caption
    // is shorter than the retained minimum and still needs the reservation.
    assert!(state.measure_footer(px(16.)));
    assert_eq!(state.tail.height(), px(32.));
}

#[test]
fn jump_status_starts_active_on_first_scroll_during_an_existing_run() {
    let at_tail = JumpStatus::new(true, false, false);
    let first_scroll = JumpStatus::new(true, false, true);
    assert!(!at_tail.wave());
    assert!(first_scroll.wave());
    assert_ne!(at_tail, first_scroll);
}

#[test]
fn returning_to_tail_and_back_renews_the_jump_animation_without_new_text() {
    let before = JumpStatus::new(true, false, true);
    let hidden = JumpStatus::new(true, false, false);
    let remounted = JumpStatus::new(true, false, true);
    // Both changes must invalidate the retained control even though neither
    // execution status nor transcript content changed in the meantime.
    assert_ne!(before, hidden);
    assert_ne!(hidden, remounted);
    assert!(before.wave());
    assert!(!hidden.wave());
    assert!(remounted.wave());
    assert_eq!(before, remounted);
}

#[test]
fn thinking_footer_drives_jump_status_without_execution_or_assistant_deltas() {
    let mut state = folded_tail(true);
    let rows = state.row_count();
    state.update_footer(FooterState {
        thinking: true,
        receipt: None,
    });
    assert!(JumpStatus::new(false, state.footer.thinking, true).wave());
    assert_eq!(state.row_count(), rows);
    state.dirty = false;
    // Sampling the animated control does not invalidate list measurements.
    assert!(JumpStatus::new(false, state.footer.thinking, true).wave());
    assert!(!state.dirty);
}

#[test]
fn finishing_while_jump_is_hidden_does_not_revive_old_activity_on_remount() {
    let running = JumpStatus::new(true, false, true);
    let finished_hidden = JumpStatus::new(false, false, false);
    let finished_visible = JumpStatus::new(false, false, true);
    assert_ne!(running, finished_hidden);
    assert_ne!(finished_hidden, finished_visible);
    assert!(!finished_visible.wave());
    // An ordinary receipt is not a running/thinking signal.
    let mut state = folded_tail(true);
    state.update_footer(FooterState {
        thinking: false,
        receipt: Some(("send-1".into(), "Отправлено", 0)),
    });
    assert!(!JumpStatus::new(false, state.footer.thinking, true).wave());
}

#[test]
fn reading_space_shares_the_tail_floor_and_does_not_count_as_status() {
    let mut state = folded_tail(true);
    let count = state.list.item_count();
    assert!(state.set_reading_gap(px(24.)));
    assert!(!state.set_reading_gap(px(24.)));
    assert_eq!(state.footer_height, px(0.));
    assert_eq!(state.trailing_blank(), px(24.));
    state.tail.hold(px(24.));
    state.following = false;
    state.wheel_up = false;
    // Natural reading space consumes this whole reservation: it must not
    // create a second 24px empty tail or keep following paused.
    assert!(state.measure_footer(px(0.)));
    assert_eq!(state.tail.height(), px(0.));
    state.step(Instant::now());
    assert!(state.following);
    assert_eq!(state.list.item_count(), count);
}

#[test]
fn visible_footer_and_reading_space_consume_one_minimum_without_hiding_status() {
    let mut state = folded_tail(true);
    state.set_reading_gap(px(24.));
    state.update_footer(FooterState {
        thinking: true,
        receipt: None,
    });
    state.measure_footer(px(32.));
    state.tail.hold(px(80.));
    // The fallback sees 32px of real status, not an 80px blank reservation.
    assert_eq!(state.trailing_blank(), px(48.));
    assert!(!state.measure_footer(px(32.)));
    assert_eq!(state.tail.height(), px(80.));
    state.tail.clear();
    state.tail.hold(px(56.));
    assert!(state.measure_footer(px(32.)));
    assert_eq!(state.tail.height(), px(0.));
    assert_eq!(state.footer_height, px(32.));
    assert_eq!(state.trailing_blank(), px(24.));
}

#[test]
fn a_growing_viewport_reserves_room_before_layout_without_moving_a_paused_reader() {
    let mut state = folded_tail(true);
    state.following = false;
    state.wheel_up = true;
    state.list.scroll_to(ListOffset {
        item_ix: 20,
        offset_in_item: px(7.),
    });
    let top = state.list.logical_scroll_top();
    assert!(!state.keep_end(size(px(800.), px(400.))));
    // Completing a plan grows the viewport before the next list prepaint.
    assert!(state.keep_end(size(px(800.), px(480.))));
    assert_eq!(state.tail.height(), px(480.));
    assert_eq!(state.list.logical_scroll_top(), top);
    assert!(!state.following);
    assert!(state.wheel_up);
    assert!(state.dirty);
    // Layout cannot grow the floor again; explicit downward input releases it.
    assert!(!state.keep_end(size(px(800.), px(480.))));
    state.user_scroll(point(px(0.), px(-3.)));
    assert_eq!(state.tail.height(), px(0.));
}

#[test]
fn viewport_growth_while_following_and_viewport_shrink_do_not_reserve_a_tail() {
    let mut state = folded_tail(true);
    state.keep_end(size(px(800.), px(400.)));
    assert!(!state.keep_end(size(px(800.), px(480.))));
    assert_eq!(state.tail.height(), px(0.));
    state.following = false;
    assert!(!state.keep_end(size(px(800.), px(320.))));
    assert_eq!(state.tail.height(), px(0.));
}

#[test]
fn disappearing_status_preserves_its_measured_space_only_for_a_paused_reader() {
    let mut state = folded_tail(true);
    state.set_reading_gap(px(24.));
    let thinking = FooterState {
        thinking: true,
        receipt: None,
    };
    state.update_footer(thinking.clone());
    state.measure_footer(px(32.));
    state.following = false;
    state.update_footer(FooterState::default());
    assert_eq!(state.tail.height(), px(56.));
    state.measure_footer(px(0.));
    assert_eq!(state.tail.height(), px(56.));
    assert!(!state.following);
    state.user_scroll(point(px(0.), px(-3.)));
    assert_eq!(state.tail.height(), px(0.));
    state.following = true;
    state.update_footer(thinking);
    state.measure_footer(px(32.));
    state.update_footer(FooterState::default());
    assert_eq!(state.tail.height(), px(0.));
}
