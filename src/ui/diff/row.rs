// Адаптировано из Zed: crates/editor/src/element.rs (diff gutters and spacer blocks)
use crate::diff::{File, Kind, Line, Pair};
use crate::ui::fade::faded;
use crate::ui::prefs;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub(super) fn row(
    file: &File,
    syntax: &super::syntax::Syntax,
    index: usize,
    side: Option<bool>,
    cx: &App,
) -> Div {
    let row = h_flex().w_full().h(prefs::scaled(super::LINE)).flex_none();
    if let Some(old) = side {
        match file.pairs[index] {
            Pair::Full(line) => row.child(full(&file.lines[line], cx)),
            Pair::Lines(before, after) => row.child(cell(
                file,
                if old { before } else { after },
                old,
                true,
                syntax,
                cx,
            )),
        }
    } else {
        let index = file.unified[index];
        let line = &file.lines[index];
        match line.kind {
            Kind::Hunk | Kind::Note => row.child(full(line, cx)),
            _ => row.child(cell(file, Some(index), false, false, syntax, cx)),
        }
    }
}

fn full(line: &Line, cx: &App) -> Div {
    h_flex()
        .w_full()
        .h_full()
        .px_3()
        .text_xs()
        .bg(cx.theme().muted.opacity(0.5))
        .text_color(cx.theme().muted_foreground)
        .child(faded(line.text.clone()))
}

fn number(value: Option<usize>, cx: &App) -> Div {
    div()
        .flex_none()
        .w(prefs::scaled(44.))
        .pr_2()
        .text_right()
        .text_color(cx.theme().muted_foreground.opacity(0.65))
        .child(value.map(|n| n.to_string()).unwrap_or_default())
}

fn cell(
    file: &File,
    index: Option<usize>,
    old: bool,
    split: bool,
    syntax: &super::syntax::Syntax,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let Some(index) = index else {
        // Transparent stripes adapt to the actual theme behind them.
        return div().flex_1().flex_basis(px(0.)).h_full().bg(pattern_slash(
            theme.border.opacity(0.35),
            1.,
            7.,
        ));
    };
    let line = &file.lines[index];
    let tint = match line.kind {
        Kind::Added => Some(theme.success),
        Kind::Removed => Some(theme.danger),
        _ => None,
    };
    h_flex()
        .flex_1()
        .flex_basis(px(0.))
        .min_w_0()
        .h_full()
        .overflow_hidden()
        .border_l_2()
        .border_color(tint.unwrap_or(theme.border.opacity(0.)))
        .when_some(tint, |cell, color| cell.bg(color.opacity(0.09)))
        .child(number(
            if old {
                line.old
            } else if split {
                line.new
            } else {
                line.old
            },
            cx,
        ))
        .when(!split, |cell| cell.child(number(line.new, cx)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pl_2()
                .whitespace_nowrap()
                .child(super::syntax::code(
                    ElementId::Name(format!("{}-{index}-{old}", file.path).into()),
                    line.text.replace('\t', "    ").into(),
                    syntax.line(index, old || line.kind == Kind::Removed),
                )),
        )
}
