//! Real list measurements while traversing and evicting heterogeneous pages.
use super::*;
use ::gpui::{TestAppContext, VisualTestContext};

fn height(row: usize) -> Pixels {
    px(match row % 5 {
        0 => 70.,
        1 => 55.,
        2 | 3 => 0., // Hidden members of folded tool groups.
        _ if (200..400).contains(&row) => 1800.,
        _ => 130.,
    })
}

struct Rows {
    bar: HistoryScrollbar,
    range: HistoryRange,
    long_tools: bool,
}

impl Render for Rows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let range = self.range;
        let long_tools = self.long_tools;
        let measured = self.bar.clone();
        list(self.bar.list.clone(), move |ix, _, _| {
            measured.measure_row(
                ix,
                div()
                    .h(if ix < range.rows {
                        row_height(range.start + ix, range, long_tools)
                    } else {
                        px(24.)
                    })
                    .w_full(),
            )
        })
        .w_full()
        .h_full()
    }
}

fn draw(cx: &mut VisualTestContext, bar: &HistoryScrollbar, range: HistoryRange) {
    draw_with_tools(cx, bar, range, false);
}

fn row_height(row: usize, range: HistoryRange, long_tools: bool) -> Pixels {
    if long_tools && (200..500).contains(&row) {
        if row == range.start.max(200) {
            px(32.)
        } else {
            px(0.)
        }
    } else {
        height(row)
    }
}

fn draw_with_tools(
    cx: &mut VisualTestContext,
    bar: &HistoryScrollbar,
    range: HistoryRange,
    long_tools: bool,
) {
    bar.set_viewport_width(px(300.));
    let view = cx.update(|_, cx| {
        cx.new(|_| Rows {
            bar: bar.clone(),
            range,
            long_tools,
        })
    });
    cx.draw(point(px(0.), px(0.)), size(px(300.), px(700.)), |_, _| {
        view.into_any_element()
    });
    bar.did_layout();
}

#[::core::prelude::v1::test]
fn loading_older_members_of_a_folded_run_keeps_the_native_reading_position() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let mut range = HistoryRange {
        start: 550,
        rows: 50,
        total: 600,
    };
    let bar = HistoryScrollbar::new(ListState::new(51, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(Some(range), false);
    bar.list.scroll_to_end();
    draw_with_tools(cx, &bar, range, true);
    // Match ChatScroller::user_scroll: page facts may refine the estimated
    // track, while the content itself keeps its native reading anchor.
    bar.begin_browsing();
    for step in 0..500 {
        bar.list.scroll_by(px(-120.));
        draw_with_tools(cx, &bar, range, true);
        let _ = bar.offset();
        if bar.list.logical_scroll_top().item_ix < 8 && range.start > 0 {
            let top = bar.list.logical_scroll_top();
            let row = range.start + top.item_ix;
            bar.list.splice(0..0, 50);
            range.start -= 50;
            range.rows += 50;
            // Keep this synthetic folded run together to isolate density
            // changes from the separate open-chain lifecycle during eviction.
            bar.set_range(Some(range), false);
            draw_with_tools(cx, &bar, range, true);
            assert!(
                range.start + bar.list.logical_scroll_top().item_ix == row
                    && (bar.list.logical_scroll_top().offset_in_item - top.offset_in_item).abs()
                        < px(0.02),
                "page {} after step {step}, old top {top:?}, new top {:?}",
                range.start,
                bar.list.logical_scroll_top()
            );
            let geometry = bar.geometry.borrow();
            let top = bar.list.logical_scroll_top();
            let gain = geometry.position(ListOffset {
                offset_in_item: top.offset_in_item + px(1.),
                ..top
            }) - geometry.position(top);
            assert!(
                gain >= px(0.) && gain <= px(1.05),
                "page facts introduced a catch-up slope: {gain:?}"
            );
        }
        if range.start == 0 && bar.list.logical_scroll_top().item_ix == 0 {
            return;
        }
    }
    panic!("did not reach beginning");
}

pub(super) fn track_fraction(bar: &HistoryScrollbar) -> f32 {
    let max = bar.max_offset();
    if max > px(0.) {
        f32::from(-bar.offset().y / max)
    } else {
        0.
    }
}

fn global_pixel(bar: &HistoryScrollbar, range: HistoryRange) -> Pixels {
    let top = bar.list.logical_scroll_top();
    (0..range.start + top.item_ix)
        .map(height)
        .fold(px(0.), |a, b| a + b)
        + top.offset_in_item
}

struct UnpagedRows(HistoryScrollbar, usize);
impl Render for UnpagedRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let long_row = self.1;
        let measured = self.0.clone();
        list(self.0.list.clone(), move |ix, _, _| {
            measured.measure_row(
                ix,
                div()
                    .h(px(if ix == long_row { 50_000. } else { 100. }))
                    .w_full(),
            )
        })
        .w_full()
        .h_full()
    }
}

