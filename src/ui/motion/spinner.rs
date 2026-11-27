//! A loading spinner turning on the shared tick: every frame of a running
//! animation redraws the window, and a spinner reads as turning long before
//! the display's rate.
//!
//! An open arc turning at an even pace: its gap shows which way it goes, and
//! without easing it never seems to stop and start again.

use std::time::Duration;

use gpui_kit::component::{Icon, IconName, Sizable, Size};
use gpui_kit::*;

use super::ticker;

const TURN: Duration = Duration::from_millis(1_200);

#[derive(IntoElement)]
pub(in crate::ui) struct Spinner {
    size: Size,
    id: ElementId,
}

impl Spinner {
    #[track_caller]
    pub(in crate::ui) fn new() -> Self {
        Self {
            size: Size::Medium,
            id: ElementId::CodeLocation(*std::panic::Location::caller()),
        }
    }
}

impl Sizable for Spinner {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl RenderOnce for Spinner {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = window.with_global_id(self.id, |id, window| {
            window.with_element_state(id, |view: Option<Entity<SpinnerView>>, _| {
                let view = view.unwrap_or_else(|| cx.new(|_| SpinnerView { size: self.size }));
                if view.read(cx).size != self.size {
                    view.update(cx, |view, cx| {
                        view.size = self.size;
                        cx.notify();
                    });
                }
                (view.clone(), view)
            })
        });
        div().child(view)
    }
}

struct SpinnerView {
    size: Size,
}

impl Render for SpinnerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let turn = if super::plays(cx) {
            ticker::next_tick(window, cx);
            ticker::phase(TURN)
        } else {
            0.
        };
        div().child(
            Icon::new(IconName::LoaderCircle)
                .with_size(self.size)
                .transform(Transformation::rotate(percentage(turn))),
        )
    }
}
