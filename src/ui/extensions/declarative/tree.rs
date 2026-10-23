//! Virtualized hierarchical rows; domain actions belong to the process.
//! A branch that opens or closes draws its children as one block that grows
//! or shrinks, like every other disclosure.

use crate::declarative::tree::Row;
use crate::ui::motion;
use gpui_kit::component::v_flex;
use gpui_kit::*;
use std::collections::HashMap;
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
    list: ListState,
    /// Branches opening or closing right now.
    folds: Vec<Fold>,
    /// Keeps the motion keys of this tree apart from other trees'.
    entity: EntityId,
    pub(super) view: WeakEntity<super::view::DeclarativeView>,
    action: RowAction,
}

/// A branch whose children grow or shrink; a closing one keeps the children
/// the process no longer sends until they have folded away.
struct Fold {
    id: String,
    open: bool,
    gone: Vec<Row>,
}

/// Where a moving fold sits in the current rows.
#[derive(Clone)]
struct Span {
    branch: usize,
    children: Range<usize>,
    fold: usize,
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
            list: ListState::new(0, ListAlignment::Top, px(240.)),
            folds: Vec::new(),
            entity: cx.entity_id(),
            view,
            action: Rc::new(action),
        };
        tree.list.splice(0..0, tree.rows.len());
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
        for fold in folds(&self.rows, &rows) {
            motion::toggled(self.fold_key(&fold.id), fold.open, cx);
            self.folds.retain(|old| old.id != fold.id);
            self.folds.push(fold);
        }
        // Rows keep their measured heights where the ids stay in place.
        let (old, new) = (self.rows.len(), rows.len());
        let same = |a: &Row, b: &Row| a.id == b.id;
        let prefix = self
            .rows
            .iter()
            .zip(&rows)
            .take_while(|(a, b)| same(a, b))
            .count();
        let suffix = self.rows[prefix..]
            .iter()
            .rev()
            .zip(rows[prefix..].iter().rev())
            .take_while(|(a, b)| same(a, b))
            .count();
        self.list
            .splice(prefix..old - suffix, new - prefix - suffix);
        self.rows = rows;
        self.cursor = self.step(self.cursor, 1);
        if moved {
            self.list.scroll_to_reveal_item(self.cursor);
        }
        cx.notify();
    }

    fn fold_key(&self, id: &str) -> SharedString {
        format!("tree-{}-{id}", self.entity).into()
    }

    /// Moving folds by row, dropping the ones that have settled.
    fn spans(&mut self, cx: &App) -> Vec<Span> {
        let mut settled = Vec::new();
        let keys: Vec<_> = self
            .folds
            .iter()
            .map(|fold| self.fold_key(&fold.id))
            .collect();
        let mut index = 0;
        self.folds.retain(|fold| {
            let moving = motion::moving(&keys[index], fold.open, cx);
            index += 1;
            if !moving {
                settled.push(fold.id.clone());
            }
            moving
        });
        let at: HashMap<&str, usize> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id.as_str(), index))
            .collect();
        for id in settled {
            if let Some(&branch) = at.get(id.as_str()) {
                let end = children(&self.rows, branch).end;
                self.list.remeasure_items(branch..end);
            }
        }
        self.folds
            .iter()
            .enumerate()
            .filter_map(|(fold, item)| {
                let branch = *at.get(item.id.as_str())?;
                Some(Span {
                    branch,
                    children: children(&self.rows, branch),
                    fold,
                })
            })
            .collect()
    }

    /// Row `index`, or the branch with its moving children, or nothing for
    /// a child drawn by its moving branch.
    fn entry(
        &self,
        index: usize,
        spans: &[Span],
        focused: bool,
        guide: Option<(Range<usize>, usize)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if spans
            .iter()
            .any(|span| self.folds[span.fold].open && span.children.contains(&index))
        {
            return div().into_any_element();
        }
        let row = self.row(index, focused, guide.clone(), cx);
        let Some(span) = spans.iter().find(|span| span.branch == index) else {
            return row;
        };
        let fold = &self.folds[span.fold];
        let block: Vec<AnyElement> = if fold.open {
            span.children
                .clone()
                .map(|child| self.row(child, focused, guide.clone(), cx))
                .collect()
        } else {
            fold.gone
                .iter()
                .enumerate()
                .map(|(n, gone)| self.ghost(gone, n, cx))
                .collect()
        };
        let block = motion::reveal(self.fold_key(&fold.id), fold.open, cx, || {
            v_flex().w_full().children(block).into_any_element()
        });
        v_flex()
            .w_full()
            .child(row)
            .children(block)
            .into_any_element()
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
        self.list.scroll_to_reveal_item(self.cursor);
        self.act("preview", cx);
        cx.notify();
        cx.stop_propagation();
    }
}

impl Render for Tree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        motion::drive(window, cx);
        let focused = self.focus.contains_focused(window, cx);
        let guide = self.active_guide();
        let spans = self.spans(cx);
        let list = list(
            self.list.clone(),
            cx.processor(move |this, index: usize, _, cx| {
                this.entry(index, &spans, focused, guide.clone(), cx)
            }),
        )
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

/// Rows under the branch at `at`.
fn children(rows: &[Row], at: usize) -> Range<usize> {
    let depth = rows[at].depth;
    let end = rows[at + 1..]
        .iter()
        .position(|row| row.depth <= depth)
        .map_or(rows.len(), |offset| at + 1 + offset);
    at + 1..end
}

/// Branches that opened or closed between two sets of rows and have
/// children to show.
fn folds(old: &[Row], new: &[Row]) -> Vec<Fold> {
    let before: HashMap<&str, (usize, bool)> = old
        .iter()
        .enumerate()
        .filter_map(|(index, row)| Some((row.id.as_str(), (index, row.expanded?))))
        .collect();
    new.iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let open = row.expanded?;
            let &(was_at, was_open) = before.get(row.id.as_str())?;
            if was_open == open {
                return None;
            }
            let gone = if open {
                Vec::new()
            } else {
                old[children(old, was_at)].to_vec()
            };
            let shown = if open {
                children(new, index).len()
            } else {
                gone.len()
            };
            (shown > 0).then(|| Fold {
                id: row.id.clone(),
                open,
                gone,
            })
        })
        .collect()
}
