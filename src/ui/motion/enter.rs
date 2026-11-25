//! Blocks that fade in when they appear.

use std::time::Duration;

use gpui_kit::component::animation::EffectTransition;
use gpui_kit::*;

use super::{BASE, FAST, ease_enter, shift};

/// Rows of a list that join a staggered entrance; later ones appear at once.
const STAGGER_ROWS: usize = 14;
const STAGGER_STEP: Duration = Duration::from_millis(18);
/// Loads shorter than this show no waiting indicator.
const LATE: Duration = Duration::from_millis(200);

/// Fade plus a short rise; for freshly shown blocks.
pub(in crate::ui) fn rise_in<E: IntoElement + Styled + 'static>(
    element: E,
    id: impl Into<ElementId>,
    cx: &App,
) -> AnimationElement<E> {
    EffectTransition::new(BASE)
        .ease(ease_enter)
        .slide_y(shift(6., cx), px(0.))
        .fade(0., 1.)
        .apply(element, id)
}

/// A short fade for a screen or page that replaces another: content that
/// moves on every switch reads as a jump.
pub(in crate::ui) fn switch_in<E: IntoElement + Styled + 'static>(
    element: E,
    id: impl Into<ElementId>,
) -> AnimationElement<E> {
    EffectTransition::new(FAST)
        .ease(ease_enter)
        .fade(0., 1.)
        .apply(element, id)
}

/// `rise_in` only when `play`, keeping the element ids below it stable
/// either way: a block shown again (another tab, a remount) stays put.
pub(in crate::ui) fn rise_in_if<E: IntoElement + Styled + 'static>(
    element: E,
    id: impl Into<ElementId>,
    play: bool,
    cx: &App,
) -> AnimationElement<E> {
    let rise = shift(6., cx);
    element
        .with_animation(
            id,
            Animation::new(BASE).with_easing(ease_enter),
            move |element, delta| element.opacity(delta).top(rise * (1. - delta)),
        )
        .playing(play)
}

/// Opacity only; safe inside virtualized lists that cache row heights.
///
/// The wrapper is always present so the element ids below it stay stable;
/// `play` decides whether this mount actually fades in. Virtualized rows
/// remount on scroll, so callers pass `play` only for fresh content.
pub(in crate::ui) fn fade_in_if<E: IntoElement + Styled + 'static>(
    element: E,
    id: impl Into<ElementId>,
    play: bool,
) -> AnimationElement<E> {
    element
        .with_animation(
            id,
            Animation::new(BASE).with_easing(ease_enter),
            |element, delta| element.opacity(delta),
        )
        .playing(play)
}

/// A waiting indicator held back for a moment, then faded in: a load that
/// finishes sooner does not flash it.
pub(in crate::ui) fn late_in<E: IntoElement + Styled + 'static>(
    element: E,
    id: impl Into<ElementId>,
) -> AnimationElement<E> {
    let total = LATE + FAST;
    let late = LATE.as_secs_f32() / total.as_secs_f32();
    element.with_animation(id, Animation::new(total), move |element, delta| {
        element.opacity(((delta - late) / (1. - late)).clamp(0., 1.))
    })
}

/// Row `index` of a list shown for the first time: rows fade in one after
/// another from the top. Pass `play` only while the list has just loaded,
/// so rows scrolled into view later do not animate.
pub(in crate::ui) fn stagger_in<E: IntoElement + Styled + 'static>(
    element: E,
    id: impl Into<ElementId>,
    index: usize,
    play: bool,
    cx: &App,
) -> AnimationElement<E> {
    let play = play && index < STAGGER_ROWS;
    let delay = STAGGER_STEP * index.min(STAGGER_ROWS) as u32;
    let duration = FAST + delay;
    let total = duration.as_secs_f32();
    let (delay, fast) = (delay.as_secs_f32(), FAST.as_secs_f32());
    let rise = shift(4., cx);
    element
        .with_animation(
            id,
            Animation::new(duration)
                .with_easing(move |t| ease_enter(((t * total - delay) / fast).clamp(0., 1.))),
            move |element, delta| element.opacity(delta).top(rise * (1. - delta)),
        )
        .playing(play)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::gpui::TestAppContext;

    struct EntranceProbe {
        draw: fn(&mut App) -> Div,
        reduce_motion: bool,
    }

    impl Render for EntranceProbe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            if self.reduce_motion {
                cx.set_reduce_motion(true);
            }
            (self.draw)(cx)
        }
    }

    fn pending_frames(draw: fn(&mut App) -> Div, reduce_motion: bool) -> usize {
        let mut app = TestAppContext::single();
        app.update(crate::ui::prefs::init_in_memory);
        let window = app.open_window(size(px(500.), px(500.)), |_, _| EntranceProbe {
            draw,
            reduce_motion,
        });
        app.run_until_parked();
        window
            .update(&mut app, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap()
    }

    fn row() -> Div {
        div().h(px(10.))
    }

    #[::core::prelude::v1::test]
    fn static_entrances_do_not_request_frames() {
        let reference = pending_frames(
            |_| {
                div().children((0usize..42).map(|index| {
                    // The old conditional effects still ran an animation whose
                    // animator returned the original row.
                    row().with_animation(("reference", index), Animation::new(BASE), |row, _| row)
                }))
            },
            false,
        );
        assert_eq!(reference, 42);

        let idle = pending_frames(
            |cx| {
                div()
                    .children((0usize..14).map(|index| fade_in_if(row(), ("fade", index), false)))
                    .children(
                        (0usize..14).map(|index| rise_in_if(row(), ("rise", index), false, cx)),
                    )
                    .children(
                        (STAGGER_ROWS..STAGGER_ROWS * 2)
                            .map(|index| stagger_in(row(), ("stagger", index), index, true, cx)),
                    )
            },
            false,
        );
        assert_eq!(idle, 0, "42 static rows must schedule no frames");

        let draw_active = |cx: &mut App| {
            div()
                .child(fade_in_if(row(), "active-fade", true))
                .child(rise_in_if(row(), "active-rise", true, cx))
                .child(stagger_in(row(), "active-stagger", 0, true, cx))
        };
        assert_eq!(pending_frames(draw_active, false), 3);
        assert_eq!(pending_frames(draw_active, true), 0);
    }
}
