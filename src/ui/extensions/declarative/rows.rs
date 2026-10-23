//! A virtualized list of process rows, shared by `lanes` and `list`: the
//! cursor that survives new rows, the keyboard, focus, clicks and the row
//! menu. Each kind draws its own rows and says what the keys and clicks send.

use crate::declarative::control::MenuItem;
use crate::ui::context::RightClickMenu as _;
use gpui_kit::component::*;
use gpui_kit::*;
use std::ops::Range;
use std::rc::Rc;

/// What one kind of rows looks like and sends.
pub(super) trait Kind: Default + 'static {
    type Row: Clone + PartialEq + 'static;
    /// Id of the virtualized list.
    const NAME: &'static str;
    /// Event the keyboard sends after it moves the cursor, if any.
    const MOVED: Option<&'static str>;
    /// Where the cursor's row scrolls to when it leaves the view.
    const SCROLL: ScrollStrategy;
    /// The cursor is ringed only after the keyboard moved it, not after a
    /// click.
    const RING_AFTER_KEYS: bool;

    fn id(row: &Self::Row) -> &str;
    fn selected(row: &Self::Row) -> bool;
    fn menu(row: &Self::Row) -> &[MenuItem];
    /// Event of a click with this click count.
    fn clicked(count: usize) -> &'static str;
    /// Layout shared by all rows, recomputed when they change.
    fn measure(&mut self, rows: &[Rc<Self::Row>]);
    /// One row without its click and menu; `ring` marks the cursor.
    fn draw(&self, row: &Self::Row, index: usize, ring: bool, cx: &App) -> Stateful<Div>;
}

pub(super) struct Rows<K: Kind> {
    kind: K,
    rows: Vec<Rc<K::Row>>,
    cursor: usize,
    /// The keyboard moved the cursor last, not the mouse.
    keyboard: bool,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    view: WeakEntity<super::view::DeclarativeView>,
    action: super::tree::RowAction,
}

impl<K: Kind> Rows<K> {
    pub fn new(
        rows: Vec<K::Row>,
        view: WeakEntity<super::view::DeclarativeView>,
        cx: &mut Context<Self>,
        action: impl Fn(String, &'static str, &mut App) + 'static,
    ) -> Self {
        let mut list = Self {
            kind: K::default(),
            rows: Vec::new(),
            cursor: 0,
            keyboard: true,
            focus: cx.focus_handle().tab_stop(true),
            scroll: UniformListScrollHandle::new(),
            view,
            action: Rc::new(action),
        };
        list.set_rows(rows, cx);
        list
    }

    pub fn set_rows(&mut self, rows: Vec<K::Row>, cx: &mut Context<Self>) {
        if self.rows.len() == rows.len() && self.rows.iter().zip(&rows).all(|(a, b)| **a == *b) {
            return;
        }
        let selected = rows.iter().position(K::selected);
        let moved = selected.map(|index| K::id(&rows[index]))
            != self
                .rows
                .iter()
                .find(|row| K::selected(row))
                .map(|row| K::id(row));
        let previous = self.rows.get(self.cursor).map(|row| K::id(row).to_owned());
        self.cursor = selected
            .or_else(|| {
                rows.iter()
                    .position(|row| Some(K::id(row)) == previous.as_deref())
            })
            .unwrap_or(self.cursor)
            .min(rows.len().saturating_sub(1));
        self.rows = rows.into_iter().map(Rc::new).collect();
        self.kind.measure(&self.rows);
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
            (self.action)(K::id(row).to_owned(), event, cx);
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
        self.keyboard = true;
        self.scroll.scroll_to_item(self.cursor, K::SCROLL);
        if let Some(event) = K::MOVED {
            self.act_at(self.cursor, event, cx);
        }
        cx.notify();
        cx.stop_propagation();
    }

    fn row(&self, index: usize, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let row = self.rows[index].clone();
        let ring = index == self.cursor && focused && (self.keyboard || !K::RING_AFTER_KEYS);
        let element = self
            .kind
            .draw(&row, index, ring, cx)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.cursor = index;
                    this.keyboard = false;
                    cx.notify();
                }),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                this.cursor = index;
                this.act_at(index, K::clicked(event.click_count()), cx);
                cx.notify();
            }));
        if K::menu(&row).is_empty() {
            return element.into_any_element();
        }
        let view = self.view.clone();
        element
            .right_click_menu(move |menu, _, _| super::menu::fill(menu, K::menu(&row), &view))
            .trigger_style(StyleRefinement::default().w_full())
            .into_any_element()
    }
}

impl<K: Kind> Render for Rows<K> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        let focused = self.focus.contains_focused(window, cx);
        let list = uniform_list(
            K::NAME,
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
