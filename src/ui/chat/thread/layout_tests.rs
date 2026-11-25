//! Measure both actual layout trees during GPUI drawing and pixel snapping.
use super::*;
use ::gpui::{TestAppContext, VisualTestContext};
use std::{cell::Cell, rc::Rc};

#[::core::prelude::v1::test]
fn folded_chain_skips_the_chat_update_for_its_measured_tail() {
    let rows = 10..310;
    let folded = ChainRender {
        rows: rows.clone(),
        open: false,
        moving: None,
    };
    let moving = ChainRender {
        rows: rows.clone(),
        open: true,
        moving: Some("chain".into()),
    };
    let open = ChainRender {
        rows: rows.clone(),
        open: true,
        moving: None,
    };
    let updates = |chain: &ChainRender| rows.clone().filter(|index| !chain.hides(*index)).count();
    assert_eq!(updates(&folded), 1);
    assert_eq!(updates(&moving), 1);
    assert_eq!(updates(&open), 300);
    assert!(!folded.hides(rows.start - 1));
    assert!(!folded.hides(rows.end));
}

struct FoldedRows {
    state: ListState,
    wrapped: bool,
}

impl Render for FoldedRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let wrapped = self.wrapped;
        list(self.state.clone(), move |index, _, _| {
            let row = match index {
                0 => div().h(px(24.)).into_any_element(),
                3 => div().h(px(32.)).into_any_element(),
                _ => Empty.into_any_element(),
            };
            if wrapped {
                div().w_full().min_w_0().child(row).into_any_element()
            } else {
                row
            }
        })
        .size_full()
    }
}

#[::core::prelude::v1::test]
fn empty_folded_rows_keep_the_following_list_row_in_place() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let viewport = size(px(400.), px(240.));
    for wrapped in [false, true] {
        let state = ListState::new(4, ListAlignment::Top, px(0.));
        let view = cx.update(|_, cx| {
            cx.new(|_| FoldedRows {
                state: state.clone(),
                wrapped,
            })
        });
        cx.draw(point(px(0.), px(0.)), viewport, |_, _| {
            view.into_any_element()
        });
        let head = state.bounds_for_item(0).unwrap();
        let next = state.bounds_for_item(3).unwrap();
        for index in 1..3 {
            let tail = state.bounds_for_item(index).unwrap();
            assert_eq!(tail.size.height, px(0.));
            assert_eq!(tail.top(), head.bottom());
        }
        assert_eq!(next.top(), head.bottom());
        assert_eq!(next.size.height, px(32.));
    }
}

#[derive(Clone, Copy)]
enum Layout {
    Combined(usize),
    Head,
    Tail,
}

struct MeasuredRow {
    layout: Layout,
    height: Rc<Cell<Option<Pixels>>>,
}

impl Render for MeasuredRow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let call = || div().child(div().h(px(20.)));
        let header = || div().h(px(24.));
        let body = match self.layout {
            Layout::Combined(count) => v_flex()
                .child(header())
                .child(
                    v_flex().children((0..count).map(|n| run_call(call(), n == 0, n + 1 == count))),
                )
                .into_any_element(),
            Layout::Head => v_flex()
                .child(header())
                .child(call().pt_1())
                .into_any_element(),
            Layout::Tail => call().into_any_element(),
        };
        let height = self.height.clone();
        div()
            .pt(prefs::scaled(TOOL_SPACING.top * 4.))
            .pb(prefs::scaled(TOOL_SPACING.bottom * 4.))
            .child(body)
            .on_prepaint(move |bounds, _, _| height.set(Some(bounds.size.height)))
    }
}

fn measure(cx: &mut VisualTestContext, layout: Layout) -> Pixels {
    let height = Rc::new(Cell::new(None));
    let view = cx.update(|_, cx| {
        cx.new(|_| MeasuredRow {
            layout,
            height: height.clone(),
        })
    });
    cx.draw(point(px(0.), px(0.)), size(px(400.), px(2000.)), |_, _| {
        view.into_any_element()
    });
    height.get().expect("row must receive a real prepaint")
}

#[::core::prelude::v1::test]
fn combined_tool_run_and_settled_rows_have_the_same_native_height() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    for scale in [0.75, 0.8, 0.9, 1., 1.1, 1.25, 1.5, 1.75, 2.] {
        cx.update(|window, _| window.set_rem_size(px(16. * scale)));
        for count in [2, 3, 20] {
            let moving = measure(cx, Layout::Combined(count));
            let head = measure(cx, Layout::Head);
            let tail = measure(cx, Layout::Tail);
            assert_eq!(
                moving,
                head + tail * (count - 1) as f32,
                "scale={scale}, tools={count}: final split changed height"
            );
        }
    }
}
