// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs
// (shared layout for native and JSON-backed settings).
//! One disclosure heading layout for native forms and extension widgets.
//! Density belongs to its host surface; identity, children and toggling stay
//! with the caller, so sharing the layout does not create shared UI state.

use gpui_kit::component::h_flex;
use gpui_kit::{App, Div, ElementId, InteractiveElement, Stateful, Styled};

use super::Pressable as _;

#[derive(Clone, Copy)]
pub(crate) enum DisclosureDensity {
    Settings,
    Compact,
}

pub(crate) fn disclosure_header(
    id: impl Into<ElementId>,
    density: DisclosureDensity,
    cx: &App,
) -> Stateful<Div> {
    let header = h_flex()
        .id(id)
        .w_full()
        .items_center()
        .gap_2()
        .disclosure_surface(cx);
    match density {
        DisclosureDensity::Settings => header.px_4().py_2p5().text_sm(),
        // Compact hosts retain their inherited text size and existing inset.
        DisclosureDensity::Compact => header.px_2().py_1p5(),
    }
}
