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
    fn sample(&self, open: bool, now: Instant) -> Option<f32> {
        (self.open == open && self.tween.running(now)).then(|| self.progress(now))
    }

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
    reveal_at(key, open, Instant::now(), cx, content)
}

/// Split virtual rows must share this timestamp for both their visibility
/// and the head's disclosure. Work done between rows cannot end the motion
/// halfway through a layout and render the run twice.
pub(in crate::ui) fn reveal_at(
    key: impl Into<SharedString>,
    open: bool,
    at: Instant,
    cx: &App,
    content: impl FnOnce() -> AnyElement,
) -> Option<AnyElement> {
    let key = key.into();
    let progress = progress_at(&key, open, at, cx);
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

/// A disclosure with a persistent preview instead of an empty closed state.
/// Both endpoints are measured at the same width; the shared transition
/// animates between their heights and reverses from its current position.
/// The branches have separate element paths, so selection state cannot leak
/// from the shortened preview into the complete content.
pub(in crate::ui) fn preview_reveal<E: IntoElement>(
    key: impl Into<SharedString>,
    open: bool,
    cx: &App,
    preview: impl IntoElement,
    content: impl FnOnce() -> E,
) -> AnyElement {
    let key = key.into();
    preview_with_progress(
        key.clone(),
        open,
        progress(&key, open, cx),
        preview,
        content,
    )
}

fn preview_with_progress<E: IntoElement>(
    key: SharedString,
    open: bool,
    progress: Option<f32>,
    preview: impl IntoElement,
    content: impl FnOnce() -> E,
) -> AnyElement {
    // Both endpoints are needed while moving, including a closing or reversed
    // transition. A settled preview never builds, lays out or paints its tail.
    // Consume the builder now; Reveal retains elements, never this closure.
    let content = (open || progress.is_some()).then(content);
    Reveal::preview(
        ElementId::Name(key),
        progress,
        open,
        div()
            .id("preview")
            .w_full()
            .child(preview)
            .into_any_element(),
        div()
            .id("content")
            .w_full()
            .children(content)
            .into_any_element(),
    )
    .into_any_element()
}

/// Whether `toggled` is animating `key` towards `open` right now. A caller
/// that splits a disclosure into several rows at rest draws them as one
/// block meanwhile, so the whole block grows like any other disclosure.
pub(in crate::ui) fn moving(key: &str, open: bool, cx: &App) -> bool {
    moving_at(key, open, Instant::now(), cx)
}

pub(in crate::ui) fn moving_at(key: &str, open: bool, at: Instant, cx: &App) -> bool {
    progress_at(key, open, at, cx).is_some()
}

/// Eased share of the content shown while `toggled` animates `key`.
fn progress(key: &str, open: bool, cx: &App) -> Option<f32> {
    progress_at(key, open, Instant::now(), cx)
}

fn progress_at(key: &str, open: bool, at: Instant, cx: &App) -> Option<f32> {
    cx.try_global::<Toggles>()
        .and_then(|toggles| toggles.0.get(key).copied())
        .and_then(|toggle| toggle.sample(open, at))
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

#[cfg(test)]
mod tests {
    use super::{SLOW, Toggle, Tween, preview_with_progress};
    use gpui_kit::{ParentElement as _, div};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    #[test]
    fn settled_preview_drops_the_full_builder_without_calling_or_retaining_it() {
        struct Payload(Rc<Cell<bool>>);
        impl Drop for Payload {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }
        let calls = Cell::new(0);
        let dropped = Rc::new(Cell::new(false));
        let payload = Payload(dropped.clone());
        let element = preview_with_progress("closed".into(), false, None, div(), || {
            calls.set(calls.get() + 1);
            drop(payload);
            div()
        });
        // This must happen before the returned element is dropped: otherwise
        // each closed row could still retain a full-content builder/payload.
        assert_eq!(calls.get(), 0);
        assert!(dropped.get());
        drop(element);
    }

    #[test]
    fn collapsed_source_updates_build_only_the_latest_content_when_opened() {
        let built = RefCell::new(Vec::new());
        let mut source = String::from("initial result");
        let closed = preview_with_progress("updated".into(), false, None, div(), || {
            built.borrow_mut().push(source.clone());
            div()
        });
        // The returned element must not keep borrowing the old source.
        source.replace_range(.., "latest result");
        assert!(built.borrow().is_empty());
        let _open = preview_with_progress("updated".into(), true, None, div(), || {
            built.borrow_mut().push(source.clone());
            div()
        });
        assert_eq!(*built.borrow(), ["latest result"]);
        drop(closed);
    }

    #[test]
    fn open_and_moving_previews_consume_the_full_builder_once() {
        for (open, progress) in [
            (true, None),
            (true, Some(0.)),
            (true, Some(0.5)),
            (true, Some(1.)),
            (false, Some(1.)),
            (false, Some(0.5)),
            (false, Some(0.)),
        ] {
            let calls = Cell::new(0);
            // Moving the value out makes this FnOnce, rather than a reusable
            // closure or a builder stored for a later layout/paint phase.
            let payload = String::from("full content");
            let element = preview_with_progress("phase".into(), open, progress, div(), || {
                calls.set(calls.get() + 1);
                div().child(payload)
            });
            assert_eq!(calls.get(), 1, "open={open}, progress={progress:?}");
            drop(element);
            assert_eq!(calls.get(), 1);
        }
    }

    #[test]
    fn reversing_preview_keeps_full_content_until_closing_settles() {
        let start = Instant::now();
        let opening = Toggle {
            tween: Tween {
                at: start,
                duration: SLOW,
            },
            from: 0.,
            open: true,
        };
        let reversed_at = start + SLOW / 2;
        let closing = Toggle {
            tween: Tween {
                at: reversed_at,
                duration: SLOW,
            },
            from: opening.progress(reversed_at),
            open: false,
        };
        assert_eq!(closing.progress(reversed_at), opening.progress(reversed_at));
        for elapsed in [
            Duration::ZERO,
            SLOW / 2,
            SLOW - Duration::from_nanos(1),
            SLOW,
        ] {
            let calls = Cell::new(0);
            let progress = closing.sample(false, reversed_at + elapsed);
            let _element = preview_with_progress("reversed".into(), false, progress, div(), || {
                calls.set(calls.get() + 1);
                div()
            });
            assert_eq!(calls.get(), usize::from(elapsed < SLOW));
        }
    }

    #[test]
    fn split_rows_share_the_last_moving_frame_and_settle_together() {
        let start = Instant::now();
        for open in [true, false] {
            let toggle = Toggle {
                tween: Tween {
                    at: start,
                    duration: SLOW,
                },
                from: if open { 0. } else { 1. },
                open,
            };
            let frame_at = start + SLOW - Duration::from_nanos(1);
            let head = toggle.sample(open, frame_at);
            assert!(head.is_some());
            // A fresh clock read after rendering a costly head would have
            // settled the tails already. The captured frame stays uniform.
            assert!(toggle.sample(open, start + SLOW).is_none());
            for _ in 0..3 {
                assert_eq!(toggle.sample(open, frame_at), head);
            }
            assert!(toggle.sample(!open, frame_at).is_none());
        }
    }
}
