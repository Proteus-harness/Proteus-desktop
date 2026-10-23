//! Virtualized hierarchical rows; domain actions belong to the process.

use crate::declarative::tree::Row;
use gpui_kit::component::v_flex;
use gpui_kit::*;
use std::ops::Range;
use std::rc::Rc;

/// How rows are indented and joined.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Layout {
    pub indent: f32,
    pub chevrons: bool,
    pub guides: bool,
}

/// Sends an event of the row with this id to the process.
pub(super) type RowAction = Rc<dyn Fn(String, &'static str, &mut App)>;

pub(super) struct Tree {
    pub rows: Vec<Row>,
    pub layout: Layout,
    pub(super) cursor: usize,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    pub(super) view: WeakEntity<super::view::DeclarativeView>,
    action: RowAction,
}

impl Tree {
    pub fn new(
        rows: Vec<Row>,
        layout: Layout,
        view: WeakEntity<super::view::DeclarativeView>,
        cx: &mut Context<Self>,
        action: impl Fn(String, &'static str, &mut App) + 'static,
    ) -> Self {
        let mut tree = Self {
            rows,
            layout,
            cursor: 0,
            focus: cx.focus_handle().tab_stop(true),
            scroll: UniformListScrollHandle::new(),
            view,
            action: Rc::new(action),
        };
        tree.cursor = tree.rows.iter().position(|row| row.selected).unwrap_or(0);
        tree.cursor = tree.step(tree.cursor, 1);
        tree
    }

    pub fn set_rows(&mut self, rows: Vec<Row>, layout: Layout, cx: &mut Context<Self>) {
        if self.rows == rows && self.layout == layout {
            return;
        }
        self.layout = layout;
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
        self.cursor = self.step(self.cursor, 1);
        if moved {
            self.scroll
                .scroll_to_item(self.cursor, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    pub fn focus(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub(super) fn act_at(&self, index: usize, event: &'static str, cx: &mut App) {
        if let Some(row) = self.rows.get(index) {
            (self.action)(row.id.clone(), event, cx);
        }
    }

    fn act(&self, event: &'static str, cx: &mut App) {
        self.act_at(self.cursor, event, cx);
    }

    /// The nearest item from `index` in the direction of `toward` (±1),
    /// passing headers by; the other way when none is there.
    fn step(&self, index: usize, toward: isize) -> usize {
        let item = |index: &usize| self.rows.get(*index).is_some_and(|row| !row.header());
        let index = index.min(self.rows.len().saturating_sub(1));
        let forward = (index..self.rows.len()).find(item);
        let backward = (0..=index).rev().find(item);
        let found = if toward >= 0 {
            forward.or(backward)
        } else {
            backward.or(forward)
        };
        found.unwrap_or(self.cursor)
    }

    /// Rows under the open branch nearest to the cursor and that branch's
    /// depth: its guide is drawn brighter.
    pub(super) fn active_guide(&self) -> Option<(Range<usize>, usize)> {
        let row = self.rows.get(self.cursor)?;
        let opened = row.expanded == Some(true)
            && self
                .rows
                .get(self.cursor + 1)
                .is_some_and(|next| next.depth > row.depth);
        let anchor = if opened {
            self.cursor
        } else {
            self.rows[..self.cursor]
                .iter()
                .rposition(|parent| parent.depth < row.depth)?
        };
        let depth = self.rows[anchor].depth;
        let end = self.rows[anchor + 1..]
            .iter()
            .position(|row| row.depth <= depth)
            .map_or(self.rows.len(), |offset| anchor + 1 + offset);
        Some((anchor + 1..end, depth))
    }

    fn key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let last = self.rows.len().saturating_sub(1);
        match event.keystroke.key.as_str() {
            "up" => self.cursor = self.step(self.cursor.saturating_sub(1), -1),
            "down" => self.cursor = self.step((self.cursor + 1).min(last), 1),
            "home" => self.cursor = self.step(0, 1),
            "end" => self.cursor = self.step(last, -1),
            "pageup" => self.cursor = self.step(self.cursor.saturating_sub(12), -1),
            "pagedown" => self.cursor = self.step((self.cursor + 12).min(last), 1),
            "space" => {
                self.act("select", cx);
                cx.stop_propagation();
                return;
            }
            "enter" => {
                self.act("open", cx);
                cx.stop_propagation();
                return;
            }
            "left" => {
                let Some(row) = self.rows.get(self.cursor) else {
                    return;
                };
                if row.expanded == Some(true) {
                    self.act("collapse", cx);
                    cx.stop_propagation();
                    return;
                }
                let depth = row.depth;
                match self.rows[..self.cursor]
                    .iter()
                    .rposition(|row| row.depth < depth && !row.header())
                {
                    Some(parent) => self.cursor = parent,
                    None => return,
                }
            }
            "right" => {
                let Some(row) = self.rows.get(self.cursor) else {
                    return;
                };
                let child = self
                    .rows
                    .get(self.cursor + 1)
                    .is_some_and(|next| next.depth > row.depth && !next.header());
                if row.expanded != Some(true) || !child {
                    self.act("expand", cx);
                    cx.stop_propagation();
                    return;
                }
                self.cursor += 1;
            }
            _ => return,
        }
        self.scroll
            .scroll_to_item(self.cursor, ScrollStrategy::Nearest);
        self.act("preview", cx);
        cx.notify();
        cx.stop_propagation();
    }
}

impl Render for Tree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        let focused = self.focus.contains_focused(window, cx);
        let guide = self.active_guide();
        let list = uniform_list(
            "extension-tree",
            self.rows.len(),
            cx.processor(move |this, range: Range<usize>, _, cx| {
                range
                    .map(|index| this.row(index, focused, guide.clone(), cx))
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
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.key(event, cx)))
            .child(list)
    }
}
