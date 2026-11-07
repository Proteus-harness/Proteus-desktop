// Адаптировано из Zed: crates/settings_ui/src/components/input_field.rs
// (thin input frames), crates/editor/src/element/mouse.rs (wheel bubbling).
// Popup surfaces follow Zed: crates/ui/src/traits/styled_ext.rs (elevated).
// Filled buttons follow Zed: crates/ui/src/components/button/button_like.rs.
//! gpui-kit controls as the client builds them. A focused control is ringed
//! only while the keyboard drives the window, so one focused by a click or
//! by the client itself stays plain. Hints are the tooltips of controls,
//! shown as the hints preference asks.

use std::cell::Cell;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::FocusableExt as _;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::{App, ElementId, MouseButton, SharedString, StatefulInteractiveElement, Styled};

use super::prefs;
use super::tip::{Tip as _, TooltipContent};
use crate::preferences::Hints;
use crate::shortcuts::{self, Command};

mod controls;
mod frame;
mod icon_button;
mod selection;
mod setting_reset;
mod settings_row;
mod surface;
pub(crate) use controls::{Clipboard, Switch};
pub(crate) use frame::{Frame, RowRule, record_first_child, surface_hover, surface_selected};
pub(crate) use icon_button::icon_button;
pub(crate) use selection::SelectionCopyMenu;
pub(crate) use setting_reset::setting_reset;
pub(crate) use settings_row::{SettingsRow, settings_row};
pub(crate) use surface::{surface, surface_highlight};

/// Shared button captions and quiet surfaces. Icons, sizes, focus and
/// keyboard behavior stay native.
pub(crate) trait ButtonLabel {
    /// Transparent at rest; the same hover and committed fill as list rows,
    /// without the native Ghost variant's extra dark-mode attenuation.
    fn quiet(self, cx: &App) -> Self;

    /// Keep the full accessible name while the visible line fades instead
    /// of inserting the component's built-in ellipsis.
    fn faded_label(self, label: impl Into<SharedString>) -> Self;
}

impl ButtonLabel for Button {
    fn quiet(self, cx: &App) -> Self {
        self.custom(
            ButtonCustomVariant::new(cx)
                .hover(cx.theme().list_hover)
                .active(cx.theme().sidebar_accent),
        )
    }

    fn faded_label(self, label: impl Into<SharedString>) -> Self {
        use gpui_kit::{IntoElement as _, ParentElement as _, Styled as _, div};
        let label = label.into();
        self.accessibility_label(label.clone())
            .label(label)
            .px_2p5()
            .min_h(prefs::scaled(26.))
            .render_label(|label, clipped, _, _| {
                div()
                    .text_sm()
                    .min_w_0()
                    .child(super::fade::faded(label).report_clipping(clipped))
                    .into_any_element()
            })
    }
}

thread_local! {
    static KEYBOARD: Cell<bool> = const { Cell::new(false) };
}

/// Any keystroke enables keyboard presentation until a pointer press.
/// Pointer movement deliberately has no hook here. Return whether retained
/// windows need redrawing; native focus and input routing are unchanged.
pub(crate) fn keyboard_input(keyboard: bool) -> bool {
    KEYBOARD.replace(keyboard) != keyboard
}

pub(crate) fn keyboard() -> bool {
    KEYBOARD.get()
}

pub(crate) fn button(id: impl Into<ElementId>) -> Button {
    use gpui_kit::IntoElement as _;
    button_hints(
        Button::new(id)
            .border_0()
            .shadow_none()
            .focus_ring(KEYBOARD.get())
            .render_label(|label, clipped, _, _| {
                super::fade::faded(label)
                    .report_clipping(clipped)
                    .into_any_element()
            }),
        None,
        None,
    )
}

pub(crate) fn switch(id: impl Into<ElementId>) -> Switch {
    Switch::new(id).focus_ring(KEYBOARD.get())
}

pub(crate) fn checkbox(id: impl Into<ElementId>) -> Checkbox {
    Checkbox::new(id).focus_ring(KEYBOARD.get())
}

/// Keep the native field's fill and geometry, with no decorative edge.
/// The theme's input color also supplies dark-mode fills, so hide only this
/// instance's border instead of making that shared token transparent.
pub(crate) fn input(state: &gpui_kit::Entity<InputState>) -> Input {
    Input::new(state)
        .focus_bordered(false)
        .border_color(gpui_kit::transparent_black())
}

