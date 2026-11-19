//! Parameter rows of one object level, and an object field shown as a
//! foldable group of such rows.

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use proteus_contracts::domain::{ConfigField, ConfigSection};
use serde_json::Value;

use super::SettingsView;
use super::widgets::{
    changed_dot, described, disclosure, hashed_id, nested, nested_group, note, reset_mark,
};
use crate::settings::{FieldPath, field_sections, section_scope};
use crate::ui::kit::{self, RowRule as _};

// A directly visible singleton needs a section caption only for extra context.
fn adds_context(section: &str, field: &str) -> bool {
    let section = section.trim();
    !section.is_empty() && section != field.trim()
}

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
        let mut rows = Vec::new();
        for section in field_sections(fields) {
            let section_scope = section_scope(scope, section.fields[0]);
            if let Some(metadata) = section.metadata
                && (section.fields.iter().any(|field| !field.advanced)
                    || matches!(section.fields.as_slice(), [field]
                        if adds_context(&metadata.title, &field.title)
                            || adds_context(&metadata.description, &field.description)))
            {
                rows.push(
                    v_flex()
                        .px_4()
                        .pt_4()
                        .pb_2()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .font_semibold()
                                .child(metadata.title.clone()),
                        )
                        .children((!metadata.description.is_empty()).then(|| {
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(metadata.description.clone())
                        }))
                        .into_any_element(),
                );
            }
            rows.extend(self.section_rows(
                &section.fields,
                &path_of,
                &section_scope,
                section.metadata,
                cx,
            ));
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
            rows.push(note(format!(
                "Дополнительные параметры: {}. Они сохраняются вместе с профилем; изменить их можно в JSON.",
                extra.join(", ")
            ), cx).into_any_element());
        }
        rows
    }

    fn section_rows(
        &self,
        fields: &[&ConfigField],
        path_of: &impl Fn(&str) -> FieldPath,
        scope: &str,
        metadata: Option<&ConfigSection>,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let field_row = |field: &&ConfigField| self.render_field(&path_of(&field.key), field, cx);
        let mut rows: Vec<AnyElement> = fields
            .iter()
            .filter(|field| !field.advanced)
            .map(field_row)
            .collect();
        let advanced: Vec<_> = fields.iter().filter(|field| field.advanced).collect();
        if advanced.len() == 1 {
            rows.push(field_row(advanced[0]));
            return rows;
        }
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
                    v_flex()
                        .children(
                            metadata
                                .filter(|metadata| {
                                    advanced_count == fields.len()
                                        && !metadata.description.is_empty()
                                })
                                .map(|metadata| note(metadata.description.clone(), cx)),
                        )
                        .children(
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
                        described(
                            hashed_id("params-advanced-title", &key),
                            if advanced_count == fields.len() {
                                metadata
                                    .map(|metadata| metadata.title.clone())
                                    .unwrap_or_else(|| format!("Дополнительно · {advanced_count}"))
                            } else {
                                format!("Дополнительно · {advanced_count}")
                            }
                            .into(),
                            metadata
                                .filter(|_| advanced_count == fields.len())
                                .map(|metadata| metadata.description.clone().into()),
                            None,
                            cx,
                        ),
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
        let scope = crate::settings::form_scope(&path.slot, &path.module, &path.keys);
        let key = format!("group:{scope}");
        let open = self.is_open(&key);
        let value = self.draft.value(path);
        let changed = value != self.baseline.value(path);
        let reset = changed.then(|| {
            let (view, path) = (cx.weak_entity(), path.clone());
            reset_mark(
                hashed_id("group-reset", &path),
                "Вернуть сохранённые параметры",
                move |_, cx| {
                    let _ = view.update(cx, |this, cx| this.reset_field(&path, cx));
                },
                cx,
            )
        });
        let view = cx.weak_entity();
        let toggle_key = key.clone();
        let header = kit::disclosure_header(
            hashed_id("group", path),
            kit::DisclosureDensity::Settings,
            cx,
        )
        .child(crate::ui::motion::chevron(
            ElementId::Name(format!("{key}-chevron").into()),
            open,
            theme.muted_foreground,
            cx,
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
            v_flex().pl_3().children(rows).into_any_element()
        });
        v_flex()
            .child(self.spotted(&super::spot::field(path), header, cx))
            .children(content)
            .into_any_element()
    }
}
