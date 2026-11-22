//! Pages of select-one slots: implementation choice and its parameters.

use std::path::Path;

use gpui_kit::component::*;
use gpui_kit::*;

use super::widgets::{
    Tone, card, chip, disclosure, hashed_id, item_row, mono, nested, nested_group, note,
    radio_card, row, section,
};
use super::{SettingsView, spot};
use crate::settings::catalog::{
    CONTEXT_PROVIDER_SLOT, CONTEXT_SLOT, POLICY_SLOT, SESSION_STORAGE_SLOT, exports, find_slot,
    permission_description, selected_module,
};
use crate::ui::kit::{self, ButtonLabel as _};

pub(super) const POLICY_RULES: &str = "policy-tool-rules";

/// Implementation links and parameter search share the same disclosure path.
pub(super) fn implementation_disclosures(slot: &str) -> Vec<String> {
    if slot == POLICY_SLOT {
        vec![POLICY_RULES.to_owned()]
    } else {
        Vec::new()
    }
}

fn storage_title(id: &str) -> &str {
    match id {
        "jsonl" => "JSONL",
        "sqlite" => "SQLite",
        other => other,
    }
}

impl SettingsView {
    pub(super) fn slot_page(&self, slot: &str, cx: &Context<Self>) -> Vec<AnyElement> {
        if slot == SESSION_STORAGE_SLOT {
            return self.storage_page(cx);
        }
        let mut parts = Vec::new();
        parts.extend(self.implementation(slot, cx));
        if slot == CONTEXT_SLOT {
            parts.extend(self.providers_section(cx));
        }
        parts
    }

    pub(super) fn policy_rules(&self, cx: &Context<Self>) -> AnyElement {
        let view = cx.weak_entity();
        let changed = self.draft.modules.get(POLICY_SLOT) != self.baseline.modules.get(POLICY_SLOT)
            || self.draft.config.get(POLICY_SLOT) != self.baseline.config.get(POLICY_SLOT);
        let header = disclosure(
            "policy-tool-rules-toggle".into(),
            self.is_open(POLICY_RULES),
            "Правила инструментов",
            changed,
            move |cx| {
                let _ = view.update(cx, |this, cx| this.toggle_open(POLICY_RULES, cx));
            },
            cx,
        );
        let body = self.reveal(POLICY_RULES, cx, || {
            nested(
                v_flex()
                    .gap_4()
                    .children(self.implementation(POLICY_SLOT, cx)),
            )
            .into_any_element()
        });
        nested_group(header, body, cx)
    }

