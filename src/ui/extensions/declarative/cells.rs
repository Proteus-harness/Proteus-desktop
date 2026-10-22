//! The `cells` widget: a square grid split into colored segments and a
//! legend that names them.

use gpui_kit::component::*;
use gpui_kit::*;

use super::render::tone_color;
use crate::declarative::widget::{Segment, hex_color};
use crate::ui::fade::faded;
use crate::ui::prefs;
use crate::ui::tip::Tip as _;

fn color(segment: &Segment, cx: &App) -> Hsla {
    segment
        .color
        .as_deref()
        .and_then(hex_color)
        .map(|color| rgb(color).into())
        .unwrap_or_else(|| tone_color(segment.tone, cx))
}

fn hint(segment: &Segment) -> SharedString {
    [
        Some(segment.label.as_str()),
        segment.value.as_deref(),
        segment.detail.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
    .into()
}

pub(super) fn cells(path: &str, columns: usize, segments: &[Segment], cx: &App) -> AnyElement {
    let theme = cx.theme();
    let radius = theme.radius / 2.;
    let flat: Vec<usize> = segments
        .iter()
        .enumerate()
        .flat_map(|(index, segment)| std::iter::repeat_n(index, segment.cells))
        .collect();
    let grid =
        v_flex()
            .flex_none()
            .gap(px(3.))
            .children(flat.chunks(columns.max(1)).enumerate().map(|(row, chunk)| {
                h_flex()
                    .gap(px(3.))
                    .children(chunk.iter().enumerate().map(|(column, &index)| {
                        let segment = &segments[index];
                        div()
                            .id(SharedString::from(format!(
                                "{path}-cell-{}",
                                row * columns + column
                            )))
                            .size(prefs::scaled(14.))
                            .rounded(radius)
                            .bg(color(segment, cx))
                            .tip_text(hint(segment))
                    }))
            }));
    let legend = v_flex()
        .gap_1p5()
        .text_xs()
        .children(segments.iter().map(|segment| {
            h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_none()
                        .size_2p5()
                        .rounded_full()
                        .bg(color(segment, cx)),
                )
                .child(
                    div()
                        .w(prefs::scaled(170.))
                        .flex_none()
                        .child(faded(segment.label.clone())),
                )
                .child(
                    div()
                        .w(prefs::scaled(80.))
                        .flex_none()
                        .text_right()
                        .child(segment.value.clone().unwrap_or_default()),
                )
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(segment.detail.clone().unwrap_or_default()),
                )
        }));
    h_flex()
        .w_full()
        .gap_6()
        .flex_wrap()
        .items_start()
        .child(grid)
        .child(legend)
        .into_any_element()
}
