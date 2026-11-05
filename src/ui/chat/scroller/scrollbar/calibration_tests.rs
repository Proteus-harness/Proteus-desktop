//! Calibration with real list measurements and heterogeneous visible samples.
use super::*;
use ::gpui::{TestAppContext, VisualTestContext};

struct TallTailRows(ListState, HistoryRange);

impl Render for TallTailRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let range = self.1;
        list(self.0.clone(), move |ix, _, _| {
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
            div().h(px(height)).w_full().into_any_element()
        })
        .size_full()
    }
}

struct MixedRows(ListState);

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
        list(self.0.clone(), move |ix, _, _| {
            div().h(px(mixed_height(ix))).w_full().into_any_element()
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
        let view = cx.update(|_, cx| cx.new(|_| MixedRows(bar.list.clone())));
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
        let view = cx.update(|_, cx| cx.new(|_| TallTailRows(bar.list.clone(), range)));
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
