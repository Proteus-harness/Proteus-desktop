//! Native wheel movement while paged renderer facts replace an initial sample.
use super::layout_tests::{paint_short_prefix, track_fraction};
use super::*;
use ::gpui::{TestAppContext, VisualTestContext};

const TOTAL: usize = 245;

fn row_height(row: usize) -> Pixels {
    px(if row == TOTAL - 1 {
        10_000.
    } else {
        match row % 11 {
            0 => 1800.,
            1 => 120.,
            9 => 34.,
            10 => 80.,
            _ => 0.,
        }
    })
}

struct DensityRows {
    bar: HistoryScrollbar,
    range: HistoryRange,
    resolved: bool,
}

impl Render for DensityRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let range = self.range;
        let resolved = self.resolved;
        let measured = self.bar.clone();
        list(self.bar.list.clone(), move |index, _, _| {
            let height = if index == range.rows {
                px(0.)
            } else if resolved {
                row_height(range.start + index)
            } else {
                px(600.)
            };
            measured.measure_row(index, div().w_full().h(height))
        })
        .size_full()
    }
}

fn paint(cx: &mut VisualTestContext, bar: &HistoryScrollbar, range: HistoryRange, resolved: bool) {
    bar.set_viewport_width(px(300.));
    let view = cx.update(|_, cx| {
        cx.new(|_| DensityRows {
            bar: bar.clone(),
            range,
            resolved,
        })
    });
    cx.draw(point(px(0.), px(0.)), size(px(300.), px(700.)), |_, _| {
        view.into_any_element()
    });
    bar.did_layout();
}

fn physical(bar: &HistoryScrollbar, range: HistoryRange) -> Pixels {
    let top = bar.list.logical_scroll_top();
    (0..range.start + top.item_ix)
        .map(row_height)
        .fold(px(0.), |sum, height| sum + height)
        + top.offset_in_item
}

