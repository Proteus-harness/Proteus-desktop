//! Exercise caption layout, mouse dispatch, text paint and retained invalidation.
use super::*;
use ::gpui::{TestAppContext, VisualTestContext};
use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use std::{cell::Cell, rc::Rc};

struct Footer {
    caption: Entity<Shimmer>,
    renders: Rc<Cell<usize>>,
    updated: bool,
}

impl Render for Footer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        self.caption.update(cx, |caption, cx| {
            caption.set_caption("Думаю", true, cx);
        });
        // Match the chat footer: its outer block owns padding/truncation, while
        // the inner flex row gives the caption its intrinsic hitbox.
        div()
            .pt_4()
            .pb_1()
            .truncate()
            .underline()
            .child(h_flex().min_w_0().child(self.caption.clone()))
            .when(self.updated, |footer| {
                footer.child(
                    div()
                        .debug_selector(|| "footer-update".into())
                        .child("Updated footer"),
                )
            })
    }
}

struct Root(Entity<Footer>);

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

fn tick(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    draw(cx);
}

#[::core::prelude::v1::test]
fn caption_hover_is_compact_uses_foreground_and_stops_its_ticker() {
    let mut app = TestAppContext::single();
    app.update(gpui_kit::component::theme::init);
    let caption = app.new(|_| Shimmer::new("Думаю"));
    let renders = Rc::new(Cell::new(0));
    let notifications = Rc::new(Cell::new(0));
    let _observer = app.update(|cx| {
        cx.observe(&caption, {
            let notifications = notifications.clone();
            move |_, _| notifications.set(notifications.get() + 1)
        })
    });
    let footer = app.new(|_| Footer {
        caption: caption.clone(),
        renders: renders.clone(),
        updated: false,
    });
    let window = app.add_window(|_, _| Root(footer.clone()));
    let mut cx = VisualTestContext::from_window(window.into(), &app);
    cx.simulate_resize(size(px(400.), px(100.)));
    draw(&mut cx);

    let bounds = cx.debug_bounds("chat-status-caption").unwrap();
    assert!(bounds.size.width > px(0.) && bounds.size.width < px(200.));
    let blank = point(px(350.), bounds.center().y);
    cx.simulate_mouse_move(blank, None, Default::default());
    draw(&mut cx);
    assert!(!caption.read_with(&cx, |caption, _| caption.hovered));

    draw(&mut cx); // Settle the initial resize and pointer presentation.
    let parent_renders = renders.get();
    for _ in 0..3 {
        let before_tick = notifications.get();
        tick(&mut cx);
        assert!(
            notifications.get() > before_tick,
            "moving caption gets ticks"
        );
        assert_eq!(
            renders.get(),
            parent_renders,
            "caption ticks do not render its parent"
        );
    }

    cx.simulate_mouse_move(bounds.center(), None, Default::default());
    draw(&mut cx);
    assert!(caption.read_with(&cx, |caption, _| caption.hovered));
    let foreground = cx.update(|_, cx| cx.theme().foreground);
    let painted = cx.update(|window, _| window.painted_underlines());
    // Inherit underlining only in this fixture so the actual TextRun paint
    // exposes its color through GPUI's scene API, without a test-only renderer.
    assert!(!painted.is_empty(), "caption text must have painted");
    assert!(
        painted
            .iter()
            .all(|underline| underline.color == foreground)
    );

    // Mouse transitions may rebuild an ancestor to update GPUI hover records;
    // the animation must not keep rebuilding it on subsequent frames.
    tick(&mut cx); // A tick already queued before hovering may still arrive.
    let hovered_renders = renders.get();
    let paused = notifications.get();
    tick(&mut cx);
    tick(&mut cx);
    assert_eq!(
        notifications.get(),
        paused,
        "hovered caption schedules no more ticks"
    );
    assert_eq!(renders.get(), hovered_renders);

    cx.simulate_mouse_move(blank, None, Default::default());
    draw(&mut cx);
    assert!(!caption.read_with(&cx, |caption, _| caption.hovered));
    let resumed = notifications.get();
    tick(&mut cx);
    assert!(
        notifications.get() > resumed,
        "leaving caption resumes ticks"
    );
    let resumed_renders = renders.get();
    for _ in 0..3 {
        let before_tick = notifications.get();
        tick(&mut cx);
        assert!(notifications.get() > before_tick);
        assert_eq!(renders.get(), resumed_renders);
    }

    cx.update(|_, cx| cx.set_reduce_motion(true));
    caption.update(&mut cx, |_, cx| cx.notify());
    draw(&mut cx);
    tick(&mut cx); // Drain the pre-existing moving tick.
    let reduced_renders = renders.get();
    let reduced = notifications.get();
    tick(&mut cx);
    tick(&mut cx);
    assert_eq!(
        notifications.get(),
        reduced,
        "reduced motion schedules no more ticks"
    );
    assert_eq!(renders.get(), reduced_renders);

    assert!(cx.debug_bounds("footer-update").is_none());
    footer.update(&mut cx, |footer, cx| {
        footer.updated = true;
        cx.notify();
    });
    draw(&mut cx);
    assert_eq!(
        renders.get(),
        reduced_renders + 1,
        "the parent's own update still renders it"
    );
    assert!(
        cx.debug_bounds("footer-update").is_some(),
        "the parent's new content is laid out"
    );
}