pub(super) fn paint_unpaged(cx: &mut VisualTestContext, bar: &HistoryScrollbar, long_row: usize) {
    bar.set_viewport_width(px(300.));
    let view = cx.update(|_, cx| cx.new(|_| UnpagedRows(bar.clone(), long_row)));
    cx.draw(point(px(0.), px(0.)), size(px(300.), px(700.)), |_, _| {
        view.into_any_element()
    });
    bar.did_layout();
}

struct ShortPrefixRows {
    bar: HistoryScrollbar,
    heights: [f32; 7],
}

impl Render for ShortPrefixRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let heights = self.heights;
        let rows = self.bar.state.get().range.unwrap().rows;
        let measured = self.bar.clone();
        list(self.bar.list.clone(), move |ix, _, _| {
            measured.measure_row(
                ix,
                div()
                    .w_full()
                    .h(px(if ix < rows { heights[ix] } else { 0. })),
            )
        })
        .size_full()
    }
}

pub(super) fn paint_short_prefix(
    cx: &mut VisualTestContext,
    bar: &HistoryScrollbar,
    heights: [f32; 7],
    viewport: Pixels,
) {
    bar.set_viewport_width(px(300.));
    let view = cx.update(|_, cx| {
        cx.new(|_| ShortPrefixRows {
            bar: bar.clone(),
            heights,
        })
    });
    cx.draw(point(px(0.), px(0.)), size(px(300.), viewport), |_, _| {
        view.into_any_element()
    });
    bar.did_layout();
}

