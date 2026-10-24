//! Popup presence shared by native menus, project pickers and process views.
//! Logical dismissal restores focus immediately. Only the inert drawing is
//! retained until its reversible reveal/fade reaches zero; no removal timer
//! can race a freshly reopened popup.

use std::{cell::Cell, rc::Rc, time::Instant};

use gpui_kit::base::{Align, POPUP_PRIORITY, Placement, Positioner, ResolvedPosition};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{PopupOrigin, SLOW, ease_enter};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Effect {
    Reveal,
    Fade,
    None,
}

fn effect(moves: bool, plays: bool, reduced: bool) -> Effect {
    if !plays || reduced {
        Effect::None
    } else if moves {
        Effect::Reveal
    } else {
        Effect::Fade
    }
}

#[derive(Clone)]
pub(in crate::ui) struct PopupFrame {
    pub open: bool,
    placement: Placement,
    align: Align,
    gap: Pixels,
    attachment: Option<PopupPanel>,
    progress: f32,
    effect: Effect,
    anchor: Bounds<Pixels>,
    position: Rc<Cell<Option<ResolvedPosition>>>,
}

impl PopupFrame {
    /// Join this popup to a panel edge without covering either frame.
    pub fn attached(mut self, panel: PopupPanel) -> Self {
        self.placement = panel.placement;
        self.align = panel.align;
        self.gap = px(0.);
        self.attachment = Some(panel);
        self
    }
}

/// Geometry and presentation of a panel that can open an attached popup.
#[derive(Clone, Copy)]
pub(in crate::ui) struct PopupPanel {
    pub bounds: Bounds<Pixels>,
    pub placement: Placement,
    pub align: Align,
    pub background: Hsla,
}

impl PopupPanel {
    /// Extend the trigger across the panel's normal axis while retaining its
    /// alignment along the popup's other axis.
    pub fn anchor(self, mut trigger: Bounds<Pixels>) -> Bounds<Pixels> {
        match self.placement {
            Placement::Top | Placement::Bottom => {
                trigger.origin.y = self.bounds.origin.y;
                trigger.size.height = self.bounds.size.height;
            }
            Placement::Left | Placement::Right => {
                trigger.origin.x = self.bounds.origin.x;
                trigger.size.width = self.bounds.size.width;
            }
        }
        trigger
    }
}

fn destination(open: bool) -> f32 {
    if open { 1. } else { 0. }
}

struct Presence {
    open: bool,
    from: f32,
    at: Instant,
}

impl Default for Presence {
    fn default() -> Self {
        Self {
            open: false,
            from: 0.,
            at: Instant::now(),
        }
    }
}

impl Presence {
    fn sample(&self, now: Instant) -> f32 {
        let target = destination(self.open);
        let distance = (target - self.from).abs();
        if distance == 0. {
            return target;
        }
        let duration = SLOW.mul_f32(distance);
        let progress =
            now.saturating_duration_since(self.at).as_secs_f32() / duration.as_secs_f32();
        self.from + (target - self.from) * ease_enter(progress.min(1.))
    }

    fn target(&mut self, open: bool, now: Instant) -> bool {
        if self.open == open {
            return false;
        }
        self.from = self.sample(now);
        self.at = now;
        self.open = open;
        true
    }
}

pub(in crate::ui) struct Popup<T> {
    value: Option<T>,
    presence: Presence,
    generation: u64,
    anchor: Option<Bounds<Pixels>>,
    position: Rc<Cell<Option<ResolvedPosition>>>,
}

impl<T> Default for Popup<T> {
    fn default() -> Self {
        Self {
            value: None,
            presence: Presence::default(),
            generation: 0,
            anchor: None,
            position: Rc::default(),
        }
    }
}

