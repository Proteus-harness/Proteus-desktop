//! A virtualized list with a lane column, such as a commit history; what
//! the rows mean belongs to the process.

use super::lanes_paint::{self, LANE, LEFT};
use crate::declarative::lanes::Row;
use crate::ui::context::RightClickMenu as _;
use crate::ui::fade::faded;
use crate::ui::prefs;
use crate::ui::tip::Tip as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::ops::Range;
use std::rc::Rc;

const HEIGHT: f32 = 24.;

pub(super) struct Lanes {
    rows: Vec<Rc<Row>>,
    /// Lanes the widest row uses, so every row's column is as wide.
    lanes: usize,
    cursor: usize,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    view: WeakEntity<super::view::DeclarativeView>,
    action: super::tree::RowAction,
}

impl Lanes {
    pub fn new(
        rows: Vec<Row>,
        view: WeakEntity<super::view::DeclarativeView>,
        cx: &mut Context<Self>,
        action: impl Fn(String, &'static str, &mut App) + 'static,
    ) -> Self {
        let mut lanes = Self {
            rows: Vec::new(),
            lanes: 1,
            cursor: 0,
            focus: cx.focus_handle().tab_stop(true),
            scroll: UniformListScrollHandle::new(),
            view,
            action: Rc::new(action),
        };
        lanes.set_rows(rows, cx);
        lanes
    }

    pub fn set_rows(&mut self, rows: Vec<Row>, cx: &mut Context<Self>) {
        if self.rows.len() == rows.len() && self.rows.iter().zip(&rows).all(|(a, b)| **a == *b) {
            return;
        }
        let selected = rows.iter().position(|row| row.selected);
        let moved = selected.map(|index| rows[index].id.as_str())
            != self
                .rows
                .iter()
                .find(|row| row.selected)
                .map(|row| row.id.as_str());
        let previous = self.rows.get(self.cursor).map(|row| row.id.clone());
        self.cursor = selected
            .or_else(|| {
                rows.iter()
                    .position(|row| Some(&row.id) == previous.as_ref())
            })
            .unwrap_or(self.cursor)
            .min(rows.len().saturating_sub(1));
        self.lanes = rows.iter().map(Row::width).max().unwrap_or(1);
        self.rows = rows.into_iter().map(Rc::new).collect();
        if moved {
            self.scroll
                .scroll_to_item(self.cursor, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    pub fn focus(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn act_at(&self, index: usize, event: &'static str, cx: &mut App) {
        if let Some(row) = self.rows.get(index) {
            (self.action)(row.id.clone(), event, cx);
        }
    }

    fn key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let last = self.rows.len().saturating_sub(1);
        self.cursor = match event.keystroke.key.as_str() {
            "up" => self.cursor.saturating_sub(1),
            "down" => (self.cursor + 1).min(last),
            "home" => 0,
            "end" => last,
            "pageup" => self.cursor.saturating_sub(12),
            "pagedown" => (self.cursor + 12).min(last),
            "enter" => {
                self.act_at(self.cursor, "open", cx);
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        self.scroll
            .scroll_to_item(self.cursor, ScrollStrategy::Nearest);
        self.act_at(self.cursor, "select", cx);
        cx.notify();
        cx.stop_propagation();
    }

    fn row(&self, index: usize, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let row = self.rows[index].clone();
        let theme = cx.theme();
        let (hover, active, primary, ring) = (
            theme.list_hover,
            theme.list_active,
            theme.primary,
            theme.ring,
        );
        let muted = theme.muted_foreground;
        let selected = row.selected;
        let painted = row.clone();
        let graph = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| lanes_paint::paint(bounds, &painted, window),
        )
        .flex_none()
        .h_full()
        .w(px(LEFT * 2. + self.lanes as f32 * LANE));
        let refs = row.refs.iter().map(|chip| {
            let color = super::render::tone_color(chip.tone, cx);
            div()
                .flex_none()
                .px_1()
                .rounded(px(3.))
                .border_1()
                .border_color(color.opacity(0.4))
                .bg(color.opacity(0.08))
                .text_xs()
                .line_height(prefs::scaled(14.))
                .text_color(color)
                .child(chip.label.clone())
        });
        let element = h_flex()
            .id(("lanes-row", index))
            .relative()
            .w_full()
            .h(prefs::scaled(HEIGHT))
            .pr_2()
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
            .child(graph)
            .child(div().min_w_0().child(faded(row.label.clone())))
            .children(refs)
            .when_some(row.detail.clone(), |element, detail| {
                element.child(
                    div()
                        .flex_none()
                        .max_w(relative(0.3))
                        .text_xs()
                        .text_color(muted)
                        .child(faded(detail)),
                )
            })
            .child(div().flex_1())
            .when_some(row.time.clone(), |element, time| {
                element.child(div().flex_none().text_xs().text_color(muted).child(time))
            })
            .when(index == self.cursor && focused, |element| {
                element.child(div().absolute().inset_0().border_1().border_color(ring))
            })
            .when_some(row.hint.clone(), |element, hint| element.tip_text(hint))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                this.cursor = index;
                let event = if event.click_count() > 1 {
                    "open"
                } else {
                    "select"
                };
                this.act_at(index, event, cx);
                cx.notify();
            }));
        if row.menu.is_empty() {
            return element.into_any_element();
        }
        let view = self.view.clone();
        element
            .right_click_menu(move |menu, _, _| super::menu::fill(menu, &row.menu, &view))
            .trigger_style(StyleRefinement::default().w_full())
            .into_any_element()
    }
}

impl Render for Lanes {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        let focused = self.focus.contains_focused(window, cx);
        let list = uniform_list(
            "extension-lanes",
            self.rows.len(),
            cx.processor(move |this, range: Range<usize>, _, cx| {
                range.map(|index| this.row(index, focused, cx)).collect()
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
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.key(event, cx)))
            .child(list)
    }
}
