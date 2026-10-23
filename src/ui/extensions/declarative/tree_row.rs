// Адаптировано из Zed: crates/ui/src/components/list/list_item.rs, crates/ui/src/components/indent_guides.rs, crates/git_ui/src/git_panel.rs
//! One row of [`Tree`]: an item like a Zed list item or a section header
//! like the Zed git panel's, with indent guides.

use super::toggle;
use super::tree::Tree;
use crate::declarative::tree::{Check, Row};
use crate::declarative::widget::Tone;
use crate::ui::context::RightClickMenu as _;
use crate::ui::fade::{faded, faded_start};
use crate::ui::prefs;
use crate::ui::tip::Tip as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::ops::Range;

const HEIGHT: f32 = 24.;
/// Room before the first level.
const INSET: f32 = 6.;
/// Width of the chevron or, without chevrons, of the icon.
const SLOT: f32 = 14.;

impl Tree {
    pub(super) fn row(
        &self,
        index: usize,
        focused: bool,
        guide: Option<(Range<usize>, usize)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = &self.rows[index];
        let theme = cx.theme();
        let (hover, active, primary) = (theme.list_hover, theme.list_active, theme.primary);
        let ring = theme.ring;
        let (line, lit_line) = (theme.border, theme.muted_foreground.opacity(0.5));
        let indent = self.layout.indent;
        let start = move |depth: usize| INSET + depth as f32 * indent;
        let guides = self.layout.guides.then(|| {
            (0..row.depth).map(move |depth| {
                let lit = guide
                    .as_ref()
                    .is_some_and(|(rows, level)| *level == depth && rows.contains(&index));
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(prefs::scaled(start(depth) + SLOT / 2.))
                    .w(px(1.))
                    .bg(if lit { lit_line } else { line })
            })
        });
        let checkbox = row.checked.map(|state| self.checkbox(index, state, cx));
        let body = if row.header() {
            self.header(row, cx)
        } else {
            self.item(row, cx)
        };
        let selected = row.selected && !row.header();
        let element = h_flex()
            .id(("tree-row", index))
            .relative()
            .w_full()
            .h(prefs::scaled(HEIGHT))
            .pl(prefs::scaled(start(row.depth)))
            .pr_1()
            .gap_1p5()
            .text_sm()
            .cursor_pointer()
            .when(selected, |element| element.bg(primary.opacity(0.1)))
            .hover(move |style| {
                style.bg(if selected {
                    primary.opacity(0.14)
                } else {
                    hover
                })
            })
            .active(move |style| {
                style.bg(if selected {
                    primary.opacity(0.18)
                } else {
                    active
                })
            })
            .children(guides.into_iter().flatten())
            .child(body)
            .when_some(row.stats, |element, stats| {
                element.child(toggle::diff_stat(stats.added, stats.deleted, cx))
            })
            .when_some(row.count, |element, count| {
                element.child(toggle::chip(count, cx))
            })
            .children(checkbox)
            .when(
                index == self.cursor && focused && !row.header(),
                |element| element.child(div().absolute().inset_0().border_1().border_color(ring)),
            )
            .when_some(row.hint.clone(), |element, hint| element.tip_text(hint))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                let row = &this.rows[index];
                if row.header() {
                    let event = if row.expanded == Some(true) {
                        "collapse"
                    } else {
                        "expand"
                    };
                    this.act_at(index, event, cx);
                    return;
                }
                this.cursor = index;
                // The first click already toggled a branch.
                let branch = row.expanded.is_some();
                match event.click_count() {
                    1 => this.act_at(index, "select", cx),
                    _ if branch => {}
                    _ => this.act_at(index, "open", cx),
                }
                cx.notify();
            }));
        if row.menu.is_empty() {
            return element.into_any_element();
        }
        let (items, view) = (row.menu.clone(), self.view.clone());
        element
            .right_click_menu(move |menu, _, _| super::menu::fill(menu, &items, &view))
            .trigger_style(StyleRefinement::default().w_full())
            .into_any_element()
    }

    /// A section title: a chevron and a small muted label.
    fn header(&self, row: &Row, cx: &App) -> Div {
        let muted = cx.theme().muted_foreground;
        h_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .text_xs()
            .text_color(muted)
            .child(chevron(row.expanded.unwrap_or(true), muted))
            .child(div().flex_1().min_w_0().child(faded(row.label.clone())))
    }

    /// Chevron, icon, label and the muted detail after it.
    fn item(&self, row: &Row, cx: &App) -> Div {
        let muted = cx.theme().muted_foreground;
        let color = super::render::tone_color(row.tone, cx);
        let icon_color = super::render::tone_color(
            row.icon_tone.unwrap_or(match row.expanded {
                Some(_) => Tone::Muted,
                None => row.tone,
            }),
            cx,
        );
        let icon = row
            .icon
            .as_deref()
            .and_then(crate::declarative::icons::lucide)
            .unwrap_or(match row.expanded {
                Some(true) => Lucide::FolderOpen,
                Some(false) => Lucide::Folder,
                None => Lucide::File,
            });
        h_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .when(self.layout.chevrons, |item| {
                item.child(match row.expanded {
                    Some(open) => chevron(open, muted).into_any_element(),
                    None => div().flex_none().w(prefs::scaled(SLOT)).into_any_element(),
                })
            })
            .child(
                h_flex()
                    .flex_none()
                    .w(prefs::scaled(SLOT))
                    .justify_center()
                    .child(Icon::new(icon).small().text_color(icon_color)),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1p5()
                    .when(row.strike, |text| text.line_through())
                    .child(
                        div()
                            .min_w_0()
                            .text_color(color)
                            .child(faded(row.label.clone())),
                    )
                    .when(!row.detail.is_empty(), |text| {
                        text.child(
                            div()
                                .flex_1()
                                .flex_basis(px(0.))
                                .min_w_0()
                                .text_color(muted)
                                .child(faded_start(row.detail.clone())),
                        )
                    }),
            )
    }

    fn checkbox(&self, index: usize, state: Check, cx: &mut Context<Self>) -> Stateful<Div> {
        toggle::checkbox(("tree-check", index), state, cx).on_click(cx.listener(
            move |this, _, _, cx| {
                cx.stop_propagation();
                this.act_at(index, "check", cx);
            },
        ))
    }
}

fn chevron(open: bool, color: Hsla) -> Div {
    let icon = if open {
        IconName::ChevronDown
    } else {
        IconName::ChevronRight
    };
    h_flex()
        .flex_none()
        .w(prefs::scaled(SLOT))
        .justify_center()
        .text_color(color)
        .child(Icon::new(icon).xsmall())
}
