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
    state: ListState,
    range: HistoryRange,
    long_tools: bool,
}

impl Render for Rows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let range = self.range;
        let long_tools = self.long_tools;
        list(self.state.clone(), move |ix, _, _| {
            div()
                .h(if ix < range.rows {
                    row_height(range.start + ix, range, long_tools)
                } else {
                    px(24.)
                })
                .w_full()
                .into_any_element()
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
    let view = cx.update(|_, cx| {
        cx.new(|_| Rows {
            state: bar.list.clone(),
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
fn loading_older_members_of_a_folded_run_keeps_the_thumb_at_the_reading_position() {
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
    for step in 0..500 {
        bar.list.scroll_by(px(-120.));
        draw_with_tools(cx, &bar, range, true);
        let _ = bar.offset();
        if bar.list.logical_scroll_top().item_ix < 8 && range.start > 0 {
            let before = bar.offset().y;
            let top = bar.list.logical_scroll_top();
            bar.list.splice(0..0, 50);
            range.start -= 50;
            range.rows += 50;
            // Keep the visible run indivisible, as HistoryWindow::trim does.
            bar.set_range(Some(range), false);
            draw_with_tools(cx, &bar, range, true);
            assert!(
                (bar.offset().y - before).abs() < px(0.02),
                "page {} after step {step}, old top {top:?}: thumb {before:?} -> {:?}, new top {:?}",
                range.start,
                bar.offset().y,
                bar.list.logical_scroll_top()
            );
        }
        if range.start == 0 && bar.list.logical_scroll_top().item_ix == 0 {
            return;
        }
    }
    panic!("did not reach beginning");
}

fn global_pixel(bar: &HistoryScrollbar, range: HistoryRange) -> Pixels {
    let top = bar.list.logical_scroll_top();
    (0..range.start + top.item_ix)
        .map(height)
        .fold(px(0.), |a, b| a + b)
        + top.offset_in_item
}

struct UnpagedRows(ListState, usize);
impl Render for UnpagedRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let long_row = self.1;
        list(self.0.clone(), move |ix, _, _| {
            div()
                .h(px(if ix == long_row { 50_000. } else { 100. }))
                .w_full()
                .into_any_element()
        })
        .w_full()
        .h_full()
    }
}

fn paint_unpaged(cx: &mut VisualTestContext, bar: &HistoryScrollbar, long_row: usize) {
    let view = cx.update(|_, cx| cx.new(|_| UnpagedRows(bar.list.clone(), long_row)));
    cx.draw(point(px(0.), px(0.)), size(px(300.), px(700.)), |_, _| {
        view.into_any_element()
    });
    bar.did_layout();
}

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
    bar.lock_unit(); // An upward gesture while everything fits must not freeze scale.
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
fn measuring_a_long_earlier_reply_does_not_resize_an_unpaged_chat_scrollbar() {
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
    bar.lock_unit();
    let extent = bar.content_size().height;
    let native_extent = bar.list.content_size().height;
    for _ in 0..20 {
        bar.list.scroll_by(px(-120.));
        paint_unpaged(cx, &bar, 4);
        assert_eq!(bar.content_size().height, extent);
    }
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
        let thumb = bar.offset().y;
        // No input and no changed source: a fresh measurement must not move it.
        draw(cx, &bar, range);
        assert!(
            (bar.offset().y - thumb).abs() < px(0.02),
            "idle frame {step}"
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
        let moved_thumb = bar.offset().y;
        assert!(
            moved_thumb >= thumb - px(0.02),
            "thumb moved down on upward input at {step}: {thumb:?} -> {moved_thumb:?}"
        );
        if bar.list.logical_scroll_top().item_ix < 8 && range.start > 0 {
            let before_pixel = global_pixel(&bar, range);
            let before_thumb = bar.offset().y;
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
            assert!(
                (bar.offset().y - before_thumb).abs() < px(0.02),
                "thumb at page {}: {before_thumb:?} -> {:?}",
                range.start,
                bar.offset().y
            );
        }
        if range.start == 0 && bar.list.logical_scroll_top().item_ix == 0 {
            return;
        }
    }
    panic!("did not reach the beginning");
}
