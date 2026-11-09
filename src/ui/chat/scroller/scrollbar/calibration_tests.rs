//! Calibration with real list measurements and heterogeneous visible samples.
use super::layout_tests::{paint_short_prefix, paint_unpaged};
use super::*;
use ::gpui::{TestAppContext, VisualTestContext};

struct TallTailRows(HistoryScrollbar, HistoryRange);

impl Render for TallTailRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let range = self.1;
        let measured = self.0.clone();
        list(self.0.list.clone(), move |ix, _, _| {
            let row = range.start + ix;
            let height = if ix == range.rows {
                24.
            } else if row == 59 {
                2900. // Code fence ending just above two short messages.
            } else if row < 59 && matches!(row % 6, 3..=5) {
                0. // Folded tools in the earlier history.
            } else {
                70.
            };
            measured.measure_row(ix, div().h(px(height)).w_full())
        })
        .size_full()
    }
}

struct MixedRows(HistoryScrollbar);

fn mixed_height(ix: usize) -> f32 {
    if ix == 50 {
        return 0.;
    }
    match ix % 4 {
        0 => 180.,
        1 => 60.,
        2 => 960.,
        _ => 36.,
    }
}

impl Render for MixedRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let measured = self.0.clone();
        list(self.0.list.clone(), move |ix, _, _| {
            measured.measure_row(ix, div().h(px(mixed_height(ix))).w_full())
        })
        .size_full()
    }
}

#[::core::prelude::v1::test]
fn an_ordinary_clipped_row_keeps_wheel_and_drag_uniform_after_the_first_window_resize() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let range = HistoryRange {
        start: 4950,
        rows: 50,
        total: 5000,
    };
    let bar = HistoryScrollbar::new(ListState::new(51, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(Some(range), false);
    bar.list.scroll_to(ListOffset {
        item_ix: 3,
        offset_in_item: px(0.),
    });
    let paint = |cx: &mut VisualTestContext, height| {
        bar.set_viewport_width(px(300.));
        let view = cx.update(|_, cx| cx.new(|_| MixedRows(bar.clone())));
        cx.draw(point(px(0.), px(0.)), size(px(300.), height), |_, _| {
            view.into_any_element()
        });
        bar.did_layout();
    };
    // Match the requested native size, followed by compositor tiling.
    paint(cx, px(668.));
    bar.list.scroll_to(ListOffset {
        item_ix: 0,
        offset_in_item: px(0.),
    });
    paint(cx, px(856.));
    let mut previous = -bar.offset().y;
    let mut min = px(f32::INFINITY);
    let mut max = px(0.);
    for _ in 0..384 {
        bar.list.scroll_by(px(8.));
        paint(cx, px(856.));
        let current = -bar.offset().y;
        min = min.min(current - previous);
        max = max.max(current - previous);
        previous = current;
    }
    assert!(
        min > px(0.) && max <= min * 1.25 + px(0.25),
        "ordinary partially visible replies must not bias toward short rows: {min:?}..{max:?}"
    );

    let physical = || {
        let top = bar.list.logical_scroll_top();
        px((0..top.item_ix).map(mixed_height).sum::<f32>()) + top.offset_in_item
    };
    let drag_start = -bar.offset().y;
    bar.start_drag();
    let mut previous = physical();
    let mut min = px(f32::INFINITY);
    let mut max = px(0.);
    for step in 1..=96 {
        let requested = drag_start + px(step as f32 * 16.);
        bar.set_offset(point(px(0.), -requested));
        assert!(bar.scroll_to(bar.take_target().unwrap()));
        paint(cx, px(856.));
        bar.settle();
        paint(cx, px(856.));
        let current = physical();
        min = min.min(current - previous);
        max = max.max(current - previous);
        previous = current;
        assert!((-bar.offset().y - requested).abs() < px(0.5));
    }
    bar.end_drag();
    assert!(
        min > px(0.) && max <= min * 1.25 + px(0.25),
        "content speed changed across row heights while dragging: {min:?}..{max:?}"
    );
}

#[::core::prelude::v1::test]
fn a_clipped_tall_tail_does_not_leave_the_whole_thumb_motion_until_the_first_page() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let mut range = HistoryRange {
        start: 12,
        rows: 50,
        total: 62,
    };
    let bar = HistoryScrollbar::new(ListState::new(51, ListAlignment::Top, px(400.)), px(0.));
    let paint = |cx: &mut VisualTestContext, range| {
        bar.set_viewport_width(px(300.));
        let view = cx.update(|_, cx| cx.new(|_| TallTailRows(bar.clone(), range)));
        cx.draw(point(px(0.), px(0.)), size(px(300.), px(700.)), |_, _| {
            view.into_any_element()
        });
        bar.did_layout();
    };
    bar.set_range(Some(range), false);
    bar.list.scroll_to_end();
    paint(cx, range);
    assert_eq!(range.start + bar.list.logical_scroll_top().item_ix, 59);
    assert!(bar.list.logical_scroll_top().offset_in_item > px(2000.));
    bar.lock_unit();
    let max = -bar.offset().y;
    let mut previous = max;
    let mut largest_step = px(0.);
    for _ in 0..12 {
        // Equal wheel bursts, laid out in bounded animation steps.
        for _ in 0..6 {
            bar.list.scroll_by(px(-120.));
            paint(cx, range);
            if range.start > 0 && bar.list.logical_scroll_top().item_ix < 8 {
                let before = bar.offset().y;
                bar.list.splice(0..0, 12);
                range = HistoryRange {
                    start: 0,
                    rows: 62,
                    total: 62,
                };
                bar.set_range(Some(range), false);
                paint(cx, range);
                assert!((bar.offset().y - before).abs() < px(0.02));
            }
        }
        let current = -bar.offset().y;
        assert!(current <= previous + px(0.02));
        largest_step = largest_step.max(previous - current);
        previous = current;
        if current == px(0.) {
            break;
        }
    }
    assert_eq!(range.start, 0);
    assert_eq!(previous, px(0.), "must reach the actual beginning");
    assert!(
        largest_step < max * 0.3,
        "one equal burst consumed most of the track: {largest_step:?} / {max:?}"
    );
}

