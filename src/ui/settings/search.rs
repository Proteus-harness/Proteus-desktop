//! Settings search over what the screen can show: pages, parameters of the
//! chosen modules, tools, modules, skills, connections and client rows. A
//! hit names its page, the disclosures on the way and the row to point at.

use gpui_kit::*;
use proteus_contracts::app_protocol::addons::AppAddonsSnapshot;
use proteus_contracts::app_protocol::config_builder::ConfigBuilderSnapshot;
use proteus_contracts::domain::{ConfigField, ConfigValueSchema};

use super::SettingsView;
use super::client::{APPEARANCE_ROWS, CHAT_ROWS};
use super::mcp::mcp_key;
use super::nav::nav_title;
use super::spot;
use super::tools::pack_key;
use crate::settings::FieldPath;
use crate::settings::addons;
use crate::settings::catalog::{
    CONTEXT_PROVIDER_SLOT, CONTEXT_SLOT, HOOK_SLOT, MODEL_SLOT, PageId, TOOL_SLOT, agent_pages,
    editable_field, exports, find_slot, missing_tools, module_info, page_description, page_title,
    selected_module,
};
use crate::shortcuts::Command;
use crate::ui::extensions;

/// More hits than fit the navigation column only scroll it.
const LIMIT: usize = 60;

pub(super) struct Hit {
    pub(super) title: String,
    pub(super) place: String,
    pub(super) target: spot::Destination,
}

impl Hit {
    fn new(title: impl Into<String>, place: impl Into<String>, page: PageId) -> Self {
        Self {
            title: title.into(),
            place: place.into(),
            target: spot::Destination::new(page),
        }
    }

    fn spot(mut self, key: String) -> Self {
        self.target.spot = Some(key);
        self
    }

    fn open(mut self, keys: Vec<String>) -> Self {
        self.target.open = keys;
        self
    }
}

/// Every word of the query must occur in the title or the other texts of a
/// hit. Exact titles come first, then titles starting with the first term,
/// then other title-only matches, then the rest, stably in screen order.
struct Finder {
    terms: Vec<String>,
    hits: Vec<(u8, Hit)>,
}

impl Finder {
    fn new(query: &str) -> Self {
        Self {
            terms: query.split_whitespace().map(str::to_lowercase).collect(),
            hits: Vec::new(),
        }
    }

    fn add(&mut self, hit: Hit, texts: &[&str]) {
        let title = hit.title.to_lowercase();
        let rest = texts.join("\n").to_lowercase();
        if !self
            .terms
            .iter()
            .all(|term| title.contains(term.as_str()) || rest.contains(term.as_str()))
        {
            return;
        }
        let in_title = self.terms.iter().all(|term| title.contains(term.as_str()));
        let rank = if !in_title {
            3
        } else if title
            .split_whitespace()
            .eq(self.terms.iter().map(String::as_str))
        {
            0
        } else if title.starts_with(self.terms[0].as_str()) {
            1
        } else {
            2
        };
        self.hits.push((rank, hit));
    }

    fn finish(mut self) -> Vec<Hit> {
        self.hits.sort_by_key(|(rank, _)| *rank);
        self.hits
            .into_iter()
            .take(LIMIT)
            .map(|(_, hit)| hit)
            .collect()
    }
}

/// Russian names of the pages titled in English.
fn page_words(page: &PageId) -> &'static str {
    match page {
        PageId::Configurations => {
            "config desktop визуальный агент профиль файл default умолчанию привязка переключение"
        }
        PageId::Providers => "подключение провайдер авторизация ключ API",
        PageId::Models => "модель effort reasoning набор имя preset субагент наследование",
        PageId::Modes => "режим поведения инструкции разрешения tools hooks наследование",
        PageId::Slots => "слот реализация workflow context policy model hooks storage",
        PageId::Slot(slot) if slot == crate::settings::catalog::SESSION_STORAGE_SLOT => {
            "session_storage session storage история чатов хранилище jsonl sqlite"
        }
        PageId::Slot(slot) if slot == "workflow" => "workflow",
        PageId::Slot(slot) if slot == "context" => "context",
        PageId::Slot(slot) if slot == "policy" => "policy",
        PageId::Tools => "инструменты",
        PageId::Hooks => "обработчики хуки",
        PageId::Modules => "модули компоненты плагины",
        PageId::Mcp => "серверы",
        PageId::Skills => "навыки",
        PageId::Packages => "пакеты плагины",
        PageId::History => "история изменения",
        _ => "",
    }
}

