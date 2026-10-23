// Адаптировано из Zed: crates/git_ui/src/git_graph.rs (table_rows, render_ref_chip)
//! A virtualized graph table. Processes supply columns and data; the native
//! control owns layout, resizing, visibility, selection and keyboard.
use super::lanes_paint::{self, LANE, LEFT};
use super::rows::{Kind, Rows};
use crate::declarative::control::MenuItem;
use crate::declarative::lanes::{Column, ColumnKind, Row};
use crate::ui::context::RightClickMenu as _;
use crate::ui::fade::faded;
use crate::ui::prefs;
use crate::ui::tip::Tip as _;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

pub(super) type Lanes = Rows<LaneRows>;
#[derive(Default)]
pub(super) struct LaneRows {
    lanes: usize,
    columns: Vec<Column>,
    weights: Vec<f32>,
    visible: Vec<bool>,
}
#[derive(Clone)]
struct ColumnDrag {
    owner: EntityId,
    index: usize,
}
impl Render for ColumnDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
impl Lanes {
    pub fn set_columns(&mut self, columns: &[Column], cx: &mut Context<Self>) {
        if self.kind.columns == columns {
            return;
        }
        self.kind.columns = columns.to_vec();
        self.kind.weights = columns.iter().map(|c| c.weight).collect();
        self.kind.visible = vec![true; columns.len()];
        cx.notify();
    }
}
impl LaneRows {
    fn total(&self) -> f32 {
        self.weights
            .iter()
            .zip(&self.visible)
            .filter(|(_, v)| **v)
            .map(|(w, _)| w)
            .sum::<f32>()
            .max(0.01)
    }
    fn slot(&self, i: usize, cx: &App) -> Div {
        div()
            .relative()
            .flex_none()
            .min_w_0()
            .h_full()
            .w(relative(self.weights[i] / self.total()))
            .border_r_1()
            .border_color(cx.theme().border.opacity(0.6))
            .overflow_hidden()
    }
    fn graph(&self, row: &Row, cx: &App) -> Div {
        let painted = Rc::new(row.clone());
        let graph = canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                lanes_paint::paint(bounds, &painted, window, prefs::scale(cx))
            },
        )
        .flex_none()
        .h_full()
        .w(prefs::scaled(LEFT * 2. + self.lanes as f32 * LANE));
        h_flex()
            .size_full()
            .justify_end()
            .gap_1()
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .pl_2()
                    .gap_1()
                    .overflow_hidden()
                    .children(row.refs.iter().enumerate().map(|(i, chip)| {
                        let color = super::render::tone_color(chip.tone, cx);
                        div()
                            .id(("lane-ref", i))
                            .min_w_0()
                            .px_1()
                            .rounded(prefs::scaled(3.))
                            .border_1()
                            .border_color(color.opacity(0.4))
                            .bg(color.opacity(0.08))
                            .text_xs()
                            .text_color(color)
                            .child(faded(chip.label.clone()))
                            .tip_text(chip.label.clone())
                    })),
            )
            .child(graph)
    }
}
impl Kind for LaneRows {
    type Row = Row;
    const NAME: &'static str = "extension-lanes";
    const MOVED: Option<&'static str> = Some("select");
    const SCROLL: ScrollStrategy = ScrollStrategy::Nearest;
    const RING_AFTER_KEYS: bool = false;
    fn id(row: &Row) -> &str {
        &row.id
    }
    fn selected(row: &Row) -> bool {
        row.selected
    }
    fn menu(row: &Row) -> &[MenuItem] {
        &row.menu
    }
    fn clicked(count: usize) -> &'static str {
        if count > 1 { "open" } else { "select" }
    }
    fn measure(&mut self, rows: &[Rc<Row>]) {
        self.lanes = rows.iter().map(|r| r.width()).max().unwrap_or(1);
    }
    fn draw(&self, row: &Row, index: usize, ring: bool, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        let (hover, selected, focus) = (theme.list_hover, theme.list_active, theme.ring);
        h_flex()
            .id(("lanes-row", index))
            .relative()
            .w_full()
            .h(prefs::scaled(28.))
            .text_sm()
            .cursor_pointer()
            .when(row.selected, |r| r.bg(selected))
            .hover(move |r| r.bg(hover))
            .children(
                self.columns
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| self.visible[*i])
                    .map(|(i, column)| {
                        let cell = self.slot(i, cx);
                        if column.kind == ColumnKind::Graph {
                            return cell.child(self.graph(row, cx));
                        }
                        let text = row.cells.get(i).cloned().unwrap_or_default();
                        let content = h_flex()
                            .size_full()
                            .gap_1p5()
                            .px_2()
                            .min_w_0()
                            .when(column.kind == ColumnKind::Mono, |r| {
                                r.font_family(theme.mono_font_family.clone()).text_xs()
                            })
                            .when(column.kind == ColumnKind::Person, |r| {
                                r.child(super::person::avatar(&text, 14., cx))
                            })
                            .child(div().min_w_0().child(faded(text)));
                        cell.child(content)
                    }),
            )
            .when(ring, |r| {
                r.child(div().absolute().inset_0().border_1().border_color(focus))
            })
            .when_some(row.hint.clone(), |r, hint| r.tip_text(hint))
    }
    fn header(&self, cx: &mut Context<Lanes>) -> Option<AnyElement> {
        let owner = cx.entity_id();
        let view = cx.weak_entity();
        let labels = self.columns.clone();
        let visible = self.visible.clone();
        let accent = cx.theme().primary.opacity(0.3);
        let header = h_flex()
            .id("graph-columns")
            .w_full()
            .flex_none()
            .h(prefs::scaled(32.))
            .border_b_1()
            .border_color(cx.theme().border)
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .on_drag_move::<ColumnDrag>(cx.listener(
                |this, event: &DragMoveEvent<ColumnDrag>, _, cx| {
                    let drag = event.drag(cx);
                    if drag.owner != cx.entity_id() {
                        return;
                    }
                    let i = drag.index;
                    let Some(j) =
                        ((i + 1)..this.kind.columns.len()).find(|j| this.kind.visible[*j])
                    else {
                        return;
                    };
                    let total = this.kind.total();
                    let before: f32 = this.kind.weights[..i]
                        .iter()
                        .zip(&this.kind.visible[..i])
                        .filter(|(_, v)| **v)
                        .map(|(w, _)| w)
                        .sum();
                    let pair = this.kind.weights[i] + this.kind.weights[j];
                    let width = event.bounds.size.width.as_f32();
                    if width <= 0. {
                        return;
                    }
                    let min = (48. * prefs::scale(cx) / width * total).min(pair / 2.);
                    let at = (event.event.position.x - event.bounds.left()).as_f32() / width
                        * total
                        - before;
                    this.kind.weights[i] = at.clamp(min, pair - min);
                    this.kind.weights[j] = pair - this.kind.weights[i];
                    cx.notify();
                },
            ))
            .children(
                self.columns
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| self.visible[*i])
                    .map(|(i, c)| {
                        let last = !self.visible[i + 1..].iter().any(|v| *v);
                        let grip = div()
                            .id(("column-grip", i))
                            .absolute()
                            .right_0()
                            .top_0()
                            .bottom_0()
                            .w(prefs::scaled(6.))
                            .cursor(CursorStyle::ResizeLeftRight)
                            .hover(move |s| s.bg(accent))
                            .on_drag(ColumnDrag { owner, index: i }, |d, _, _, cx| {
                                cx.new(|_| d.clone())
                            });
                        self.slot(i, cx)
                            .child(h_flex().size_full().px_2().child(faded(c.label.clone())))
                            .when(!last, |cell| cell.child(grip))
                    }),
            );
        Some(
            header
                .right_click_menu(move |menu, _, _| {
                    labels.iter().enumerate().fold(menu, |menu, (i, c)| {
                        let view = view.clone();
                        menu.item(
                            PopupMenuItem::new(c.label.clone())
                                .checked(visible[i])
                                .disabled(c.kind == ColumnKind::Graph)
                                .on_click(move |_, _, cx| {
                                    let _ = view.update(cx, |v, cx| {
                                        v.kind.visible[i] = !v.kind.visible[i];
                                        cx.notify();
                                    });
                                }),
                        )
                    })
                })
                .trigger_style(StyleRefinement::default().w_full().flex_none())
                .into_any_element(),
        )
    }
}
