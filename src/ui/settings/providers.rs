//! Connections are model exports, not the named model presets that use them.
// Layout follows Zed: crates/settings_ui/src/pages/llm_providers_page.rs.

use super::{
    SettingsView, spot,
    widgets::{card, mono, note, section},
};
use crate::ui::kit::ButtonLabel as _;
use crate::{
    settings::catalog::{MODEL_SLOT, PageId},
    ui::kit,
};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::*;

impl SettingsView {
    pub(super) fn providers_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        let add = kit::button("provider-add")
            .label("Добавить провайдера")
            .icon(Lucide::Plus)
            .small()
            .disabled(!self.catalog_idle() || self.provider_creation.is_some())
            .on_click(cx.listener(|this, _, window, cx| this.begin_provider_creation(window, cx)));
        let mut parts = vec![
            section("Подключения", None, Some(add.into_any_element()), div(), cx)
                .into_any_element(),
        ];
        if let Some(creation) = self.provider_creation.as_ref() {
            parts.push(self.provider_creation_form(creation, cx));
        }
        if snapshot.model_modules.is_empty() {
            parts.push(card(
                [note(
                    "Подключений пока нет. Добавьте провайдера из установленных модулей.",
                    cx,
                )
                .into_any_element()],
                cx,
            ));
            return parts;
        }
        for module in &snapshot.model_modules {
            let title = mono(module.id.clone(), cx).text_sm();
            let to_models = kit::button(super::widgets::hashed_id("provider-models", &module.id))
                .faded_label("Модели")
                .quiet(cx)
                .xsmall()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.go_to(&spot::Destination::new(PageId::Models), window, cx)
                }));
            let header = h_flex()
                .px_4()
                .py_3()
                .gap_3()
                .child(div().flex_1().min_w_0().child(title))
                .child(to_models);
            let body = card(
                [
                    header.into_any_element(),
                    self.params_nested(MODEL_SLOT, &module.id, cx),
                ],
                cx,
            );
            parts.push(self.spotted(&spot::module(MODEL_SLOT, &module.id), body, cx));
        }
        parts
    }
}
