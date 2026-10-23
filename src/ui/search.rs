// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs (render_search, clear_search), crates/git_ui/src/git_graph.rs (search focus)
//! Inline toolbar search. The caller keeps its InputState and filtering;
//! this control owns the compact trigger, quiet field and Esc behavior.
//! An inactive empty field occupies only one icon, never another toolbar row.
//! All search surfaces, including popup pickers, use `field` for the same
//! quiet appearance with a thin border and no focus ring. Header titles
//! yield space to the field throughout the shared opening/closing animation.

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::kit::{self, Hint as _};
use super::{motion, prefs};

pub(in crate::ui) fn field(input: &Entity<InputState>) -> Input {
    Input::new(input).focus_bordered(false)
}

#[derive(IntoElement)]
pub(in crate::ui) struct Search {
    id: ElementId,
    input: Entity<InputState>,
    restore: FocusHandle,
    hint: SharedString,
    title: Option<SharedString>,
}

impl Search {
    pub fn new(
        id: impl Into<ElementId>,
        input: Entity<InputState>,
        restore: FocusHandle,
        hint: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            input,
            restore,
            hint: hint.into(),
            title: None,
        }
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }
}

/// Explicit search commands work even while its field is not mounted.
/// Opening does not clear the query or move the text cursor unnecessarily.
pub(in crate::ui) fn focus(input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    input.update(cx, |input, cx| input.focus(window, cx));
    window.refresh();
}

fn active(input: &Entity<InputState>, window: &Window, cx: &App) -> bool {
    let input = input.read(cx);
    input.focus_handle(cx).is_focused(window) || !input.value().is_empty()
}

impl RenderOnce for Search {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let open = active(&self.input, window, cx);
        let input = self.input.clone();
        let trigger = kit::button(ElementId::Name(format!("{:?}:trigger", self.id).into()))
            .ghost()
            .xsmall()
            .icon(IconName::Search)
            .selected(open)
            .hint_keys(&self.hint, Some("Ctrl+F"), cx)
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                focus(&input, window, cx);
            });
        let progress = motion::glide(
            ElementId::Name(format!("{:?}:reveal", self.id).into()),
            open,
            motion::SLOW,
            window,
            cx,
        );
        let has_title = self.title.is_some();
        let mut row = h_flex()
            .id(self.id)
            .min_w_0()
            .h(prefs::scaled(28.))
            .max_w_full()
            .items_center()
            .when(has_title, |row| row.flex_1())
            .when(!has_title, |row| row.flex_none().max_w(prefs::scaled(240.)))
            .when_some(self.title, |row, title| {
                row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .opacity(1. - progress)
                        .text_sm()
                        .font_semibold()
                        .child(super::fade::faded(title)),
                )
            })
            .child(trigger);
        if open || progress > 0. {
            let input = self.input.clone();
            let restore = self.restore;
            row = row
                .child(
                    div()
                        .w(prefs::scaled(200. * progress))
                        .h_full()
                        .min_w_0()
                        .overflow_hidden()
                        .child(field(&self.input).small().w_full().cleanable(true)),
                )
                .on_action(move |_: &Escape, window, cx| {
                    if !input.read(cx).value().is_empty() {
                        input.update(cx, |input, cx| {
                            input.set_value("", window, cx);
                            cx.emit(InputEvent::Change);
                        });
                    } else {
                        restore.focus(window, cx);
                    }
                    window.refresh();
                    cx.stop_propagation();
                });
        }
        row
    }
}
