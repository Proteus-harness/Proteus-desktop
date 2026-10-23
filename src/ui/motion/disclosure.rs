//! Disclosures: chevrons that turn and content that grows or shrinks.

use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;
use std::time::Instant;

use gpui_kit::component::*;
use gpui_kit::*;

use super::reveal::Reveal;
use super::{SLOW, Tween, ease_enter, moves};

const SPRING: SpringConfig = SpringConfig::new(420., 34., 1.);

/// Disclosure chevron that turns towards the open state.
pub(in crate::ui) fn chevron(id: impl Into<ElementId>, open: bool, color: Hsla) -> AnyElement {
    let icon = Icon::new(IconName::ChevronRight).xsmall().text_color(color);
    if !moves() {
        let angle = if open { FRAC_PI_2 } else { 0. };
        return icon.rotate(radians(angle)).into_any_element();
    }
    icon.with_spring(id, SpringAnimation::new(SPRING).to(open), |icon, phase| {
        icon.rotate(radians(phase.interpolate(0., FRAC_PI_2)))
    })
    .into_any_element()
}

/// Dropdown chevron, using the same spring and reduced-motion policy.
pub(in crate::ui) fn menu_chevron(id: impl Into<ElementId>, open: bool, color: Hsla) -> AnyElement {
    let icon = Icon::new(IconName::ChevronDown).xsmall().text_color(color);
    let target = std::f32::consts::PI;
    if !moves() {
        return icon
            .rotate(radians(if open { target } else { 0. }))
            .into_any_element();
    }
    icon.with_spring(
        id,
        SpringAnimation::new(SPRING).to(open),
        move |icon, phase| icon.rotate(radians(phase.interpolate(0., target))),
    )
    .into_any_element()
}

/// Disclosures toggled a moment ago, keyed by caller-chosen names.
#[derive(Default)]
struct Toggles(HashMap<SharedString, Toggle>);

impl Global for Toggles {}

#[derive(Clone, Copy)]
struct Toggle {
    tween: Tween,
    from: f32,
    open: bool,
}

impl Toggle {
    fn progress(&self, now: Instant) -> f32 {
        let target = if self.open { 1. } else { 0. };
        self.from + (target - self.from) * ease_enter(self.tween.linear(now))
    }
}

/// Records that the disclosure `key` was just opened or closed, so its
/// content grows or shrinks instead of jumping. A toggle during a running
/// animation continues from the current height.
pub(in crate::ui) fn toggled(key: impl Into<SharedString>, open: bool, cx: &mut App) {
    let key = key.into();
    let animate = moves() && !cx.reduce_motion();
    let now = Instant::now();
    let toggles = cx.default_global::<Toggles>();
    toggles.0.retain(|_, toggle| toggle.tween.running(now));
    if !animate {
        toggles.0.remove(&key);
        return;
    }
    let from = toggles
        .0
        .get(&key)
        .map(|toggle| toggle.progress(now))
        .unwrap_or(if open { 0. } else { 1. });
    toggles.0.insert(
        key,
        Toggle {
            tween: Tween::start(SLOW),
            from,
            open,
        },
    );
}

/// Content of a disclosure. While `toggled` animates it, the content is
/// clipped to a growing or shrinking height and stays mounted until it has
/// closed; otherwise it is simply present when open.
pub(in crate::ui) fn reveal(
    key: impl Into<SharedString>,
    open: bool,
    cx: &App,
    content: impl FnOnce() -> AnyElement,
) -> Option<AnyElement> {
    let key = key.into();
    let progress = progress(&key, open, cx);
    if !open && progress.is_none() {
        return None;
    }
    // Moving, the reveal lays its child out on its own; without a full width
    // the child would shrink to its content and widen only once it rests.
    // The content fades along with its height, so a tall block does not
    // show up at full contrast behind the moving edge.
    let mut child = div().w_full();
    if let Some(progress) = progress {
        child = child.opacity(progress);
    }
    let child = child.child(content()).into_any_element();
    Some(Reveal::new(ElementId::Name(key), progress, child).into_any_element())
}

/// Whether `toggled` is animating `key` towards `open` right now. A caller
/// that splits a disclosure into several rows at rest draws them as one
/// block meanwhile, so the whole block grows like any other disclosure.
pub(in crate::ui) fn moving(key: &str, open: bool, cx: &App) -> bool {
    progress(key, open, cx).is_some()
}

/// Eased share of the content shown while `toggled` animates `key`.
fn progress(key: &str, open: bool, cx: &App) -> Option<f32> {
    let now = Instant::now();
    cx.try_global::<Toggles>()
        .and_then(|toggles| toggles.0.get(key).copied())
        .filter(|toggle| toggle.open == open && toggle.tween.running(now))
        .map(|toggle| toggle.progress(now))
}

/// Keeps a view redrawing while any disclosure animates; call from `render`.
pub(in crate::ui) fn drive(window: &mut Window, cx: &App) {
    let now = Instant::now();
    let running = cx
        .try_global::<Toggles>()
        .is_some_and(|toggles| toggles.0.values().any(|toggle| toggle.tween.running(now)));
    if running {
        window.request_animation_frame();
    }
}
