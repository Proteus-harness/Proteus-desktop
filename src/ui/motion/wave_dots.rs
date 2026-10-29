//! Three quiet status dots on the shared clock. Mount inside a retained view,
//! so the ticker invalidates only that small control, as with `Shimmer`.

use std::time::Duration;

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::*;

use crate::ui::prefs;

const WAVE: Duration = Duration::from_millis(1_200);

#[derive(IntoElement)]
pub(in crate::ui) struct WaveDots;

impl RenderOnce for WaveDots {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let moving = super::moves() && !cx.reduce_motion();
        let phase = if moving {
            super::ticker::next_tick(window, cx);
            super::ticker::phase(WAVE)
        } else {
            0.
        };
        h_flex()
            .gap(prefs::scaled(2.))
            .items_center()
            .children((0..3).map(|index| {
                let lift = if moving {
                    ((phase * std::f32::consts::TAU - index as f32 * 0.7).sin()).max(0.)
                } else {
                    0.
                };
                div()
                    .relative()
                    .top(prefs::scaled(-2. * lift))
                    .size(prefs::scaled(3.))
                    .rounded(cx.theme().radius_full())
                    .bg(cx.theme().muted_foreground.opacity(0.65 + 0.35 * lift))
            }))
    }
}
