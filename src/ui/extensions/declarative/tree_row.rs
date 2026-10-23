// Адаптировано из Zed: crates/ui/src/components/list/list_item.rs, crates/ui/src/components/indent_guides.rs, crates/git_ui/src/git_panel.rs
//! One row of [`Tree`]: an item like a Zed list item or a section header
//! like the Zed git panel's, with indent guides.

use super::toggle;
use super::tree::Tree;
use crate::declarative::tree::{Check, Row};
use crate::declarative::widget::Tone;
use crate::ui::context::RightClickMenu as _;
use crate::ui::fade::{caption, faded_start};
use crate::ui::motion;
use crate::ui::prefs;
use crate::ui::tip::{Tip as _, TooltipAnchor};
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
        let id = ElementId::NamedInteger("tree-row".into(), index as u64);
        self.draw(&self.rows[index], Some(index), id, focused, guide, cx)
    }

    /// A row the process no longer sends, kept while its branch closes; it
    /// looks the same and takes no input.
    pub(super) fn ghost(&self, row: &Row, n: usize, cx: &mut Context<Self>) -> AnyElement {
        let id = ElementId::NamedInteger("tree-ghost".into(), n as u64);
        self.draw(row, None, id, false, None, cx)
    }

    fn draw(
        &self,
        row: &Row,
        index: Option<usize>,
        id: ElementId,
        focused: bool,
        guide: Option<(Range<usize>, usize)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let selected_foreground = theme.sidebar_accent_foreground;
        let (line, lit_line) = (theme.border, theme.muted_foreground.opacity(0.5));
        let indent = self.layout.indent;
        let start = move |depth: usize| INSET + depth as f32 * indent;
        let guides = self.layout.guides.then(|| {
            (0..row.depth).map(move |depth| {
                let lit = guide
                    .as_ref()
                    .zip(index)
                    .is_some_and(|((rows, level), index)| *level == depth && rows.contains(&index));
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(prefs::scaled(start(depth) + SLOT / 2.))
                    .w(px(1.))
                    .bg(if lit { lit_line } else { line })
            })
        });
        let checkbox = row.checked.map(|state| match index {
            Some(index) => self.checkbox(index, state, cx).into_any_element(),
            None => toggle::checkbox(id.clone(), state, cx).into_any_element(),
        });
        let anchor = TooltipAnchor::default();
        let body = if row.header() {
            self.header(row, &anchor, cx)
        } else {
            self.item(row, &anchor, cx)
        };
        let hint = row.hint.clone().filter(|hint| {
            let hint = hint.trim();
            !hint.is_empty() && hint != row.label.trim() && hint != row.detail.trim()
        });
        let selected = row.selected && !row.header();
        let element = h_flex()
            .id(id)
            .tip_anchor(anchor)
            .relative()
            .w_full()
            .h(prefs::scaled(HEIGHT))
            .pl(prefs::scaled(start(row.depth)))
            .pr_1()
            .gap_1p5()
            .text_sm()
            .cursor_pointer()
            .group("tree-selection")
            .child(crate::ui::selection::background(
                selected,
                false,
                "tree-selection",
                true,
                cx,
            ))
            .when(selected, |element| element.text_color(selected_foreground))
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
                index == Some(self.cursor)
                    && focused
                    && !row.header()
                    && crate::ui::kit::keyboard(),
                |element| element.child(crate::ui::selection::focus(cx)),
            )
            .when_some(hint, |element, hint| element.tip_text(hint));
        let Some(index) = index else {
            return element.into_any_element();
        };
        let element = element.on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
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
    fn header(&self, row: &Row, anchor: &TooltipAnchor, cx: &App) -> Div {
        let muted = cx.theme().muted_foreground;
        h_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .text_xs()
            .text_color(muted)
            .child(chevron(&row.id, row.expanded.unwrap_or(true), muted))
            .child(
                div().flex_1().min_w_0().child(
                    caption("tree-header-caption", row.label.clone()).anchor(anchor.clone()),
                ),
            )
    }

    /// Chevron, icon, label and the muted detail after it.
    fn item(&self, row: &Row, anchor: &TooltipAnchor, cx: &App) -> Div {
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
                    Some(open) => chevron(&row.id, open, muted).into_any_element(),
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
                    .child(div().min_w_0().text_color(color).child(
                        caption("tree-label-caption", row.label.clone()).anchor(anchor.clone()),
                    ))
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

fn chevron(id: &str, open: bool, color: Hsla) -> Div {
    h_flex()
        .flex_none()
        .w(prefs::scaled(SLOT))
        .justify_center()
        .child(motion::chevron(
            ElementId::Name(format!("tree-chevron-{id}").into()),
            open,
            color,
        ))
}
