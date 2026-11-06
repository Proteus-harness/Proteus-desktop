//! Modules overview: process components, overall switches and exact links to
//! the pages that own tool, hook and slot settings.

#[cfg(test)]
mod tests;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderPlugin, ConfigBuilderPluginExport, ConfigBuilderSnapshot, ConfigBuilderToolPack,
};

use super::SettingsView;
use super::spot::{self, Destination};
use super::tools::pack_key;
use super::widgets::{
    Tone, card, card_cell, chip, described, hashed_id, item_row, named, note, package_heading,
};
use crate::settings::catalog::{
    CONTEXT_PROVIDER_SLOT, HOOK_SLOT, MODEL_SLOT, PackState, PageId, TOOL_SLOT, export_selected,
    pack_summary, page_title, plural, slot_page, slot_title,
};
use crate::ui::kit;
use crate::ui::kit::Hint as _;
use crate::ui::kit::Pressable as _;
use crate::ui::tip::Tip as _;

fn module_key(id: &str) -> String {
    format!("module:{id}")
}

/// Every route to a Modules row unfolds its exact configured owner before
/// scrolling. Matching uses the same keys as the rendered rows, not ID parsing.
pub(super) fn disclosure_for_spot(snapshot: &ConfigBuilderSnapshot, key: &str) -> Option<String> {
    snapshot.plugins.iter().find_map(|plugin| {
        (key == spot::component(&plugin.id)
            || plugin
                .tool_packs
                .iter()
                .any(|pack| key == pack_key(&plugin.id, &pack.id))
            || plugin
                .exports
                .iter()
                .any(|export| key == spot::export(&export.slot, &export.id)))
        .then(|| module_key(&plugin.id))
    })
}

