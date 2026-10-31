// Адаптировано из Zed: crates/ui/src/components/tooltip.rs (bounded title and compact surface).
//! Hover tooltips anchored to the hovered element. GPUI's own `.tooltip()`
//! opens next to the pointer; this module anchors tooltips to the trigger
//! through one overlay of our own, for plain elements and gpui-kit buttons
//! alike, and hides them on a press of any mouse button.

use std::{cell::Cell, rc::Rc, time::Duration};

use gpui_kit::base::animation::{EffectTransition, ease_in_out_cubic, ease_out_cubic};
use gpui_kit::base::{ElementExt as _, TooltipOverlay, TooltipRequest, TooltipTransition};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, Bounds, ElementId, Entity, Global,
    InteractiveElement, IntoElement, ParentElement as _, Pixels, Point, SharedString, Styled as _,
    Window, div, px,
};

use super::{motion, prefs};

const ENTER: Duration = Duration::from_millis(150);
const SLIDE: Duration = Duration::from_millis(200);
/// A full caption wraps like Zed's tooltip title rather than widening to
/// its unconstrained single-line width.
const MAX_WIDTH: f32 = 420.;

/// Bounds of the complete control that owns a nested caption's tooltip.
/// The parent measures itself once; children may use that same stable anchor.
#[derive(Clone, Default)]
pub(crate) struct TooltipAnchor(Rc<Cell<Bounds<Pixels>>>);

impl TooltipAnchor {
    fn bounds(&self) -> Bounds<Pixels> {
        self.0.get()
    }
}

type HoverObserver = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

type TooltipBuilder = Rc<dyn Fn(&mut Window, &mut App) -> AnyView>;

struct Overlay(Entity<TooltipOverlay>);

thread_local! {
    /// The element pressed last; its tooltip stays away until the pointer leaves.
    static PRESSED: Cell<Option<Bounds<Pixels>>> = const { Cell::new(None) };
    /// Hover callbacks for adjacent controls can arrive enter-before-leave.
    /// A late leave may hide only its own request, never the new trigger's.
    static HOVERED: Cell<Option<Bounds<Pixels>>> = const { Cell::new(None) };
}

impl Global for Overlay {}

pub(super) fn init(cx: &mut App) {
    cx.intercept_keystrokes(|_, window, cx| dismiss(window, cx))
        .detach();
}

/// Captured window input invalidates both visible and delayed requests before
/// scrolling, virtualisation or a click can remove the old hover target.
pub(super) fn dismiss(window: &mut Window, cx: &mut App) {
    HOVERED.set(None);
    PRESSED.set(None);
    update(cx, window, |overlay, _, cx| overlay.hide(cx));
}

pub(super) fn moved(position: Point<Pixels>, window: &mut Window, cx: &mut App) {
    if HOVERED
        .get()
        .is_some_and(|bounds| !bounds.contains(&position))
    {
        dismiss(window, cx);
    }
}

/// The overlay that draws the open tooltip; render it once at the window root.
pub(super) fn layer(cx: &mut App) -> Entity<TooltipOverlay> {
    if let Some(overlay) = cx.try_global::<Overlay>() {
        return overlay.0.clone();
    }
    let overlay = cx.new(|_| TooltipOverlay::new().render_with(present));
    cx.set_global(Overlay(overlay.clone()));
    overlay
}

fn present(
    view: AnyView,
    transition: TooltipTransition,
    _: &mut Window,
    _: &mut App,
) -> AnyElement {
    let element = div()
        .p(prefs::scaled(4.))
        .max_w(prefs::scaled(MAX_WIDTH + 8.))
        .child(view);
    if !motion::plays() {
        return element.into_any_element();
    }
    match transition {
        TooltipTransition::Switch {
            epoch,
            previous,
            current,
        } => {
            if (current.origin.y - previous.origin.y).abs() >= px(10.) {
                return element.into_any_element();
            }
            let shift = current.center().x - previous.center().x;
            EffectTransition::new(SLIDE)
                .ease(ease_in_out_cubic)
                .slide_x(-shift, px(0.))
                .apply(
                    element,
                    ElementId::NamedInteger("tip-slide".into(), epoch as u64),
                )
                .into_any_element()
        }
        TooltipTransition::Enter { epoch } => EffectTransition::new(ENTER)
            .ease(ease_out_cubic)
            .slide_y(px(4.), px(0.))
            .fade(0., 1.)
            .apply(
                element,
                ElementId::NamedInteger("tip-enter".into(), epoch as u64),
            )
            .into_any_element(),
    }
}

