// Адаптировано из Zed: crates/settings_ui/src/pages/llm_providers_page.rs
// and crates/settings_ui/src/components/input_field.rs (compact editable rows).
//! Reusable named models and explicit/inherited choices. Connections live on
//! the Providers page, independently of whichever preset is selected.

use super::widgets::{
    Fit, MenuOption, Tone, card, chip, disclosure, field_row, hashed_id, mono_line, nested_group,
    note, row, section, select_menu,
};
use super::{SettingsView, spot};
use crate::settings::catalog::PageId;
use crate::settings::models::{ModelText, model_problems, new_model_name_problem};
use crate::ui::kit::{self, ButtonLabel as _, Hint as _};
use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderModelPreset, ConfigBuilderModelRole,
};

pub(super) fn model_key(name: &str) -> String {
    format!("model-preset:{name}")
}

impl SettingsView {
    pub(super) fn models_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        let mut parts = Vec::new();
        let root = if snapshot.model_writable {
            self.model_choice(
                "root",
                self.draft.model.clone(),
                Some("По умолчанию"),
                cx,
                |this, value, cx| this.update_draft(cx, |draft| draft.model = value),
            )
        } else {
            chip(
                self.draft
                    .model
                    .as_deref()
                    .map(|name| self.model_label(name))
                    .unwrap_or_else(|| "Собственная конфигурация".into()),
                Tone::Muted,
                cx,
            )
            .into_any_element()
        };
        parts.push(card(
            [row(
                "Модель основного агента",
                Some("Для отдельного чата можно выбрать другую модель.".into()),
                root,
                false,
                cx,
            )
            .into_any_element()],
            cx,
        ));
        let presets: Vec<_> = self
            .draft
            .models
            .iter()
            .map(|(name, model)| self.model_preset(name, model, snapshot.models_writable, cx))
            .collect();
        let mut body = v_flex().children(presets);
        if self.draft.models.is_empty() {
            body = body.child(note("Добавьте имя для модели, которую хотите выбрать.", cx));
        }
        if snapshot.models_writable {
            body = body.child(div().mt_2().child(self.new_model_row(cx)));
        } else {
            body = body.child(note("Эти модели редактируются в исходном конфиге.", cx));
        }
        parts.push(section("Имена моделей", None, None, body, cx).into_any_element());
        if !snapshot.model_roles.is_empty() {
            parts.push(
                section(
                    "Модели субагентов",
                    None,
                    None,
                    card(
                        snapshot
                            .model_roles
                            .iter()
                            .map(|role| self.model_role_row(role, cx)),
                        cx,
                    ),
                    cx,
                )
                .into_any_element(),
            );
        }
        let problems = model_problems(&self.draft, snapshot);
        if !problems.is_empty() {
            parts.push(
                v_flex()
                    .gap_1()
                    .children(problems.into_iter().map(|problem| {
                        div().text_sm().text_color(cx.theme().danger).child(problem)
                    }))
                    .into_any_element(),
            );
        }
        parts
    }

    fn new_model_row(&self, cx: &Context<Self>) -> AnyElement {
        let Some(input) = &self.editors.models.name else {
            return div().into_any_element();
        };
        let name = input.state.read(cx).value();
        let problem = new_model_name_problem(&self.draft, &name);
        let no_connection = self
            .snapshot()
            .is_none_or(|snapshot| snapshot.model_modules.is_empty());
        let add = kit::button("model-preset-add")
            .faded_label("Добавить имя")
            .small()
            .disabled(problem.is_some() || no_connection)
            .on_click(cx.listener(|this, _, window, cx| this.add_model_preset(window, cx)));
        card(
            [v_flex()
                .gap_2()
                .px_4()
                .py_3()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(kit::input(&input.state).small().w_full()),
                        )
                        .child(add),
                )
                .children(
                    (!name.is_empty())
                        .then_some(problem)
                        .flatten()
                        .map(|problem| {
                            div().text_xs().text_color(cx.theme().danger).child(problem)
                        }),
                )
                .children(no_connection.then(|| {
                    self.settings_link(
                        "models-open-providers".into(),
                        "Сначала добавьте подключение",
                        spot::Destination::new(PageId::Providers),
                        cx,
                    )
                }))
                .into_any_element()],
            cx,
        )
    }

    pub(super) fn add_model_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .snapshot()
            .is_some_and(|snapshot| snapshot.models_writable)
        {
            return;
        }
        let Some(input) = self
            .editors
            .models
            .name
            .as_ref()
            .map(|input| input.state.clone())
        else {
            return;
        };
        let name = input.read(cx).value().to_string();
        if new_model_name_problem(&self.draft, &name).is_some() {
            return;
        }
        let Some(provider) = self
            .snapshot()
            .and_then(|snapshot| snapshot.model_modules.first())
            .map(|module| module.id.clone())
        else {
            return;
        };
        self.update_draft(cx, |draft| {
            draft.models.insert(
                name.clone(),
                ConfigBuilderModelPreset {
                    provider,
                    model: String::new(),
                    stream: true,
                    reasoning: Default::default(),
                    reasoning_efforts: Vec::new(),
                },
            );
        });
        self.open.insert(model_key(&name));
        input.update(cx, |input, cx| input.set_value("", window, cx));
    }

    fn model_preset(
        &self,
        name: &str,
        model: &ConfigBuilderModelPreset,
        editable: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let key = model_key(name);
        let open = self.is_open(&key);
        let changed = self.baseline.models.get(name) != Some(model)
            || self
                .editors
                .models
                .fields
                .iter()
                .any(|((alias, _), field)| alias == name && field.error.is_some());
        let (view, toggle_key) = (cx.weak_entity(), key.clone());
        let header = disclosure(
            hashed_id("model-preset", &key),
            open,
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(mono_line(name.to_owned(), cx))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .min_w_0()
                        .child(crate::ui::fade::faded(format!(
                            "{} · {}",
                            model.model, model.provider
                        ))),
                ),
            changed,
            move |cx| {
                let _ = view.update(cx, |this, cx| this.toggle_open(&toggle_key, cx));
            },
            cx,
        );
        let content = self.reveal(&key, cx, || {
            self.model_preset_fields(name, model, editable, cx)
        });
        self.spotted(&spot::model(name), nested_group(header, content, cx), cx)
    }

    fn model_preset_fields(
        &self,
        name: &str,
        model: &ConfigBuilderModelPreset,
        editable: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        if !editable {
            let values = [
                ("Подключение", model.provider.clone()),
                ("ID модели", model.model.clone()),
                (
                    "Effort",
                    model
                        .reasoning
                        .effort
                        .clone()
                        .unwrap_or_else(|| "По умолчанию".into()),
                ),
                (
                    "Бюджет рассуждения",
                    model
                        .reasoning
                        .budget_tokens
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "Не задано".into()),
                ),
                (
                    "Доступные уровни effort",
                    if model.reasoning_efforts.is_empty() {
                        "Не заданы".into()
                    } else {
                        model.reasoning_efforts.join(", ")
                    },
                ),
                (
                    "Потоковый ответ",
                    if model.stream { "Да" } else { "Нет" }.into(),
                ),
                (
                    "Сводка рассуждений",
                    if model.reasoning.summary {
                        "Да"
                    } else {
                        "Нет"
                    }
                    .into(),
                ),
            ];
            return card(
                values.into_iter().map(|(title, value)| {
                    row(title, None, mono_line(value, cx), false, cx).into_any_element()
                }),
                cx,
            );
        }
        let snapshot = self.snapshot().expect("model fields require snapshot");
        let options = snapshot
            .model_modules
            .iter()
            .map(|module| MenuOption {
                label: module.id.clone().into(),
                value: module.id.clone(),
                checked: module.id == model.provider,
            })
            .collect();
        let (view, alias) = (cx.weak_entity(), name.to_owned());
        let connection = select_menu(
            hashed_id("model-connection", &name),
            model.provider.clone(),
            options,
            move |value, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.update_draft(cx, |draft| {
                        if let Some(model) = draft.models.get_mut(&alias) {
                            model.provider = value;
                        }
                    })
                });
            },
        );
        let mut rows = vec![row("Подключение", None, connection, false, cx).into_any_element()];
        for field in ModelText::ALL {
            if let Some(input) = self.editors.models.fields.get(&(name.to_owned(), field)) {
                let editor =
                    v_flex()
                        .gap_1()
                        .child(kit::input(&input.state).small().w_full())
                        .children(input.error.clone().map(|error| {
                            div().text_xs().text_color(cx.theme().danger).child(error)
                        }));
                rows.push(field_row(field.title(), editor, Fit::Fill).into_any_element());
            }
        }
        for (field, title, checked) in [
            (false, "Потоковый ответ", model.stream),
            (true, "Сводка рассуждений", model.reasoning.summary),
        ] {
            let (view, alias) = (cx.weak_entity(), name.to_owned());
            let switch = kit::switch(hashed_id("model-switch", &(name, field)))
                .checked(checked)
                .on_click(move |checked, _, cx| {
                    let checked = *checked;
                    let _ = view.update(cx, |this, cx| {
                        this.update_draft(cx, |draft| {
                            if let Some(model) = draft.models.get_mut(&alias) {
                                if field {
                                    model.reasoning.summary = checked;
                                } else {
                                    model.stream = checked;
                                }
                            }
                        })
                    });
                });
            rows.push(row(title, None, switch, false, cx).into_any_element());
        }
        let used = self.draft.model.as_deref() == Some(name)
            || self
                .draft
                .role_models
                .values()
                .any(|choice| choice.as_deref() == Some(name));
        let (view, alias) = (cx.weak_entity(), name.to_owned());
        let remove = kit::button(hashed_id("model-remove", &name))
            .faded_label("Удалить набор")
            .quiet(cx)
            .small()
            .disabled(used)
            .hint(
                if used {
                    "Сначала выберите другую модель у использующих этот набор агентов"
                } else {
                    "Удалить набор"
                },
                cx,
            )
            .on_click(move |_, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.update_draft(cx, |draft| {
                        draft.models.remove(&alias);
                    });
                    this.editors
                        .models
                        .fields
                        .retain(|(name, _), _| name != &alias);
                });
            });
        rows.push(
            h_flex()
                .px_4()
                .py_2()
                .justify_end()
                .child(remove)
                .into_any_element(),
        );
        card(rows, cx)
    }

    fn model_label(&self, name: &str) -> String {
        self.draft
            .models
            .get(name)
            .filter(|preset| !preset.model.is_empty() && preset.model != name)
            .map_or_else(
                || name.to_owned(),
                |preset| format!("{name} · {}", preset.model),
            )
    }

    fn model_choice(
        &self,
        id: &str,
        selected: Option<String>,
        inherited: Option<&str>,
        cx: &Context<Self>,
        change: impl Fn(&mut Self, Option<String>, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let mut options = Vec::new();
        if let Some(inherited) = inherited {
            options.push(MenuOption {
                label: inherited.to_owned().into(),
                value: None,
                checked: selected.is_none(),
            });
        }
        options.extend(self.draft.models.keys().map(|name| MenuOption {
            label: self.model_label(name).into(),
            value: Some(name.clone()),
            checked: selected.as_ref() == Some(name),
        }));
        let label = selected
            .as_deref()
            .map(|name| self.model_label(name))
            .unwrap_or_else(|| inherited.unwrap_or("Не выбрана").into());
        let view = cx.weak_entity();
        select_menu(
            hashed_id("model-choice", &id),
            label,
            options,
            move |value, cx| {
                let _ = view.update(cx, |this, cx| change(this, value, cx));
            },
        )
    }

    fn model_role_row(&self, role: &ConfigBuilderModelRole, cx: &Context<Self>) -> AnyElement {
        let choice = if role.editable {
            let name = role.name.clone();
            self.model_choice(
                &format!("role:{}", role.name),
                self.draft
                    .role_models
                    .get(&role.name)
                    .cloned()
                    .unwrap_or_else(|| role.model.clone()),
                Some("Наследовать модель родителя"),
                cx,
                move |this, value, cx| {
                    this.update_draft(cx, |draft| {
                        draft.role_models.insert(name.clone(), value);
                    })
                },
            )
        } else {
            chip(
                role.model
                    .as_deref()
                    .map(|name| self.model_label(name))
                    .unwrap_or_else(|| "Собственная конфигурация".into()),
                Tone::Muted,
                cx,
            )
            .into_any_element()
        };
        self.spotted(
            &spot::model_role(&role.name),
            row(
                role.name.clone(),
                Some(role.description.clone().into()),
                choice,
                false,
                cx,
            ),
            cx,
        )
    }
}