impl SettingsView {
    pub(super) fn modules_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        let mut parts: Vec<AnyElement> = snapshot
            .plugins
            .iter()
            .map(|component| self.component_card(component, cx))
            .collect();
        if parts.is_empty() {
            parts.push(card(
                [note("В профиле нет процессных модулей.", cx).into_any_element()],
                cx,
            ));
        }
        vec![v_flex().gap_2().children(parts).into_any_element()]
    }

    fn component_card(&self, plugin: &ConfigBuilderPlugin, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let key = module_key(&plugin.id);
        let hooks: Vec<_> = plugin
            .exports
            .iter()
            .filter(|export| export.slot == HOOK_SLOT)
            .collect();
        let modules: Vec<_> = plugin
            .exports
            .iter()
            .filter(|export| export.slot != HOOK_SLOT && export.slot != TOOL_SLOT)
            .collect();
        let mut counts = Vec::new();
        if !plugin.tool_packs.is_empty() {
            counts.push(plural(
                plugin.tool_packs.len(),
                "пакет инструментов",
                "пакета инструментов",
                "пакетов инструментов",
            ));
        }
        if !hooks.is_empty() {
            counts.push(plural(hooks.len(), "hook", "hooks", "hooks"));
        }
        if !modules.is_empty() {
            counts.push(plural(
                modules.len(),
                "реализация слота",
                "реализации слотов",
                "реализаций слотов",
            ));
        }
        let header = item_row(
            package_heading(
                Icon::new(Lucide::Boxes).text_color(theme.muted_foreground),
                described(
                    hashed_id("component", &plugin.id),
                    plugin.id.clone().into(),
                    plugin.description.clone().map(Into::into),
                    Some(plugin.command.clone().into()),
                    cx,
                ),
                counts
                    .into_iter()
                    .map(|text| chip(text, Tone::Muted, cx).into_any_element()),
            ),
            [],
        );
        let body = self.reveal(&key, cx, || {
            let mut rows = Vec::new();
            if plugin.exports.is_empty() && plugin.tool_packs.is_empty() {
                rows.push(note("Модуль не объявил exports.", cx).into_any_element());
            }
            if !plugin.tool_packs.is_empty() {
                rows.extend(
                    plugin
                        .tool_packs
                        .iter()
                        .map(|pack| self.pack_overview(plugin, pack, cx)),
                );
            }
            if !hooks.is_empty() {
                rows.extend(
                    hooks
                        .into_iter()
                        .map(|export| self.hook_export_row(export, cx)),
                );
            }
            if !modules.is_empty() {
                rows.extend(
                    modules
                        .into_iter()
                        .map(|export| self.module_export_row(export, cx)),
                );
            }
            v_flex()
                .children(rows.into_iter().map(|row| card_cell(row, true, cx)))
                .into_any_element()
        });
        let header = h_flex()
            .id(hashed_id("module-open", &plugin.id))
            .w_full()
            .min_w_0()
            .relative()
            .group("module-surface")
            .cursor_pointer()
            .pressable(cx)
            .child(kit::surface_highlight("module-surface", cx))
            .child(header.w_full().min_w_0())
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_open(&key, cx)));
        card(
            [v_flex()
                .child(self.spotted(&spot::component(&plugin.id), header, cx))
                .children(body)
                .into_any_element()],
            cx,
        )
    }

    fn pack_overview(
        &self,
        plugin: &ConfigBuilderPlugin,
        pack: &ConfigBuilderToolPack,
        cx: &Context<Self>,
    ) -> AnyElement {
        let snapshot = self.snapshot().expect("modules page needs a snapshot");
        let summary = pack_summary(snapshot, &self.draft, pack);
        let key = pack_key(&plugin.id, &pack.id);
        let switchable = summary.switchable.clone();
        let target = summary.state != PackState::On;
        let switch = kit::switch(hashed_id("pack-switch", &key))
            .checked(summary.state != PackState::Off)
            .disabled(switchable.is_empty())
            .hint(
                match summary.state {
                    PackState::On => "Выключить пакет",
                    PackState::Off => "Включить пакет",
                    PackState::Mixed => "Включить все",
                },
                cx,
            )
            .on_click(cx.listener(move |this, _: &bool, _, cx| {
                this.update_draft(cx, |draft| {
                    for name in &switchable {
                        draft.set_tool(name, target);
                    }
                });
            }));
        let mut trailing = vec![
            chip(
                format!("{} из {}", summary.enabled, summary.total),
                Tone::Muted,
                cx,
            )
            .into_any_element(),
        ];
        if summary.state == PackState::Mixed {
            trailing.push(chip("частично", Tone::Warning, cx).into_any_element());
        }
        trailing.push(
            self.settings_link(
                hashed_id("pack-settings", &key),
                "Открыть Tools",
                Destination::new(PageId::Tools)
                    .open(vec![format!("params:{TOOL_SLOT}:{}", pack.id)])
                    .spot(key.clone()),
                cx,
            ),
        );
        trailing.push(switch.into_any_element());
        let about = plugin
            .exports
            .iter()
            .find(|export| export.slot == TOOL_SLOT && export.id == pack.id)
            .and_then(|export| export.description.clone());
        let changed = pack
            .tools
            .iter()
            .any(|name| self.draft.tools.contains(name) != self.baseline.tools.contains(name));
        let title = h_flex()
            .min_w_0()
            .gap_2()
            .items_center()
            .child(type_icon(TOOL_SLOT, &key, cx))
            .child(named(
                hashed_id("pack-name", &key),
                pack.id.clone(),
                about.map(Into::into),
                None,
                cx,
            ))
            .when(changed, |line| line.child(super::widgets::changed_dot(cx)));
        self.spotted(&key, item_row(title, trailing), cx)
    }

    fn hook_export_row(
        &self,
        export: &ConfigBuilderPluginExport,
        cx: &Context<Self>,
    ) -> AnyElement {
        let position = self.draft.hooks.iter().position(|hook| hook == &export.id);
        let (id, on) = (export.id.clone(), position.is_none());
        let state = match position {
            Some(ix) => chip(format!("№ {} в цепочке", ix + 1), Tone::Success, cx),
            None => chip("выключен", Tone::Muted, cx),
        };
        let link = self.settings_link(
            hashed_id("hook-settings", &export.id),
            "Открыть Hooks",
            Destination::new(PageId::Hooks)
                .open(vec![format!("params:{HOOK_SLOT}:{}", export.id)])
                .spot(spot::hook(&export.id)),
            cx,
        );
        let switch = kit::switch(hashed_id("hook-switch", &export.id))
            .checked(position.is_some())
            .on_click(cx.listener(move |this, _: &bool, _, cx| {
                this.update_draft(cx, |draft| draft.set_hook(&id, on))
            }));
        self.spotted(
            &spot::export(HOOK_SLOT, &export.id),
            export_row(
                export,
                [state.into_any_element(), link, switch.into_any_element()],
                cx,
            ),
            cx,
        )
    }

    fn module_export_row(
        &self,
        export: &ConfigBuilderPluginExport,
        cx: &Context<Self>,
    ) -> AnyElement {
        let snapshot = self.snapshot().expect("modules page needs a snapshot");
        let mut trailing =
            vec![chip(slot_title(snapshot, &export.slot), Tone::Muted, cx).into_any_element()];
        if export_selected(snapshot, &self.draft, export) {
            trailing.push(chip("выбран", Tone::Success, cx).into_any_element());
        }
        let page = slot_page(&export.slot);
        let destination = if export.slot == MODEL_SLOT {
            let target = Destination::new(page.clone());
            // Model settings belong to provider connections, not a second
            // module form. Point at an actual matching connection when present.
            match snapshot
                .providers
                .iter()
                .find(|provider| provider.provider == export.id)
            {
                Some(provider) => target.spot(spot::provider(&provider.id)),
                None => target,
            }
        } else {
            let key = if export.slot == CONTEXT_PROVIDER_SLOT {
                spot::export(&export.slot, &export.id)
            } else {
                spot::module(&export.slot, &export.id)
            };
            Destination::new(page.clone()).spot(key)
        };
        trailing.push(self.settings_link(
            hashed_id("open-slot", &(&export.slot, &export.id)),
            format!("Открыть {}", page_title(Some(snapshot), &page)),
            destination,
            cx,
        ));
        self.spotted(
            &spot::export(&export.slot, &export.id),
            export_row(export, trailing, cx),
            cx,
        )
    }
}

fn type_icon(slot: &str, id: &str, cx: &App) -> Stateful<Div> {
    let title = match slot {
        TOOL_SLOT => "Пакет инструментов".to_string(),
        HOOK_SLOT => "Hook".to_string(),
        _ => format!("Реализация слота · {slot}"),
    };
    div()
        .id(hashed_id("module-kind", &(slot, id)))
        .flex_none()
        .text_color(cx.theme().muted_foreground)
        .tip_text(title)
        .child(super::nav::page_icon(&slot_page(slot)).small())
}

fn export_row(
    export: &ConfigBuilderPluginExport,
    trailing: impl IntoIterator<Item = AnyElement>,
    cx: &App,
) -> kit::SettingsRow {
    item_row(
        h_flex()
            .min_w_0()
            .gap_2()
            .items_center()
            .child(type_icon(&export.slot, &export.id, cx))
            .child(named(
                hashed_id("export", &(&export.slot, &export.id)),
                export.id.clone(),
                export.description.clone().map(Into::into),
                Some(format!("{}/{}", export.slot, export.id).into()),
                cx,
            )),
        trailing,
    )
}
