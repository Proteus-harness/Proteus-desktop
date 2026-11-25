//! Virtualized hierarchical rows; domain actions belong to the process.
//! A branch that opens or closes draws its children as one block that grows
//! or shrinks, like every other disclosure.

use super::fields::WidgetAction;
use super::tree_row::ROW_HEIGHT;
use crate::declarative::tree::Row;
use crate::ui::{motion, prefs};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

const LIST_OVERDRAW: Pixels = px(240.);

/// How rows are indented and joined.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Layout {
    pub indent: f32,
    pub chevrons: bool,
    pub guides: bool,
}

pub(super) struct Tree {
    pub rows: Arc<Vec<Row>>,
    pub layout: Layout,
    pub(super) cursor: usize,
    focus: FocusHandle,
    list: ListState,
    /// Branches opening or closing right now.
    folds: Vec<Fold>,
    /// Keeps the motion keys of this tree apart from other trees'.
    entity: EntityId,
    pub(super) view: WeakEntity<super::view::DeclarativeView>,
    pub(super) action: WidgetAction,
    /// Embedded surfaces belong to the host scroller, so small trees render
    /// existing rows directly instead of requiring a virtual-list viewport.
    natural: bool,
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
        rows: Arc<Vec<Row>>,
        layout: Layout,
        natural: bool,
        view: WeakEntity<super::view::DeclarativeView>,
        cx: &mut Context<Self>,
        action: WidgetAction,
    ) -> Self {
        let mut tree = Self {
            rows,
            layout,
            cursor: 0,
            focus: cx.focus_handle().tab_stop(true),
            list: ListState::new(0, ListAlignment::Top, LIST_OVERDRAW),
            folds: Vec::new(),
            entity: cx.entity_id(),
            view,
            action,
            natural,
        };
        tree.list.splice(0..0, tree.rows.len());
        tree.cursor = tree.rows.iter().position(|row| row.selected).unwrap_or(0);
        tree.cursor = tree.step(tree.cursor, 1);
        tree
    }

    pub fn set_rows(&mut self, rows: Arc<Vec<Row>>, layout: Layout, cx: &mut Context<Self>) {
        // Equal payloads keep the current snapshot, including menu value representation.
        if self.rows.as_slice() == rows.as_slice() && self.layout == layout {
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
        // Rows keep their measured heights where the ids stay in place.
        let (old, new) = (self.rows.len(), rows.len());
        let same = |a: &Row, b: &Row| a.id == b.id;
        let prefix = self
            .rows
            .iter()
            .zip(rows.iter())
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
        // A fold builds every child as part of a single List item. Larger
        // branches must use normal virtual rows, including on collapse,
        // where keeping the children would also clone the whole branch.
        let max_fold_rows = self.fold_row_limit(cx);
        for (branch, open, child_rows) in fold_changes(&self.rows, &rows) {
            let id = &rows[branch].id;
            if let Some(index) = self.folds.iter().position(|fold| fold.id == *id) {
                self.folds.remove(index);
                self.list
                    .remeasure_items(branch..children(&rows, branch).end);
            }
            if child_rows.len() > max_fold_rows {
                continue;
            }
            let gone = if open {
                Vec::new()
            } else {
                self.rows[child_rows].to_vec()
            };
            motion::toggled(Self::fold_key(self.entity, id), open, cx);
            self.folds.push(Fold {
                id: id.clone(),
                open,
                gone,
            });
        }
        self.rows = rows;
        self.cursor = self.step(self.cursor, 1);
        if moved {
            self.list.scroll_to_reveal_item(self.cursor);
        }
        cx.notify();
    }

    fn fold_key(entity: EntityId, id: &str) -> SharedString {
        format!("tree-{entity}-{id}").into()
    }

    fn fold_row_limit(&self, cx: &App) -> usize {
        if self.natural {
            return usize::MAX;
        }
        let viewport = self.list.viewport_bounds().size.height;
        ((viewport + LIST_OVERDRAW * 2.) / px(ROW_HEIGHT * prefs::scale(cx))).ceil() as usize
    }

    /// Moving folds by row, dropping the ones that have settled.
    fn spans(&mut self, cx: &App) -> Vec<Span> {
        if self.folds.is_empty() {
            return Vec::new();
        }
        let rows = &self.rows;
        let at: HashMap<&str, usize> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id.as_str(), index))
            .collect();
        let max_fold_rows = self.fold_row_limit(cx);
        let list = &self.list;
        let entity = self.entity;
        let mut spans = Vec::new();
        self.folds.retain(|fold| {
            let Some(&branch) = at.get(fold.id.as_str()) else {
                return false;
            };
            let children = children(rows, branch);
            let shown = if fold.open {
                children.len()
            } else {
                fold.gone.len()
            };
            let key = Self::fold_key(entity, &fold.id);
            if shown > max_fold_rows || !motion::moving(&key, fold.open, cx) {
                list.remeasure_items(branch..children.end);
                return false;
            }
            spans.push(Span {
                branch,
                children,
                fold: spans.len(),
            });
            true
        });
        spans
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
        let block = motion::reveal(Self::fold_key(self.entity, &fold.id), fold.open, cx, || {
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
            self.action.row(&row.id, event, cx);
        }
    }

    fn act(&self, event: &'static str, cx: &mut App) {
        // Only a header-only tree can have keyboard focus on a section.
        // Navigation stays local; activation uses the section's mouse events.
        let event = match self.rows.get(self.cursor).filter(|row| row.header()) {
            Some(_) if event == "preview" => return,
            Some(row) if matches!(event, "open" | "select") => {
                if row.expanded == Some(true) {
                    "collapse"
                } else {
                    "expand"
                }
            }
            _ => event,
        };
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
        // If no items remain, let the keyboard reach each section to reopen it.
        found.unwrap_or(index)
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
        let body = if self.natural {
            v_flex()
                .w_full()
                .children(
                    (0..self.rows.len())
                        .map(|index| self.entry(index, &spans, focused, guide.clone(), cx)),
                )
                .into_any_element()
        } else {
            list(
                self.list.clone(),
                cx.processor(move |this, index: usize, _, cx| {
                    this.entry(index, &spans, focused, guide.clone(), cx)
                }),
            )
            .flex_1()
            .min_h_0()
            .w_full()
            .into_any_element()
        };
        v_flex()
            .w_full()
            .when(!self.natural, |view| view.h_full())
            .track_focus(&self.focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus.focus(window, cx)),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.key(event, cx)))
            .child(body)
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

