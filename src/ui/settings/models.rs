// Адаптировано из Zed: crates/settings_ui/src/pages/llm_providers_page.rs
// and crates/settings_ui/src/components/input_field.rs (compact editable rows).
//! Model configuration is edited where it is used: the main agent or a role.

use super::widgets::{
    Fit, MenuOption, card, disclosure, field_row, hashed_id, mono_line, nested_group, note, row,
    section, select_menu,
};
use super::{SettingsView, spot};
use crate::settings::catalog::PageId;
use crate::settings::models::{ModelTarget, ModelText, model_problems, model_text};
use crate::ui::kit;
use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderModelPreset, ConfigBuilderModelRole, ConfigBuilderSnapshot,
};
use std::collections::HashSet;

pub(super) fn model_target_spot(target: &ModelTarget) -> String {
    match target {
        ModelTarget::Root => spot::model("root"),
        ModelTarget::Role(name) => spot::model_role(name),
    }
}

impl SettingsView {
    pub(super) fn models_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        let target = ModelTarget::Root;
        let root = section(
            "Модель основного агента",
            Some("Для отдельного чата можно выбрать другую модель.".into()),
            None,
            self.model_settings(&target, target.configured(&self.draft), snapshot, cx),
            cx,
        );
        let mut parts = vec![self.spotted(&model_target_spot(&target), root, cx)];
        if !snapshot.model_roles.is_empty() {
            let roles = snapshot
                .model_roles
                .iter()
                .map(|role| self.model_role_group(role, snapshot, cx));
            parts.push(
                section(
                    "Модели субагентов",
                    None,
                    None,
                    v_flex().children(roles),
                    cx,
                )
                .into_any_element(),
            );
        }
        parts.extend(
            model_problems(&self.draft, snapshot)
                .into_iter()
                .map(|problem| {
                    div()
                        .text_sm()
                        .text_color(crate::ui::theme::danger_text(cx.theme()))
                        .child(problem)
                        .into_any_element()
                }),
        );
        parts
    }

    fn model_role_group(
        &self,
        role: &ConfigBuilderModelRole,
        snapshot: &ConfigBuilderSnapshot,
        cx: &Context<Self>,
    ) -> AnyElement {
        let target = ModelTarget::Role(role.name.clone());
        let key = model_target_spot(&target);
        let model = if role.editable {
            target.configured(&self.draft)
        } else {
            role.model
                .as_ref()
                .and_then(|alias| self.draft.models.get(alias))
        };
        let detail = |model: &ConfigBuilderModelPreset| {
            if model.model.is_empty() {
                model.provider.clone()
            } else {
                model.model.clone()
            }
        };
        let summary = model.map(detail).unwrap_or_else(|| {
            if role.editable {
                ModelTarget::Root.configured(&self.draft).map_or_else(
                    || "Наследуется".into(),
                    |model| format!("Наследуется · {}", detail(model)),
                )
            } else {
                "Собственная конфигурация".into()
            }
        });
        let changed = target.alias(&self.draft) != target.alias(&self.baseline)
            || target.configured(&self.draft) != target.configured(&self.baseline)
            || self
                .editors
                .models
                .fields
                .iter()
                .any(|((key, _), input)| *key == target && input.error.is_some());
        let view = cx.weak_entity();
        let toggle_key = key.clone();
        let header = disclosure(
            hashed_id("model-role", &role.name),
            self.is_open(&key),
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(mono_line(role.name.clone(), cx))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .min_w_0()
                        .child(crate::ui::fade::faded(summary)),
                ),
            changed,
            move |cx| {
                let _ = view.update(cx, |this, cx| this.toggle_open(&toggle_key, cx));
            },
            cx,
        );
        let content = self.reveal(&key, cx, || {
            v_flex()
                .children(
                    (!role.description.is_empty()).then(|| note(role.description.clone(), cx)),
                )
                .child(self.model_settings(&target, model, snapshot, cx))
                .into_any_element()
        });
        self.spotted(&key, nested_group(header, content, cx), cx)
    }

    fn model_settings(
        &self,
        target: &ModelTarget,
        model: Option<&ConfigBuilderModelPreset>,
        snapshot: &ConfigBuilderSnapshot,
        cx: &Context<Self>,
    ) -> AnyElement {
        let choice_writable = target.choice_writable(snapshot);
        let editable = choice_writable && snapshot.models_writable;
        let mut rows = Vec::new();
        if editable {
            rows.push(
                row(
                    "Подключение",
                    None,
                    self.model_connection(target, model, snapshot, cx),
                    false,
                    cx,
                )
                .into_any_element(),
            );
        } else if choice_writable {
            rows.push(
                row(
                    "Модель",
                    None,
                    self.readonly_model_choice(target, model, cx),
                    false,
                    cx,
                )
                .into_any_element(),
            );
        } else if let Some(model) = model {
            rows.push(
                row(
                    "Подключение",
                    None,
                    mono_line(model.provider.clone(), cx),
                    false,
                    cx,
                )
                .into_any_element(),
            );
        }
        if let Some(model) = model {
            for field in ModelText::ALL {
                if editable {
                    if let Some(input) = self.editors.models.fields.get(&(target.clone(), field)) {
                        let editor = v_flex()
                            .gap_1()
                            .child(kit::input(&input.state).small().w_full())
                            .children(input.error.clone().map(|error| {
                                div()
                                    .text_xs()
                                    .text_color(crate::ui::theme::danger_text(cx.theme()))
                                    .child(error)
                            }));
                        rows.push(field_row(field.title(), editor, Fit::Fill).into_any_element());
                    }
                } else {
                    let value = model_text(model, field);
                    let value = if value.is_empty() {
                        field.placeholder().into()
                    } else {
                        value
                    };
                    rows.push(
                        row(field.title(), None, mono_line(value, cx), false, cx)
                            .into_any_element(),
                    );
                }
            }
            for (summary, title, checked) in [
                (false, "Потоковый ответ", model.stream),
                (true, "Сводка рассуждений", model.reasoning.summary),
            ] {
                let control = if editable {
                    let view = cx.weak_entity();
                    let target = target.clone();
                    kit::switch(hashed_id("model-switch", &(&target, summary)))
                        .checked(checked)
                        .on_click(move |checked, _, cx| {
                            let checked = *checked;
                            let _ = view.update(cx, |this, cx| {
                                if let Some(mut model) = target.configured(&this.draft).cloned() {
                                    if summary {
                                        model.reasoning.summary = checked;
                                    } else {
                                        model.stream = checked;
                                    }
                                    this.set_target_model(&target, Some(model), cx);
                                }
                            });
                        })
                        .into_any_element()
                } else {
                    mono_line(if checked { "Да" } else { "Нет" }, cx).into_any_element()
                };
                rows.push(row(title, None, control, false, cx).into_any_element());
            }
        }
        if editable && snapshot.model_modules.is_empty() {
            rows.push(
                div()
                    .px_4()
                    .py_2()
                    .child(self.settings_link(
                        format!("models-open-providers-{}", model_target_spot(target)).into(),
                        "Добавить подключение",
                        spot::Destination::new(PageId::Providers),
                        cx,
                    ))
                    .into_any_element(),
            );
        }
        if !choice_writable {
            rows.push(
                div()
                    .px_4()
                    .py_2()
                    .child(note("Модель задаётся в исходном конфиге.", cx))
                    .into_any_element(),
            );
        }
        card(rows, cx)
    }

    fn model_connection(
        &self,
        target: &ModelTarget,
        model: Option<&ConfigBuilderModelPreset>,
        snapshot: &ConfigBuilderSnapshot,
        cx: &Context<Self>,
    ) -> AnyElement {
        let default = default_label(target);
        let options = std::iter::once(MenuOption {
            label: default.into(),
            value: None,
            checked: model.is_none(),
        })
        .chain(snapshot.model_modules.iter().map(|module| MenuOption {
            label: module.id.clone().into(),
            value: Some(module.id.clone()),
            checked: model.is_some_and(|model| model.provider == module.id),
        }))
        .collect();
        let view = cx.weak_entity();
        let target = target.clone();
        select_menu(
            hashed_id("model-connection", &target),
            model
                .map_or(default, |model| model.provider.as_str())
                .to_owned(),
            options,
            move |provider, cx| {
                let _ = view.update(cx, |this, cx| {
                    let next = provider.map(|provider| {
                        let mut model = target
                            .configured(&this.draft)
                            .filter(|model| model.provider == provider)
                            .cloned()
                            .or_else(|| {
                                matches!(target, ModelTarget::Role(_))
                                    .then(|| ModelTarget::Root.configured(&this.draft))
                                    .flatten()
                                    .filter(|model| model.provider == provider)
                                    .cloned()
                            })
                            .or_else(|| {
                                this.draft
                                    .models
                                    .values()
                                    .find(|model| {
                                        model.provider == provider && !model.model.trim().is_empty()
                                    })
                                    .cloned()
                            })
                            .unwrap_or_else(|| ConfigBuilderModelPreset {
                                stream: true,
                                ..Default::default()
                            });
                        model.provider = provider;
                        model
                    });
                    this.set_target_model(&target, next, cx);
                });
            },
        )
    }

    fn readonly_model_choice(
        &self,
        target: &ModelTarget,
        model: Option<&ConfigBuilderModelPreset>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let default = default_label(target);
        let selected = target.alias(&self.draft);
        let route = |model: &ConfigBuilderModelPreset| {
            (
                model.provider.clone(),
                model.model.clone(),
                model.reasoning.effort.clone(),
            )
        };
        let mut seen: HashSet<_> = model.map(route).into_iter().collect();
        let options = std::iter::once(MenuOption {
            label: default.into(),
            value: None,
            checked: selected.is_none(),
        })
        .chain(self.draft.models.iter().filter_map(|(alias, candidate)| {
            let checked = Some(alias.as_str()) == selected;
            if !checked && !seen.insert(route(candidate)) {
                return None;
            }
            Some(MenuOption {
                label: model_label(candidate).into(),
                value: Some(alias.clone()),
                checked,
            })
        }))
        .collect();
        let view = cx.weak_entity();
        let target = target.clone();
        select_menu(
            hashed_id("model-choice", &target),
            model.map_or_else(|| default.into(), model_label),
            options,
            move |value, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.update_draft(cx, |draft| target.set_choice(draft, value))
                });
            },
        )
    }
}

fn default_label(target: &ModelTarget) -> &'static str {
    match target {
        ModelTarget::Root => "Не выбрано",
        ModelTarget::Role(_) => "Наследовать от основного агента",
    }
}

fn model_label(model: &ConfigBuilderModelPreset) -> String {
    match &model.reasoning.effort {
        Some(effort) => format!("{} · {} · {effort}", model.model, model.provider),
        None => format!("{} · {}", model.model, model.provider),
    }
}
