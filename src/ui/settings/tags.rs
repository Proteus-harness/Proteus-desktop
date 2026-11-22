// Адаптировано из Zed: crates/ui/src/components/chip.rs (compact labels).
//! Ordered string arrays. The schema owns suggestions; they do not normalize
//! existing values. Reordering uses the shared native drag list.

use std::{collections::HashMap, rc::Rc};

use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::domain::{ConfigField, ConfigValueSchema};
use serde_json::Value;

use super::{SettingsView, editors::TextInput, fields::FieldSpec};
use crate::settings::FieldPath;

pub(super) mod editor;
mod suggest;

#[cfg(test)]
mod tests;

pub(super) fn is_tag_field(field: &ConfigField) -> bool {
    match &field.value {
        ConfigValueSchema::Array { items } => match items.as_ref() {
            ConfigValueSchema::String {
                multiline: false,
                secret: false,
            } => true,
            ConfigValueSchema::Enum { options } => {
                options.iter().all(|option| option.value.is_string())
            }
            _ => false,
        },
        _ => false,
    }
}

fn array(value: Option<&Value>) -> Option<&[Value]> {
    match value {
        None => Some(&[]),
        Some(Value::Array(values)) => Some(values),
        _ => None,
    }
}

fn tag_label(value: &Value) -> String {
    match value {
        Value::String(text) if !text.is_empty() => text.clone(),
        Value::String(_) => "\"\"".into(),
        _ => format!("JSON: {value}"),
    }
}

/// Occurrence identities keep duplicates independently removable, and retain
/// focus when a different value moves. Identical occurrences are equivalent.
fn item_ids(values: &[Value]) -> Vec<SharedString> {
    let mut occurrences = HashMap::<String, usize>::new();
    values
        .iter()
        .map(|value| {
            let encoded = value.to_string();
            let occurrence = occurrences.entry(encoded.clone()).or_default();
            let id = format!("{encoded}:{}", *occurrence);
            *occurrence += 1;
            id.into()
        })
        .collect()
}

#[derive(Clone, Debug)]
pub(super) enum Edit {
    Add(Value),
    Remove(usize),
    MoveBefore { from: usize, before: Option<usize> },
}

pub(super) fn edited(values: &[Value], edit: Edit) -> Option<Vec<Value>> {
    let mut next = values.to_vec();
    match edit {
        Edit::Add(value) => next.push(value),
        Edit::Remove(index) if index < next.len() => {
            next.remove(index);
        }
        Edit::MoveBefore { from, before }
            if from < next.len() && before.is_none_or(|index| index < next.len()) =>
        {
            if before == Some(from) {
                return None;
            }
            let target = before.unwrap_or(next.len());
            let value = next.remove(from);
            next.insert(if from < target { target - 1 } else { target }, value);
        }
        _ => return None,
    }
    (next != values).then_some(next)
}

fn added_value(field: &ConfigField, text: &str) -> Option<Value> {
    if text.is_empty() {
        return None;
    }
    if let ConfigValueSchema::Array { items } = &field.value
        && let ConfigValueSchema::Enum { options } = items.as_ref()
    {
        if let Some(value) = options
            .iter()
            .find(|option| option.value.as_str() == Some(text))
            .or_else(|| options.iter().find(|option| option.title == text))
            .map(|option| option.value.clone())
        {
            return Some(value);
        }
    }
    Some(Value::String(text.into()))
}

impl SettingsView {
    fn apply_tag_edit(
        &mut self,
        path: &FieldPath,
        field: &ConfigField,
        expected: &[Value],
        edit: Edit,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.snapshot().is_some_and(|snapshot| snapshot.writable) {
            return false;
        }
        let current = self.draft.value(path).or(field.default.as_ref());
        let Some(current) = array(current).filter(|current| *current == expected) else {
            return false;
        };
        let Some(next) = edited(current, edit) else {
            return false;
        };
        self.edit_field(path, Some(Value::Array(next)), field.default.as_ref(), cx);
        true
    }

    pub(super) fn add_string_tag(
        &mut self,
        path: &FieldPath,
        field: &ConfigField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(TextInput::Line(input)) = self
            .editors
            .fields
            .get(path)
            .map(|field| field.input.clone())
        else {
            return;
        };
        let text = input.read(cx).value();
        let Some(value) = added_value(field, &text) else {
            return;
        };
        let expected =
            array(self.draft.value(path).or(field.default.as_ref())).map(<[Value]>::to_vec);
        if let Some(expected) = expected
            && self.apply_tag_edit(path, field, &expected, Edit::Add(value), cx)
        {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    pub(super) fn string_tags(&self, spec: &Rc<FieldSpec>, cx: &Context<Self>) -> AnyElement {
        let Some(values) = array(spec.effective()) else {
            return div()
                .text_sm()
                .text_color(cx.theme().warning)
                .child("Здесь сохранён не список. Проверьте значение в JSON.")
                .into_any_element();
        };
        let path = spec.path.clone();
        let field = spec.field.clone();
        let view = spec.view.clone();
        editor::render(
            Rc::new(editor::TagEditor {
                key: format!("schema:{path:?}"),
                values: values.to_vec(),
                field: spec.field.clone(),
                input: self
                    .editors
                    .fields
                    .get(&spec.path)
                    .filter(|_| self.snapshot().is_some_and(|snapshot| snapshot.writable))
                    .and_then(|field| match &field.input {
                        TextInput::Line(input) => Some(input.clone()),
                        _ => None,
                    }),
                revision: self.profile_revision.rotate_left(32) ^ self.edit_revision,
                apply: Rc::new(move |expected, edit, cx| {
                    view.update(cx, |this, cx| {
                        this.apply_tag_edit(&path, &field, expected, edit, cx)
                    })
                    .unwrap_or(false)
                }),
            }),
            self.snapshot(),
            cx,
        )
    }
}