/// One opaque popup with clipped child paint. Menus share the flat surface of
/// fields and cards, without a decorative outline or outer shadow.
pub(crate) fn popup_surface(cx: &App) -> gpui_kit::Div {
    use gpui_kit::{Styled as _, div};
    div()
        .bg(cx.theme().popover)
        .text_color(cx.theme().popover_foreground)
        .surface_frame(cx)
        .overflow_hidden()
}

/// Preformatted tool output and request previews share their geometry and
/// typography; callers supply selectable content and semantic error colors.
pub(crate) fn code_surface(cx: &App) -> gpui_kit::Div {
    let theme = cx.theme();
    gpui_kit::div()
        .w_full()
        .px_3()
        .py_2()
        .rounded(theme.radius_lg)
        .bg(theme.muted)
        .font_family(theme.mono_font_family.clone())
        .text_xs()
}

/// Textarea has no public focus-ring opt-out. Keep its native appearance and
/// reserve the shared transparent edge after disabling its bordered focus style.
pub(crate) fn textarea(state: &gpui_kit::Entity<TextareaState>, cx: &App) -> Textarea {
    Textarea::new(state).bordered(false).frame(cx)
}

/// Let the descendant editor consume wheel motion first. At either edge,
/// contain its unconsumed bubble instead of scrolling the surrounding page.
/// No default prevention: native scrolling, selection and drag are unchanged.
pub(crate) fn contain_scroll(content: impl gpui_kit::IntoElement) -> gpui_kit::Div {
    use gpui_kit::{InteractiveElement as _, ParentElement as _, div};
    div()
        .w_full()
        .min_w_0()
        .on_scroll(|_, _, cx| cx.stop_propagation())
        .child(content)
}

/// A clickable row or card that Tab reaches like a button: Enter or Space
/// presses it, and it is marked while the keyboard holds focus on it. A
/// mouse press leaves focus where it was, as with buttons.
pub(crate) trait Pressable: StatefulInteractiveElement + Styled {
    /// Quiet tool headings brighten their text without adding a fill; child
    /// status/error colors and the expanded body keep their own appearance.
    fn disclosure(self, cx: &App) -> Self {
        let theme = cx.theme();
        self.pressable(cx)
            .cursor_pointer()
            .rounded(theme.radius)
            .text_color(theme.muted_foreground)
            .hover(|style| style.text_color(theme.foreground))
    }

    /// Bounded form disclosures also highlight their shared surface.
    fn disclosure_surface(self, cx: &App) -> Self
    where
        Self: gpui_kit::ParentElement,
    {
        self.disclosure(cx)
            .relative()
            .rounded(cx.theme().radius_lg)
            .group("disclosure-surface")
            .child(surface_highlight("disclosure-surface", cx))
    }

    fn pressable(self, cx: &App) -> Self {
        let theme = cx.theme();
        let ring = theme.ring;
        let hover = theme.list_hover;
        let foreground = theme.foreground;
        let row = self.focusable().tab_stop(true);
        let row = if keyboard() {
            row.focus(move |style| {
                // Zed ButtonLike uses the existing hover treatment for focus.
                // This remains visible on borderless rows without adding
                // outside shadows, changing corners or shifting their layout.
                style.border_color(ring).bg(hover).text_color(foreground)
            })
        } else {
            row
        };
        row.on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
    }
}

impl<E: StatefulInteractiveElement + Styled> Pressable for E {}

/// Moves focus to the next or the previous stop inside `container`, coming
/// round from its end to its start.
pub(crate) fn cycle_focus(
    container: &gpui_kit::FocusHandle,
    forward: bool,
    window: &mut gpui_kit::Window,
    cx: &mut App,
) {
    let start = window.focused(cx);
    // The container itself is no stop, so a walk from it never comes back
    // to it when it holds no stops either.
    for _ in 0..256 {
        if forward {
            window.focus_next(cx);
        } else {
            window.focus_prev(cx);
        }
        let focused = window.focused(cx);
        if container.contains_focused(window, cx) || focused == start {
            return;
        }
    }
}

/// Moves focus to the next or the previous stop inside `container`; at its
/// end the focus stays where it was, and the result says whether it moved.
pub(crate) fn step_focus(
    container: &gpui_kit::FocusHandle,
    forward: bool,
    window: &mut gpui_kit::Window,
    cx: &mut App,
) -> bool {
    let start = window.focused(cx);
    if forward {
        window.focus_next(cx);
    } else {
        window.focus_prev(cx);
    }
    if container.contains_focused(window, cx) && window.focused(cx) != start {
        return true;
    }
    if let Some(start) = start {
        window.focus(&start, cx);
    }
    false
}