impl<T> Popup<T> {
    pub fn is_open(&self) -> bool {
        self.value.is_some() && self.presence.open
    }
    pub fn value(&self) -> Option<&T> {
        self.value.as_ref()
    }
    pub fn current(&self) -> Option<&T> {
        self.is_open().then(|| self.value.as_ref()).flatten()
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn is_current(&self, generation: u64) -> bool {
        self.is_open() && self.generation == generation
    }

    pub fn show(&mut self, value: T) -> u64 {
        self.show_at(value, Instant::now())
    }
    fn show_at(&mut self, value: T, now: Instant) -> u64 {
        self.value = Some(value);
        self.anchor = None;
        self.position = Rc::default();
        self.generation = self.generation.wrapping_add(1);
        self.presence.target(true, now);
        self.generation
    }

    pub fn close(&mut self) -> bool {
        self.close_at(Instant::now())
    }
    fn close_at(&mut self, now: Instant) -> bool {
        self.value.is_some() && self.presence.target(false, now)
    }

    pub fn close_if(&mut self, generation: u64) -> bool {
        self.is_current(generation) && self.close()
    }

    /// The owner/anchor disappeared, rather than a normal dismissal.
    pub fn clear(&mut self) {
        self.value = None;
        self.presence = Presence::default();
        self.generation = self.generation.wrapping_add(1);
    }

    fn update_anchor(&mut self, bounds: Bounds<Pixels>) {
        if self.is_open() || self.anchor.is_none() {
            self.anchor = Some(bounds);
        }
    }

    pub fn frame(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &App,
    ) -> Option<PopupFrame> {
        self.update_anchor(bounds);
        let frame = self.frame_at(
            Instant::now(),
            effect(super::moves(), super::plays(), cx.reduce_motion()),
        )?;
        // A first position pass resolves possible flipping. Schedule one more
        // frame so attached styling and the reveal mask use that actual side,
        // including when motion is disabled.
        if frame.progress != destination(frame.open) || frame.position.get().is_none() {
            window.request_animation_frame();
        }
        Some(frame)
    }

    fn frame_at(&mut self, now: Instant, effect: Effect) -> Option<PopupFrame> {
        self.value.as_ref()?;
        if effect == Effect::None {
            self.presence.from = destination(self.presence.open);
            self.presence.at = now;
        }
        let progress = self.presence.sample(now);
        if !self.presence.open && progress == 0. {
            self.value = None;
            return None;
        }
        Some(PopupFrame {
            open: self.presence.open,
            placement: Placement::Bottom,
            align: Align::Start,
            gap: px(4.),
            attachment: None,
            progress,
            effect,
            anchor: self.anchor.unwrap_or_default(),
            position: self.position.clone(),
        })
    }
}

/// The same anchor, flipping, mask and inert exit for all popup owners.
pub(in crate::ui) fn popup(
    element: impl IntoElement + 'static,
    id: impl Into<ElementId>,
    frame: PopupFrame,
    cx: &App,
) -> AnyElement {
    let id = id.into();
    let fixed = (!frame.open).then(|| frame.position.get()).flatten();
    let position = frame.position.get();
    let resolved_placement = position
        .and_then(|position| position.placement)
        .unwrap_or(frame.placement);
    let surface = crate::ui::kit::popup_surface(cx)
        .when_some(frame.attachment, |surface, panel| {
            let surface = surface.bg(panel.background);
            let limit = fixed.map_or(panel.bounds.size, |position| position.bounds.size);
            let surface = if panel.placement.is_horizontal() {
                surface.max_h(limit.height)
            } else {
                surface.max_w(limit.width)
            };
            // The panel paints the shared edge. With zero overlap it stays
            // visible, so drawing it again here would make a double rule.
            match resolved_placement {
                Placement::Top => surface.border_b_0().rounded_bl(px(0.)).rounded_br(px(0.)),
                Placement::Bottom => surface.border_t_0().rounded_tl(px(0.)).rounded_tr(px(0.)),
                Placement::Left => surface.border_r_0().rounded_tr(px(0.)).rounded_br(px(0.)),
                Placement::Right => surface.border_l_0().rounded_tl(px(0.)).rounded_bl(px(0.)),
            }
        })
        .relative()
        .child(element)
        .when_some(fixed, |surface, position| {
            surface
                .w(position.bounds.size.width)
                .h(position.bounds.size.height)
                .overflow_hidden()
        })
        .when(!frame.open, |surface| {
            surface.child(
                div()
                    .absolute()
                    .inset_0()
                    .occlude()
                    .capture_any_mouse_down(|_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .capture_any_mouse_up(|_, _, cx| cx.stop_propagation())
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation()),
            )
        });
    let surface = match frame.effect {
        Effect::Reveal => super::reveal::Reveal::popup(
            id,
            if frame.attachment.is_some() && resolved_placement.is_horizontal() {
                PopupOrigin::Horizontal(frame.anchor)
            } else if fixed.is_some_and(|position| position.placement == Some(Placement::Top)) {
                PopupOrigin::Bottom
            } else {
                PopupOrigin::Anchor(frame.anchor)
            },
            surface.into_any_element(),
        )
        .with_progress(frame.progress)
        .into_any_element(),
        Effect::Fade => surface.opacity(frame.progress).into_any_element(),
        Effect::None => surface.into_any_element(),
    };
    let positioner = if let Some(position) = fixed {
        match position.placement.unwrap_or(frame.placement) {
            Placement::Top => Positioner::corner(Anchor::BottomLeft, position.bounds.bottom_left()),
            Placement::Bottom | Placement::Right => {
                Positioner::corner(Anchor::TopLeft, position.bounds.origin)
            }
            Placement::Left => Positioner::corner(Anchor::TopRight, position.bounds.top_right()),
        }
    } else {
        let position = frame.position.clone();
        Positioner::side(frame.anchor)
            .placement(frame.placement)
            .align(frame.align)
            .offset(frame.gap)
            .on_position(move |resolved| position.set(Some(resolved)))
    };
    deferred(
        positioner
            .margin(if frame.attachment.is_some() {
                px(0.)
            } else {
                px(8.)
            })
            .when(frame.open, |positioner| positioner.occlude())
            .child(surface),
    )
    .with_priority(POPUP_PRIORITY)
    .into_any_element()
}

pub(in crate::ui) fn restore_focus(
    from: &FocusHandle,
    to: Option<FocusHandle>,
    window: &mut Window,
    cx: &mut App,
) {
    if from.contains_focused(window, cx) {
        if let Some(to) = to {
            to.focus(window, cx);
        } else {
            window.blur(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[::core::prelude::v1::test]
    fn close_retains_only_until_the_reverse_transition_finishes() {
        let now = Instant::now();
        let mut popup = Popup::default();
        popup.show_at("menu", now);
        assert_eq!(popup.frame_at(now, Effect::Reveal).unwrap().progress, 0.);
        let middle = now + SLOW / 2;
        let progress = popup.frame_at(middle, Effect::Reveal).unwrap().progress;
        assert!(progress > 0. && progress < 1.);
        assert!(popup.close_at(middle));
        assert!(!popup.is_open());
        assert_eq!(popup.current(), None);
        assert_eq!(popup.value(), Some(&"menu"));
        assert_eq!(
            popup.frame_at(middle, Effect::Reveal).unwrap().progress,
            progress
        );
        assert!(!popup.close_at(middle + Duration::from_millis(10)));
        assert!(popup.frame_at(middle + SLOW, Effect::Reveal).is_none());
        assert_eq!(popup.value(), None);
    }

    #[::core::prelude::v1::test]
    fn reopening_reverses_from_the_current_frame_and_rejects_old_dismissal() {
        let now = Instant::now();
        let mut popup = Popup::default();
        let old = popup.show_at("old", now);
        popup.close_at(now + SLOW);
        let again = now + SLOW + SLOW / 2;
        let progress = popup.frame_at(again, Effect::Reveal).unwrap().progress;
        let new = popup.show_at("new", again);
        assert_ne!(new, old);
        assert!(!popup.close_if(old));
        assert_eq!(
            popup.frame_at(again, Effect::Reveal).unwrap().progress,
            progress
        );
        let complete = popup.frame_at(again + SLOW * 2, Effect::Reveal).unwrap();
        assert_eq!(complete.progress, 1.);
        assert!(complete.open);
        assert_eq!(popup.current(), Some(&"new"));
    }

    #[::core::prelude::v1::test]
    fn reduced_fades_and_off_snaps_without_replaying_when_motion_returns() {
        assert_eq!(effect(true, true, false), Effect::Reveal);
        assert_eq!(effect(false, true, false), Effect::Fade);
        assert_eq!(effect(false, false, true), Effect::None);
        let now = Instant::now();
        let mut popup = Popup::default();
        popup.show_at(1, now);
        assert!(
            popup
                .frame_at(now + SLOW / 2, Effect::Fade)
                .unwrap()
                .progress
                < 1.
        );
        assert_eq!(
            popup
                .frame_at(now + SLOW / 2, Effect::None)
                .unwrap()
                .progress,
            1.
        );
        assert_eq!(
            popup
                .frame_at(now + SLOW / 2, Effect::Reveal)
                .unwrap()
                .progress,
            1.
        );
        popup.close_at(now + SLOW);
        assert!(popup.frame_at(now + SLOW, Effect::None).is_none());
    }

    #[::core::prelude::v1::test]
    fn closing_freezes_anchor_and_resolved_side_until_a_new_open() {
        let now = Instant::now();
        let a = Bounds::new(point(px(100.), px(200.)), size(px(50.), px(24.)));
        let b = Bounds::new(point(px(40.), px(200.)), size(px(110.), px(24.)));
        let mut popup = Popup::default();
        popup.show_at(1, now);
        popup.update_anchor(a);
        let open = popup.frame_at(now + SLOW, Effect::Reveal).unwrap();
        let resolved = ResolvedPosition {
            bounds: a,
            placement: Some(Placement::Top),
        };
        open.position.set(Some(resolved));
        popup.close_at(now + SLOW);
        popup.update_anchor(b);
        let closing = popup.frame_at(now + SLOW, Effect::Reveal).unwrap();
        assert_eq!(closing.anchor, a);
        assert_eq!(closing.position.get(), Some(resolved));
        popup.show_at(2, now + SLOW);
        popup.update_anchor(b);
        let reopened = popup.frame_at(now + SLOW, Effect::Reveal).unwrap();
        assert_eq!(reopened.anchor, b);
        assert!(reopened.position.get().is_none());
        open.position.set(Some(resolved));
        assert!(reopened.position.get().is_none());
    }

    #[::core::prelude::v1::test]
    fn panel_anchor_replaces_only_the_normal_axis() {
        let panel_bounds = Bounds::new(point(px(100.), px(40.)), size(px(300.), px(500.)));
        let trigger = Bounds::new(point(px(160.), px(220.)), size(px(80.), px(32.)));
        for placement in [Placement::Top, Placement::Bottom] {
            assert_eq!(
                PopupPanel {
                    bounds: panel_bounds,
                    placement,
                    align: Align::Start,
                    background: Hsla::default(),
                }
                .anchor(trigger),
                Bounds::new(point(px(160.), px(40.)), size(px(80.), px(500.)))
            );
        }
        for placement in [Placement::Left, Placement::Right] {
            assert_eq!(
                PopupPanel {
                    bounds: panel_bounds,
                    placement,
                    align: Align::Start,
                    background: Hsla::default(),
                }
                .anchor(trigger),
                Bounds::new(point(px(100.), px(220.)), size(px(300.), px(32.)))
            );
        }
    }

    #[::core::prelude::v1::test]
    fn owner_removal_or_close_before_first_paint_cannot_leave_a_ghost() {
        let now = Instant::now();
        let mut popup = Popup::default();
        popup.show_at(1, now);
        popup.close_at(now);
        assert!(popup.frame_at(now, Effect::Reveal).is_none());
        let old = popup.show_at(2, now);
        popup.clear();
        assert!(!popup.close_if(old));
        assert!(popup.value().is_none());
        assert!(popup.frame_at(now, Effect::Reveal).is_none());
    }
}
