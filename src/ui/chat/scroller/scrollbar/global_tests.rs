//! Global scalar geometry must outlive resident widgets without becoming native authority.
use super::layout_tests::{draw, height, paint_short_prefix};
use super::*;
use ::gpui::{TestAppContext, VisualTestContext};

const VISITED: [f32; 7] = [70., 0., 5000., 0., 20., 30., 40.];

fn visited_then_evicted(cx: &mut VisualTestContext) -> HistoryScrollbar {
    let bar = HistoryScrollbar::new(ListState::new(8, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(
        Some(HistoryRange {
            start: 0,
            rows: 7,
            total: 107,
        }),
        false,
    );
    paint_short_prefix(cx, &bar, VISITED, px(700.));
    bar.list.scroll_to_end();
    paint_short_prefix(cx, &bar, VISITED, px(700.));
    assert_eq!(bar.content_size().height, px(15_160.));
    bar.list.splice(0..7, 7);
    bar.set_range(
        Some(HistoryRange {
            start: 100,
            rows: 7,
            total: 107,
        }),
        false,
    );
    bar.list.scroll_to(ListOffset {
        item_ix: 0,
        offset_in_item: px(0.),
    });
    paint_short_prefix(cx, &bar, [100.; 7], px(700.));
    assert_eq!(bar.content_size().height, px(15_160.));
    bar
}

#[::core::prelude::v1::test]
fn visited_long_and_zero_rows_survive_eviction_but_fractional_seek_waits_for_layout() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let bar = visited_then_evicted(cx);
    assert_eq!(bar.geometry.borrow().estimated_position(2, px(0.)), px(70.));
    assert_eq!(
        bar.geometry.borrow().estimated_position(3, px(0.)),
        px(5070.)
    );
    assert_eq!(bar.geometry.borrow().measured_height(2), None);
    bar.set_offset(point(px(0.), px(-2570.)));
    let target = bar.take_target().unwrap();
    assert_eq!(
        target.row, 2,
        "inverse lookup must use the visited 5000px row"
    );
    assert!((target.fraction - 0.5).abs() < 0.0001);
    assert!(!bar.scroll_to(target));

    bar.list.splice(0..7, 7);
    bar.set_range(
        Some(HistoryRange {
            start: 0,
            rows: 7,
            total: 107,
        }),
        false,
    );
    assert!(bar.scroll_to(target));
    assert_eq!(
        bar.list.logical_scroll_top().offset_in_item,
        px(0.),
        "a cached scalar cannot settle a remounted native row"
    );
    assert!(bar.state.get().local.is_some());
    paint_short_prefix(cx, &bar, VISITED, px(700.));
    bar.settle();
    paint_short_prefix(cx, &bar, VISITED, px(700.));
    assert_eq!(bar.list.logical_scroll_top().item_ix, 2);
    assert_eq!(bar.list.logical_scroll_top().offset_in_item, px(2500.));
    assert_eq!(-bar.offset().y, px(2570.));
}

#[::core::prelude::v1::test]
fn a_new_measurement_replaces_only_its_own_fixed_prior() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let bar = HistoryScrollbar::new(ListState::new(8, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(
        Some(HistoryRange {
            start: 500,
            rows: 7,
            total: 1000,
        }),
        false,
    );
    let mut heights = [100.; 7];
    paint_short_prefix(cx, &bar, heights, px(700.));
    assert_eq!(bar.content_size().height, px(100_000.));
    let anchor = bar.list.logical_scroll_top();
    heights[3] = 5000.;
    bar.invalidate(3..4);
    bar.list.remeasure_items(3..4);
    paint_short_prefix(cx, &bar, heights, px(700.));
    assert_eq!(bar.content_size().height, px(104_900.));
    assert_eq!(bar.unit(), UNKNOWN_ROW_HEIGHT);
    assert_eq!(bar.list.logical_scroll_top(), anchor);
    let geometry = bar.geometry.borrow();
    assert_eq!(geometry.estimated_position(400, px(0.)), px(40_000.));
    assert_eq!(geometry.estimated_position(503, px(0.)), px(50_300.));
    assert_eq!(geometry.estimated_position(504, px(0.)), px(55_300.));
    drop(geometry);
    heights[3] = 0.;
    bar.invalidate(3..4);
    bar.list.remeasure_items(3..4);
    paint_short_prefix(cx, &bar, heights, px(700.));
    assert_eq!(bar.content_size().height, px(99_900.));
    assert_eq!(bar.geometry.borrow().measured_height(503), Some(px(0.)));
    assert_eq!(bar.list.logical_scroll_top(), anchor);
}

#[::core::prelude::v1::test]
fn canonical_rebase_retains_only_the_unchanged_global_prefix() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let bar = visited_then_evicted(cx);
    bar.rebase_history(Some(3));
    bar.did_layout();
    assert_eq!(bar.content_size().height, px(15_470.));
    assert_eq!(
        bar.geometry.borrow().estimated_position(3, px(0.)),
        px(5070.)
    );
    assert_eq!(
        bar.geometry.borrow().estimated_position(100, px(0.)),
        px(14_770.)
    );
    bar.rebase_history(None);
    bar.did_layout();
    assert_eq!(bar.content_size().height, px(10_700.));
    assert_eq!(
        bar.geometry.borrow().estimated_position(3, px(0.)),
        px(300.)
    );
}

#[::core::prelude::v1::test]
fn width_reset_discards_all_visited_facts_and_rejects_old_same_range_measurement() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let bar = visited_then_evicted(cx);
    let old_wrapper = bar.measure_row(0, div().w_full().h(px(9000.)));
    bar.set_viewport_width(px(320.));
    cx.draw(point(px(0.), px(0.)), size(px(320.), px(700.)), |_, _| {
        old_wrapper
    });
    bar.did_layout();
    assert_eq!(bar.content_size().height, px(10_700.));
    assert_eq!(
        bar.geometry.borrow().estimated_position(3, px(0.)),
        px(300.)
    );
    assert_eq!(
        bar.geometry.borrow().measured_height(100),
        None,
        "the old wrapper has the same start/count, but belongs to the previous layout epoch"
    );
    // The explicit font/renderer layout reset has the same whole-history contract.
    paint_short_prefix(cx, &bar, [100.; 7], px(700.));
    bar.invalidate_layout();
    bar.did_layout();
    assert_eq!(bar.content_size().height, px(10_700.));
    assert_eq!(bar.geometry.borrow().measured_height(100), None);
}

