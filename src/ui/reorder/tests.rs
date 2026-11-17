//! Native event coverage for holding, dragging and dropping ordered rows.
use super::*;
use ::gpui::{TestAppContext, VisualTestContext};

struct OrderedRows {
    rows: Vec<SharedString>,
    moved: usize,
}

impl Render for OrderedRows {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self
            .rows
            .iter()
            .map(|id| Row::new(id.clone(), id.clone(), div().h(px(32.)).child(id.clone())))
            .collect();
        let view = cx.entity().downgrade();
        div()
            .w(px(240.))
            .child(List::new("ordered-rows", rows, move |id, before, _, cx| {
                let _ = view.update(cx, |view, cx| {
                    if move_before(&mut view.rows, &id, before.as_ref()) {
                        view.moved += 1;
                        cx.notify();
                    }
                });
            }))
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

#[::core::prelude::v1::test]
fn native_hold_moves_from_row_body_and_commits_only_on_drop() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        gpui_kit::component::theme::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let (view, cx) = app.add_window_view(|_, _| OrderedRows {
        rows: vec!["a".into(), "b".into(), "c".into()],
        moved: 0,
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    draw(cx);
    let position = point(px(160.), px(16.));
    cx.simulate_mouse_move(position, None, Modifiers::none());
    for button in [MouseButton::Left, MouseButton::Right] {
        cx.simulate_mouse_down(position, button, Modifiers::none());
        cx.simulate_mouse_up(position, button, Modifiers::none());
        draw(cx);
        assert!(!cx.update(|_, cx| cx.has_active_drag()));
        assert_eq!(view.read_with(cx, |view, _| view.moved), 0);
    }
    cx.simulate_mouse_move(position, None, Modifiers::none());
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::none());
    draw(cx);
    cx.executor().advance_clock(motion::HOLD);
    cx.run_until_parked();
    draw(cx);
    let target = point(px(160.), px(90.));
    cx.simulate_mouse_move(target, Some(MouseButton::Left), Modifiers::none());
    draw(cx);
    assert!(
        cx.update(|_, cx| cx.has_active_drag()),
        "hold must start the native drag"
    );
    assert_eq!(view.read_with(cx, |view, _| view.moved), 0);
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
    draw(cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.rows.clone()),
        ["b", "c", "a"]
    );
    assert_eq!(view.read_with(cx, |view, _| view.moved), 1);
}
