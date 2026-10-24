//! Parameter rows of one object level, and an object field shown as a
//! foldable group of such rows.

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use proteus_contracts::domain::ConfigField;
use serde_json::Value;

use super::SettingsView;
use super::widgets::{
    changed_dot, described, disclosure, hashed_id, nested, nested_group, note, reset_mark,
};
use crate::settings::FieldPath;
use crate::settings::catalog::editable_field;
use crate::ui::kit::{Pressable as _, RowRule as _};

impl SettingsView {
    /// Rows of `fields` living in `object`: ordinary fields, advanced ones
    /// folded, then a note on keys the schema does not describe.
    pub(super) fn field_rows(
        &self,
        fields: &[ConfigField],
        path_of: impl Fn(&str) -> FieldPath,
        object: Option<&Value>,
        scope: &str,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let field_row = |field: &ConfigField| self.render_field(&path_of(&field.key), field, cx);
        let mut rows: Vec<AnyElement> = fields
            .iter()
            .filter(|field| !field.advanced && editable_field(field))
            .map(field_row)
            .collect();
        let advanced: Vec<_> = fields
            .iter()
            .filter(|field| field.advanced && editable_field(field))
            .collect();
        if !advanced.is_empty() {
            let key = format!("advanced:{scope}");
            let open = self.is_open(&key);
            let changed = advanced.iter().any(|field| {
                let path = path_of(&field.key);
                self.draft.value(&path) != self.baseline.value(&path)
            });
            let advanced_count = advanced.len();
            // The divider under the toggle is part of the revealed rows, so
            // nothing is left over once they close.
            let content = self.reveal(&key, cx, || {
                nested(
                    v_flex().children(
                        advanced
                            .into_iter()
                            .map(|field| div().row_rule(cx).child(field_row(field))),
                    ),
                )
                .into_any_element()
            });
            let view = cx.weak_entity();
            let toggle_key = key.clone();
            rows.push(
                nested_group(
                    disclosure(
                        hashed_id("params-advanced", &key),
                        open,
                        format!("Дополнительно · {}", advanced_count),
                        changed,
                        move |cx| {
                            let _ = view.update(cx, |this, cx| this.toggle_open(&toggle_key, cx));
                        },
                        cx,
                    ),
                    content,
                    cx,
                )
                .into_any_element(),
            );
        }
        let extra: Vec<&str> = object
            .and_then(Value::as_object)
            .map(|object| {
                object
                    .keys()
                    .filter(|key| !fields.iter().any(|field| &field.key == *key))
                    .map(String::as_str)
                    .collect()
            })
            .unwrap_or_default();
        if !extra.is_empty() {
            rows.push(
                note(
                    format!(
                        "Дополнительные параметры: {}. Они сохраняются вместе с профилем; изменить их можно в JSON.",
                        extra.join(", ")
                    ),
                    cx,
                )
                .into_any_element(),
            );
        }
        rows
    }

    /// Object field: a header that folds its nested fields and resets them
    /// all at once.
    pub(super) fn render_group(
        &self,
        path: &FieldPath,
        field: &ConfigField,
        fields: &[ConfigField],
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let scope = format!("{}:{}:{}", path.slot, path.module, path.dotted());
        let key = format!("group:{scope}");
        let open = self.is_open(&key);
        let value = self.draft.value(path);
        let changed = value != self.baseline.value(path);
        let reset = value.is_some().then(|| {
            let (view, path) = (cx.weak_entity(), path.clone());
            reset_mark(
                hashed_id("group-reset", &path),
                "Своя группа · сбросить",
                move |_, cx| {
                    let _ = view.update(cx, |this, cx| this.reset_field(&path, cx));
                },
                cx,
            )
        });
        let view = cx.weak_entity();
        let toggle_key = key.clone();
        let header = h_flex()
            .id(hashed_id("group", path))
            .px_4()
            .py_2p5()
            .gap_2()
            .items_center()
            .text_sm()
            .disclosure_surface(cx)
            .child(crate::ui::motion::chevron(
                ElementId::Name(format!("{key}-chevron").into()),
                open,
                theme.muted_foreground,
            ))
            .child(described(
                hashed_id("group-title", path),
                field.title.clone().into(),
                Some(field.description.clone().into()),
                Some(path.dotted().into()),
                cx,
            ))
            .children(reset)
            .when(changed, |title| title.child(changed_dot(cx)))
            .on_click(move |_, _, cx| {
                let _ = view.update(cx, |this, cx| this.toggle_open(&toggle_key, cx));
            });
        let content = self.reveal(&key, cx, || {
            let rows = self.field_rows(fields, |key| path.child(key), value, &scope, cx);
            nested(
                v_flex().children(
                    rows.into_iter()
                        .enumerate()
                        .map(|(ix, row)| div().when(ix > 0, |cell| cell.row_rule(cx)).child(row)),
                ),
            )
            .into_any_element()
        });
        nested_group(
            self.spotted(&super::spot::field(path), header, cx),
            content,
            cx,
        )
        .into_any_element()
    }
}