fn physical(bar: &HistoryScrollbar, range: HistoryRange, prefix: &[Pixels]) -> Pixels {
    let top = bar.list.logical_scroll_top();
    prefix[range.start + top.item_ix] + top.offset_in_item
}

fn traverse(
    cx: &mut VisualTestContext,
    bar: &HistoryScrollbar,
    range: &mut HistoryRange,
    prefix: &[Pixels],
    up: bool,
    warm_extent: Option<Pixels>,
) {
    bar.begin_browsing();
    let direction = if up { -1. } else { 1. };
    for step in 0..10_000 {
        let before = physical(bar, *range, prefix);
        let track = -bar.offset().y;
        bar.list.scroll_by(px(direction * 240.));
        draw(cx, bar, *range);
        let after = physical(bar, *range, prefix);
        let movement = (after - before) * direction;
        assert!(
            movement >= px(-0.05) && movement <= px(240.05),
            "native wheel changed direction or skipped pixels: up={up}, step={step}, delta={movement:?}"
        );
        if let Some(extent) = warm_extent {
            assert_eq!(
                bar.content_size().height,
                extent,
                "visiting an already measured row resized the global document"
            );
            assert!(
                (-bar.offset().y - track - (after - before)).abs() < px(0.2),
                "warm native pixels and track pixels must stay 1:1: up={up}, step={step}"
            );
        }
        let top = bar.list.logical_scroll_top();
        let needs_older = up && top.item_ix < 8 && range.start > 0;
        let needs_newer =
            !up && top.item_ix + 16 >= range.rows && range.start + range.rows < range.total;
        if needs_older || needs_newer {
            let anchor = physical(bar, *range, prefix);
            if needs_older {
                let added = range.start.min(50);
                bar.list.splice(0..0, added);
                range.start -= added;
                range.rows += added;
                if range.rows > 200 {
                    bar.list.splice(200..range.rows, 0);
                    range.rows = 200;
                }
            } else {
                let added = (range.total - range.start - range.rows).min(50);
                bar.list.splice(range.rows..range.rows, added);
                range.rows += added;
                if range.rows > 200 {
                    let removed = range.rows - 200;
                    bar.list.splice(0..removed, 0);
                    range.start += removed;
                    range.rows = 200;
                }
            }
            bar.set_range(Some(*range), false);
            draw(cx, bar, *range);
            assert!(
                (physical(bar, *range, prefix) - anchor).abs() < px(0.1),
                "resident page replacement moved the content anchor"
            );
            if let Some(extent) = warm_extent {
                assert_eq!(
                    bar.content_size().height,
                    extent,
                    "a pure page change cannot resize a warmed global document"
                );
            }
        }
        if up && range.start == 0 && physical(bar, *range, prefix) <= px(0.05)
            || !up && bar.at_bottom(*range)
        {
            return;
        }
    }
    panic!(
        "failed to traverse {total} heterogeneous rows, up={up}",
        total = range.total
    );
}