/// Each changed branch in the new list and its revealed/removed child range.
fn fold_changes<'a>(
    old: &'a [Row],
    new: &'a [Row],
) -> impl Iterator<Item = (usize, bool, Range<usize>)> + 'a {
    let before: HashMap<&str, (usize, bool)> = old
        .iter()
        .enumerate()
        .filter_map(|(index, row)| Some((row.id.as_str(), (index, row.expanded?))))
        .collect();
    new.iter().enumerate().filter_map(move |(index, row)| {
        let open = row.expanded?;
        let &(was_at, was_open) = before.get(row.id.as_str())?;
        if was_open == open {
            return None;
        }
        let range = if open {
            children(new, index)
        } else {
            children(old, was_at)
        };
        (!range.is_empty()).then_some((index, open, range))
    })
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    use serde_json::json;
    use std::{cell::RefCell, rc::Rc};

    #[::core::prelude::v1::test]
    fn large_folders_keep_virtual_rows_and_preserve_the_cursor() {
        let rows = |open: bool, count: usize| {
            let mut rows = vec![
                serde_json::from_value(json!({
                    "id":"folder", "label":"folder", "expanded":open
                }))
                .unwrap(),
            ];
            rows.extend((0..count).map(|index| {
                serde_json::from_value(json!({
                    "id":format!("file-{index}"), "label":"file", "depth":1
                }))
                .unwrap()
            }));
            rows.push(serde_json::from_value(json!({"id":"sibling","label":"sibling"})).unwrap());
            Arc::new(rows)
        };
        let mut app = ::gpui::TestAppContext::single();
        app.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::prefs::init_in_memory(cx);
        });
        let window = app.add_empty_window();
        window.update(|_, cx| {
            let tree = cx.new(|cx| {
                Tree::new(
                    rows(false, 0),
                    Layout {
                        indent: 16.,
                        chevrons: true,
                        guides: false,
                    },
                    false,
                    WeakEntity::new_invalid(),
                    cx,
                    WidgetAction::new("files", |_, _, _| {}),
                )
            });
            tree.update(cx, |tree, cx| {
                tree.cursor = 1;
                tree.set_rows(rows(true, 2), tree.layout, cx);
                assert_eq!(tree.folds.len(), 1);
                tree.set_rows(rows(false, 0), tree.layout, cx);
                assert_eq!(tree.folds[0].gone.len(), 2);

                tree.set_rows(rows(true, 100), tree.layout, cx);
                assert!(tree.folds.is_empty());
                assert_eq!(tree.list.item_count(), 102);
                assert_eq!(tree.rows[tree.cursor].id, "sibling");
                tree.set_rows(rows(false, 0), tree.layout, cx);
                assert!(tree.folds.is_empty());
                assert_eq!(tree.list.item_count(), 2);
                assert_eq!(tree.rows[tree.cursor].id, "sibling");

                // A small branch may receive more rows while opening.
                tree.set_rows(rows(true, 2), tree.layout, cx);
                assert_eq!(tree.folds.len(), 1);
                tree.set_rows(rows(true, 100), tree.layout, cx);
                assert!(tree.spans(cx).is_empty());
                assert_eq!(tree.rows[tree.cursor].id, "sibling");
            });
        });
    }

    #[::core::prelude::v1::test]
    fn all_collapsed_sections_remain_keyboard_reachable_and_expandable() {
        let mut app = ::gpui::TestAppContext::single();
        app.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::prefs::init_in_memory(cx);
        });
        let window = app.add_empty_window();
        let sent = Rc::new(RefCell::new(Vec::new()));
        window.update(|_, cx| {
            let observed = sent.clone();
            let action = WidgetAction::new("sections", move |_, value, _| {
                observed.borrow_mut().push(value)
            });
            let row = |id| {
                serde_json::from_value(json!({
                    "id":id, "label":id, "kind":"header", "expanded":false
                }))
                .unwrap()
            };
            let tree = cx.new(|cx| {
                Tree::new(
                    Arc::new(vec![row("staged"), row("unstaged")]),
                    Layout {
                        indent: 16.,
                        chevrons: true,
                        guides: false,
                    },
                    false,
                    WeakEntity::new_invalid(),
                    cx,
                    action,
                )
            });
            tree.update(cx, |tree, cx| {
                tree.key(
                    &KeyDownEvent {
                        keystroke: Keystroke::parse("down").unwrap(),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    cx,
                );
                assert_eq!(tree.cursor, 1);
                tree.key(
                    &KeyDownEvent {
                        keystroke: Keystroke::parse("enter").unwrap(),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    cx,
                );
                tree.key(
                    &KeyDownEvent {
                        keystroke: Keystroke::parse("up").unwrap(),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    cx,
                );
                assert_eq!(tree.cursor, 0);
                tree.key(
                    &KeyDownEvent {
                        keystroke: Keystroke::parse("space").unwrap(),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    cx,
                );
            });
        });
        assert_eq!(
            *sent.borrow(),
            vec![
                json!({"id":"unstaged","event":"expand"}),
                json!({"id":"staged","event":"expand"}),
            ]
        );
    }
}