#[::core::prelude::v1::test]
fn paged_density_learning_never_compresses_wheel_motion_into_a_short_prefix() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let mut range = HistoryRange {
        start: TOTAL - 50,
        rows: 50,
        total: TOTAL,
    };
    let bar = HistoryScrollbar::new(ListState::new(51, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(Some(range), false);
    bar.list.scroll_to_end();
    paint(cx, &bar, range, false);
    bar.invalidate(0..range.rows);
    bar.list.remeasure();
    bar.list.scroll_to_end();
    paint(cx, &bar, range, true);
    bar.begin_browsing();
    let mut ordinary_frames = 0;
    for step in 0..2000 {
        let before_pixel = physical(&bar, range);
        let before_thumb = -bar.offset().y;
        let before_unit = bar.unit();
        let before_max = bar.max_offset();
        let before_facts: Vec<_> = (range.start..range.start + range.rows)
            .map(|row| bar.geometry.borrow().measured_height(row))
            .collect();
        bar.list.scroll_by(px(-200.));
        paint(cx, &bar, range, true);
        let pixel = physical(&bar, range);
        let physical_step = before_pixel - pixel;
        assert!(
            physical_step >= px(-0.1) && physical_step <= px(200.1),
            "native wheel skipped or reversed content at step {step}: {physical_step:?}"
        );
        let after_facts: Vec<_> = (range.start..range.start + range.rows)
            .map(|row| bar.geometry.borrow().measured_height(row))
            .collect();
        if before_facts == after_facts
            && before_unit == bar.unit()
            && before_max == bar.max_offset()
            && physical_step > px(0.1)
        {
            let thumb_step = before_thumb + bar.offset().y;
            assert!(
                (thumb_step - physical_step).abs() < px(0.2),
                "stable height facts cannot store a wheel catch-up at step {step}: native {physical_step:?}, thumb {thumb_step:?}"
            );
            ordinary_frames += 1;
        }
        // Probe the observable handle's local pixel gain independently of
        // measurement corrections. A historical pivot produced gains >100.
        let top = bar.list.logical_scroll_top();
        let next = ListOffset {
            offset_in_item: top.offset_in_item + px(1.),
            ..top
        };
        let geometry = bar.geometry.borrow();
        let gain = geometry.position(next) - geometry.position(top);
        assert!(
            gain >= px(-0.01) && gain <= px(1.05),
            "ordinary wheel mapping amplified one physical pixel at step {step}: {gain:?}"
        );
        drop(geometry);
        if top.item_ix < 8 && range.start > 0 {
            let anchor = physical(&bar, range);
            let added = range.start.min(50);
            bar.list.splice(0..0, added);
            range.start -= added;
            range.rows += added;
            if range.rows > 200 {
                bar.list.splice(200..range.rows, 0);
                range.rows = 200;
            }
            bar.set_range(Some(range), false);
            paint(cx, &bar, range, true);
            assert!(
                (physical(&bar, range) - anchor).abs() < px(0.1),
                "loading/eviction changed the native reading anchor at page {}",
                range.start
            );
        }
        if range.start == 0 && physical(&bar, range) <= px(0.1) {
            assert!(
                ordinary_frames >= 20,
                "exercise stable wheel frames as well as page changes"
            );
            return;
        }
    }
    panic!("did not traverse the synthetic paged history");
}

#[::core::prelude::v1::test]
fn folded_rows_participate_in_the_unknown_row_density_sample() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let range = HistoryRange {
        start: 0,
        rows: 11,
        total: 11,
    };
    let bar = HistoryScrollbar::new(ListState::new(12, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(Some(range), false);
    bar.list.scroll_to_end();
    paint(cx, &bar, range, true);
    assert!(
        bar.unit() < px(80.),
        "raw-row density must include the folded zero-height members"
    );
}

#[::core::prelude::v1::test]
fn a_zero_only_loaded_page_keeps_unloaded_history_reachable() {
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
    paint_short_prefix(cx, &bar, [0.; 7], px(700.));
    assert!(
        bar.max_offset() > px(0.),
        "known folded rows cannot prove that the unloaded prefix also has zero height"
    );
    bar.set_offset(point(px(0.), px(-1000.)));
    let target = bar.take_target().unwrap();
    assert!(target.row < 100);
    assert!(!bar.scroll_to(target));
}

#[::core::prelude::v1::test]
fn subsequent_drag_motion_keeps_its_modal_mapping_after_a_height_resolves() {
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
    let heights = [100., 100., 100., 3000., 100., 100., 4000.];
    bar.list.scroll_to_end();
    paint_short_prefix(cx, &bar, heights, px(700.));
    bar.start_drag();
    bar.set_offset(point(px(0.), px(-350.)));
    let target = bar.take_target().unwrap();
    assert!(bar.scroll_to(target));
    paint_short_prefix(cx, &bar, heights, px(700.));
    bar.settle();
    paint_short_prefix(cx, &bar, heights, px(700.));
    let before = bar.list.logical_scroll_top();
    bar.set_offset(point(px(0.), px(-351.)));
    assert!(bar.scroll_to(bar.take_target().unwrap()));
    paint_short_prefix(cx, &bar, heights, px(700.));
    bar.settle();
    paint_short_prefix(cx, &bar, heights, px(700.));
    let after = bar.list.logical_scroll_top();
    assert_eq!(after.item_ix, before.item_ix);
    assert!(
        after.offset_in_item >= before.offset_in_item
            && after.offset_in_item - before.offset_in_item < px(5.),
        "one more track pixel must not rebase the same drag to another content position: {before:?} -> {after:?}"
    );
    assert!((-bar.offset().y - px(351.)).abs() < px(0.5));
    bar.end_drag();
    paint_short_prefix(cx, &bar, heights, px(700.));
    assert!(
        (-bar.offset().y - px(351.)).abs() < px(0.5),
        "release keeps the explicit seek anchor"
    );
}

#[::core::prelude::v1::test]
fn invalidating_an_offscreen_long_row_keeps_its_last_mass_until_replacement_commits() {
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
    let mut heights = [100., 100., 100., 3000., 100., 100., 4000.];
    bar.list.scroll_to(ListOffset {
        item_ix: 3,
        offset_in_item: px(1000.),
    });
    paint_short_prefix(cx, &bar, heights, px(700.));
    assert_eq!(bar.geometry.borrow().measured_height(3), Some(px(3000.)));
    bar.list.scroll_to(ListOffset {
        item_ix: 6,
        offset_in_item: px(1000.),
    });
    paint_short_prefix(cx, &bar, heights, px(700.));
    let top = bar.list.logical_scroll_top();
    let extent = bar.content_size().height;
    let offset = bar.offset();
    bar.invalidate(3..4);
    assert_eq!(
        bar.geometry.borrow().measured_height(3),
        None,
        "exact seeks must wait for a fresh measurement"
    );
    for _ in 0..3 {
        // The invalidated offscreen row has not produced a replacement yet.
        // No observer batch may turn its known long mass into an unknown mean.
        bar.did_layout();
        assert_eq!(bar.list.logical_scroll_top(), top);
        assert_eq!(bar.content_size().height, extent);
        assert_eq!(bar.offset(), offset);
    }
    heights[3] = 5000.;
    bar.list.remeasure_items(3..4);
    bar.list.scroll_to(ListOffset {
        item_ix: 3,
        offset_in_item: px(1000.),
    });
    paint_short_prefix(cx, &bar, heights, px(700.));
    assert_eq!(bar.geometry.borrow().measured_height(3), Some(px(5000.)));
    assert_eq!(bar.content_size().height, extent + px(2000.));
    assert_eq!(bar.list.logical_scroll_top().item_ix, 3);
    assert_eq!(bar.list.logical_scroll_top().offset_in_item, px(1000.));
    let replaced_offset = bar.offset();
    paint_short_prefix(cx, &bar, heights, px(700.));
    assert_eq!(bar.content_size().height, extent + px(2000.));
    assert_eq!(
        bar.offset(),
        replaced_offset,
        "a replacement fact commits once"
    );
}

#[::core::prelude::v1::test]
fn drag_release_keeps_the_requested_visual_fraction_after_a_real_height_change() {
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
    let mut heights = [100., 100., 100., 3000., 100., 100., 4000.];
    bar.list.scroll_to_end();
    paint_short_prefix(cx, &bar, heights, px(700.));
    bar.start_drag();
    let held_extent = bar.content_size().height;
    bar.set_offset(point(px(0.), px(-350.)));
    assert!(bar.scroll_to(bar.take_target().unwrap()));
    paint_short_prefix(cx, &bar, heights, px(700.));
    bar.settle();
    paint_short_prefix(cx, &bar, heights, px(700.));
    let requested = track_fraction(&bar);
    heights[3] = 6000.;
    bar.invalidate(3..4);
    bar.list.remeasure_items(3..4);
    paint_short_prefix(cx, &bar, heights, px(700.));
    assert_eq!(bar.geometry.borrow().measured_height(3), Some(px(6000.)));
    assert_eq!(
        bar.content_size().height,
        held_extent,
        "active drag owns its extent"
    );
    assert!((-bar.offset().y - px(350.)).abs() < px(0.5));
    let content = bar.list.logical_scroll_top();
    bar.end_drag();
    paint_short_prefix(cx, &bar, heights, px(700.));
    assert_eq!(
        bar.list.logical_scroll_top(),
        content,
        "release cannot move the content to match a thumb correction"
    );
    assert_eq!(bar.content_size().height, held_extent);
    assert!(
        (track_fraction(&bar) - requested).abs() < 0.00001,
        "release must preserve the requested visual location: {requested} -> {}",
        track_fraction(&bar)
    );
    bar.begin_browsing();
    bar.cancel_target();
    assert!(bar.content_size().height > held_extent + px(4000.));
    assert_eq!(bar.list.logical_scroll_top(), content);
    let thumb = -bar.offset().y;
    bar.list.scroll_by(px(-200.));
    paint_short_prefix(cx, &bar, heights, px(700.));
    let moved = bar.list.logical_scroll_top();
    assert_eq!(moved.item_ix, content.item_ix);
    assert!((content.offset_in_item - moved.offset_in_item - px(200.)).abs() < px(0.05));
    assert!(
        (thumb + bar.offset().y - px(200.)).abs() < px(0.05),
        "ordinary wheel releases the seek's modal mapping"
    );
}

#[::core::prelude::v1::test]
fn holding_and_releasing_the_thumb_without_moving_it_keeps_its_visual_location() {
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
    let mut heights = [100., 100., 100., 3000., 100., 100., 4000.];
    bar.list.scroll_to_end();
    paint_short_prefix(cx, &bar, heights, px(700.));
    let phi = track_fraction(&bar);
    let extent = bar.content_size().height;
    bar.start_drag();
    heights[6] = 6000.;
    bar.invalidate(6..7);
    bar.list.remeasure_items(6..7);
    paint_short_prefix(cx, &bar, heights, px(700.));
    let content = bar.list.logical_scroll_top();
    assert_eq!(bar.geometry.borrow().measured_height(6), Some(px(6000.)));
    assert!(
        (track_fraction(&bar) - phi).abs() < 0.00001,
        "holding at the track edge must keep the modal anchor"
    );
    bar.end_drag();
    for _ in 0..2 {
        paint_short_prefix(cx, &bar, heights, px(700.));
        assert_eq!(bar.list.logical_scroll_top(), content);
        assert_eq!(bar.content_size().height, extent);
        assert!((track_fraction(&bar) - phi).abs() < 0.00001);
    }
    bar.begin_browsing();
    assert_eq!(bar.content_size().height, extent + px(2000.));
    assert_eq!(bar.list.logical_scroll_top(), content);
}
