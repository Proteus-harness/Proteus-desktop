// Адаптировано из Zed: crates/settings_ui/src/pages/tool_permissions_setup.rs
// (open tool rows within the page's scroll container).
//! Canonical Tools controls, pack parameters and residual origin groups.

use std::collections::BTreeMap;

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderPlugin, ConfigBuilderTool, ConfigBuilderToolPack,
};

use super::SettingsView;
use super::mcp::mcp_key;
use super::spot::Destination;
use super::widgets::{Tone, card, changed_dot, chip, hashed_id, item_row, named, note, section};
use crate::settings::addons::{ToolOrigin, package_state, tool_origin, tool_server};
use crate::settings::catalog::{PageId, find_tool, missing_tools, tool_matches, tool_pack};
use crate::ui::kit;
use crate::ui::kit::Hint as _;
use crate::ui::prefs;

pub(super) fn pack_key(component: &str, module: &str) -> String {
    format!("pack:{component}:{module}")
}

impl SettingsView {
    pub(super) fn tools_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        let enabled = snapshot
            .tools
            .iter()
            .filter(|tool| tool.runtime_managed || self.draft.tools.contains(&tool.name))
            .count();
        let mut bodies = Vec::new();
        for plugin in &snapshot.plugins {
            for pack in &plugin.tool_packs {
                if self.tool_query.is_empty()
                    || pack.tools.iter().any(|name| {
                        find_tool(snapshot, name)
                            .is_some_and(|tool| tool_matches(tool, &self.tool_query))
                            && tool_pack(snapshot, name).is_some_and(|(owner, group)| {
                                owner.id == plugin.id && group.id == pack.id
                            })
                    })
                {
                    bodies.push(self.tool_pack_group(plugin, pack, cx));
                }
            }
        }
        bodies.extend(self.tool_groups(cx));
        if bodies.is_empty() {
            bodies.push(note("Ничего не найдено.", cx).into_any_element());
        }
        let search = crate::ui::search::field(&self.tool_search)
            .small()
            .cleanable(true)
            .w(prefs::scaled(260.));
        vec![
            section(
                format!(
                    "Разрешённые инструменты · {enabled} из {}",
                    snapshot.tools.len()
                ),
                None,
                Some(search.into_any_element()),
                v_flex().gap_4().children(bodies),
                cx,
            )
            .into_any_element(),
        ]
    }

    fn tool_pack_group(
        &self,
        plugin: &ConfigBuilderPlugin,
        pack: &ConfigBuilderToolPack,
        cx: &Context<Self>,
    ) -> AnyElement {
        let snapshot = self.snapshot().expect("tools page needs a snapshot");
        let key = pack_key(&plugin.id, &pack.id);
        let source = self.settings_link(
            hashed_id("pack-source", &key),
            "Открыть модуль",
            Destination::new(PageId::Modules).spot(key.clone()),
            cx,
        );
        let export = plugin
            .exports
            .iter()
            .find(|export| export.slot == "tool" && export.id == pack.id);
        let changed = self.draft.module_config("tool", &pack.id)
            != self.baseline.module_config("tool", &pack.id);
        let title = h_flex()
            .min_w_0()
            .gap_2()
            .items_center()
            .child(named(
                hashed_id("tool-pack-title", &key),
                format!("{} / {}", plugin.id, pack.id),
                export
                    .and_then(|export| export.description.clone())
                    .map(Into::into),
                None,
                cx,
            ))
            .when(changed, |line| line.child(changed_dot(cx)));
        let mut rows = vec![self.spotted(&key, item_row(title, [source]), cx)];
        let mut shown = std::collections::HashSet::new();
        for name in &pack.tools {
            if !shown.insert(name) {
                continue;
            }
            if let Some(tool) = find_tool(snapshot, name) {
                if tool_pack(snapshot, name)
                    .is_some_and(|(owner, group)| owner.id == plugin.id && group.id == pack.id)
                    && tool_matches(tool, &self.tool_query)
                {
                    rows.push(self.tool_row(tool, cx));
                }
            } else if self.tool_query.is_empty() {
                rows.push(
                    item_row(
                        named(
                            hashed_id("pack-missing-name", &(&plugin.id, &pack.id, name)),
                            name.clone(),
                            None,
                            None,
                            cx,
                        ),
                        [chip("нет в списке инструментов", Tone::Warning, cx).into_any_element()],
                    )
                    .into_any_element(),
                );
            }
        }
        if pack.tools.is_empty() {
            rows.push(note("Пакет не сообщил инструментов.", cx).into_any_element());
        }
        let params_key = format!("params:tool:{}", pack.id);
        rows.extend(self.params_disclosure(
            "tool",
            &pack.id,
            &params_key,
            self.is_open(&params_key),
            cx,
        ));
        card(rows, cx)
    }

    /// Tools not actually placed in a process pack keep their reported origin.
    pub(super) fn tool_groups(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        let mut groups = BTreeMap::<ToolOrigin<'_>, Vec<&ConfigBuilderTool>>::new();
        for tool in snapshot.tools.iter().filter(|tool| {
            tool_pack(snapshot, &tool.name).is_none() && tool_matches(tool, &self.tool_query)
        }) {
            groups
                .entry(tool_origin(tool, self.addons()))
                .or_default()
                .push(tool);
        }
        let mut bodies: Vec<AnyElement> = groups
            .into_iter()
            .map(|(origin, tools)| {
                section(
                    origin_title(origin),
                    None,
                    self.tool_source_action(origin, cx),
                    card(tools.into_iter().map(|tool| self.tool_row(tool, cx)), cx),
                    cx,
                )
                .into_any_element()
            })
            .collect();
        let query = self.tool_query.to_lowercase();
        let missing: Vec<_> = missing_tools(snapshot, &self.draft)
            .into_iter()
            .filter(|name| name.to_lowercase().contains(&query))
            .collect();
        if !missing.is_empty() {
            bodies.push(
                section(
                    "Недоступные инструменты",
                    None,
                    None,
                    card(
                        missing.iter().map(|name| self.missing_tool_row(name, cx)),
                        cx,
                    ),
                    cx,
                )
                .into_any_element(),
            );
        }
        bodies
    }

    pub(super) fn tool_destination(&self, name: &str, _cx: &App) -> Option<Destination> {
        let snapshot = self.snapshot()?;
        (find_tool(snapshot, name).is_some() || self.draft.tools.contains(name))
            .then(|| tool_target(name))
    }

    /// Source links use the same exact row navigation as settings search.
    fn tool_source_action(&self, origin: ToolOrigin<'_>, cx: &Context<Self>) -> Option<AnyElement> {
        let target = match origin {
            ToolOrigin::Module { component, module } => {
                let plugin = self
                    .snapshot()?
                    .plugins
                    .iter()
                    .find(|plugin| plugin.id == component)?;
                let key = pack_key(component, module);
                if plugin.tool_packs.iter().any(|pack| pack.id == module) {
                    Destination::new(PageId::Modules).spot(key)
                } else {
                    Destination::new(PageId::Modules).spot(super::spot::component(component))
                }
            }
            ToolOrigin::Mcp { server, .. } => Destination::new(PageId::Mcp)
                .open(vec![mcp_key(server)])
                .spot(super::spot::mcp(server)),
            ToolOrigin::Package(package) => {
                let addons = self.addons()?;
                let entry = self.draft.addons.addons.plugins.iter().find(|entry| {
                    package_state(addons, &entry.path).and_then(|state| state.name.as_deref())
                        == Some(package)
                        || entry.path.to_string_lossy() == package
                })?;
                Destination::new(PageId::Packages).spot(super::spot::package(&entry.path))
            }
            ToolOrigin::Source(_) => return None,
        };
        Some(self.settings_link(
            hashed_id("tool-source", &origin_title(origin)),
            "Открыть источник",
            target,
            cx,
        ))
    }

    pub(super) fn tool_row(&self, tool: &ConfigBuilderTool, cx: &Context<Self>) -> AnyElement {
        let on = tool.runtime_managed || self.draft.tools.contains(&tool.name);
        let changed = !tool.runtime_managed
            && self.draft.tools.contains(&tool.name) != self.baseline.tools.contains(&tool.name);
        let mut chips = vec![chip(
            tool.safety.clone(),
            if tool.safety == "Dangerous" {
                Tone::Danger
            } else {
                Tone::Muted
            },
            cx,
        )];
        let server = self
            .addons()
            .and_then(|addons| tool_server(addons, &tool.name));
        if tool.runtime_managed && server.is_none() {
            chips.push(chip("управляется runtime", Tone::Accent, cx));
        }
        if !tool.registered {
            chips.push(chip("не зарегистрирован", Tone::Warning, cx));
        }
        let name = tool.name.clone();
        let switch = kit::switch(hashed_id("tool-switch", &tool.name))
            .checked(on)
            .disabled(tool.runtime_managed)
            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                let checked = *checked;
                this.update_draft(cx, |draft| draft.set_tool(&name, checked));
            }));
        let title = h_flex()
            .min_w_0()
            .gap_2()
            .items_center()
            .child(named(
                hashed_id("tool-name", &tool.name),
                tool.name.clone(),
                Some(tool.description.clone().into()),
                Some(format!("Источник: {}", tool.source).into()),
                cx,
            ))
            .when(changed, |line| line.child(changed_dot(cx)));
        let trailing = chips
            .into_iter()
            .map(IntoElement::into_any_element)
            .chain([switch.into_any_element()]);
        self.spotted(
            &super::spot::tool(&tool.name),
            item_row(title, trailing).when(!tool.registered, |row| row.opacity(0.65)),
            cx,
        )
    }

    fn missing_tool_row(&self, name: &str, cx: &Context<Self>) -> AnyElement {
        let target = name.to_string();
        let switch = kit::switch(hashed_id("missing-tool", &name))
            .checked(true)
            .hint("Убрать из разрешённых", cx)
            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                let checked = *checked;
                this.update_draft(cx, |draft| draft.set_tool(&target, checked));
            }));
        self.spotted(
            &super::spot::tool(name),
            item_row(
                named(
                    hashed_id("missing-tool-name", &name),
                    name.to_string(),
                    None,
                    None,
                    cx,
                ),
                [
                    chip("Не найден в текущей сборке", Tone::Danger, cx).into_any_element(),
                    switch.into_any_element(),
                ],
            ),
            cx,
        )
    }
}

/// The canonical row and the disclosures needed to mount it.
pub(super) fn tool_target(name: &str) -> Destination {
    // Pack tool rows are always visible; only their parameters fold.
    Destination::new(PageId::Tools).spot(super::spot::tool(name))
}

fn origin_title(origin: ToolOrigin<'_>) -> String {
    match origin {
        ToolOrigin::Module { component, module } => format!("Модуль · {component} / {module}"),
        ToolOrigin::Mcp {
            package: Some(package),
            server,
        } => format!("MCP · {server} · пакет {package}"),
        ToolOrigin::Mcp {
            package: None,
            server,
        } => format!("MCP · {server}"),
        ToolOrigin::Package(package) => format!("Пакет · {package}"),
        ToolOrigin::Source("") => "Источник не указан".to_string(),
        ToolOrigin::Source(source) => format!("Источник · {source}"),
    }
}
