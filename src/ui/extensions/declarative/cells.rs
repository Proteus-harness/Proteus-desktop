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

/// Expand runs lazily, resolving a style only for segments that contribute cells.
fn cell_styles<T: Clone>(
    segments: &[Segment],
    mut prepare: impl FnMut(&Segment) -> T,
) -> impl Iterator<Item = T> {
    segments
        .iter()
        .filter(|segment| segment.cells > 0)
        .flat_map(move |segment| std::iter::repeat_n(prepare(segment), segment.cells))
}

pub(super) fn cells(path: &str, columns: usize, segments: &[Segment], cx: &App) -> AnyElement {
    let theme = cx.theme();
    let radius = theme.radius / 2.;
    // The color and captured tooltip are shared by all cells of one segment.
    let mut styles = cell_styles(segments, |segment| (color(segment, cx), hint(segment)));
    let count = segments.iter().map(|s| s.cells).sum::<usize>();
    let columns = columns.max(1);
    let grid =
        v_flex()
            .flex_none()
            .gap(prefs::scaled(3.))
            .children((0..count).step_by(columns).map(|start| {
                h_flex()
                    .gap(prefs::scaled(3.))
                    .children((start..count.min(start + columns)).map(|index| {
                        let (color, hint) = styles.next().expect("one style per counted cell");
                        div()
                            .id(SharedString::from(format!("{path}-cell-{index}")))
                            .size(prefs::scaled(14.))
                            .rounded(radius)
                            .bg(color)
                            .tip_text(hint)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn cell_styles_prepare_each_nonempty_segment_once() {
        let segments = [0, 3, 0, 2, 0].map(|cells| Segment {
            cells,
            label: String::new(),
            value: None,
            detail: None,
            color: None,
            tone: Default::default(),
        });
        let mut count = 0;
        let styles: Vec<_> = cell_styles(&segments, |_| {
            count += 1;
            count
        })
        .collect();
        assert_eq!(styles, [1, 1, 1, 2, 2]);
        assert_eq!(count, 2);
    }
}
