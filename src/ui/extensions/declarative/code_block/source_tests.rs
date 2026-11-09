//! Actual Source body, native horizontal range and wheel routing to its parent.
use super::*;
use ::gpui::{TestAppContext, VisualTestContext};
use gpui_kit::base::ElementExt as _;
use std::{cell::Cell, rc::Rc};

struct Fixture {
    source: Entity<SourceScroll>,
    parent: ScrollHandle,
    text: SharedString,
    source_height: Rc<Cell<Pixels>>,
}

impl Render for Fixture {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let measured = self.source_height.clone();
        div()
            .id("source-parent")
            .w(px(300.))
            .h(px(160.))
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&self.parent)
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .child(
                        div()
                            .w_full()
                            .child(source_view(
                                "source-fixture".into(),
                                self.text.clone(),
                                self.source.clone(),
                                window,
                                cx,
                            ))
                            .on_prepaint(move |bounds, _, _| {
                                measured.set(bounds.size.height);
                            }),
                    )
                    .child(div().h(px(600.))),
            )
    }
}

fn draw(cx: &mut VisualTestContext, fixture: &Entity<Fixture>) {
    // One first layout observes the actual width; the next reserves its track.
    // Further draws must be stable, rather than growing a private Y range.
    for _ in 0..3 {
        cx.draw(point(px(0.), px(0.)), size(px(400.), px(1000.)), |_, _| {
            fixture.clone().into_any_element()
        });
    }
}

fn wheel(cx: &mut VisualTestContext, fixture: &Entity<Fixture>, dx: f32, dy: f32) {
    cx.simulate_event(ScrollEvent {
        position: point(px(60.), px(10.)),
        delta: ScrollDelta::Pixels(point(px(dx), px(dy))),
        ..Default::default()
    });
    draw(cx, fixture);
}

#[::core::prelude::v1::test]
fn source_overflow_has_a_track_and_vertical_wheel_stays_with_the_chat() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let cx = app.add_empty_window();
    let parent = ScrollHandle::new();
    let source_height = Rc::new(Cell::new(px(0.)));
    let (source, fixture) = cx.update(|_, cx| {
        let source = cx.new(|_| SourceScroll::default());
        let fixture = cx.new(|_| Fixture {
            source: source.clone(),
            parent: parent.clone(),
            text: ("long unwrapped source ".repeat(100) + "\nlast source row").into(),
            source_height: source_height.clone(),
        });
        (source, fixture)
    });
    draw(cx, &fixture);
    let (handle, track) = cx.update(|_, cx| {
        let source = source.read(cx);
        assert!(source.horizontal_overflow);
        (
            source.handle.clone(),
            super::super::code_geometry::horizontal_track_height(cx),
        )
    });
    assert!(handle.max_offset().x > px(0.));
    assert_eq!(handle.max_offset().y, px(0.));
    assert_eq!(source_height.get() - handle.bounds().size.height, track);

    wheel(cx, &fixture, 0., -40.);
    assert!(
        parent.offset().y < px(0.),
        "vertical wheel must reach the chat"
    );
    assert_eq!(
        handle.offset().x,
        px(0.),
        "vertical wheel must not become X"
    );

    parent.set_offset(point(px(0.), px(0.)));
    draw(cx, &fixture);
    wheel(cx, &fixture, -80., 0.);
    assert!(
        handle.offset().x < px(0.),
        "horizontal wheel must reach source X"
    );
    assert_eq!(parent.offset().y, px(0.));

    fixture.update(cx, |fixture, cx| {
        fixture.text = "short source".into();
        cx.notify();
    });
    draw(cx, &fixture);
    cx.update(|_, cx| assert!(!source.read(cx).horizontal_overflow));
    assert_eq!(handle.max_offset().x, px(0.));
    assert_eq!(handle.max_offset().y, px(0.));
    assert_eq!(source_height.get(), handle.bounds().size.height);
}