#[::core::prelude::v1::test]
fn learning_short_prefix_above_a_tall_tail_does_not_defer_upward_thumb_motion() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let rows = 72;
    let long_row = rows - 1;
    let bar = HistoryScrollbar::new(
        ListState::new(rows + 1, ListAlignment::Top, px(400.)),
        px(0.),
    );
    bar.set_range(
        Some(HistoryRange {
            start: 0,
            rows,
            total: rows,
        }),
        false,
    );
    bar.list.scroll_to_end();
    paint_unpaged(cx, &bar, long_row);
    bar.lock_unit();
    let physical = || {
        let top = bar.list.logical_scroll_top();
        px((0..top.item_ix)
            .map(|row| if row == long_row { 50_000. } else { 100. })
            .sum::<f32>())
            + top.offset_in_item
    };
    let mut previous_pixel = physical();
    let mut previous_thumb = -bar.offset().y;
    let mut smallest_step = f32::INFINITY;
    let mut largest_step = 0f32;
    let mut steps = 0;
    while previous_pixel >= px(120.) {
        bar.list.scroll_by(px(-120.));
        paint_unpaged(cx, &bar, long_row);
        let pixel = physical();
        let thumb = -bar.offset().y;
        assert!(
            (previous_pixel - pixel - px(120.)).abs() < px(0.1),
            "native content must consume equal 120px steps before comparing thumb speed"
        );
        let step = f32::from(previous_thumb - thumb);
        smallest_step = smallest_step.min(step);
        largest_step = largest_step.max(step);
        previous_pixel = pixel;
        previous_thumb = thumb;
        steps += 1;
        assert!(
            steps < 1000,
            "did not reach the start of the synthetic history"
        );
    }
    // Unknown history can refine its estimate, but it must not defer nearly
    // all track movement until a short prefix is finally measured. The exact
    // 120px native displacement above rules out smoothing by stalling content.
    assert!(
        smallest_step > 0. && largest_step <= smallest_step * 4.,
        "equal upward input accumulated a thumb jump while learning rows: {smallest_step}..{largest_step} over {steps} steps"
    );
}

