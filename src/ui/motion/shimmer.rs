//! A quiet moving highlight within a status caption. Uses the shared ticker;
//! reduced motion leaves a static caption and schedules no animation frames.
use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

const SWEEP: Duration = Duration::from_millis(1_800);

pub(in crate::ui) struct Shimmer {
    text: SharedString,
    animated: bool,
}

impl Shimmer {
    pub(in crate::ui) fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            animated: true,
        }
    }

    pub(in crate::ui) fn set_caption(
        &mut self,
        text: &str,
        animated: bool,
        cx: &mut Context<Self>,
    ) {
        if self.text.as_ref() != text || self.animated != animated {
            self.text = text.to_owned().into();
            self.animated = animated;
            cx.notify();
        }
    }
}

/// The ticker invalidates this small retained view, not its containing chat.
/// Chat rendering prepares diffs and the virtual list and must not run merely
/// because one status caption changed its paint. Store its Entity directly:
/// use_keyed_state observes child changes and would invalidate the parent too.
impl Render for Shimmer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let moving = self.animated && super::moves() && !cx.reduce_motion();
        let phase = if moving {
            super::ticker::next_tick(window, cx);
            super::ticker::phase(SWEEP)
        } else {
            0.
        };
        let style = window.text_style();
        let count = self.text.chars().count().max(1) as f32;
        let theme = cx.theme();
        let runs = self
            .text
            .chars()
            .enumerate()
            .map(|(index, ch)| {
                let mut run = style.to_run(ch.len_utf8());
                let distance = (index as f32 / count - (phase * 1.8 - 0.4)).abs();
                let highlight = if moving {
                    (1. - distance / 0.35).max(0.)
                } else {
                    0.
                };
                run.color = theme
                    .muted_foreground
                    .blend(theme.foreground.opacity(highlight))
                    .into();
                run
            })
            .collect();
        StyledText::new(self.text.clone()).with_runs(runs)
    }
}