    fn storage_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        if find_slot(snapshot, SESSION_STORAGE_SLOT).is_none() {
            return vec![card(
                [
                    note("Эта версия агента не поддерживает выбор хранилища.", cx)
                        .into_any_element(),
                ],
                cx,
            )];
        }
        let selected = selected_module(snapshot, &self.draft, SESSION_STORAGE_SLOT);
        let title = selected.map_or("Только в памяти", storage_title);
        let file = snapshot
            .target_path
            .clone()
            .or_else(|| snapshot.config_path.clone());
        let action = file.map(|path| {
            kit::button("storage-profile")
                .faded_label("Файл профиля")
                .quiet(cx)
                .xsmall()
                .on_click(move |_, _, cx| cx.open_with_system(Path::new(&path)))
                .into_any_element()
        });
        let row = item_row(mono(title, cx), action).into_any_element();
        let row = match selected {
            Some(id) => self.spotted(&spot::module(SESSION_STORAGE_SLOT, id), row, cx),
            None => row,
        };
        vec![
            card([row], cx),
            note(
                "Выбор хранилища в файле профиля применяется при следующем запуске агента.",
                cx,
            )
            .into_any_element(),
        ]
    }

    pub(super) fn mode_section(&self, cx: &Context<Self>) -> AnyElement {
        let Some(snapshot) = self.snapshot() else {
            return div().into_any_element();
        };
        let view = cx.weak_entity();
        let cards = snapshot.permission_modes.iter().map(|mode| {
            let description = permission_description(mode);
            let (view, value) = (view.clone(), mode.clone());
            radio_card(
                hashed_id("mode", mode),
                self.draft.mode == *mode,
                crate::settings::catalog::permission_title(mode).to_owned(),
                Some(description.into()),
                Vec::new(),
                move |cx| {
                    let value = value.clone();
                    let _ = view.update(cx, |this, cx| {
                        if this
                            .snapshot()
                            .is_some_and(|snapshot| snapshot.permission_modes.contains(&value))
                        {
                            this.update_draft(cx, |draft| draft.mode = value);
                        }
                    });
                },
                cx,
            )
        });
        section(
            "Режим прав по умолчанию",
            None,
            None,
            v_flex()
                .gap_2()
                .children(
                    (!snapshot.permission_modes.contains(&self.draft.mode)).then(|| {
                        note(
                            format!(
                                "Сохранённый режим: {}. Можно оставить его или выбрать другой.",
                                crate::settings::catalog::permission_title(&self.draft.mode)
                            ),
                            cx,
                        )
                    }),
                )
                .children(cards),
            cx,
        )
        .into_any_element()
    }

    fn implementation(&self, slot: &str, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        let Some(info) = find_slot(snapshot, slot) else {
            return vec![
                section(
                    "Реализация",
                    None,
                    None,
                    card(
                        [note("Этот слот недоступен в текущей сборке.", cx).into_any_element()],
                        cx,
                    ),
                    cx,
                )
                .into_any_element(),
            ];
        };
        if info.modules.is_empty() {
            return vec![
                section(
                    "Реализация",
                    None,
                    None,
                    card(
                        [note(
                            "Профиль не подключает модулей для этого слота. Реализация появится здесь, когда компонент профиля предоставит такой export.",
                            cx,
                        )
                        .into_any_element()],
                        cx,
                    ),
                    cx,
                )
                .into_any_element(),
            ];
        }
        let selected = selected_module(snapshot, &self.draft, slot).map(str::to_string);
        let view = cx.weak_entity();
        let cards = info.modules.iter().map(|module| {
            let mut chips = Vec::new();
            for label in [&module.version] {
                if !label.is_empty()
                    && label != "external"
                    && !chips.iter().any(|(text, _)| text == label)
                {
                    chips.push((label.clone(), chip(label.clone(), Tone::Muted, cx)));
                }
            }
            if info.modules.len() == 1 && selected.as_deref() == Some(module.id.as_str()) {
                return self.spotted(
                    &spot::module(slot, &module.id),
                    card(
                        [row(
                            module.id.clone(),
                            module.description.clone().map(Into::into),
                            h_flex()
                                .min_w_0()
                                .gap_2()
                                .children(chips.into_iter().map(|(_, chip)| chip)),
                            false,
                            cx,
                        )
                        .into_any_element()],
                        cx,
                    ),
                    cx,
                );
            }
            let (view, slot_id, id) = (view.clone(), slot.to_string(), module.id.clone());
            let card = radio_card(
                hashed_id("module", &(slot, &module.id)),
                selected.as_deref() == Some(module.id.as_str()),
                mono(module.id.clone(), cx),
                module.description.clone().map(Into::into),
                chips.into_iter().map(|(_, chip)| chip).collect(),
                move |cx| {
                    let (slot, id) = (slot_id.clone(), id.clone());
                    let _ = view.update(cx, |this, cx| {
                        this.update_draft(cx, |draft| draft.set_module(&slot, &id))
                    });
                },
                cx,
            );
            self.spotted(&spot::module(slot, &module.id), card, cx)
        });
        let mut parts = vec![
            section(
                "Реализация",
                None,
                None,
                v_flex().gap_2().children(cards),
                cx,
            )
            .into_any_element(),
        ];
        if let Some(module) = selected {
            parts.extend(self.params_section(slot, &module, cx));
        }
        parts
    }

    /// Context providers of the profile, under the context module that reads
    /// them.
    fn providers_section(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let snapshot = self.snapshot()?;
        let theme = cx.theme();
        let rows: Vec<AnyElement> = exports(snapshot, CONTEXT_PROVIDER_SLOT)
            .map(|(plugin, export)| {
                let key = format!("params:{CONTEXT_PROVIDER_SLOT}:{}", export.id);
                let open = self.is_open(&key);
                let header = h_flex().px_4().py_3().gap_3().items_start().child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(mono(export.id.clone(), cx).text_sm())
                        .children(export.description.clone().map(|text| {
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(text)
                        }))
                        .child(
                            h_flex()
                                .gap_1()
                                .child(chip(plugin.id.clone(), Tone::Muted, cx))
                                .child(if export.active {
                                    chip("подключён", Tone::Success, cx)
                                } else {
                                    chip("не подключён", Tone::Muted, cx)
                                }),
                        ),
                );
                v_flex()
                    .child(self.spotted(
                        &spot::export(CONTEXT_PROVIDER_SLOT, &export.id),
                        header,
                        cx,
                    ))
                    .children(self.params_disclosure(
                        CONTEXT_PROVIDER_SLOT,
                        &export.id,
                        &key,
                        open,
                        cx,
                    ))
                    .into_any_element()
            })
            .collect();
        let providers = if rows.is_empty() {
            card(
                [note("Профиль не подключает поставщиков контекста.", cx).into_any_element()],
                cx,
            )
        } else {
            card(rows, cx)
        };
        Some(
            section(
                "Context Providers",
                Some(
                    "Какие поставщики читаются автоматически, задают параметры модуля Context выше."
                        .into(),
                ),
                None,
                providers,
                cx,
            )
            .into_any_element(),
        )
    }

    /// Collapsible parameters inside a card row.
    pub(super) fn params_disclosure(
        &self,
        slot: &str,
        module: &str,
        key: &str,
        open: bool,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if !self.has_params(slot, module) {
            return None;
        }
        if self
            .snapshot()
            .and_then(|snapshot| crate::settings::catalog::module_info(snapshot, slot, module))
            .and_then(|info| info.schema)
            .is_some_and(|schema| {
                schema
                    .fields
                    .iter()
                    .filter(|field| crate::settings::catalog::editable_field(field))
                    .count()
                    == 1
            })
        {
            return Some(
                crate::ui::reorder::controls(self.params_nested(slot, module, cx))
                    .w_full()
                    .into_any_element(),
            );
        }
        let changed =
            self.draft.module_config(slot, module) != self.baseline.module_config(slot, module);
        let view = cx.weak_entity();
        let toggle_key = key.to_string();
        Some(super::widgets::nested_group(
            super::widgets::disclosure(
                hashed_id("params-open", &key),
                open,
                "Параметры",
                changed,
                move |cx| {
                    let _ = view.update(cx, |this, cx| this.toggle_open(&toggle_key, cx));
                },
                cx,
            ),
            self.reveal(key, cx, || {
                crate::ui::reorder::controls(self.params_nested(slot, module, cx))
                    .w_full()
                    .into_any_element()
            }),
            cx,
        ))
    }
}