/// Filter independent semantic parts without flattening their visual roles.
fn tooltip_content(
    hints: Hints,
    title: Option<(&str, bool)>,
    description: Option<&str>,
    keys: Option<&str>,
) -> Option<TooltipContent> {
    let title = title
        .filter(|(text, clipped)| !text.is_empty() && hints.title.shows(*clipped))
        .map(|(text, _)| text);
    let description =
        description.filter(|text| hints.description && !text.is_empty() && Some(*text) != title);
    let shortcut = keys.filter(|text| hints.shortcut && !text.is_empty());
    (title.is_some() || description.is_some() || shortcut.is_some()).then(|| TooltipContent {
        title: title.map(|text| text.to_owned().into()),
        description: description.map(|text| text.to_owned().into()),
        shortcut: shortcut.map(|text| text.to_owned().into()),
        context: None,
    })
}

/// Configuration paths are secondary semantic context, governed by the
/// description flag rather than the keyboard-shortcut flag.
pub(crate) fn caption_hint_content(
    hints: Hints,
    title: &str,
    clipped: bool,
    description: Option<&str>,
    context: Option<&str>,
) -> Option<TooltipContent> {
    let content = tooltip_content(hints, Some((title, clipped)), description, None);
    let context = context.filter(|context| hints.description && !context.is_empty());
    if content.is_none() && context.is_none() {
        return None;
    }
    Some(TooltipContent {
        context: context.map(|text| text.to_owned().into()),
        ..content.unwrap_or_default()
    })
}

fn hint_content(text: &str, keys: Option<&str>, cx: &App) -> Option<TooltipContent> {
    tooltip_content(prefs::current(cx).hints, None, Some(text), keys)
}

fn button_title<'a>(
    visible: Option<&'a str>,
    clipped: bool,
    accessible: Option<&'a str>,
) -> Option<(&'a str, bool)> {
    visible
        .map(|label| (label, clipped))
        // An icon-only name is absent from the visible caption, so the
        // default clipped-title policy must still make it discoverable.
        .or_else(|| accessible.map(|name| (name, true)))
}

pub(super) fn button_hints(
    button: Button,
    description: Option<SharedString>,
    keys: Option<SharedString>,
) -> Button {
    button.render_with(move |button, context, _, _| {
        let description = description.clone();
        let keys = keys.clone();
        button.tip_content_observing_focus(
            context.on_hover.clone(),
            Some(context.focus_handle.clone()),
            move |cx| {
                tooltip_content(
                    prefs::current(cx).hints,
                    button_title(
                        context.label.as_deref(),
                        context.label_clipped.get(),
                        context.accessibility_label.as_deref(),
                    ),
                    description.as_deref(),
                    keys.as_deref(),
                )
            },
        )
    })
}

/// The first shortcut of a window command, as people read it.
pub(crate) fn command_keys(command: Command, cx: &App) -> Option<String> {
    let chords = prefs::current(cx).shortcuts.chords(command);
    chords.first().map(|chord| shortcuts::label(chord))
}

pub(crate) trait Hint: Sized {
    /// A tooltip saying what the control does and how to reach it from
    /// the keyboard, filtered by the hints preference.
    fn hint_keys(self, text: &str, keys: Option<&str>, cx: &App) -> Self;

    fn hint(self, text: &str, cx: &App) -> Self {
        self.hint_keys(text, None, cx)
    }
}

impl Hint for Button {
    fn hint_keys(self, text: &str, keys: Option<&str>, _: &App) -> Self {
        button_hints(
            self,
            Some(text.to_owned().into()),
            keys.map(|keys| keys.to_owned().into()),
        )
    }
}

impl Hint for Switch {
    fn hint_keys(self, text: &str, keys: Option<&str>, cx: &App) -> Self {
        match hint_content(text, keys, cx) {
            Some(content) => self.tooltip_content(content),
            None => self,
        }
    }
}

impl Hint for Clipboard {
    fn hint_keys(self, text: &str, keys: Option<&str>, cx: &App) -> Self {
        match hint_content(text, keys, cx) {
            Some(content) => self.tooltip_content(content),
            None => self,
        }
    }
}

#[cfg(test)]
mod hint_tests {
    use super::{button_title, caption_hint_content, tooltip_content};
    use crate::preferences::{Hints, TitleHint};
    use crate::ui::tip::TooltipContent;