#[::core::prelude::v1::test]
fn a_lone_clipped_reply_keeps_its_real_height_without_biasing_unknown_rows() {
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
    paint_unpaged(cx, &bar, 0);
    let extent = bar.content_size().height;
    assert!(
        extent >= px(50_000.),
        "the known long reply must remain fully scrollable"
    );
    assert!(
        bar.unit() <= bar.list.viewport_bounds().size.height,
        "a censored lone reply must not become the unknown-row mean"
    );
    bar.lock_unit();
    let mut previous = -bar.offset().y;
    for _ in 0..3 {
        bar.list.scroll_by(px(-120.));
        paint_unpaged(cx, &bar, 0);
        let current = -bar.offset().y;
        assert!((previous - current - px(120.)).abs() < px(0.1));
        assert_eq!(bar.content_size().height, extent);
        previous = current;
    }
}

#[::core::prelude::v1::test]
fn a_censored_viewport_uses_ordinary_cached_rows_before_the_default_prior() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let bar = HistoryScrollbar::new(ListState::new(8, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(
        Some(HistoryRange {
            start: 0,
            rows: 7,
            total: 7,
        }),
        false,
    );
    let heights = [70., 70., 70., 70., 70., 70., 50_000.];
    let mut paint = || paint_short_prefix(cx, &bar, heights, px(700.));
    paint();
    bar.list.scroll_to_end();
    paint();
    assert_eq!(bar.list.logical_scroll_top().item_ix, 6);
    assert_eq!(
        bar.unit(),
        px(70.),
        "previous ordinary rows are independent of viewport censoring"
    );
    assert_eq!(
        bar.content_size().height,
        px(50_420.),
        "the measured long row is counted exactly once"
    );
}

#[::core::prelude::v1::test]
fn paged_async_measurements_calibrate_extent_until_browsing_begins() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let bar = HistoryScrollbar::new(ListState::new(8, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(
        Some(HistoryRange {
            start: 100,
            rows: 7,
            total: 107,
        }),
        false,
    );
    bar.list.scroll_to_end();
    paint_short_prefix(cx, &bar, [70.; 7], px(700.));
    let placeholder_extent = bar.content_size().height;
    let mut heights = [70.; 7];
    heights[6] = 50_000.;
    bar.invalidate(6..7);
    bar.list.remeasure_items(6..7);
    paint_short_prefix(cx, &bar, heights, px(700.));
    let resolved_extent = bar.content_size().height;
    assert!(
        resolved_extent > placeholder_extent + px(40_000.),
        "paged scale must not freeze a renderer's initial placeholder height"
    );
    bar.lock_unit();
    heights[6] = 60_000.;
    bar.invalidate(6..7);
    bar.list.remeasure_items(6..7);
    paint_short_prefix(cx, &bar, heights, px(700.));
    assert_eq!(
        bar.content_size().height,
        resolved_extent,
        "actual browsing freezes the estimated extent until an explicit source replacement"
    );
}

#[::core::prelude::v1::test]
fn a_shorter_snapshot_drops_obsolete_correction_and_seek_but_keeps_native_anchor() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let bar = HistoryScrollbar::new(ListState::new(8, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(
        Some(HistoryRange {
            start: 0,
            rows: 7,
            total: 7,
        }),
        false,
    );
    bar.list.scroll_to_end();
    paint_short_prefix(cx, &bar, [100., 0., 0., 0., 100., 100., 100.], px(100.));
    assert_eq!(bar.content_size().height, px(400.));
    bar.start_drag();
    bar.set_offset(point(px(0.), px(-150.)));
    bar.list.splice(1..4, 0);
    bar.set_range(
        Some(HistoryRange {
            start: 0,
            rows: 4,
            total: 4,
        }),
        false,
    );
    paint_short_prefix(cx, &bar, [100.; 7], px(100.));
    assert_eq!(bar.list.logical_scroll_top().item_ix, 3);
    assert_eq!(
        bar.content_size().height,
        px(400.),
        "the removed hidden rows must not leave a negative extent correction"
    );
    assert!(bar.take_target().is_none());
    assert!(bar.state.get().displayed.is_none());
    assert!(bar.state.get().local.is_none());
    assert!(bar.state.get().drag_height.is_none());
    assert!(!bar.list.is_scrollbar_dragging());
}
