//! Hooks page: the ordered hook chain and the hooks still available.

use crate::ui::kit::ButtonLabel as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::*;
use gpui_kit::*;

use super::SettingsView;
use super::widgets::{card, hashed_id, item_row, named, note, section};
use crate::settings::catalog::{HOOK_SLOT, module_info};
use crate::ui::kit;
use crate::ui::prefs;
use crate::ui::reorder;

impl SettingsView {
    pub(super) fn hooks_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        let theme = cx.theme();
        let enabled: Vec<reorder::Row> = self
            .draft
            .hooks
            .iter()
            .enumerate()
            .map(|(ix, id)| {
                let description = module_info(snapshot, HOOK_SLOT, id)
                    .map(|info| info.description.unwrap_or_default().to_string())
                    .unwrap_or_else(|| "Модуль не найден в текущей сборке".to_string());
                let key = format!("params:{HOOK_SLOT}:{id}");
                let open = self.is_open(&key);
                let off = id.clone();
                let title = h_flex()
                    .min_w_0()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .w(prefs::scaled(20.))
                            .flex_none()
                            .text_color(theme.muted_foreground)
                            .child(format!("{}.", ix + 1)),
                    )
                    .child(named(
                        hashed_id("hook-name", id),
                        id.clone(),
                        Some(description.into()),
                        None,
                        cx,
                    ));
                let header = item_row(
                    title,
                    [reorder::controls(
                        kit::button(hashed_id("hook-off", id))
                            .faded_label("Отключить")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.update_draft(cx, |draft| draft.set_hook(&off, false))
                            })),
                    )
                    .into_any_element()],
                );
                reorder::Row::new(
                    id.clone(),
                    id.clone(),
                    v_flex()
                        .child(self.spotted(&super::spot::hook(id), header, cx))
                        .child(
                            reorder::controls(
                                self.params_disclosure(HOOK_SLOT, id, &key, open, cx),
                            )
                            .w_full(),
                        ),
                )
            })
            .collect();
        let enabled = if enabled.is_empty() {
            card(
                [note("Обработчики не включены.", cx).into_any_element()],
                cx,
            )
        } else {
            let view = cx.weak_entity();
            card(
                [
                    reorder::List::new("settings-hook-order", enabled, move |id, before, _, cx| {
                        let _ = view.update(cx, |this, cx| {
                            let id = id.to_string();
                            let before = before.map(|id| id.to_string());
                            this.update_draft(cx, |draft| {
                                reorder::move_before(&mut draft.hooks, &id, before.as_ref());
                            });
                        });
                    })
                    .into_any_element(),
                ],
                cx,
            )
        };

        let available: Vec<AnyElement> = snapshot
            .hook_modules
            .iter()
            .filter(|module| !self.draft.hooks.contains(&module.id))
            .map(|module| {
                let id = module.id.clone();
                let row = item_row(
                    named(
                        hashed_id("hook-name", &module.id),
                        module.id.clone(),
                        module.description.clone().map(Into::into),
                        None,
                        cx,
                    ),
                    [kit::button(hashed_id("hook-on", &module.id))
                        .faded_label("Включить")
                        .outline()
                        .xsmall()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.update_draft(cx, |draft| draft.set_hook(&id, true))
                        }))
                        .into_any_element()],
                );
                self.spotted(&super::spot::hook(&module.id), row, cx)
            })
            .collect();
        let available = if available.is_empty() {
            let text = if snapshot.hook_modules.is_empty() {
                "В профиле нет модулей обработчиков."
            } else {
                "Все обработчики включены."
            };
            card([note(text, cx).into_any_element()], cx)
        } else {
            card(available, cx)
        };
        vec![
            section("Включены", None, None, enabled, cx).into_any_element(),
            section("Доступны", None, None, available, cx).into_any_element(),
        ]
    }
}