    fn content(
        title: Option<&str>,
        description: Option<&str>,
        shortcut: Option<&str>,
    ) -> TooltipContent {
        TooltipContent {
            title: title.map(Into::into),
            description: description.map(Into::into),
            shortcut: shortcut.map(Into::into),
            context: None,
        }
    }

    #[test]
    fn tooltip_parts_can_be_enabled_independently() {
        for (title, description, shortcut) in [
            (false, false, false),
            (false, false, true),
            (false, true, false),
            (false, true, true),
            (true, false, false),
            (true, false, true),
            (true, true, false),
            (true, true, true),
        ] {
            let parts = tooltip_content(
                Hints {
                    title: if title {
                        TitleHint::Always
                    } else {
                        TitleHint::Never
                    },
                    description,
                    shortcut,
                },
                Some(("Сохранить", false)),
                Some("Сохранить настройки"),
                Some("Ctrl+Enter"),
            );
            if !title && !description && !shortcut {
                assert_eq!(parts, None);
                continue;
            }
            assert_eq!(
                parts,
                Some(content(
                    title.then_some("Сохранить"),
                    description.then_some("Сохранить настройки"),
                    shortcut.then_some("Ctrl+Enter"),
                ))
            );
        }
    }

    #[test]
    fn icon_names_remain_discoverable_without_overriding_visible_labels() {
        let name = button_title(None, false, Some("Новый чат"));
        assert_eq!(
            tooltip_content(Hints::default(), name, None, None),
            Some(content(Some("Новый чат"), None, None))
        );
        assert_eq!(
            tooltip_content(
                Hints {
                    title: TitleHint::Never,
                    ..Hints::default()
                },
                name,
                None,
                None
            ),
            None
        );
        assert_eq!(
            button_title(Some("Сохранить"), false, Some("Сохранить изменения")),
            Some(("Сохранить", false))
        );
        assert_eq!(button_title(None, false, None), None);
    }

    #[test]
    fn setting_context_follows_description_and_not_shortcut() {
        let hints = Hints {
            title: TitleHint::Never,
            description: true,
            shortcut: false,
        };
        assert_eq!(
            caption_hint_content(
                hints,
                "Модель",
                false,
                Some("Выбор модели"),
                Some("model.name")
            ),
            Some(TooltipContent {
                context: Some("model.name".into()),
                ..content(None, Some("Выбор модели"), None)
            })
        );
        assert_eq!(
            caption_hint_content(
                Hints {
                    description: false,
                    shortcut: true,
                    ..hints
                },
                "Модель",
                true,
                Some("Выбор модели"),
                Some("model.name")
            ),
            None
        );
        assert_eq!(
            caption_hint_content(
                Hints {
                    title: TitleHint::Always,
                    description: false,
                    ..hints
                },
                "Модель",
                false,
                Some("Выбор модели"),
                Some("model.name")
            ),
            Some(content(Some("Модель"), None, None))
        );
    }

    #[test]
    fn clipped_title_and_matching_description_form_one_tooltip() {
        let hints = Hints::default();
        assert_eq!(
            tooltip_content(hints, Some(("Сохранить", false)), None, None),
            None
        );
        assert_eq!(
            tooltip_content(hints, Some(("Сохранить", true)), None, None),
            Some(content(Some("Сохранить"), None, None))
        );
        assert_eq!(
            tooltip_content(
                hints,
                Some(("Сохранить", true)),
                Some("Сохранить"),
                Some("Ctrl+S")
            ),
            Some(content(Some("Сохранить"), None, Some("Ctrl+S")))
        );
        assert_eq!(
            tooltip_content(hints, Some(("", true)), Some(""), Some("")),
            None
        );
        assert_eq!(
            tooltip_content(
                Hints {
                    title: TitleHint::Never,
                    description: false,
                    shortcut: true
                },
                None,
                Some("Сохранить"),
                None
            ),
            None
        );
    }

    #[test]
    fn multiline_description_remains_description_rather_than_becoming_a_title() {
        assert_eq!(
            tooltip_content(
                Hints {
                    title: TitleHint::Always,
                    ..Hints::default()
                },
                Some(("Модель", false)),
                Some("Первая строка\nВторая строка"),
                Some("Ctrl+M")
            ),
            Some(content(
                Some("Модель"),
                Some("Первая строка\nВторая строка"),
                Some("Ctrl+M")
            ))
        );
    }
}
