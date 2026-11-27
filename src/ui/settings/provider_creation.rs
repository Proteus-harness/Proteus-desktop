//! Adding a provider uses module-authored templates and the same authoritative
//! package registrar as the catalog. No provider ids or credentials live here.

use super::{
    SettingsView, spot,
    widgets::{card, hashed_id, note, row, section},
};
use crate::ui::kit::ButtonLabel as _;
use crate::{
    catalog::connections::{self, Discovery},
    settings::{catalog::PageId, reconcile::CatalogChange},
    ui::{kit, prefs},
};
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::*;

pub(super) struct Creation {
    generation: uuid::Uuid,
    name: Entity<InputState>,
    selected: Option<usize>,
    discovery: Option<Discovery>,
    error: Option<String>,
    _changes: Subscription,
}

impl SettingsView {
    pub(super) fn begin_provider_creation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(reason) = self.profile_blocked() {
            self.show_feedback(reason, true, cx);
            return;
        }
        if !self.catalog_idle() {
            return;
        }
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Например, work-api"));
        let changes = cx.subscribe(&name, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let generation = uuid::Uuid::new_v4();
        self.provider_creation = Some(Creation {
            generation,
            name,
            selected: None,
            discovery: None,
            error: None,
            _changes: changes,
        });
        let installed = prefs::current(cx).catalogs.installed.clone();
        cx.spawn(async move |this, cx| {
            let discovery = cx
                .background_executor()
                .spawn(async move { connections::discover(&installed) })
                .await;
            this.update(cx, |this, cx| {
                if let Some(state) = this
                    .provider_creation
                    .as_mut()
                    .filter(|state| state.generation == generation)
                {
                    state.discovery = Some(discovery);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn choose_provider_template(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.snapshot() else {
            return;
        };
        let Some(choice) = self
            .provider_creation
            .as_ref()
            .and_then(|state| state.discovery.as_ref())
            .and_then(|d| d.choices.get(index))
        else {
            return;
        };
        let id = choice.available_id(snapshot);
        let Some(state) = self.provider_creation.as_mut() else {
            return;
        };
        state.selected = Some(index);
        state.error = None;
        state
            .name
            .update(cx, |name, cx| name.set_value(id, window, cx));
        cx.notify();
    }

    pub(super) fn provider_creation_form(
        &self,
        state: &Creation,
        cx: &Context<Self>,
    ) -> AnyElement {
        let cancel = kit::button("provider-create-cancel")
            .faded_label("Отмена")
            .quiet(cx)
            .small()
            .disabled(self.catalogs.busy)
            .on_click(cx.listener(|this, _, _, cx| {
                this.provider_creation = None;
                cx.notify();
            }));
        let Some(discovery) = &state.discovery else {
            return section(
                "Новое подключение",
                None,
                Some(cancel.into_any_element()),
                card(
                    [note("Читаю установленные модули…", cx).into_any_element()],
                    cx,
                ),
                cx,
            )
            .into_any_element();
        };
        let choices: Vec<_> = discovery
            .choices
            .iter()
            .enumerate()
            .map(|(index, choice)| (index, choice.template.title.clone()))
            .collect();
        let selected = state.selected;
        let label = selected
            .and_then(|index| discovery.choices.get(index))
            .map(|choice| choice.template.title.clone())
            .unwrap_or_else(|| "Выбрать провайдера".into());
        let view = cx.weak_entity();
        let selector = crate::ui::context::menu_button(
            "provider-template-menu",
            kit::button("provider-template")
                .faded_label(label)
                .small()
                .disabled(self.catalogs.busy || choices.is_empty()),
            true,
            move |menu, _, _| {
                choices.iter().fold(menu, |menu, (index, title)| {
                    let (view, index) = (view.clone(), *index);
                    menu.item(
                        crate::ui::selection::menu_option(title.clone(), selected == Some(index))
                            .on_click(move |_, window, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.choose_provider_template(index, window, cx);
                                });
                            }),
                    )
                })
            },
        )
        .settings_selector()
        .into_any_element();
        let description = selected
            .and_then(|index| discovery.choices.get(index))
            .map(|choice| choice.template.description.clone().into());
        let mut rows = vec![row("Провайдер", description, selector, false, cx).into_any_element()];
        if let Some((index, choice)) = state
            .selected
            .and_then(|index| discovery.choices.get(index).map(|choice| (index, choice)))
        {
            rows.push(
                row(
                    "Имя подключения",
                    Some(
                        "Уникальное имя в этом конфиге. На него ссылаются настройки моделей."
                            .into(),
                    ),
                    kit::input(&state.name)
                        .small()
                        .w(prefs::scaled(260.))
                        .disabled(!choice.named || self.catalogs.busy),
                    false,
                    cx,
                )
                .into_any_element(),
            );
            let id = state.name.read(cx).value().to_string();
            let problem = self
                .snapshot()
                .and_then(|snapshot| choice.check_id(&id, snapshot).err())
                .map(|error| error.to_string());
            rows.extend(problem.as_ref().map(|error| {
                note(error.clone(), cx)
                    .text_color(crate::ui::theme::danger_text(cx.theme()))
                    .into_any_element()
            }));
            rows.push(
                h_flex()
                    .px_4()
                    .pb_3()
                    .justify_end()
                    .child(
                        kit::button(hashed_id("provider-create-submit", &index))
                            .label("Добавить подключение")
                            .small()
                            .disabled(
                                problem.is_some()
                                    || !self.catalog_idle()
                                    || self.profile_blocked().is_some(),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.create_provider(cx))),
                    )
                    .into_any_element(),
            );
        } else {
            if discovery.choices.is_empty() {
                rows.push(note("В установленных модулях нет вариантов подключения. Добавьте модуль из каталога.",cx).into_any_element());
            }
            rows.extend(
                discovery
                    .problems
                    .iter()
                    .map(|problem| note(problem.clone(), cx).into_any_element()),
            );
            rows.push(
                h_flex()
                    .px_4()
                    .pb_3()
                    .child(
                        kit::button("provider-create-catalog")
                            .faded_label("Открыть каталог модулей")
                            .quiet(cx)
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.provider_creation = None;
                                this.go_to(&spot::Destination::new(PageId::Catalog), window, cx);
                            })),
                    )
                    .into_any_element(),
            );
        }
        rows.extend(state.error.as_ref().map(|error| {
            note(error.clone(), cx)
                .text_color(crate::ui::theme::danger_text(cx.theme()))
                .into_any_element()
        }));
        section(
            "Новое подключение",
            None,
            Some(cancel.into_any_element()),
            card(rows, cx),
            cx,
        )
        .into_any_element()
    }

    fn create_provider(&mut self, cx: &mut Context<Self>) {
        if !self.catalog_idle() {
            return;
        }
        if let Some(reason) = self.profile_blocked() {
            self.show_feedback(reason, true, cx);
            return;
        }
        let Some(state) = self.provider_creation.as_ref() else {
            return;
        };
        let Some(choice) = state
            .discovery
            .as_ref()
            .and_then(|d| state.selected.and_then(|index| d.choices.get(index)))
        else {
            return;
        };
        let id = state.name.read(cx).value().to_string();
        let Some(snapshot) = self.snapshot() else {
            return;
        };
        let request = match choice.request(&id, snapshot) {
            Ok(request) => request,
            Err(error) => {
                if let Some(state) = self.provider_creation.as_mut() {
                    state.error = Some(error.to_string());
                }
                cx.notify();
                return;
            }
        };
        let Some(super::context::Profile { api, dir }) = self.context.profile.clone() else {
            return;
        };
        let submitted_revision = self.edit_revision;
        let previous = self.baseline.clone();
        self.catalogs.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.save_config_builder(&dir, &request) })
                .await;
            this.update(cx, |this, cx| {
                this.finish_provider_registration(result, submitted_revision, &previous, cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn finish_provider_registration(
        &mut self,
        result: anyhow::Result<
            proteus_contracts::app_protocol::config_builder::ConfigBuilderSnapshot,
        >,
        submitted_revision: u64,
        previous: &crate::settings::Draft,
        cx: &mut Context<Self>,
    ) {
        let accepted = result.is_ok();
        if let Err(error) = &result
            && let Some(state) = self.provider_creation.as_mut()
        {
            state.error = Some(format!("Не удалось добавить подключение. {error:#}"));
        }
        self.finish_provider_write(
            result,
            submitted_revision,
            previous,
            "Подключение добавлено",
            cx,
        );
        if accepted {
            self.provider_creation = None;
        }
    }

    fn finish_provider_write(
        &mut self,
        result: anyhow::Result<
            proteus_contracts::app_protocol::config_builder::ConfigBuilderSnapshot,
        >,
        submitted_revision: u64,
        previous: &crate::settings::Draft,
        message: &'static str,
        cx: &mut Context<Self>,
    ) {
        if let Ok(snapshot) = &result
            && self.edit_revision != submitted_revision
        {
            self.draft
                .rebase_registration(previous, &crate::settings::Draft::from_snapshot(snapshot));
            self.editors
                .registered(previous, &self.draft, submitted_revision);
        }
        let change = CatalogChange {
            before: previous.addons.clone(),
            replacement: None,
        };
        self.finish_catalog_profile_write(result, submitted_revision, &change, |_| {}, message, cx);
        cx.notify();
    }

    pub(super) fn provider_models(&self, id: &str) -> Vec<String> {
        self.draft
            .models
            .iter()
            .filter(|(_, model)| model.provider == id)
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub(super) fn confirm_provider_removal(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.catalog_idle() || self.profile_blocked().is_some() {
            return;
        }
        let users = self.provider_models(id);
        if !users.is_empty() {
            self.show_feedback(
                format!("Подключение используется в моделях {}", users.join(", ")),
                true,
                cx,
            );
            return;
        }
        let (view, id) = (cx.weak_entity(), id.to_owned());
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (view, id) = (view.clone(), id.clone());
            alert
                .title("Удалить подключение?")
                .description(format!("«{id}» будет удалено из профиля."))
                .ok_text("Удалить")
                .ok_variant(ButtonVariant::Danger)
                .show_cancel(true)
                .cancel_text("Отмена")
                .on_ok(move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| this.remove_provider(&id, cx));
                    true
                })
        });
    }

    fn remove_provider(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.catalog_idle()
            || self.profile_blocked().is_some()
            || !self.provider_models(id).is_empty()
        {
            return;
        }
        let Some(super::context::Profile { api, dir }) = self.context.profile.clone() else {
            return;
        };
        let request = proteus_contracts::app_protocol::http::SetConfigBuilderRequest {
            remove_model_connections: vec![id.to_owned()],
            ..Default::default()
        };
        let previous = self.baseline.clone();
        let submitted_revision = self.edit_revision;
        let id = id.to_owned();
        self.catalogs.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.save_config_builder(&dir, &request) })
                .await;
            this.update(cx, |this, cx| {
                this.finish_provider_removal(&id, result, submitted_revision, &previous, cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn finish_provider_removal(
        &mut self,
        id: &str,
        result: anyhow::Result<
            proteus_contracts::app_protocol::config_builder::ConfigBuilderSnapshot,
        >,
        submitted_revision: u64,
        previous: &crate::settings::Draft,
        cx: &mut Context<Self>,
    ) {
        if result.is_ok() && self.edit_revision != submitted_revision {
            if let Some(config) = self.draft.config.get_mut("model") {
                config.remove(id);
            }
            self.editors
                .fields
                .retain(|path, _| path.slot != "model" || path.module != id);
            self.editors.json.remove(&("model".into(), id.to_owned()));
        }
        self.finish_provider_write(
            result,
            submitted_revision,
            previous,
            "Подключение удалено",
            cx,
        );
    }
}

#[cfg(test)]
mod tests;
