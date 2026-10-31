// Адаптировано из Zed: crates/ui/src/components/button/icon_button.rs (square shape).
use gpui_kit::{ElementId, Styled as _, component::Sizable as _, component::button::Button};

/// A square icon action, including controls with custom icon children and
/// tooltip measurement decorations. Those children must not opt the native
/// Button into caption padding. Equivalent to Zed's IconButtonShape::Square;
/// native focus, disabled state and activation remain unchanged.
pub(crate) fn icon_button(id: impl Into<ElementId>) -> Button {
    super::button(id).small().size_6().p_0()
}