impl SettingsView {
    pub(super) fn search(&self, query: &str, cx: &App) -> Vec<Hit> {
        let mut finder = Finder::new(query);
        if finder.terms.is_empty() {
            return Vec::new();
        }
        let snapshot = self.snapshot();
        for page in self.page_list(cx) {
            let about = match &page {
                PageId::Extension(id) => extensions::find(id)
                    .map(|manifest| manifest.description.to_string())
                    .unwrap_or_default(),
                page => page_description(snapshot, page),
            };
            finder.add(
                Hit::new(nav_title(snapshot, &page), "Раздел", page.clone()),
                &[&about, page_words(&page)],
            );
        }
        if let Some(snapshot) = snapshot {
            self.search_profile(snapshot, &mut finder);
        }
        if let Some(addons) = self.addons() {
            self.search_addons(addons, &mut finder);
        }
        for (page, rows) in [
            (PageId::Appearance, APPEARANCE_ROWS.as_slice()),
            (PageId::Chat, CHAT_ROWS.as_slice()),
        ] {
            let place = page_title(None, &page);
            for row in rows {
                finder.add(
                    Hit::new(row.title, place.clone(), page.clone()).spot(spot::row(row.title)),
                    &[row.about.unwrap_or_default()],
                );
            }
        }
        let place = page_title(None, &PageId::Shortcuts);
        for command in Command::ALL {
            finder.add(
                Hit::new(command.title(), place.clone(), PageId::Shortcuts)
                    .spot(spot::row(command.title())),
                &["сочетание клавиш"],
            );
        }
        finder.finish()
    }