fn assert_uniform_upward_motion_across_short_prefix(prefix_height: f32) {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let heights = [
        100.,
        prefix_height,
        prefix_height,
        prefix_height,
        100.,
        100.,
        100.,
    ];
    let bar = HistoryScrollbar::new(ListState::new(8, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(
        Some(HistoryRange {
            start: 0,
            rows: 7,
            total: 7,
        }),
        false,
    );
    let mut first_layout = true;
    let mut draw = || {
        bar.set_viewport_width(px(300.));
        let view = cx.update(|_, cx| {
            cx.new(|_| ShortPrefixRows {
                bar: bar.clone(),
                heights,
            })
        });
        cx.draw(point(px(0.), px(0.)), size(px(300.), px(100.)), |_, _| {
            view.into_any_element()
        });
        if first_layout {
            // GPUI knows this leading row, but intentionally exposes no item
            // bounds above the logical top. Provisional reads must not commit
            // the new measurements with the preceding frame's mapping.
            assert!(bar.list.bounds_for_item(1).is_none());
            assert_eq!(bar.geometry.borrow().measured_height(1), None);
            let _ = bar.offset();
            assert_eq!(bar.geometry.borrow().measured_height(1), None);
        }
        bar.did_layout();
        if first_layout {
            assert_eq!(
                bar.geometry.borrow().measured_height(1),
                Some(px(prefix_height))
            );
            first_layout = false;
        }
    };
    let physical = || {
        let top = bar.list.logical_scroll_top();
        px(heights.iter().take(top.item_ix).sum::<f32>()) + top.offset_in_item
    };
    bar.list.scroll_to_end();
    draw();
    bar.begin_browsing();
    let mut previous_pixel = physical();
    let mut previous_thumb = -bar.offset().y;
    let mut steps = Vec::new();
    for _ in 0..3 {
        bar.list.scroll_by(px(-100.));
        draw();
        let pixel = physical();
        let thumb = -bar.offset().y;
        assert!((previous_pixel - pixel - px(100.)).abs() < px(0.05));
        steps.push(f32::from(previous_thumb - thumb));
        previous_pixel = pixel;
        previous_thumb = thumb;
    }
    assert!(
        steps[0] > 0.,
        "thumb must follow upward physical movement: {steps:?}"
    );
    assert!(
        steps.iter().all(|step| (step - steps[0]).abs() < 0.1),
        "equal upward pixel steps must move the thumb equally across a measured short prefix: {steps:?}"
    );
}

#[::core::prelude::v1::test]
fn upward_thumb_does_not_accumulate_motion_across_folded_rows() {
    assert_uniform_upward_motion_across_short_prefix(0.);
}

#[::core::prelude::v1::test]
fn upward_thumb_does_not_accumulate_motion_across_short_positive_rows() {
    assert_uniform_upward_motion_across_short_prefix(20.);
}

#[::core::prelude::v1::test]
fn a_short_new_chat_still_gets_a_scrollbar_when_its_first_reply_grows() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let bar = HistoryScrollbar::new(ListState::new(2, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(
        Some(HistoryRange {
            start: 0,
            rows: 1,
            total: 1,
        }),
        false,
    );
    bar.list.scroll_to_end();
    paint_unpaged(cx, &bar, usize::MAX);
    assert_eq!(bar.content_size().height, px(700.));
    bar.begin_browsing(); // An upward gesture while everything fits must not freeze scale.
    bar.list.splice(1..1, 1);
    bar.set_range(
        Some(HistoryRange {
            start: 0,
            rows: 2,
            total: 2,
        }),
        false,
    );
    bar.list.scroll_to_end();
    paint_unpaged(cx, &bar, 1);
    assert!(bar.content_size().height > px(700.));
    assert!(bar.offset().y < px(0.));
}

#[::core::prelude::v1::test]
fn measuring_a_long_earlier_reply_refines_extent_without_moving_native_content() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let range = HistoryRange {
        start: 0,
        rows: 26,
        total: 26,
    };
    let bar = HistoryScrollbar::new(ListState::new(27, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(Some(range), false);
    bar.list.scroll_to_end();
    paint_unpaged(cx, &bar, 4);
    bar.begin_browsing();
    let extent = bar.content_size().height;
    let native_extent = bar.list.content_size().height;
    let physical = || {
        let top = bar.list.logical_scroll_top();
        px((0..top.item_ix)
            .map(|ix| if ix == 4 { 50_000. } else { 100. })
            .sum::<f32>())
            + top.offset_in_item
    };
    for _ in 0..20 {
        let before = physical();
        bar.list.scroll_by(px(-120.));
        paint_unpaged(cx, &bar, 4);
        assert!((before - physical() - px(120.)).abs() < px(0.02));
    }
    assert!(
        bar.content_size().height > extent + px(40_000.),
        "newly measured content must appear in the estimate instead of being compressed into the old extent"
    );
    assert!(
        bar.list.content_size().height > native_extent * 10.,
        "must actually measure the previously unknown long reply"
    );
}

#[::core::prelude::v1::test]
fn native_wheel_observer_cannot_reanchor_the_thumb_before_parent_replays_pixels() {
    use crate::ui::chat::scroller::wheel::WheelScroll;
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let mut range = HistoryRange {
        start: 550,
        rows: 50,
        total: 600,
    };
    let make = |cx: &mut VisualTestContext| {
        let bar = HistoryScrollbar::new(
            ListState::new(range.rows + 1, ListAlignment::Top, px(400.)),
            px(0.),
        );
        bar.set_range(Some(range), false);
        bar.list.scroll_to_end();
        draw(cx, &bar, range);
        let wheel = WheelScroll::default();
        wheel.painted.set(Some(bar.list.logical_scroll_top()));
        let _ = bar.offset();
        (bar, wheel)
    };
    let (control, mut control_wheel) = make(cx);
    let (observed, mut observed_wheel) = make(cx);
    let mut pages = 0;
    let mut largest_difference = px(0.);
    for burst in 0..70 {
        control.list.scroll_by(px(-1800.));
        observed.list.scroll_by(px(-1800.));
        // Kit's scrollbar bubble listener reads offset after the native list
        // moved, before ChatList's parent restores/replays that movement.
        let _ = observed.offset();
        control_wheel.push(px(1800.), &control.list);
        observed_wheel.push(px(1800.), &observed.list);
        for frame in 0..9 {
            control_wheel.step(&control.list, px(400.));
            observed_wheel.step(&observed.list, px(400.));
            draw(cx, &control, range);
            draw(cx, &observed, range);
            control_wheel
                .painted
                .set(Some(control.list.logical_scroll_top()));
            observed_wheel
                .painted
                .set(Some(observed.list.logical_scroll_top()));
            assert_eq!(
                control.list.logical_scroll_top(),
                observed.list.logical_scroll_top()
            );
            largest_difference =
                largest_difference.max((control.offset().y - observed.offset().y).abs());
            if control.list.logical_scroll_top().item_ix < 8 && range.start > 0 {
                range.start -= 50;
                range.rows = (range.rows + 50).min(200);
                for (bar, wheel) in [
                    (&control, &mut control_wheel),
                    (&observed, &mut observed_wheel),
                ] {
                    let old_rows = bar.list.item_count() - 1;
                    bar.list.splice(0..0, 50);
                    if old_rows + 50 > range.rows {
                        bar.list.splice(range.rows..old_rows + 50, 0);
                    }
                    bar.set_range(Some(range), false);
                    wheel.invalidate(&bar.list);
                    draw(cx, bar, range);
                    wheel.painted.set(Some(bar.list.logical_scroll_top()));
                }
                pages += 1;
                largest_difference =
                    largest_difference.max((control.offset().y - observed.offset().y).abs());
            }
            if range.start == 0 && control.list.logical_scroll_top().item_ix == 0 {
                assert!(
                    pages > 4,
                    "must exercise eviction, not just first-page loading"
                );
                assert!(
                    largest_difference < px(0.02),
                    "wheel offset queries caused {largest_difference:?} of thumb drift through {pages} pages (burst {burst}, frame {frame})"
                );
                return;
            }
        }
    }
    panic!("did not traverse the complete history");
}

#[::core::prelude::v1::test]
fn upward_traversal_keeps_content_and_thumb_steady_across_real_page_layouts() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let mut range = HistoryRange {
        start: 550,
        rows: 50,
        total: 600,
    };
    let bar = HistoryScrollbar::new(ListState::new(51, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(Some(range), false);
    bar.list.scroll_to_end();
    draw(cx, &bar, range);
    let mut previous = global_pixel(&bar, range);
    for step in 0..1500 {
        let thumb = track_fraction(&bar);
        // No input and no changed source: a fresh measurement must not move it.
        draw(cx, &bar, range);
        assert!(
            (track_fraction(&bar) - thumb).abs() < 0.00001,
            "idle frame {step}"
        );
        let top = bar.list.logical_scroll_top();
        let before = previous;
        let model = (
            bar.geometry.borrow().facts_revision(),
            bar.unit(),
            bar.max_offset(),
        );
        bar.list.scroll_by(px(-120.));
        draw(cx, &bar, range);
        let current = global_pixel(&bar, range);
        assert!(
            current <= previous + px(0.02),
            "upward input moved down at {step}"
        );
        assert!(
            previous - current <= px(120.02),
            "content skipped unmeasured rows at {step}: {previous:?} -> {current:?}"
        );
        previous = current;
        let moved_thumb = track_fraction(&bar);
        let geometry = bar.geometry.borrow();
        let motion = geometry.position(top) - geometry.position(bar.list.logical_scroll_top());
        assert!(
            (motion - (before - current)).abs() < px(0.1),
            "current height facts must map native upward pixels 1:1 at {step}: {motion:?} vs {:?}",
            before - current
        );
        if model == (geometry.facts_revision(), bar.unit(), bar.max_offset()) {
            assert!(
                moved_thumb <= thumb + 0.00001,
                "unchanged facts moved the thumb down on upward input at {step}: {thumb:?} -> {moved_thumb:?}"
            );
        }
        drop(geometry);
        if bar.list.logical_scroll_top().item_ix < 8 && range.start > 0 {
            let before_pixel = global_pixel(&bar, range);
            bar.list.splice(0..0, 50);
            range.start -= 50;
            range.rows += 50;
            if range.rows > 200 {
                bar.list.splice(200..range.rows, 0);
                range.rows = 200;
            }
            bar.set_range(Some(range), false);
            draw(cx, &bar, range);
            assert!(
                (global_pixel(&bar, range) - before_pixel).abs() < px(0.02),
                "reading anchor at page {}",
                range.start
            );
            let geometry = bar.geometry.borrow();
            let top = bar.list.logical_scroll_top();
            let gain = geometry.position(ListOffset {
                offset_in_item: top.offset_in_item + px(1.),
                ..top
            }) - geometry.position(top);
            assert!(
                gain >= px(0.) && gain <= px(1.05),
                "page {} introduced a catch-up slope: {gain:?}",
                range.start
            );
        }
        if range.start == 0 && bar.list.logical_scroll_top().item_ix == 0 {
            return;
        }
    }
    panic!("did not reach the beginning");
}
