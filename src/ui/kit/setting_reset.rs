// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs (reset-to-default-btn).
//! One quiet reset action beside a setting caption. Callers expose it only
//! for an override and own the meaning of default, inheritance or draft reset.

use gpui_kit::component::*;
use gpui_kit::*;

use super::{ButtonLabel as _, Hint as _};
use crate::ui::prefs;

pub(crate) fn setting_reset(
    id: ElementId,
    hint: impl Into<SharedString>,
    reset: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let hint: SharedString = hint.into();
    super::button(id)
        .icon(IconName::Undo2)
        .quiet(cx)
        .xsmall()
        .size(prefs::scaled(20.))
        .p_0()
        .flex_none()
        .accessibility_label(hint.clone())
        .hint(hint.as_ref(), cx)
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            // The reset mark disappears at the default value. Move keyboard
            // focus to a surviving control before unmounting its owner.
            if matches!(event, ClickEvent::Keyboard(_)) {
                window.focus_next(cx);
            }
            reset(window, cx);
        })
        .into_any_element()
}
