//! Native rendering of the widget tree an extension sends.

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Editor, Input, Textarea};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use super::view::DeclarativeView;
use crate::declarative::widget::{Direction, Gap, Item, Node, TextSize, Tone};
use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::kit::Pressable as _;
use crate::ui::prefs;

pub(super) fn tone_color(tone: Tone, cx: &App) -> Hsla {
    let theme = cx.theme();
    match tone {
        Tone::Default => theme.foreground,
        Tone::Muted => theme.muted_foreground,
        Tone::Accent => theme.primary,
        Tone::Success => theme.success,
        Tone::Warning => theme.warning,
        Tone::Danger => theme.danger,
        Tone::Color(color) => rgb(color).into(),
    }
}

impl DeclarativeView {
    /// `path` is the position in the tree; it keys element ids and the
    /// selected tab, so they survive a new tree of the same shape.
    pub(super) fn node(&self, node: &Node, path: &str, cx: &mut Context<Self>) -> AnyElement {
        match node {
            Node::Cells { columns, segments } => super::cells::cells(path, *columns, segments, cx),
            Node::Graph { id, .. } => div()
                .flex_1()
                .min_h_0()
                .children(self.fields.graphs.get(id).cloned())
                .into_any_element(),
            Node::Select {
                id,
                label,
                options,
                action,
            } => {
                let (options, action, this) = (options.clone(), action.clone(), cx.weak_entity());
                kit::button(ElementId::Name(format!("select-{id}").into()))
                    .small()
                    .label(label.clone())
                    .outline()
                    .dropdown_caret(true)
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu: PopupMenu, _, _| {
                        options.iter().fold(
                            menu.min_w(px(200.)).max_h(px(360.)).scrollable(true),
                            |menu, option| {
                                let (value, action, this) =
                                    (option.value.clone(), action.clone(), this.clone());
                                menu.item(
                                    PopupMenuItem::new(option.label.clone())
                                        .checked(option.selected)
                                        .on_click(move |_, _, cx| {
                                            let _ = this.update(cx, |this, cx| {
                                                this.act(action.clone(), Some(value.clone()), cx)
                                            });
                                        }),
                                )
                            },
                        )
                    })
                    .into_any_element()
            }
            Node::Menu {
                label,
                icon,
                hint,
                items,
            } => {
                let (items, view) = (items.clone(), cx.weak_entity());
                let button = kit::button(ElementId::Name(format!("decl-menu-{path}").into()))
                    .small()
                    .ghost()
                    .when(!label.is_empty(), |button| button.label(label.clone()))
                    .when_some(hint.clone(), |button, hint| button.tooltip(hint))
                    .icon(super::icon(Some(icon.as_deref().unwrap_or("ellipsis"))))
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu: PopupMenu, _, _| {
                        super::menu::fill(menu.min_w(px(180.)), &items, &view)
                    });
                button.into_any_element()
            }
            Node::Checkbox {
                label,
                checked,
                action,
                value,
            } => {
                let (action, value, checked) = (action.clone(), value.clone(), *checked);
                Checkbox::new(ElementId::Name(format!("decl-check-{path}").into()))
                    .label(label.clone())
                    .checked(checked)
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let value = serde_json::json!({"checked": !checked, "value": value});
                        this.act(action.clone(), Some(value), cx)
                    }))
                    .into_any_element()
            }
            Node::Unknown => div()
                .px_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Этот элемент требует более новой версии Proteus.")
                .into_any_element(),
            Node::Fill { children } => self
                .fill(children, path, cx)
                .flex_1()
                .w_full()
                .min_h_0()
                .into_any_element(),
            Node::Split { children, widths } => {
                let border = cx.theme().border;
                let last = children.len().saturating_sub(1);
                h_flex()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .children(children.iter().enumerate().map(|(index, child)| {
                        let path = format!("{path}.{index}");
                        let element = self.node(child, &path, cx);
                        let column = v_flex()
                            .h_full()
                            .min_h_0()
                            .min_w_0()
                            .when(index < last, |column| {
                                column.border_r_1().border_color(border)
                            });
                        let column = match widths.get(index).copied().flatten() {
                            Some(width) => column.w(prefs::scaled(width)).flex_none(),
                            None => column.flex_1(),
                        };
                        match child {
                            Node::Fill { .. }
                            | Node::Tree { .. }
                            | Node::Lanes { .. }
                            | Node::Terminal { .. } => column.child(element),
                            // Ordinary content scrolls inside its own column.
                            _ => column.child(
                                v_flex()
                                    .id(SharedString::from(format!("split-{path}")))
                                    .size_full()
                                    .overflow_y_scroll()
                                    .p_3()
                                    .gap_3()
                                    .child(element),
                            ),
                        }
                    }))
                    .into_any_element()
            }
            Node::Input { id, .. } => div()
                .p_2()
                .children(self.fields.inputs.get(id).map(|input| match &input.state {
                    super::fields::Text::Line(state) => {
                        Input::new(state).w_full().into_any_element()
                    }
                    super::fields::Text::Area(state) => {
                        Textarea::new(state).small().w_full().into_any_element()
                    }
                }))
                .into_any_element(),
            Node::Code { id, height, .. } => div()
                .when(height.is_none(), |node| node.flex_1())
                .when_some(*height, |node, height| node.h(prefs::scaled(height)))
                .min_h_0()
                .children(self.fields.codes.get(id).map(|editor| {
                    Editor::new(editor)
                        .readonly(true)
                        .bordered(false)
                        .appearance(false)
                        .h_full()
                }))
                .into_any_element(),
            Node::Terminal { id, .. } => div()
                .flex_1()
                .min_h_0()
                .children(self.fields.terminals.get(id).cloned())
                .into_any_element(),
            Node::Tree { id, .. } => div()
                .flex_1()
                .min_h_0()
                .children(self.fields.trees.get(id).cloned())
                .into_any_element(),
            Node::Lanes { id, .. } => div()
                .flex_1()
                .min_h_0()
                .children(self.fields.lanes.get(id).cloned())
                .into_any_element(),
            Node::Steps { id, .. } => div()
                .w_full()
                .p_1()
                .children(self.steps.get(id).cloned())
                .into_any_element(),
            Node::Choice {
                label,
                selected,
                action,
                value,
            } => {
                let theme = cx.theme();
                let (hover, primary) = (theme.list_hover, theme.primary);
                let cursor = self
                    .choices()
                    .iter()
                    .position(|(candidate, candidate_value, _)| {
                        candidate == action && candidate_value == value
                    });
                let keyboard = cursor.is_some() && cursor == self.choice_cursor;
                let (action, value) = (action.clone(), value.clone());
                div()
                    .id(ElementId::Name(format!("choice-{path}").into()))
                    .px_2()
                    .py_1()
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .pressable(cx)
                    .when(keyboard, |row| row.bg(hover))
                    .when(*selected, |row| {
                        row.text_color(primary)
                            .font_medium()
                            .bg(primary.opacity(0.12))
                    })
                    .when(!*selected, |row| row.hover(move |style| style.bg(hover)))
                    .child(faded(label.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.act(action.clone(), Some(value.clone()), cx)
                    }))
                    .into_any_element()
            }
            Node::Stack {
                direction,
                gap,
                children,
            } => {
                let base = match direction {
                    Direction::Column => v_flex(),
                    Direction::Row if children.iter().all(inline) => {
                        h_flex().flex_wrap().items_center()
                    }
                    Direction::Row => h_flex().flex_wrap().items_start(),
                };
                let base = match gap {
                    Gap::None => base,
                    Gap::Small => base.gap_1p5(),
                    Gap::Medium => base.gap_3(),
                    Gap::Large => base.gap_5(),
                };
                base.min_w_0()
                    .children(self.children(children, path, cx))
                    .into_any_element()
            }
            Node::Card { title, children } => {
                let theme = cx.theme();
                v_flex()
                    .min_w_0()
                    .gap_2()
                    .p_3()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.secondary)
                    .when_some(title.clone(), |card, title| {
                        card.child(div().text_sm().font_semibold().child(title))
                    })
                    .children(self.children(children, path, cx))
                    .into_any_element()
            }
            Node::Text {
                text,
                tone,
                size,
                mono,
            } => {
                let text = div()
                    .min_w_0()
                    .text_color(tone_color(*tone, cx))
                    .child(text.clone());
                let text = match size {
                    TextSize::Small => text.text_xs(),
                    TextSize::Normal => text.text_sm(),
                    TextSize::Large => text.text_lg().font_semibold(),
                };
                text.when(*mono, |text| {
                    text.font_family(cx.theme().mono_font_family.clone())
                })
                .into_any_element()
            }
            Node::Metric {
                label,
                value,
                detail,
                tone,
            } => {
                let muted = cx.theme().muted_foreground;
                v_flex()
                    .min_w(prefs::scaled(110.))
                    .gap_0p5()
                    .child(div().text_xs().text_color(muted).child(label.clone()))
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .text_color(tone_color(*tone, cx))
                            .child(value.clone()),
                    )
                    .when_some(detail.clone(), |metric, detail| {
                        metric.child(div().text_xs().text_color(muted).child(detail))
                    })
                    .into_any_element()
            }
            Node::Progress { value, max, label } => {
                let theme = cx.theme();
                let fraction = (value / max).clamp(0., 1.) as f32;
                v_flex()
                    .gap_1()
                    .when_some(label.clone(), |progress, label| {
                        progress.child(
                            h_flex()
                                .justify_between()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(label)
                                .child(format!("{:.0}%", fraction * 100.)),
                        )
                    })
                    .child(
                        div().h_1p5().w_full().rounded_full().bg(theme.muted).child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(theme.primary)
                                .w(relative(fraction)),
                        ),
                    )
                    .into_any_element()
            }
            Node::Table { columns, rows } => table(columns, rows, cx).into_any_element(),
            Node::BarChart { bars, unit } => {
                let peak = bars.iter().map(|bar| bar.value).fold(0., f64::max);
                let theme = cx.theme();
                let unit = unit.clone().unwrap_or_default();
                v_flex()
                    .gap_1()
                    .children(bars.iter().map(|bar| {
                        let fraction = if peak > 0. {
                            (bar.value / peak) as f32
                        } else {
                            0.
                        };
                        let color = match bar.tone {
                            Tone::Default => theme.primary,
                            tone => tone_color(tone, cx),
                        };
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .child(
                                div()
                                    .w(prefs::scaled(120.))
                                    .flex_none()
                                    .text_color(theme.muted_foreground)
                                    .child(faded(bar.label.clone())),
                            )
                            .child(
                                div().flex_1().h_2().child(
                                    div()
                                        .h_full()
                                        .rounded(theme.radius / 2.)
                                        .bg(color)
                                        .w(relative(fraction)),
                                ),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .min_w(prefs::scaled(56.))
                                    .text_right()
                                    .child(format!("{}{unit}", number(bar.value))),
                            )
                    }))
                    .into_any_element()
            }
            Node::Tabs { tabs, action } => self
                .tabs(tabs, action.as_deref(), path, false, cx)
                .into_any_element(),
            Node::List { items } => v_flex()
                .children(
                    items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| self.item(item, &format!("{path}.{index}"), cx)),
                )
                .into_any_element(),
            Node::Button { .. } => super::button::button(node, path, cx),
            Node::Badge { label, tone } => {
                let color = tone_color(*tone, cx);
                div()
                    .px_1p5()
                    .rounded(cx.theme().radius / 2.)
                    .border_1()
                    .border_color(color.opacity(0.4))
                    .text_xs()
                    .text_color(color)
                    .child(label.clone())
                    .into_any_element()
            }
            Node::Spacer {} => div().flex_1().into_any_element(),
            Node::Divider {} => div()
                .h(px(1.))
                .w_full()
                .bg(cx.theme().border)
                .into_any_element(),
        }
    }

    pub(super) fn children(
        &self,
        nodes: &[Node],
        path: &str,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        nodes
            .iter()
            .enumerate()
            .map(|(index, node)| self.node(node, &format!("{path}.{index}"), cx))
            .collect()
    }

    fn item(&self, item: &Item, path: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let hover = theme.list_hover;
        let row = v_flex()
            .id(ElementId::Name(format!("decl-item-{path}").into()))
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .child(div().text_sm().child(item.title.clone()))
            .when_some(item.detail.clone(), |row, detail| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(detail),
                )
            });
        match &item.action {
            Some(action) => {
                let send = self.sender(action.clone(), item.value.clone());
                row.cursor_pointer()
                    .pressable(cx)
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener(move |this, _, _, cx| send(this, cx)))
                    .into_any_element()
            }
            None => row.into_any_element(),
        }
    }

    fn sender(
        &self,
        action: String,
        value: Option<Value>,
    ) -> impl Fn(&mut Self, &mut Context<Self>) + 'static {
        move |this, cx| this.act(action.clone(), value.clone(), cx)
    }
}

