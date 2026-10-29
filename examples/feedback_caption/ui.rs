//! Import the production animation under its actual visibility boundary.
use gpui_kit::AppContext as _;
#[path = "../../src/ui/motion/shimmer.rs"]
mod shimmer;
#[path = "../../src/ui/motion/ticker.rs"]
mod ticker;

fn moves() -> bool {
    crate::moves()
}

pub(super) fn caption(cx: &mut gpui_kit::App) -> gpui_kit::AnyView {
    cx.new(|_| shimmer::Shimmer::new("Думаю")).into()
}
