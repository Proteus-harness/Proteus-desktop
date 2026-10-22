//! Virtualized hierarchical rows; domain actions belong to the process.

use crate::declarative::tree::Row;
use crate::declarative::widget::Tone;
use crate::ui::{fade::faded, kit::Pressable as _, prefs};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

pub(super) struct Tree {
    pub rows: Vec<Row>,
    cursor: usize,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    view: WeakEntity<super::view::DeclarativeView>,
    action: Rc<dyn Fn(String, &'static str, &mut App)>,
}

impl Tree {
    pub fn new(
        rows: Vec<Row>,
        view: WeakEntity<super::view::DeclarativeView>,
        cx: &mut Context<Self>,
        action: impl Fn(String, &'static str, &mut App) + 'static,
    ) -> Self {
        Self {
            rows,
            cursor: 0,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            view,
            action: Rc::new(action),
        }
    }

    pub fn set_rows(&mut self, rows: Vec<Row>, cx: &mut Context<Self>) {
        if self.rows == rows {
            return;
        }
        let previous = self.rows.get(self.cursor).map(|row| row.id.as_str());
        let selected = rows.iter().position(|row| row.selected);
        let moved = selected.map(|index| rows[index].id.as_str())
            != self
                .rows
                .iter()
                .find(|row| row.selected)
                .map(|row| row.id.as_str());
        self.cursor = selected
            .or_else(|| {
                rows.iter()
                    .position(|row| Some(row.id.as_str()) == previous)
            })
            .unwrap_or(self.cursor)
            .min(rows.len().saturating_sub(1));
        self.rows = rows;
        if moved {
            self.scroll
                .scroll_to_item(self.cursor, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    pub fn focus(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn act(&self, event: &'static str, cx: &mut App) {
        if let Some(row) = self.rows.get(self.cursor) {
            (self.action)(row.id.clone(), event, cx);
        }
    }
}

impl Render for Tree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        let hover = cx.theme().list_hover;
        let selected = cx.theme().primary.opacity(0.12);
        let border = cx.theme().primary.opacity(0.5);
        let focused = self.focus.contains_focused(window, cx);
        let list = uniform_list(
            "extension-tree",
            self.rows.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| {
                        let row = &this.rows[index];
                        let color = super::render::tone_color(row.tone, cx);
                        let theme = cx.theme();
                        let (muted, mono, guide) = (
                            theme.muted_foreground,
                            theme.mono_font_family.clone(),
                            theme.border.opacity(0.6),
                        );
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
                        let chevron = row.expanded.map(|open| {
                            if open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            }
                        });
                        let checkbox = row.checked.map(|checked| {
                            Checkbox::new(("tree-check", index))
                                .checked(checked)
                                .small()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.cursor = index;
                                    this.act("check", cx);
                                    cx.stop_propagation();
                                }))
                        });
                        let menu =
                            (!row.menu.is_empty()).then(|| (row.menu.clone(), this.view.clone()));
                        let indent = |depth: usize| prefs::scaled(4. + depth as f32 * 14.);
                        let element = h_flex()
                            .id(("tree-row", index))
                            .relative()
                            .w_full()
                            .h(prefs::scaled(24.))
                            .gap_1p5()
                            .pr_2()
                            .pl(indent(row.depth))
                            .text_sm()
                            .text_color(color)
                            .cursor_pointer()
                            .pressable(cx)
                            .when(row.selected, |row| row.bg(selected))
                            .when(index == this.cursor && focused, |row| {
                                row.border_1().border_color(border)
                            })
                            .hover(move |row| row.bg(hover))
                            .children((0..row.depth).map(|depth| {
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left(indent(depth) + prefs::scaled(6.))
                                    .w(px(1.))
                                    .bg(guide)
                            }))
                            .children(checkbox)
                            .child(
                                div()
                                    .flex_none()
                                    .w(prefs::scaled(12.))
                                    .text_color(muted)
                                    .children(chevron.map(|chevron| Icon::new(chevron).xsmall())),
                            )
                            .child(Icon::new(icon).small().text_color(icon_color))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .when(row.strike, |label| label.line_through())
                                    .child(faded(row.label.clone())),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .font_family(mono)
                                    .child(row.detail.clone()),
                            )
                            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                                this.cursor = index;
                                // The first click already toggled a branch.
                                let branch = this.rows[index].expanded.is_some();
                                match event.click_count() {
                                    1 => this.act("select", cx),
                                    _ if branch => {}
                                    _ => this.act("open", cx),
                                }
                                cx.notify();
                            }));
                        match menu {
                            Some((items, view)) => element
                                .context_menu(move |menu, _, _| {
                                    super::menu::fill(menu, &items, &view)
                                })
                                .into_any_element(),
                            None => element.into_any_element(),
                        }
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0()
        .w_full();
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus.focus(window, cx)),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let last = this.rows.len().saturating_sub(1);
                match event.keystroke.key.as_str() {
                    "up" => this.cursor = this.cursor.saturating_sub(1),
                    "down" => this.cursor = (this.cursor + 1).min(last),
                    "home" => this.cursor = 0,
                    "end" => this.cursor = last,
                    "pageup" => this.cursor = this.cursor.saturating_sub(12),
                    "pagedown" => this.cursor = (this.cursor + 12).min(last),
                    "space" => {
                        this.act("select", cx);
                        cx.stop_propagation();
                        return;
                    }
                    "enter" => {
                        this.act("open", cx);
                        cx.stop_propagation();
                        return;
                    }
                    "left" => {
                        let Some(row) = this.rows.get(this.cursor) else {
                            return;
                        };
                        if row.expanded == Some(true) {
                            this.act("collapse", cx);
                            cx.stop_propagation();
                            return;
                        }
                        let depth = row.depth;
                        match this.rows[..this.cursor]
                            .iter()
                            .rposition(|row| row.depth < depth)
                        {
                            Some(parent) => this.cursor = parent,
                            None => return,
                        }
                    }
                    "right" => {
                        let Some(row) = this.rows.get(this.cursor) else {
                            return;
                        };
                        let child = this
                            .rows
                            .get(this.cursor + 1)
                            .is_some_and(|next| next.depth > row.depth);
                        if row.expanded != Some(true) || !child {
                            this.act("expand", cx);
                            cx.stop_propagation();
                            return;
                        }
                        this.cursor += 1;
                    }
                    _ => return,
                }
                this.scroll
                    .scroll_to_item(this.cursor, ScrollStrategy::Nearest);
                this.act("preview", cx);
                cx.notify();
                cx.stop_propagation();
            }))
            .child(list)
    }
}