/// Short controls that line up on one baseline in a row.
fn inline(node: &Node) -> bool {
    matches!(
        node,
        Node::Button { .. }
            | Node::Text { .. }
            | Node::Badge { .. }
            | Node::Select { .. }
            | Node::Menu { .. }
            | Node::Checkbox { .. }
            | Node::Spacer {}
    )
}

fn table(columns: &[String], rows: &[Vec<String>], cx: &App) -> Div {
    let theme = cx.theme();
    let line = |cells: &[String], head: bool| {
        h_flex()
            .gap_3()
            .py_1()
            .border_b_1()
            .border_color(theme.border)
            .text_xs()
            .when(head, |row| {
                row.text_color(theme.muted_foreground).font_semibold()
            })
            .children(
                cells
                    .iter()
                    .map(|cell| div().flex_1().min_w_0().child(faded(cell.clone()))),
            )
    };
    v_flex()
        .min_w_0()
        .child(line(columns, true))
        .children(rows.iter().map(|row| line(row, false)))
}

/// Integers without a fraction, others with up to two digits.
fn number(value: f64) -> String {
    if value.fract() == 0. && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

/// The face of a compact icon: short text and its color taken from a badge,
/// text, metric or progress at the top of the tree.
pub(super) fn compact(node: &Node, cx: &App) -> Option<(String, Hsla)> {
    match node {
        Node::Badge { label, tone }
        | Node::Text {
            text: label, tone, ..
        } => Some((label.clone(), tone_color(*tone, cx))),
        Node::Metric { value, tone, .. } => Some((value.clone(), tone_color(*tone, cx))),
        Node::Progress { value, max, .. } => Some((
            format!("{:.0}%", (value / max).clamp(0., 1.) * 100.),
            cx.theme().foreground,
        )),
        Node::Stack { children, .. } => children.first().and_then(|child| compact(child, cx)),
        _ => None,
    }
}
