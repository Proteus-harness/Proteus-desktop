// Адаптировано из Zed: crates/ui/src/components/tooltip.rs (bounded title and compact surface).
//! Hover and keyboard-focus help share one anchored overlay and dwell delay.
//! GPUI's own `.tooltip()` opens beside the pointer; this module anchors help
//! to the control. Pointer motion cannot dismiss keyboard-owned help; clicks,
//! activation and Escape hide it until focus changes or Tab advances.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gpui_kit::base::animation::{EffectTransition, ease_in_out_cubic, ease_out_cubic};
use gpui_kit::base::{ElementExt as _, TooltipOverlay, TooltipRequest, TooltipTransition};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, Bounds, ElementId, Entity, FocusHandle, Global,
    InteractiveElement, IntoElement, ParentElement as _, Pixels, Point, SharedString, Styled as _,
    WeakFocusHandle, Window, div, px,
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

#[derive(Clone)]
enum Owner {
    Hover(Bounds<Pixels>),
    Focus {
        handle: WeakFocusHandle,
        bounds: Bounds<Pixels>,
    },
}

thread_local! {
    /// Pressed controls stay quiet until the pointer leaves.
    static PRESSED: Cell<Option<Bounds<Pixels>>> = const { Cell::new(None) };
    /// One trigger owns the overlay. Pointer motion cannot take keyboard help.
    static OWNER: RefCell<Option<Owner>> = const { RefCell::new(None) };
    /// Escape, activation and clicks suppress reappearance on that same focus.
    static SUPPRESSED: RefCell<Option<WeakFocusHandle>> = const { RefCell::new(None) };
}

impl Global for Overlay {}

pub(super) fn init(cx: &mut App) {
    cx.intercept_keystrokes(|event, window, cx| {
        if super::kit::keyboard_input(true) {
            cx.refresh_windows();
        }
        dismiss(window, cx);
        if event.keystroke.key == "tab" {
            SUPPRESSED.replace(None);
        }
    })
    .detach();
}

/// Captured window input invalidates both visible and delayed requests before
/// scrolling, virtualisation or a click can remove the old hover target.
pub(super) fn dismiss(window: &mut Window, cx: &mut App) {
    OWNER.replace(None);
    SUPPRESSED.replace(window.focused(cx).map(|focus| focus.downgrade()));
    PRESSED.set(None);
    update(cx, window, |overlay, _, cx| overlay.hide(cx));
}

pub(super) fn moved(position: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let outside_hover = OWNER.with_borrow(|owner| {
        matches!(owner,
        Some(Owner::Hover(bounds)) if !bounds.contains(&position))
    });
    if outside_hover {
        dismiss(window, cx);
    }
}

fn focus_owns(window: &Window) -> bool {
    OWNER.with_borrow(|owner| match owner {
        Some(Owner::Focus { handle, .. }) => {
            window.is_window_active()
                && super::kit::keyboard()
                && handle
                    .upgrade()
                    .is_some_and(|handle| handle.is_focused(window))
        }
        _ => false,
    })
}

fn show(trigger: Bounds<Pixels>, build: TooltipBuilder, window: &mut Window, cx: &mut App) {
    update(cx, window, move |overlay, window, cx| {
        let request = TooltipRequest::new(trigger, move |window, cx| build(window, cx));
        // Both focus and hover use the existing dwell and animation.
        overlay.hide(cx);
        overlay.request_show(request, window, cx);
    });
}

