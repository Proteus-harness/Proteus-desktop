//! Native rendering of the widget tree an extension sends.

use crate::ui::kit::Frame as _;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::button::Fit;
use super::view::DeclarativeView;
use crate::declarative::widget::{Direction, Gap, Node, TextSize, Tone};
use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::kit::ButtonLabel as _;
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
        let element = match node {
            Node::Person { name, detail } => v_flex()
                .w_full()
                .items_center()
                .py_3()
                .gap_2()
                .child(super::person::avatar(name, 36., cx))
                .child(div().max_w_full().text_lg().child(faded(name.clone())))
                .child(
                    div()
                        .max_w_full()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(faded(detail.clone())),
                )
                .into_any_element(),
            Node::Disclosure {
                id,
                label,
                open,
                children,
            } => {
                let opened = self.fields.disclosures.get(id).copied().unwrap_or(*open);
                let key: SharedString =
                    format!("extension-disclosure-{}-{id}", cx.entity_id()).into();
                let (id, toggle) = (id.clone(), key.clone());
                let header = h_flex()
                    .id(ElementId::Name(key.clone()))
                    .w_full()
                    .px_2()
                    .py_1p5()
                    .gap_2()
                    .disclosure_surface(cx)
                    .child(crate::ui::motion::chevron(
                        ElementId::Name(key.clone()),
                        opened,
                        cx.theme().muted_foreground,
                    ))
                    .child(faded(label.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.emit(super::view::WillResize);
                        this.fields.disclosures.insert(id.clone(), !opened);
                        crate::ui::motion::toggled(toggle.clone(), !opened, cx);
                        cx.notify();
                    }));
                let content = self
                    .fill(children, path, cx)
                    .w_full()
                    .px_2()
                    .pb_2()
                    .into_any_element();
                v_flex()
                    .w_full()
                    .child(header)
                    .children(crate::ui::motion::reveal(key, opened, cx, || content))
                    .into_any_element()
            }
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
                crate::ui::context::menu_button(
                    ElementId::Name(format!("select-menu-{id}").into()),
                    kit::button(ElementId::Name(format!("select-{id}").into()))
                        .small()
                        .faded_label(label.clone())
                        .outline(),
                    true,
                    move |menu: PopupMenu, _, _| {
                        options.iter().fold(menu, |menu, option| {
                            let (value, action, this) =
                                (option.value.clone(), action.clone(), this.clone());
                            menu.item(
                                crate::ui::selection::menu_option(
                                    option.label.clone(),
                                    option.selected,
                                )
                                .on_click(move |_, _, cx| {
                                    let _ = this.update(cx, |this, cx| {
                                        this.act(action.clone(), Some(value.clone()), cx)
                                    });
                                }),
                            )
                        })
                    },
                )
                .into_any_element()
            }
            Node::Menu { .. } => super::button::menu(node, path, Fit::Normal, cx),
            Node::Checkbox {
                label,
                checked,
                action,
                value,
            } => {
                let (action, value, checked) = (action.clone(), value.clone(), *checked);
                kit::checkbox(ElementId::Name(format!("decl-check-{path}").into()))
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
            Node::Bar { children, border } => {
                self.bar(children, *border, path, cx).into_any_element()
            }
            Node::PanelEditor { children } => {
                self.panel_editor(children, path, cx).into_any_element()
            }
            Node::Loading { text } => crate::ui::state::loading(
                SharedString::from(format!("loading-{path}")),
                text.clone(),
                cx,
            ),
            Node::Empty { text } => super::layout::empty(text, cx).into_any_element(),
            Node::Icon { name, tone } => Icon::new(super::icon(Some(name)))
                .small()
                .flex_none()
                .text_color(tone_color(*tone, cx))
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
                            | Node::List { .. }
                            | Node::Diff { .. }
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
            Node::Input {
                id,
                search: true,
                placeholder,
                ..
            } => self
                .fields
                .inputs
                .get(id)
                .and_then(|input| match &input.state {
                    super::fields::Text::Line(input) => Some(
                        crate::ui::search::Search::new(
                            ElementId::Name(format!("search-{id}").into()),
                            input.clone(),
                            self.focus.clone(),
                            placeholder.clone(),
                        )
                        .into_any_element(),
                    ),
                    super::fields::Text::Area(_) => None,
                })
                .unwrap_or_else(|| div().into_any_element()),
            Node::Input {
                id, flush, mono, ..
            } => {
                let mono = mono.then(|| cx.theme().mono_font_family.clone());
                div()
                    .w_full()
                    .map(|field| if *flush { field.pt_1p5() } else { field.p_2() })
                    .when_some(mono, |field, family| field.font_family(family))
                    .children(self.fields.inputs.get(id).map(|input| {
                        match &input.state {
                            super::fields::Text::Line(state) => kit::input(state)
                                .w_full()
                                .appearance(!*flush)
                                .into_any_element(),
                            super::fields::Text::Area(state) => {
                                let area = if *flush {
                                    Textarea::new(state).appearance(false)
                                } else {
                                    kit::textarea(state, cx)
                                };
                                kit::contain_scroll(area.small().w_full()).into_any_element()
                            }
                        }
                    }))
                    .into_any_element()
            }
            Node::Svg {
                source,
                width,
                height,
                ..
            } => div()
                .w_full()
                .min_w_0()
                .children(self.images.get(source, *width, *height, cx).map(|image| {
                    img(image)
                        .w(prefs::scaled(*width))
                        .h(prefs::scaled(*height))
                        .max_w_full()
                        .object_fit(ObjectFit::Contain)
                }))
                .into_any_element(),
            Node::RichText { id, .. } => self
                .fields
                .rich_text
                .get(id)
                .map(|text| text.element())
                .unwrap_or_else(|| div().into_any_element()),
            Node::Content { id, .. } => self
                .fields
                .contents
                .get(id)
                .map(|content| content.element())
                .unwrap_or_else(|| div().into_any_element()),
            Node::Code {
                id,
                text,
                language,
                embedded: true,
                ..
            } => super::rich_text::embedded_code(id, text, language, self.fields.codes.get(id), cx),
            Node::Code { id, height, .. } => div()
                .when(height.is_none(), |node| node.flex_1())
                .when_some(*height, |node, height| node.h(prefs::scaled(height)))
                .min_h_0()
                .children(
                    self.fields
                        .codes
                        .get(id)
                        .map(|editor| super::code_block::readonly_editor(editor, cx).h_full()),
                )
                .into_any_element(),
            Node::Diff { id, height, .. } => div()
                .when(height.is_none(), |node| node.flex_1())
                .when_some(*height, |node, height| node.h(prefs::scaled(height)))
                .min_h_0()
                .min_w_0()
                .children(self.fields.diffs.get(id).cloned())
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
            Node::List { id, .. } => div()
                .flex_1()
                .min_h_0()
                .children(self.fields.lists.get(id).cloned())
                .into_any_element(),
            Node::Steps { id, .. } => div()
                .w_full()
                .p_1()
                .children(self.steps.get(id).cloned())
                .into_any_element(),
            Node::Sortable { id, items, action } => {
                let rows = items
                    .iter()
                    .map(|item| {
                        let child_path = format!("{path}.sort({}:{})", item.id.len(), item.id);
                        let child = if item.children.is_empty() {
                            div()
                                .px_3()
                                .py_2()
                                .text_sm()
                                .child(faded(item.label.clone()))
                                .into_any_element()
                        } else {
                            self.fill(&item.children, &child_path, cx)
                                .w_full()
                                .into_any_element()
                        };
                        let row = crate::ui::reorder::Row::new(
                            item.id.clone(),
                            item.label.clone(),
                            child,
                        );
                        if item.children.is_empty() {
                            row
                        } else {
                            let (view, children) = (cx.weak_entity(), item.children.clone());
                            row.preview(move |cx| {
                                view.update(cx, |this, cx| {
                                    this.fill(&children, &child_path, cx)
                                        .w_full()
                                        .into_any_element()
                                })
                                .ok()
                            })
                        }
                    })
                    .collect();
                let (this, action) = (cx.weak_entity(), action.clone());
                crate::ui::reorder::List::new(
                    ElementId::Name(format!("sortable-{}-{id}", cx.entity_id()).into()),
                    rows,
                    move |id, before, _, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.act(
                                action.clone(),
                                Some(serde_json::json!({"id":id.to_string(),"before":before.map(|id| id.to_string())})),
                                cx,
                            )
                        });
                    },
                )
                .revision(self.root_revision)
                .into_any_element()
            }
            Node::Picker { id, .. } => div()
                .w_full()
                .min_w_0()
                .children(self.fields.pickers.get(id).cloned())
                .into_any_element(),
            Node::Stack {
                direction,
                gap,
                transition,
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
                let body = base.min_w_0().children(self.children(children, path, cx));
                match transition {
                    Some(key) => crate::ui::motion::PageTransition::new(
                        ElementId::Name(format!("view-{}-{path}-page", cx.entity_id()).into()),
                        key.clone(),
                        body.into_any_element(),
                    )
                    .into_any_element(),
                    None => body.into_any_element(),
                }
            }
            Node::Card { title, children } => {
                let theme = cx.theme();
                kit::surface(
                    v_flex()
                        .min_w_0()
                        .gap_2()
                        .px_4()
                        .py_3()
                        .surface_frame(cx)
                        .bg(theme.secondary)
                        .when_some(title.clone(), |card, title| {
                            card.child(div().text_sm().font_semibold().child(title))
                        })
                        .children(self.children(children, path, cx)),
                    cx,
                )
            }
            Node::Text {
                text,
                single_line,
                tone,
                size,
                mono,
            } => {
                let text = div()
                    .min_w_0()
                    .text_color(tone_color(*tone, cx))
                    .map(|node| {
                        if *single_line {
                            node.child(faded(text.clone()))
                        } else {
                            node.child(text.clone())
                        }
                    });
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
            Node::Button { .. } => super::button::button(node, path, Fit::Normal, cx),
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
        };
        // Native actionable descendants opt out of whole-row hold while
        // preserving their layout and all normal focus/click behavior.
        if matches!(
            node,
            Node::Input { .. }
                | Node::Checkbox { .. }
                | Node::Menu { .. }
                | Node::Button { .. }
                | Node::Select { .. }
                | Node::Picker { .. }
                | Node::RichText { .. }
                | Node::Content { .. }
                | Node::Code { .. }
                | Node::Terminal { .. }
                | Node::Tree { .. }
                | Node::List { .. }
                | Node::Lanes { .. }
                | Node::Graph { .. }
                | Node::Steps { .. }
                | Node::Tabs { .. }
                | Node::Disclosure { .. }
                | Node::Sortable { .. }
                | Node::Diff { .. }
        ) {
            crate::ui::reorder::interactive(element).into_any_element()
        } else {
            element
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
            | Node::Icon { .. }
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
