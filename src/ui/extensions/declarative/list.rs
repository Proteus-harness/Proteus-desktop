// Адаптировано из Zed: crates/git_ui/src/git_panel.rs (render_commit_history), crates/git_ui/src/commit_tooltip.rs (CommitAvatar)
//! The `list` node: a virtualized list of two-line rows like the commit
//! history of Zed's git panel. A click or Enter opens a row; the arrows move
//! the cursor without sending anything.

use super::rows::{Kind, Rows};
use crate::declarative::control::MenuItem;
use crate::declarative::list::Row;
use crate::ui::fade::faded;
use crate::ui::prefs;
use crate::ui::tip::Tip as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

/// Chips shown before the «+k» one, Zed's `MAX_HISTORY_TAG_CHIPS`.
const CHIPS: usize = 3;
/// Zed's commit avatar: 14 px and a 1 px border.
const AVATAR: f32 = 14.;

pub(super) type List = Rows<ListRows>;

#[derive(Default)]
pub(super) struct ListRows;

impl Kind for ListRows {
    type Row = Row;
    const NAME: &'static str = "extension-list";
    const MOVED: Option<&'static str> = None;
    const SCROLL: ScrollStrategy = ScrollStrategy::Top;
    const RING_AFTER_KEYS: bool = true;

    fn id(row: &Row) -> &str {
        &row.id
    }

    fn selected(_: &Row) -> bool {
        false
    }

    fn menu(row: &Row) -> &[MenuItem] {
        &row.menu
    }

    fn clicked(_: usize) -> &'static str {
        "open"
    }

    fn measure(&mut self, _: &[Rc<Row>]) {}

    fn draw(&self, row: &Row, index: usize, ring: bool, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        let (hover, muted, border) = (theme.list_hover, theme.muted_foreground, theme.border);
        let hidden = row.chips.len().saturating_sub(CHIPS);
        let chips = (!row.chips.is_empty()).then(|| {
            h_flex()
                .min_w_0()
                .gap_1()
                .children(row.chips.iter().take(CHIPS).enumerate().map(|(n, chip)| {
                    super::toggle::label_chip(chip.clone(), cx)
                        .id(("list-chip", n))
                        .tip_text(chip.clone())
                }))
                .when(hidden > 0, |chips| {
                    let names = row.chips[CHIPS..].join(", ");
                    chips.child(
                        super::toggle::label_chip(format!("+{hidden}"), cx)
                            .flex_none()
                            .id("list-chip-more")
                            .tip_text(names),
                    )
                })
        });
        let mark = row
            .mark
            .as_deref()
            .and_then(crate::declarative::icons::lucide);
        let first = h_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .child(div().min_w_0().child(faded(row.label.clone())))
            .children(chips)
            .children(mark.map(|icon| {
                h_flex()
                    .size_4()
                    .flex_none()
                    .justify_center()
                    .rounded(px(3.))
                    .border_1()
                    .border_color(border)
                    .bg(theme.secondary)
                    .child(Icon::new(icon).xsmall())
            }));
        let parts: Vec<&String> = row.meta.iter().filter(|part| !part.is_empty()).collect();
        let last = parts.len().saturating_sub(1);
        let dot = || div().flex_none().opacity(0.5).child("•");
        let second = h_flex()
            .w_full()
            .min_w_0()
            .gap_1p5()
            .text_xs()
            .text_color(muted)
            .when(row.avatar, |line| line.child(avatar(cx)))
            .children(parts.iter().enumerate().flat_map(|(n, part)| {
                let text = div()
                    .when(n == 0 && last > 0, |text| text.min_w_0())
                    .when(n > 0 || last == 0, |text| text.flex_none())
                    .child(faded((*part).clone()));
                let mut pieces = vec![text.into_any_element()];
                if n < last {
                    pieces.push(dot().into_any_element());
                }
                pieces
            }));
        let (hint, detail) = (row.hint.clone(), row.hint_detail.clone());
        v_flex()
            .id(("list-row", index))
            .cursor_pointer()
            .w_full()
            .py_1()
            .px_2()
            .gap_0p5()
            .border_1()
            .border_color(transparent_black())
            .when(ring, |element| element.border_color(theme.ring))
            .text_sm()
            .hover(move |style| style.bg(hover))
            .child(first)
            .child(second)
            .when_some(hint, |element, hint| {
                element.tip_element(move |_, cx| {
                    v_flex()
                        .gap_0p5()
                        .child(hint.clone())
                        .children(detail.clone().map(|detail| {
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(detail)
                        }))
                })
            })
    }
}

/// Zed's avatar without a picture: a person in a small muted circle.
fn avatar(cx: &App) -> Div {
    let theme = cx.theme();
    h_flex()
        .flex_none()
        .size(prefs::scaled(AVATAR + 2.))
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(theme.border)
        .bg(theme.muted)
        .child(
            Icon::new(Lucide::User)
                .xsmall()
                .text_color(theme.muted_foreground),
        )
}
