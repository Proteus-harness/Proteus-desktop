// Адаптировано из Zed: crates/ui/src/components/chip.rs (compact labels).
//! Ordered values with callbacks, shared by schema fields and behavior modes.
//! The owner supplies a snapshot and verifies it before applying a stale edit.

use std::rc::Rc;

use gpui_kit::accesskit::Role;
use gpui_kit::component::input::InputState;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use proteus_contracts::app_protocol::config_builder::ConfigBuilderSnapshot;
use proteus_contracts::domain::ConfigField;
use serde_json::Value;

use super::{Edit, added_value, item_ids, suggest, tag_label};
use crate::ui::context::RightClickMenu as _;
use crate::ui::kit::{ButtonLabel as _, Frame as _};
use crate::ui::settings::widgets::hashed_id;
use crate::ui::{drag, fade::faded, kit, reorder};

pub(in crate::ui::settings) type Apply = Rc<dyn Fn(&[Value], Edit, &mut App) -> bool>;

pub(in crate::ui::settings) struct TagEditor {
    pub key: String,
    pub values: Vec<Value>,
    pub field: ConfigField,
    pub input: Option<Entity<InputState>>,
    pub revision: (u64, u64),
    pub apply: Apply,
}

impl TagEditor {
    fn edit(&self, edit: Edit, window: &mut Window, cx: &mut App) {
        let added = matches!(edit, Edit::Add(_) | Edit::AddSuggestion(_));
        if (self.apply)(&self.values, edit, cx) && added {
            if let Some(input) = &self.input {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SuggestionScope {
    input: EntityId,
    revision: (u64, u64),
}

struct SuggestionDrag {
    scope: SuggestionScope,
    value: String,
}

struct SuggestionPreview(String);

impl Render for SuggestionPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        drag::lifted(tag_pill(self.0.clone(), cx), cx)
    }
}

pub(in crate::ui::settings) fn render(
    spec: Rc<TagEditor>,
    snapshot: Option<&ConfigBuilderSnapshot>,
    cx: &App,
) -> AnyElement {
    let Some(input) = &spec.input else {
        return v_flex()
            .gap_2()
            .items_start()
            .children(
                spec.values
                    .iter()
                    .map(|value| tag_pill(tag_label(value), cx)),
            )
            .into_any_element();
    };
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
    let remove_spec = spec.clone();
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
    .drag_on_move()
    .revision(spec.revision.0.rotate_left(32) ^ spec.revision.1)
    .remove_on_drop(
        "Отпустите здесь, чтобы удалить",
        move |id, window, cx| {
            if let Some(index) = ids.iter().position(|candidate| candidate == &id) {
                remove_spec.edit(Edit::Remove(index), window, cx);
            }
        },
    );
    let query = input.read(cx).value();
    let filtered = suggest::visible(&spec.field, snapshot, &spec.values, &query);
    let scope = SuggestionScope {
        input: input.entity_id(),
        revision: spec.revision,
    };
    let drop_spec = spec.clone();
    let theme = cx.theme();
    let empty_target = spec.values.is_empty() && !filtered.is_empty();
    let drop_list = v_flex()
        .id(hashed_id("tag-list-drop", &spec.key))
        .w_full()
        .frame(cx)
        .when(empty_target, |list| {
            let label = "Перетащите сюда, чтобы добавить";
            list.role(Role::Region)
                .aria_label(label)
                .border_dashed()
                .border_color(theme.border)
                .child(
                    h_flex()
                        .min_h_8()
                        .px_2()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(label),
                )
        })
        .drag_over::<SuggestionDrag>(move |style, suggestion, _, cx| {
            if suggestion.scope == scope {
                style
                    .border_color(cx.theme().drag_border)
                    .bg(cx.theme().drop_target)
            } else {
                style
            }
        })
        .on_drop(move |suggestion: &SuggestionDrag, window, cx| {
            if suggestion.scope == scope {
                drop_spec.edit(Edit::AddSuggestion(suggestion.value.clone()), window, cx);
            }
        })
        .child(list);
    #[cfg(test)]
    let drop_list = drop_list.on_prepaint(reorder::ObservedBounds::capture(format!(
        "tag-list-drop:{}",
        spec.key
    )));
    let mut body = v_flex().w_full().gap_2().child(drop_list);
    let add = spec.clone();
    let add_input = input.clone();
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
                        if let Some(value) = added_value(&add.field, &add_input.read(cx).value()) {
                            add.edit(Edit::Add(value), window, cx);
                        }
                    }),
            ),
    );
    if !filtered.is_empty() {
        body = body.child(
            h_flex().flex_wrap().gap_1().children(
                filtered
                    .into_iter()
                    .map(|item| suggestion(&spec, item, scope, cx)),
            ),
        );
    }
    body.into_any_element()
}

fn suggestion(
    spec: &Rc<TagEditor>,
    item: suggest::Suggestion,
    scope: SuggestionScope,
    cx: &App,
) -> impl IntoElement + use<> {
    let add = spec.clone();
    let value = item.value.clone();
    let title = item.title.clone();
    let source = format!("tag-suggestion:{:?}:{value:?}", scope.input);
    #[cfg(test)]
    let bounds_key = format!("tag-suggestion:{}:{value}", spec.key);
    let mut button = kit::button(hashed_id("tag-suggestion", &(&spec.key, &value)))
        .faded_label(item.title)
        .settings_control(cx)
        .xsmall()
        .cursor_grab()
        .when(drag::is_source(&source, cx), |button| {
            button.opacity(drag::SOURCE_OPACITY)
        })
        .on_click(move |_, window, cx| {
            add.edit(Edit::AddSuggestion(value.clone()), window, cx);
        });
    button.interactivity().on_drag(
        SuggestionDrag {
            scope,
            value: item.value,
        },
        move |_, _, _, cx| {
            drag::start(source.clone(), cx);
            cx.new(|_| SuggestionPreview(title.clone()))
        },
    );
    #[cfg(test)]
    let button = button.on_prepaint(reorder::ObservedBounds::capture(bounds_key));
    button
}

fn tag_row(spec: &Rc<TagEditor>, index: usize, cx: &App) -> AnyElement {
    let remove = spec.clone();
    let row =
        tag_pill(tag_label(&spec.values[index]), cx).id(hashed_id("tag", &(&spec.key, index)));
    #[cfg(test)]
    let row = row.on_prepaint(reorder::ObservedBounds::capture(format!(
        "tag-row:{}:{index}",
        spec.key
    )));
    row.right_click_menu(move |menu, _, _| {
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
    let theme = cx.theme();
    h_flex().min_w_0().max_w_full().child(
        h_flex()
            .min_w_0()
            .max_w_full()
            .px_2()
            .py_0p5()
            .frame(cx)
            .bg(theme.input_background())
            .text_sm()
            .text_color(theme.foreground)
            .child(div().min_w_0().child(faded(label))),
    )
}
