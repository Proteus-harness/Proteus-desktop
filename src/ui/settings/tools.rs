// Адаптировано из Zed: crates/agent_ui/src/conversation_view/thread_view.rs
// (bounded previews); uses the existing chat/settings disclosure primitives.
//! Tools page: which tools the profile allows.

use std::collections::BTreeMap;

use crate::ui::kit::ButtonLabel as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use proteus_contracts::app_protocol::config_builder::ConfigBuilderTool;

use super::SettingsView;
use super::mcp::mcp_key;
use super::widgets::{
    Tone, card_cell, card_with_tail, changed_dot, chip, disclosure, hashed_id, item_row, named,
    note, section,
};
use crate::settings::addons::{ToolOrigin, tool_origin, tool_server};
use crate::settings::catalog::{PageId, missing_tools, tool_matches};
use crate::ui::kit;
use crate::ui::kit::Hint as _;
use crate::ui::{motion, prefs};

/// As in settings text previews, show a useful prefix without another scroller.
const PREVIEW_TOOLS: usize = 8;

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
        let mut groups = BTreeMap::<ToolOrigin<'_>, Vec<&ConfigBuilderTool>>::new();
        for tool in snapshot
            .tools
            .iter()
            .filter(|tool| tool_matches(tool, &self.tool_query))
        {
            groups
                .entry(tool_origin(tool, self.addons()))
                .or_default()
                .push(tool);
        }
        let missing: Vec<_> = missing_tools(snapshot, &self.draft)
            .into_iter()
            .filter(|name| name.contains(self.tool_query.as_str()))
            .collect();
        let mut bodies: Vec<AnyElement> = groups
            .into_iter()
            .map(|(origin, tools)| {
                section(
                    origin_title(origin),
                    None,
                    self.tool_source_action(origin, cx),
                    self.tool_group(
                        &format!("tool-group:{origin:?}"),
                        tools.len(),
                        |ix| self.tool_row(tools[ix], cx),
                        cx,
                    ),
                    cx,
                )
                .into_any_element()
            })
            .collect();
        if !missing.is_empty() {
            bodies.push(
                section(
                    "Недоступные инструменты",
                    None,
                    None,
                    self.tool_group(
                        "tool-group:missing",
                        missing.len(),
                        |ix| self.missing_tool_row(missing[ix], cx),
                        cx,
                    ),
                    cx,
                )
                .into_any_element(),
            );
        }
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

    /// A long source uses the same disclosure and height animation as chat
    /// blocks. Only the tail mounts on expansion; visible switches retain
    /// their ancestry, IDs and focus rather than appearing in two branches.
    fn tool_group(
        &self,
        key: &str,
        count: usize,
        row: impl Fn(usize) -> AnyElement,
        cx: &Context<Self>,
    ) -> AnyElement {
        // Search hits must never land in an unmounted tail. Searching this
        // page also shows every matching result immediately.
        let searching = !self.tool_query.is_empty() || self.spot.is_some();
        let prefix = if searching {
            count
        } else {
            count.min(PREVIEW_TOOLS)
        };
        let long = count > prefix;
        let open = self.is_open(key);
        let body = card_with_tail(
            (0..prefix).map(&row),
            || {
                long.then(|| {
                    motion::reveal(super::reveal_key(key), open, cx, || {
                        v_flex()
                            .children((prefix..count).map(|ix| card_cell(row(ix), true, cx)))
                            .into_any_element()
                    })
                })
                .flatten()
            },
            cx,
        );
        if !long {
            return body;
        }
        let view = cx.weak_entity();
        let key = key.to_owned();
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .child(disclosure(
                hashed_id("tool-group-toggle", &key),
                open,
                if open {
                    "Свернуть".to_owned()
                } else {
                    format!("Показать больше · ещё: {}", count - prefix)
                },
                false,
                move |cx| {
                    let _ = view.update(cx, |this, cx| this.toggle_open(&key, cx));
                },
                cx,
            ))
            .child(body)
            .into_any_element()
    }

    /// One source shortcut belongs to the group, rather than every tool switch.
    fn tool_source_action(&self, origin: ToolOrigin<'_>, cx: &Context<Self>) -> Option<AnyElement> {
        let (key, page) = match origin {
            ToolOrigin::Module { component, module } => {
                (pack_key(component, module), PageId::Modules)
            }
            ToolOrigin::Mcp { server, .. } => (mcp_key(server), PageId::Mcp),
            _ => return None,
        };
        let title = format!("Открыть источник: {}", origin_title(origin));
        let view = cx.weak_entity();
        Some(
            kit::icon_button(hashed_id("tool-source", &key))
                .icon(IconName::ExternalLink)
                .quiet(cx)
                .accessibility_label(title.clone())
                .hint(&title, cx)
                .on_click(move |_, window, cx| {
                    let (key, page) = (key.clone(), page.clone());
                    let _ = view.update(cx, |this, cx| {
                        this.open.insert(key);
                        this.navigate_to(page, window, cx);
                    });
                })
                .into_any_element(),
        )
    }

    fn tool_row(&self, tool: &ConfigBuilderTool, cx: &Context<Self>) -> AnyElement {
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
        )
        .into_any_element()
    }
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
