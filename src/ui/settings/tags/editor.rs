// Адаптировано из Zed: crates/ui/src/components/chip.rs (compact labels).
//! Ordered values with callbacks, shared by schema fields and behavior modes.
//! The owner supplies a snapshot and verifies it before applying a stale edit.

use std::rc::Rc;

use gpui_kit::component::input::InputState;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::app_protocol::config_builder::ConfigBuilderSnapshot;
use proteus_contracts::domain::ConfigField;
use serde_json::Value;

use super::{Edit, added_value, item_ids, suggestions, tag_label};
use crate::ui::context::RightClickMenu as _;
use crate::ui::kit::ButtonLabel as _;
use crate::ui::settings::widgets::hashed_id;
use crate::ui::{fade::faded, kit, reorder};

pub(in crate::ui::settings) type Apply = Rc<dyn Fn(&[Value], Edit, &mut App) -> bool>;

pub(in crate::ui::settings) struct TagEditor {
    pub key: String,
    pub values: Vec<Value>,
    pub field: ConfigField,
    pub input: Option<Entity<InputState>>,
    pub revision: u64,
    pub apply: Apply,
}

impl TagEditor {
    fn edit(&self, edit: Edit, window: &mut Window, cx: &mut App) {
        let added = matches!(edit, Edit::Add(_));
        if (self.apply)(&self.values, edit, cx) && added {
            if let Some(input) = &self.input {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
        }
    }
}

pub(in crate::ui::settings) fn render(
    spec: Rc<TagEditor>,
    snapshot: Option<&ConfigBuilderSnapshot>,
    cx: &App,
) -> AnyElement {
    let ids = Rc::new(item_ids(&spec.values));
    let rows = spec
        .values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let label = tag_label(value);
            reorder::Row::new(ids[index].clone(), label.clone(), tag_row(&spec, index, cx))
                .preview(move |cx| Some(tag_pill(label.clone(), cx).into_any_element()))
        })
        .collect();
    let move_spec = spec.clone();
    let move_ids = ids.clone();
    let list = reorder::List::new(
        hashed_id("tags-order", &spec.key),
        rows,
        move |id, before, window, cx| {
            let Some(from) = move_ids.iter().position(|candidate| candidate == &id) else {
                return;
            };
            let target = before
                .as_ref()
                .and_then(|id| move_ids.iter().position(|candidate| candidate == id));
            if before.is_some() && target.is_none() {
                return;
            }
            move_spec.edit(
                Edit::MoveBefore {
                    from,
                    before: target,
                },
                window,
                cx,
            );
        },
    )
    .separated()
    .revision(spec.revision);
    let mut body = v_flex().w_full().gap_2().child(list);
    if let Some(input) = &spec.input {
        let query = input.read(cx).value();
        let add = spec.clone();
        body = body.child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(kit::input(input).small().w_full()),
                )
                .child(
                    kit::button(hashed_id("tag-add", &spec.key))
                        .label("Добавить")
                        .small()
                        .disabled(query.is_empty())
                        .on_click(move |_, window, cx| {
                            let Some(input) = &add.input else {
                                return;
                            };
                            if let Some(value) = added_value(&add.field, &input.read(cx).value()) {
                                add.edit(Edit::Add(value), window, cx);
                            }
                        }),
                ),
        );
        let query = query.to_lowercase();
        let choices = suggestions(&spec.field, snapshot);
        let filtered = choices
            .iter()
            .filter(|item| {
                !spec
                    .values
                    .iter()
                    .any(|value| value.as_str() == Some(&item.value))
                    && (query.is_empty()
                        || item.value.to_lowercase().contains(&query)
                        || item.title.to_lowercase().contains(&query))
            })
            .collect::<Vec<_>>();
        if !filtered.is_empty() {
            body = body.child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .children(filtered.iter().take(8).map(|item| {
                        let add = spec.clone();
                        let value = item.value.clone();
                        kit::button(hashed_id("tag-suggestion", &(&spec.key, &item.value)))
                            .faded_label(item.title.clone())
                            .quiet(cx)
                            .xsmall()
                            .on_click(move |_, window, cx| {
                                add.edit(Edit::Add(Value::String(value.clone())), window, cx)
                            })
                    })),
            );
        }
    }
    body.into_any_element()
}

fn tag_row(spec: &Rc<TagEditor>, index: usize, cx: &App) -> AnyElement {
    let remove = spec.clone();
    tag_pill(tag_label(&spec.values[index]), cx)
        .id(hashed_id("tag", &(&spec.key, index)))
        .right_click_menu(move |menu, _, _| {
            let remove = remove.clone();
            menu.item(
                PopupMenuItem::new("Удалить").on_click(move |_, window, cx| {
                    remove.edit(Edit::Remove(index), window, cx);
                }),
            )
        })
        .into_any_element()
}

fn tag_pill(label: String, cx: &App) -> Div {
    h_flex().py_0p5().cursor_grab().child(
        h_flex()
            .min_w_0()
            .max_w_full()
            .px_2()
            .rounded(cx.theme().radius)
            .bg(cx.theme().input_background())
            .text_sm()
            .text_color(cx.theme().foreground)
            .child(div().min_w_0().child(faded(label))),
    )
}
