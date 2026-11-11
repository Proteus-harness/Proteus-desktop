//! A temporary page layout is still the native coordinate system for reading.
use super::*;
use ::gpui::{TestAppContext, VisualTestContext};

struct PartialRows(HistoryScrollbar);

impl Render for PartialRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let bar = self.0.clone();
        let rows = bar.state.get().range.unwrap().rows;
        list(bar.list.clone(), move |row, _, _| {
            bar.measure_chat_row(
                row,
                div().w_full().h(px(if row == 0 {
                    300.
                } else if row == rows {
                    0.
                } else {
                    100.
                })),
                row != 0,
            )
        })
        .size_full()
    }
}

fn draw(cx: &mut VisualTestContext, bar: &HistoryScrollbar) {
    bar.set_viewport_width(px(320.));
    let rows = cx.update(|_, cx| cx.new(|_| PartialRows(bar.clone())));
    cx.draw(point(px(0.), px(0.)), size(px(320.), px(300.)), |_, _| {
        rows.into_any_element()
    });
    bar.did_layout();
}

#[::core::prelude::v1::test]
fn temporary_page_head_keeps_pixel_motion_and_cannot_become_a_global_height_fact() {
    // The zero fact is a previously hidden member of a complete folded run;
    // after paging it is temporarily the partial run's visible header.
    for previous in [None, Some(px(0.))] {
        let mut app = TestAppContext::single();
        let cx = app.add_empty_window();
        let range = HistoryRange {
            start: 50,
            rows: 50,
            total: 150,
        };
        let bar = HistoryScrollbar::new(ListState::new(51, ListAlignment::Top, px(400.)), px(0.));
        bar.set_range(Some(range), false);
        bar.set_viewport_width(px(320.));
        if let Some(height) = previous {
            bar.geometry.borrow_mut().refresh(
                &bar.list,
                range,
                px(100.),
                px(14700.),
                px(0.),
                [(0, height)].into_iter(),
            );
        }
        draw(cx, &bar);
        assert_eq!(
            bar.list.bounds_for_item(0).unwrap().size.height,
            px(300.),
            "an incompatible zero scalar must not hide the partial header"
        );
        bar.begin_browsing();
        bar.list.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(299.),
        });
        draw(cx, &bar);
        let before = -bar.offset().y;
        bar.list.scroll_by(px(1.));
        draw(cx, &bar);
        assert_eq!(
            bar.list.logical_scroll_top(),
            ListOffset {
                item_ix: 1,
                offset_in_item: px(0.)
            }
        );
        assert!(
            (-bar.offset().y - before - px(1.)).abs() < px(0.02),
            "crossing the native 300px boundary must not jump back to a 100px estimate"
        );
        bar.list.scroll_by(px(-1.));
        draw(cx, &bar);
        assert_eq!(
            bar.list.logical_scroll_top(),
            ListOffset {
                item_ix: 0,
                offset_in_item: px(299.)
            },
            "wheel-up must reach the temporary head, including a previously hidden row"
        );
        assert_eq!(
            bar.geometry.borrow().estimated_height(50),
            previous,
            "partial context must not pollute the persistent scalar map"
        );
        assert_eq!(
            bar.native_height(50),
            None,
            "temporary native geometry cannot authorize a fractional seek"
        );
    }
}

struct PendingSurface(crate::ui::layout_readiness::Token);
impl Render for PendingSurface {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::ui::layout_readiness::track(&self.0);
        div().w_full().h(px(20.))
    }
}

struct PendingRows(HistoryScrollbar, Entity<PendingSurface>);
impl Render for PendingRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let bar = self.0.clone();
        let surface = self.1.clone();
        let rows = bar.state.get().range.unwrap().rows;
        list(bar.list.clone(), move |row, _, _| {
            let child = if row < rows {
                div().child(surface.clone())
            } else {
                div()
            };
            bar.measure_chat_row(row, child, true)
        })
        .size_full()
    }
}

#[::core::prelude::v1::test]
fn pending_native_rows_measure_a_prepended_page_before_queued_wheel_up_consumes_it() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let bar = HistoryScrollbar::new(ListState::new(101, ListAlignment::Top, px(400.)), px(0.));
    bar.set_range(
        Some(HistoryRange {
            start: 50,
            rows: 100,
            total: 150,
        }),
        false,
    );
    bar.set_viewport_width(px(320.));
    let surface = cx.update(|_, cx| cx.new(|_| PendingSurface(Default::default())));
    let view = cx.update(|_, cx| cx.new(|_| PendingRows(bar.clone(), surface)));
    let paint = |cx: &mut VisualTestContext| {
        cx.draw(point(px(0.), px(0.)), size(px(320.), px(300.)), |_, _| {
            view.clone().into_any_element()
        });
        bar.did_layout();
    };
    paint(cx);
    bar.list.scroll_to(ListOffset {
        item_ix: 0,
        offset_in_item: px(7.),
    });
    paint(cx);
    let mut wheel = super::super::wheel::WheelScroll::default();
    wheel.painted.set(Some(bar.list.logical_scroll_top()));
    wheel.push(px(607.), &bar.list);
    bar.list.splice(0..0, 50);
    bar.set_range(
        Some(HistoryRange {
            start: 0,
            rows: 150,
            total: 150,
        }),
        false,
    );
    wheel.invalidate(&bar.list);
    assert!(
        wheel.step(&bar.list, px(400.)),
        "no layout means no replay yet"
    );
    assert_eq!(bar.list.logical_scroll_top().item_ix, 50);
    for _ in 0..8 {
        paint(cx);
        wheel.painted.set(Some(bar.list.logical_scroll_top()));
        if !wheel.step(&bar.list, px(400.)) {
            break;
        }
    }
    paint(cx);
    assert_eq!(
        bar.list.logical_scroll_top(),
        ListOffset {
            item_ix: 20,
            offset_in_item: px(0.)
        },
        "the entire 607px request must traverse real 20px rows, including the formerly zero-height prepend"
    );
    assert_eq!(bar.geometry.borrow().estimated_height(20), None);
    assert_eq!(
        bar.native_height(20),
        None,
        "pending surfaces remain local geometry rather than completed height facts"
    );
}