fn update(
    cx: &mut App,
    window: &mut Window,
    f: impl FnOnce(&mut TooltipOverlay, &mut Window, &mut gpui_kit::Context<TooltipOverlay>),
) {
    if let Some(overlay) = cx.try_global::<Overlay>().map(|overlay| overlay.0.clone()) {
        overlay.update(cx, |overlay, cx| f(overlay, window, cx));
    }
}

/// Tooltips of plain elements and of gpui-kit buttons alike: every tooltip of
/// the client goes through the one overlay, so they look and behave the same.
pub(crate) trait Tip: InteractiveElement + gpui_kit::ParentElement + Sized {
    /// Measures this control for tooltips requested by a caption inside it.
    fn tip_anchor(self, anchor: TooltipAnchor) -> Self {
        self.on_prepaint(move |rect, _, _| anchor.0.set(rect))
    }

    /// Shows `build` beside this element while the pointer rests on it. A
    /// press of any button hides it until the pointer leaves, so it never
    /// covers the menu or drag the press starts.
    fn tip(self, build: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        let anchor = TooltipAnchor::default();
        self.tip_anchor(anchor.clone()).tip_at(anchor, build)
    }

    /// A nested trigger whose tooltip belongs to a larger control. Unlike
    /// `tip`, this does not overwrite the parent's measured anchor.
    fn tip_at(
        self,
        anchor: TooltipAnchor,
        build: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        let build: TooltipBuilder = Rc::new(build);
        self.tip_resolved_at(anchor, move |_, _| Some(build.clone()))
    }

    /// Resolve an optional tooltip on hover, then build it only after the
    /// normal dwell delay. An absent result never creates an empty overlay.
    fn tip_resolved_at(
        self,
        anchor: TooltipAnchor,
        resolve: impl Fn(&mut Window, &mut App) -> Option<TooltipBuilder> + 'static,
    ) -> Self {
        self.tip_resolved_observing(anchor, None, resolve)
    }

    fn tip_resolved_observing(
        mut self,
        anchor: TooltipAnchor,
        observer: Option<HoverObserver>,
        resolve: impl Fn(&mut Window, &mut App) -> Option<TooltipBuilder> + 'static,
    ) -> Self {
        let resolve = Rc::new(resolve);
        let pressed = anchor.clone();
        // Through interactivity so a disabled control can explain itself.
        self.interactivity().on_hover(move |hovered, window, cx| {
            if let Some(observer) = &observer {
                observer(hovered, window, cx);
            }
            let hovered = *hovered;
            let trigger = anchor.bounds();
            if hovered {
                HOVERED.set(Some(trigger));
            } else if HOVERED.get() != Some(trigger) {
                return;
            } else {
                HOVERED.set(None);
                PRESSED.set(None);
            }
            if hovered && PRESSED.get() == Some(trigger) {
                return;
            }
            let build = hovered.then(|| resolve(window, cx)).flatten();
            update(cx, window, move |overlay, window, cx| {
                if let Some(build) = build {
                    let request = TooltipRequest::new(trigger, move |window, cx| build(window, cx));
                    // Every new trigger earns its own dwell time.
                    overlay.hide(cx);
                    overlay.request_show(request, window, cx);
                } else if hovered {
                    overlay.hide(cx);
                } else {
                    overlay.request_hide(window, cx);
                }
            });
        });
        self.capture_any_mouse_down(move |_, window, cx| {
            PRESSED.set(Some(pressed.bounds()));
            update(cx, window, |overlay, _, cx| overlay.hide(cx));
        })
    }

