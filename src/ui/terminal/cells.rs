//! Shared cell geometry, colors, runs and cursor for local and agent terminals.
use crate::terminal::{Run, Screen};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
pub(in crate::ui) const PADDING: f32 = 8.;
const LINE_HEIGHT: f32 = 1.3;
pub(in crate::ui) fn rgb_bytes(color: Hsla) -> [u8; 3] {
    let color = color.to_rgb();
    [color.r, color.g, color.b].map(|value| (value * 255.).round() as u8)
}

fn color([r, g, b]: [u8; 3]) -> Hsla {
    rgb(u32::from_be_bytes([0, r, g, b])).into()
}

pub(in crate::ui) fn cell_size(font: &Font, size: Pixels, window: &Window) -> Size<Pixels> {
    let system = window.text_system();
    let width = system
        .advance(system.resolve_font(font), size, 'm')
        .map(|advance| advance.width)
        .unwrap_or(size * 0.6);
    Size {
        width,
        height: (size * LINE_HEIGHT).round(),
    }
}

fn render_run(run: &Run, cell: Size<Pixels>) -> Div {
    let style = &run.style;
    div()
        .absolute()
        .top_0()
        .left(cell.width * run.column as f32)
        .h(cell.height)
        .when_some(style.background, |this, background| {
            this.w(cell.width * run.width as f32).bg(color(background))
        })
        .whitespace_nowrap()
        .text_color(color(style.foreground))
        .when(style.bold, |this| this.font_weight(FontWeight::BOLD))
        .when(style.italic, |this| this.italic())
        .when(style.underline, |this| this.underline())
        .when(style.strike, |this| this.line_through())
        .child(run.text.clone())
}

pub(in crate::ui) fn screen(
    screen: &Screen,
    cell: Size<Pixels>,
    foreground: Hsla,
    focused: bool,
) -> Div {
    let rows = screen.rows.iter().enumerate().map(|(index, runs)| {
        div()
            .absolute()
            .top(px(PADDING) + cell.height * index as f32)
            .left(px(PADDING))
            .right(px(PADDING))
            .h(cell.height)
            .children(runs.iter().map(|run| render_run(run, cell)))
    });
    let cursor = screen.cursor.map(|(row, column)| {
        div()
            .absolute()
            .top(px(PADDING) + cell.height * row as f32)
            .left(px(PADDING) + cell.width * column as f32)
            .w(cell.width)
            .h(cell.height)
            .map(|this| {
                if focused {
                    this.bg(foreground.opacity(0.55))
                } else {
                    this.border_1().border_color(foreground.opacity(0.6))
                }
            })
    });
    div().absolute().size_full().children(rows).children(cursor)
}