    fn search_profile(&self, snapshot: &ConfigBuilderSnapshot, finder: &mut Finder) {
        for (name, mode) in &self.draft.modes {
            let key = super::modes::mode_key(name);
            finder.add(
                Hit::new(
                    mode.title.as_deref().unwrap_or(name),
                    "Режимы",
                    PageId::Modes,
                )
                .open(vec![key.clone()])
                .spot(key),
                &[name, mode.instructions.as_deref().unwrap_or("")],
            );
        }
        for (name, model) in &self.draft.models {
            finder.add(
                Hit::new(name, "Модели · именованный набор", PageId::Models)
                    .open(vec![super::models::model_key(name)])
                    .spot(spot::model(name)),
                &[
                    &model.model,
                    &model.provider,
                    model.reasoning.effort.as_deref().unwrap_or(""),
                ],
            );
            for title in [
                "ID модели",
                "Effort",
                "Бюджет рассуждения",
                "Потоковый ответ",
                "Сводка рассуждений",
                "Доступные уровни effort",
            ] {
                finder.add(
                    Hit::new(title, format!("Модели · {name}"), PageId::Models)
                        .open(vec![super::models::model_key(name)])
                        .spot(spot::model(name)),
                    &[name, &model.model],
                );
            }
        }
        for role in &snapshot.model_roles {
            finder.add(
                Hit::new(&role.name, "Модели · субагент", PageId::Models)
                    .spot(spot::model_role(&role.name)),
                &[
                    &role.description,
                    role.model
                        .as_deref()
                        .unwrap_or("наследовать модель родителя"),
                ],
            );
        }
        for module in &snapshot.model_modules {
            finder.add(
                Hit::new(&module.id, "Провайдеры · подключение", PageId::Providers)
                    .spot(spot::module(MODEL_SLOT, &module.id)),
                &[module.description.as_deref().unwrap_or("")],
            );
            let place = format!("Провайдеры · {}", module.id);
            self.search_params(
                snapshot,
                finder,
                MODEL_SLOT,
                &module.id,
                &PageId::Providers,
                &place,
                &[],
            );
        }
        for page in agent_pages(Some(snapshot)) {
            let PageId::Slot(slot) = &page else {
                continue;
            };
            let name = page_title(Some(snapshot), &page);
            for module in find_slot(snapshot, slot)
                .map(|info| info.modules.as_slice())
                .unwrap_or_default()
            {
                finder.add(
                    Hit::new(&module.id, format!("{name} · реализация"), page.clone())
                        .spot(spot::module(slot, &module.id)),
                    &[module.description.as_deref().unwrap_or_default()],
                );
            }
            if let Some(module) = selected_module(snapshot, &self.draft, slot) {
                let place = format!("{name} · {module}");
                self.search_params(snapshot, finder, slot, module, &page, &place, &[]);
            }
            if slot != CONTEXT_SLOT {
                continue;
            }
            for (plugin, export) in exports(snapshot, CONTEXT_PROVIDER_SLOT) {
                finder.add(
                    Hit::new(
                        &export.id,
                        format!("{name} · поставщик контекста"),
                        page.clone(),
                    )
                    .spot(spot::export(CONTEXT_PROVIDER_SLOT, &export.id)),
                    &[
                        export.description.as_deref().unwrap_or_default(),
                        &plugin.id,
                    ],
                );
                let open = [format!("params:{CONTEXT_PROVIDER_SLOT}:{}", export.id)];
                let place = format!("{name} · {}", export.id);
                self.search_params(
                    snapshot,
                    finder,
                    CONTEXT_PROVIDER_SLOT,
                    &export.id,
                    &page,
                    &place,
                    &open,
                );
            }
        }
        search_tools(snapshot, &self.draft, finder);
        for module in &snapshot.hook_modules {
            finder.add(
                Hit::new(&module.id, "Hooks", PageId::Hooks).spot(spot::hook(&module.id)),
                &[module.description.as_deref().unwrap_or_default()],
            );
            let page = PageId::Hooks;
            let place = format!("{} · {}", page_title(None, &page), module.id);
            let open = [format!("params:{HOOK_SLOT}:{}", module.id)];
            self.search_params(
                snapshot, finder, HOOK_SLOT, &module.id, &page, &place, &open,
            );
        }
        for plugin in &snapshot.plugins {
            finder.add(
                Hit::new(&plugin.id, "Modules · модуль", PageId::Modules)
                    .spot(spot::component(&plugin.id)),
                &[
                    plugin.description.as_deref().unwrap_or_default(),
                    &plugin.command,
                ],
            );
            for pack in &plugin.tool_packs {
                let key = pack_key(&plugin.id, &pack.id);
                let about = plugin
                    .exports
                    .iter()
                    .find(|export| export.slot == TOOL_SLOT && export.id == pack.id)
                    .and_then(|export| export.description.as_deref())
                    .unwrap_or_default();
                finder.add(
                    Hit::new(
                        &pack.id,
                        format!("Modules · {} · пакет инструментов", plugin.id),
                        PageId::Modules,
                    )
                    .spot(key.clone()),
                    &[about],
                );
                let place = format!("Tools · {}", pack.id);
                let open = [format!("params:{TOOL_SLOT}:{}", pack.id)];
                self.search_params(
                    snapshot,
                    finder,
                    TOOL_SLOT,
                    &pack.id,
                    &PageId::Tools,
                    &place,
                    &open,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn search_params(
        &self,
        snapshot: &ConfigBuilderSnapshot,
        finder: &mut Finder,
        slot: &str,
        module: &str,
        page: &PageId,
        place: &str,
        open: &[String],
    ) {
        let Some(schema) = module_info(snapshot, slot, module).and_then(|info| info.schema) else {
            return;
        };
        let scope = Scope { slot, module, page };
        let first = finder.hits.len();
        scope.fields(finder, &schema.fields, None, place, open);
        if finder.hits.len() > first && self.is_json(slot, module) {
            let anchor = spot::params_json(slot, module);
            for (_, hit) in &mut finder.hits[first..] {
                hit.place.push_str(" · JSON");
                open.clone_into(&mut hit.target.open);
                hit.target.spot = Some(anchor.clone());
            }
        }
    }

    fn search_addons(&self, addons: &AppAddonsSnapshot, finder: &mut Finder) {
        let skills = page_title(None, &PageId::Skills);
        for (provider, skill) in addons::skills(addons) {
            finder.add(
                Hit::new(&skill.id, skills.clone(), PageId::Skills).spot(spot::skill(&skill.id)),
                &[&skill.name, &skill.description, provider],
            );
        }
        let mcp = page_title(None, &PageId::Mcp);
        for server in addons::mcp_servers(&self.draft, Some(addons)) {
            let command = server
                .configured
                .map(|(_, server)| server.command.as_str())
                .unwrap_or_default();
            finder.add(
                Hit::new(server.name, mcp.clone(), PageId::Mcp)
                    .open(vec![mcp_key(server.name)])
                    .spot(spot::mcp(server.name)),
                &[command, server.package.unwrap_or_default()],
            );
        }
        let packages = page_title(None, &PageId::Packages);
        for package in &self.draft.addons.addons.plugins {
            let state = addons::package_state(addons, &package.path);
            let path = package.path.to_string_lossy();
            let title = state
                .and_then(|state| state.name.clone())
                .unwrap_or_else(|| path.to_string());
            let about = state.and_then(|state| state.description.as_deref());
            finder.add(
                Hit::new(title, packages.clone(), PageId::Packages)
                    .spot(spot::package(&package.path)),
                &[about.unwrap_or_default(), &path],
            );
        }
    }
}

/// All editable tool rows, including stale enabled names, share Tools.
fn search_tools(
    snapshot: &ConfigBuilderSnapshot,
    draft: &crate::settings::Draft,
    finder: &mut Finder,
) {
    for tool in &snapshot.tools {
        let mut hit = Hit::new(&tool.name, "Tools · инструмент", PageId::Tools);
        hit.target = super::tools::tool_target(&tool.name);
        finder.add(hit, &[&tool.description, &tool.source]);
    }
    for name in missing_tools(snapshot, draft) {
        finder.add(
            Hit::new(name, "Tools · недоступный инструмент", PageId::Tools).spot(spot::tool(name)),
            &["недоступный не найден"],
        );
    }
}

/// Parameters of one module on one page.
struct Scope<'a> {
    slot: &'a str,
    module: &'a str,
    page: &'a PageId,
}

impl Scope<'_> {
    /// Fields of one object level; advanced ones and nested groups add their
    /// disclosures to the way.
    fn fields(
        &self,
        finder: &mut Finder,
        fields: &[ConfigField],
        parent: Option<&FieldPath>,
        place: &str,
        open: &[String],
    ) {
        let scope = crate::settings::form_scope(
            self.slot,
            self.module,
            parent.map_or(&[], |path| path.keys.as_slice()),
        );
        for field in fields.iter().filter(|field| editable_field(field)) {
            let path = match parent {
                None => FieldPath::new(self.slot, self.module, &field.key),
                Some(parent) => parent.child(&field.key),
            };
            let mut way = open.to_vec();
            if field.advanced {
                way.push(format!(
                    "advanced:{}",
                    crate::settings::section_scope(&scope, field)
                ));
            }
            finder.add(
                Hit::new(&field.title, place, self.page.clone())
                    .open(way.clone())
                    .spot(spot::field(&path)),
                &[
                    &field.key,
                    &field.description,
                    field
                        .section
                        .as_ref()
                        .map_or("", |section| section.title.as_str()),
                    field
                        .section
                        .as_ref()
                        .map_or("", |section| section.description.as_str()),
                ],
            );
            if let ConfigValueSchema::Object { fields: nested } = &field.value
                && !nested.is_empty()
            {
                way.push(format!(
                    "group:{}",
                    crate::settings::form_scope(self.slot, self.module, &path.keys)
                ));
                let place = format!("{place} · {}", field.title);
                self.fields(finder, nested, Some(&path), &place, &way);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::app_protocol::config_builder::{
        ConfigBuilderPlugin, ConfigBuilderTool, ConfigBuilderToolPack,
    };
    use proteus_contracts::contracts::ProcessToolOwner;

    #[::core::prelude::v1::test]
    fn exact_title_remains_visible_among_many_prefix_matches() {
        let mut finder = Finder::new("  ШРИФТ   ЧАТА  ");
        for i in 0..LIMIT {
            finder.add(Hit::new(format!("Шрифт чата {i}"), "", PageId::Chat), &[]);
        }
        finder.add(Hit::new("Шрифт чата", "", PageId::Chat), &[]);
        let hits = finder.finish();
        assert_eq!(hits.len(), LIMIT);
        assert_eq!(hits[0].title, "Шрифт чата");
        assert_eq!(hits[1].title, "Шрифт чата 0");
    }

    #[::core::prelude::v1::test]
    fn section_search_opens_the_correct_advanced_group_without_changing_field_path() {
        let field = ConfigField::new("timeout", "Таймаут", "", ConfigValueSchema::integer(1))
            .advanced()
            .section("network", "Сеть", "Параметры транспорта");
        let mut finder = Finder::new("транспорта");
        Scope {
            slot: "model",
            module: "example",
            page: &PageId::Providers,
        }
        .fields(&mut finder, &[field], None, "Провайдеры", &[]);
        let hits = finder.finish();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].target.open,
            ["advanced:model:example:section:network"]
        );
        assert_eq!(
            hits[0].target.spot,
            Some(spot::field(&FieldPath::new("model", "example", "timeout")))
        );
    }

    #[::core::prelude::v1::test]
    fn tool_search_targets_exact_rows_and_indexes_stale_names() {
        let snapshot = ConfigBuilderSnapshot {
            plugins: vec![ConfigBuilderPlugin {
                id: "process".into(),
                command: "host".into(),
                description: None,
                icon: None,
                exports: Vec::new(),
                tool_packs: vec![ConfigBuilderToolPack {
                    id: "planner".into(),
                    tools: vec!["update_plan".into(), "stale_owner".into()],
                }],
            }],
            tools: vec![
                ConfigBuilderTool {
                    name: "update_plan".into(),
                    source: "process".into(),
                    owner: Some(ProcessToolOwner {
                        component_id: "process".into(),
                        module_id: "planner".into(),
                    }),
                    safety: "ReadOnly".into(),
                    description: "Edit plan".into(),
                    enabled: true,
                    runtime_managed: false,
                    registered: true,
                },
                ConfigBuilderTool {
                    name: "stale_owner".into(),
                    source: "process".into(),
                    owner: Some(ProcessToolOwner {
                        component_id: "gone".into(),
                        module_id: "planner".into(),
                    }),
                    safety: "ReadOnly".into(),
                    description: String::new(),
                    enabled: true,
                    runtime_managed: false,
                    registered: true,
                },
            ],
            ..ConfigBuilderSnapshot::default()
        };
        let mut draft = crate::settings::Draft::default();
        draft.set_tool("missing_tool", true);
        let mut finder = Finder::new("plan");
        search_tools(&snapshot, &draft, &mut finder);
        let hits = finder.finish();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].target,
            spot::Destination::new(PageId::Tools).spot(spot::tool("update_plan"))
        );
        for name in ["stale_owner", "missing_tool"] {
            let mut finder = Finder::new(name);
            search_tools(&snapshot, &draft, &mut finder);
            let hits = finder.finish();
            assert_eq!(hits.len(), 1);
            assert_eq!(
                hits[0].target,
                spot::Destination::new(PageId::Tools).spot(spot::tool(name))
            );
        }
    }
}
