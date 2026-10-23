// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs (render_search, clear_search), crates/git_ui/src/git_graph.rs (search focus)
//! Inline toolbar search. The caller keeps its InputState and filtering;
//! this control owns the compact trigger, borderless field and Esc behavior.
//! An inactive empty field occupies only one icon, never another toolbar row.

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::*;
use gpui_kit::*;

use super::kit::{self, Hint as _};
use super::{motion, prefs};

#[derive(IntoElement)]
pub(in crate::ui) struct Search {
    id: ElementId,
    input: Entity<InputState>,
    restore: FocusHandle,
    hint: SharedString,
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
        }
    }
}

/// Explicit search commands work even while its field is not mounted.
/// Opening does not clear the query or move the text cursor unnecessarily.
pub(in crate::ui) fn focus(input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    input.update(cx, |input, cx| input.focus(window, cx));
    window.refresh();
}

/// Active search may replace a header title without reserving a second row.
pub(in crate::ui) fn active(input: &Entity<InputState>, window: &Window, cx: &App) -> bool {
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
        let mut row = h_flex()
            .id(self.id)
            .min_w_0()
            .h(prefs::scaled(28.))
            .max_w_full()
            .items_center()
            .child(trigger);
        if open || progress > 0. {
            let input = self.input.clone();
            let restore = self.restore;
            let field = div().min_w_0().w_full().child(
                Input::new(&self.input)
                    .small()
                    .w_full()
                    .appearance(false)
                    .cleanable(true),
            );
            row = row
                .flex_1()
                .max_w(prefs::scaled(240.))
                .child(
                    motion::Collapse::new(
                        "search-field",
                        progress,
                        Field {
                            child: field.into_any_element(),
                        }
                        .into_any_element(),
                    )
                    .squeeze(prefs::scaled(0.), prefs::scaled(200.)),
                )
                .capture_key_down(move |event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key != "escape" || event.keystroke.modifiers.modified() {
                        return;
                    }
                    if !input.read(cx).value().is_empty() {
                        input.update(cx, |input, cx| input.set_value("", window, cx));
                    } else {
                        restore.focus(window, cx);
                    }
                    window.refresh();
                    cx.stop_propagation();
                });
        } else {
            row = row.flex_none();
        }
        row
    }
}

/// Gives a percentage-width native input an intrinsic toolbar width for the
/// shared horizontal reveal, then lays it out at the actual squeezed width.
/// The text cursor and its scroll viewport therefore never use a clipped,
/// imaginary fixed-width input on narrow toolbars.
struct Field {
    child: AnyElement,
}

impl IntoElement for Field {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Field {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, ()) {
        let preferred = prefs::scaled(200.).to_pixels(window.rem_size());
        let height = prefs::scaled(28.).to_pixels(window.rem_size());
        let style = Style::default();
        (
            window.request_measured_layout(style, move |known, available, _, _| {
                let width = known.width.unwrap_or(match available.width {
                    AvailableSpace::Definite(width) => width.min(preferred),
                    _ => preferred,
                });
                size(width, height)
            }),
            (),
        )
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.layout_as_root(
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::Definite(bounds.size.height),
            ),
            window,
            cx,
        );
        self.child.prepaint_at(bounds.origin, window, cx);
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}