    /// The text can depend on layout or preferences. Snapshot a nonempty
    /// result when entering the control, before scheduling its tooltip.
    fn tip_text_when(self, resolve: impl Fn(&App) -> Option<SharedString> + 'static) -> Self {
        self.tip_text_observing(None, resolve)
    }

    fn tip_text_observing(
        self,
        observer: Option<HoverObserver>,
        resolve: impl Fn(&App) -> Option<SharedString> + 'static,
    ) -> Self {
        let anchor = TooltipAnchor::default();
        self.tip_anchor(anchor.clone())
            .tip_resolved_observing(anchor, observer, move |_, cx| {
                let text = resolve(cx)?;
                if text.is_empty() {
                    return None;
                }
                Some(Rc::new(move |window, cx| {
                    text_view(text.clone(), window, cx)
                }))
            })
    }

    fn tip_text_when_at(
        self,
        anchor: TooltipAnchor,
        resolve: impl Fn(&App) -> Option<SharedString> + 'static,
    ) -> Self {
        self.tip_resolved_at(anchor, move |_, cx| {
            let text = resolve(cx)?;
            if text.is_empty() {
                return None;
            }
            Some(Rc::new(move |window, cx| {
                text_view(text.clone(), window, cx)
            }))
        })
    }

    /// A plain text tooltip.
    fn tip_text(self, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        self.tip(move |window, cx| text_view(text.clone(), window, cx))
    }

    /// A tooltip built from an element on every show.
    fn tip_element<E: IntoElement>(
        self,
        body: impl Fn(&mut Window, &mut App) -> E + 'static,
    ) -> Self {
        let body = Rc::new(body);
        self.tip(move |window, cx| {
            let body = body.clone();
            Tooltip::element(move |window, cx| {
                div()
                    .min_w_0()
                    .max_w(prefs::scaled(MAX_WIDTH - 20.))
                    .whitespace_normal()
                    .child(body(window, cx))
            })
            .m_0()
            .max_w(prefs::scaled(MAX_WIDTH))
            .whitespace_normal()
            .build(window, cx)
        })
    }
}

impl Tip for gpui_kit::Div {}
impl Tip for gpui_kit::Stateful<gpui_kit::Div> {}
impl Tip for gpui_kit::base::Button {}

// The native component already has a default application decorator. A direct
// rich tooltip replaces that same owner rather than attaching a second hover
// listener before native render. This also preserves `.hint`/`.tip` call order.
impl Tip for gpui_kit::component::button::Button {
    fn tip_resolved_at(
        self,
        anchor: TooltipAnchor,
        resolve: impl Fn(&mut Window, &mut App) -> Option<TooltipBuilder> + 'static,
    ) -> Self {
        let resolve = Rc::new(resolve);
        self.render_with(move |button, context, _, _| {
            let resolve = resolve.clone();
            button.tip_resolved_observing(anchor.clone(), context.on_hover, move |window, cx| {
                resolve(window, cx)
            })
        })
    }

    fn tip_resolved_observing(
        self,
        anchor: TooltipAnchor,
        observer: Option<HoverObserver>,
        resolve: impl Fn(&mut Window, &mut App) -> Option<TooltipBuilder> + 'static,
    ) -> Self {
        let resolve = Rc::new(resolve);
        self.render_with(move |button, context, _, _| {
            let resolve = resolve.clone();
            let native = context.on_hover;
            let observer = observer.clone();
            let combined: HoverObserver = Rc::new(move |hovered, window, cx| {
                if let Some(native) = &native {
                    native(hovered, window, cx);
                }
                if let Some(observer) = &observer {
                    observer(hovered, window, cx);
                }
            });
            button.tip_resolved_observing(anchor.clone(), Some(combined), move |window, cx| {
                resolve(window, cx)
            })
        })
    }
}

fn text_view(text: SharedString, window: &mut Window, cx: &mut App) -> AnyView {
    Tooltip::element(move |_, _| {
        div()
            .min_w_0()
            .max_w(prefs::scaled(MAX_WIDTH - 20.))
            .whitespace_normal()
            .child(text.clone())
    })
    .m_0()
    .max_w(prefs::scaled(MAX_WIDTH))
    .whitespace_normal()
    .build(window, cx)
}
