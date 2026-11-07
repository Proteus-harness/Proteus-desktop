//! Import the production animation under its actual visibility boundary.
use gpui_kit::AppContext as _;
use std::{cell::Cell, rc::Rc};
#[path = "../../src/ui/motion/shimmer.rs"]
mod shimmer;
#[path = "../../src/ui/motion/ticker.rs"]
mod ticker;

fn moves() -> bool {
    crate::moves()
}

pub(super) fn caption(cx: &mut gpui_kit::App) -> (gpui_kit::AnyView, Rc<Cell<usize>>) {
    let caption = cx.new(|_| shimmer::Shimmer::new("Думаю"));
    let ticks = Rc::new(Cell::new(0));
    let observed = ticks.clone();
    // App observation records the real ticker notifications without observing
    // the child from its containing Fixture entity (which would redraw it).
    cx.observe(&caption, move |_, _| observed.set(observed.get() + 1))
        .detach();
    (caption.into(), ticks)
}