#[::core::prelude::v1::test]
fn full_heterogeneous_history_keeps_warm_extent_and_wheel_ratio_across_both_page_directions() {
    for total in [245, 600] {
        let mut app = TestAppContext::single();
        let cx = app.add_empty_window();
        let mut range = HistoryRange {
            start: total - 50,
            rows: 50,
            total,
        };
        let bar = HistoryScrollbar::new(ListState::new(51, ListAlignment::Top, px(400.)), px(0.));
        let mut prefix = vec![px(0.)];
        for row in 0..total {
            prefix.push(prefix.last().copied().unwrap() + height(row));
        }
        bar.set_range(Some(range), false);
        bar.list.scroll_to_end();
        draw(cx, &bar, range);
        traverse(cx, &bar, &mut range, &prefix, true, None);
        let expected = prefix[total] + px(24.);
        assert_eq!(
            bar.content_size().height,
            expected,
            "the cold traversal must have measured every row, including folded members"
        );
        traverse(cx, &bar, &mut range, &prefix, false, Some(expected));
        traverse(cx, &bar, &mut range, &prefix, true, Some(expected));
    }
}

struct ContextRows(HistoryScrollbar);
impl Render for ContextRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let bar = self.0.clone();
        let range = bar.state.get().range.unwrap();
        list(bar.list.clone(), move |ix, _, _| {
            let height = if ix == range.rows {
                0.
            } else if range.start + ix == 100 {
                150.
            } else {
                100.
            };
            bar.measure_chat_row(ix, div().w_full().h(px(height)), true)
        })
        .size_full()
    }
}

#[::core::prelude::v1::test]
fn a_clipped_group_heading_uses_local_pixels_without_replacing_its_global_fact() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let bar = visited_then_evicted(cx);
    bar.invalidate(0..1);
    bar.list.remeasure_items(0..1);
    let draw = |cx: &mut VisualTestContext| {
        let view = cx.update(|_, cx| cx.new(|_| ContextRows(bar.clone())));
        cx.draw(point(px(0.), px(0.)), size(px(300.), px(400.)), |_, _| {
            view.into_any_element()
        });
        bar.did_layout();
    };
    draw(cx);
    assert_eq!(bar.list.bounds_for_item(0).unwrap().size.height, px(150.));
    assert_eq!(bar.geometry.borrow().estimated_height(100), Some(px(100.)));
    assert_eq!(
        bar.content_size().height,
        px(15_210.),
        "only the current resident extent includes the temporary 50px difference"
    );
    assert_eq!(
        bar.native_height(100),
        None,
        "temporary native bounds do not contain the missing group context"
    );
    let target = ScrollTarget {
        row: 100,
        fraction: 0.5,
    };
    assert!(bar.scroll_to(target));
    assert!(
        bar.state.get().local.is_some(),
        "partial-context bounds cannot settle the seek"
    );
    bar.list.splice(0..0, 1);
    bar.set_range(
        Some(HistoryRange {
            start: 99,
            rows: 8,
            total: 107,
        }),
        false,
    );
    draw(cx);
    bar.settle();
    assert_eq!(bar.native_height(100), Some(px(150.)));
    assert_eq!(
        bar.list.logical_scroll_top(),
        ListOffset {
            item_ix: 1,
            offset_in_item: px(75.)
        }
    );
}