fn focused(
    handle: &FocusHandle,
    anchor: &TooltipAnchor,
    resolve: &dyn Fn(&mut Window, &mut App) -> Option<TooltipBuilder>,
    window: &mut Window,
    cx: &mut App,
) {
    if !window.is_window_active() || !super::kit::keyboard() || !handle.is_focused(window) {
        return;
    }
    let blocked = SUPPRESSED.with_borrow_mut(|suppressed| {
        if suppressed.as_ref().is_some_and(|old| old == handle) {
            true
        } else {
            *suppressed = None;
            false
        }
    });
    if blocked {
        return;
    }
    let trigger = anchor.bounds();
    let unchanged = OWNER.with_borrow(|owner| {
        matches!(owner,
        Some(Owner::Focus { handle: old, bounds }) if old == handle && *bounds == trigger)
    });
    if unchanged {
        return;
    }
    let build = resolve(window, cx);
    OWNER.replace(Some(Owner::Focus {
        handle: handle.downgrade(),
        bounds: trigger,
    }));
    match build {
        Some(build) => show(trigger, build, window, cx),
        None => update(cx, window, |overlay, _, cx| overlay.hide(cx)),
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
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let stale_focus = OWNER.with_borrow(|owner| matches!(owner, Some(Owner::Focus { .. })))
        && !focus_owns(window);
    if stale_focus {
        OWNER.replace(None);
        window.defer(cx, |window, cx| {
            // A newer focused/hovered control may have claimed the overlay
            // after this stale view was painted.
            if OWNER.with_borrow(Option::is_none) {
                update(cx, window, |overlay, _, cx| overlay.hide(cx));
            }
        });
        return div().into_any_element();
    }
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
        self,
        anchor: TooltipAnchor,
        observer: Option<HoverObserver>,
        resolve: impl Fn(&mut Window, &mut App) -> Option<TooltipBuilder> + 'static,
    ) -> Self {
        self.tip_resolved_observing_focus(anchor, observer, None, resolve)
    }

    fn tip_resolved_observing_focus(
        mut self,
        anchor: TooltipAnchor,
        observer: Option<HoverObserver>,
        focus: Option<FocusHandle>,
        resolve: impl Fn(&mut Window, &mut App) -> Option<TooltipBuilder> + 'static,
    ) -> Self {
        let resolve = Rc::new(resolve);
        let pressed = anchor.clone();
        if let Some(focus) = focus {
            let anchor = anchor.clone();
            let resolve = resolve.clone();
            self = self.on_prepaint(move |_, window, cx| {
                focused(&focus, &anchor, resolve.as_ref(), window, cx)
            });
        }
        // Through interactivity so a disabled control can explain itself.
        self.interactivity().on_hover(move |hovered, window, cx| {
            if let Some(observer) = &observer {
                observer(hovered, window, cx);
            }
            // A moving pointer neither dismisses nor replaces keyboard help.
            if focus_owns(window) {
                return;
            }
            let hovered = *hovered;
            let trigger = anchor.bounds();
            if hovered {
                OWNER.replace(Some(Owner::Hover(trigger)));
            } else {
                let owns = OWNER.with_borrow(
                    |owner| matches!(owner, Some(Owner::Hover(bounds)) if *bounds == trigger),
                );
                if !owns {
                    return;
                }
                OWNER.replace(None);
                PRESSED.set(None);
            }
            if hovered && PRESSED.get() == Some(trigger) {
                return;
            }
            let build = hovered.then(|| resolve(window, cx)).flatten();
            match build {
                Some(build) => show(trigger, build, window, cx),
                None if hovered => update(cx, window, |overlay, _, cx| overlay.hide(cx)),
                None => update(cx, window, |overlay, window, cx| {
                    overlay.request_hide(window, cx)
                }),
            }
        });
        self.capture_any_mouse_down(move |_, window, cx| {
            // Retain the pressed pointer owner so leaving this control can
            // clear its suppression before it is hovered again.
            OWNER.replace(Some(Owner::Hover(pressed.bounds())));
            SUPPRESSED.replace(window.focused(cx).map(|focus| focus.downgrade()));
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
        self.tip_text_observing_focus(observer, None, resolve)
    }

    fn tip_text_observing_focus(
        self,
        observer: Option<HoverObserver>,
        focus: Option<FocusHandle>,
        resolve: impl Fn(&App) -> Option<SharedString> + 'static,
    ) -> Self {
        let anchor = TooltipAnchor::default();
        self.tip_anchor(anchor.clone())
            .tip_resolved_observing_focus(anchor, observer, focus, move |_, cx| {
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
    fn tip_text(self, text: impl Into<SharedString>) -> Self {
        // Plain native-control hints share the same title/description/keys
        // policy as `.hint`; rich document/tool tips keep their own content.
        super::kit::button_hints(self, Some(text.into()), None)
    }

    fn tip_resolved_at(
        self,
        anchor: TooltipAnchor,
        resolve: impl Fn(&mut Window, &mut App) -> Option<TooltipBuilder> + 'static,
    ) -> Self {
        let resolve = Rc::new(resolve);
        self.render_with(move |button, context, _, _| {
            let resolve = resolve.clone();
            button.tip_resolved_observing_focus(
                anchor.clone(),
                context.on_hover,
                Some(context.focus_handle),
                move |window, cx| resolve(window, cx),
            )
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
            button.tip_resolved_observing_focus(
                anchor.clone(),
                Some(combined),
                Some(context.focus_handle),
                move |window, cx| resolve(window, cx),
            )
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
