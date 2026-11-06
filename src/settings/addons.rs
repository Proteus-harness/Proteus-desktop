//! Skills, MCP servers and Agent Plugins packages of the profile: edits of
//! the draft and lookups in the server's addons snapshot.

use std::path::{Path, PathBuf};

use proteus_contracts::app_protocol::addons::{
    AppAddonsSnapshot, AppAddonsUpdate, AppAgentPluginState, AppMcpServerState,
};
use proteus_contracts::app_protocol::config_builder::ConfigBuilderTool;
use proteus_contracts::domain::{AgentPluginConfig, ConfiguredMcpServerConfig, SkillDescriptor};

use super::draft::Draft;

/// Disabled lists are sets: kept sorted so switching an entry back and forth
/// leaves the draft equal to its baseline.
pub fn normalized(mut addons: AppAddonsUpdate) -> AppAddonsUpdate {
    for list in [
        &mut addons.addons.disabled_skills,
        &mut addons.addons.disabled_mcp_servers,
    ] {
        list.sort();
        list.dedup();
    }
    addons
}

pub(super) fn set_listed(list: &mut Vec<String>, item: &str, listed: bool) {
    match (
        list.binary_search_by(|entry| entry.as_str().cmp(item)),
        listed,
    ) {
        (Err(ix), true) => list.insert(ix, item.to_string()),
        (Ok(ix), false) => {
            list.remove(ix);
        }
        _ => {}
    }
}

impl Draft {
    pub fn skill_enabled(&self, id: &str) -> bool {
        !self
            .addons
            .addons
            .disabled_skills
            .iter()
            .any(|skill| skill == id)
    }

    pub fn set_skill(&mut self, id: &str, enabled: bool) {
        set_listed(&mut self.addons.addons.disabled_skills, id, !enabled);
    }

    /// A server runs unless its profile entry or `disabled_mcp_servers`
    /// switches it off.
    pub fn mcp_enabled(&self, name: &str) -> bool {
        !self
            .addons
            .addons
            .disabled_mcp_servers
            .iter()
            .any(|server| server == name)
            && self
                .configured_mcp(name)
                .is_none_or(|server| server.enabled)
    }

    /// Switches a server of any origin through `disabled_mcp_servers`;
    /// switching on a profile server also clears its own `enabled = false`.
    pub fn set_mcp(&mut self, name: &str, enabled: bool) {
        set_listed(&mut self.addons.addons.disabled_mcp_servers, name, !enabled);
        if enabled
            && let Some(server) = self
                .addons
                .mcp_servers
                .iter_mut()
                .find(|server| server.name == name)
        {
            server.enabled = true;
        }
    }

    pub fn configured_mcp(&self, name: &str) -> Option<&ConfiguredMcpServerConfig> {
        self.addons
            .mcp_servers
            .iter()
            .find(|server| server.name == name)
    }

    pub fn set_package(&mut self, path: &Path, enabled: bool) {
        for plugin in &mut self.addons.addons.plugins {
            if plugin.path == path {
                plugin.enabled = enabled;
            }
        }
    }

    /// Lists a package folder, or switches on the entry that already
    /// lists it.
    pub fn add_package(&mut self, path: PathBuf) {
        let plugins = &mut self.addons.addons.plugins;
        match plugins.iter_mut().find(|plugin| plugin.path == path) {
            Some(plugin) => plugin.enabled = true,
            None => plugins.push(AgentPluginConfig {
                path,
                enabled: true,
                data_key: None,
            }),
        }
    }

    pub fn remove_package(&mut self, path: &Path) {
        self.addons
            .addons
            .plugins
            .retain(|plugin| plugin.path != path);
    }
}

/// MCP server known to the profile or to the last assembly.
pub struct McpServer<'a> {
    pub name: &'a str,
    /// Discovery result of the saved assembly.
    pub state: Option<&'a AppMcpServerState>,
    /// Position and entry in the draft's own `tools.mcp_servers`.
    pub configured: Option<(usize, &'a ConfiguredMcpServerConfig)>,
    /// Package that brings the server, by its `package:server` name.
    pub package: Option<&'a str>,
}

/// Profile servers of the draft in their order, then the package servers
/// of the last assembly in its order.
pub fn mcp_servers<'a>(
    draft: &'a Draft,
    addons: Option<&'a AppAddonsSnapshot>,
) -> Vec<McpServer<'a>> {
    let saved = |name: &str| {
        addons.and_then(|addons| {
            addons
                .settings
                .mcp_servers
                .iter()
                .any(|server| server.name == name)
                .then(|| addons.mcp_servers.iter().find(|state| state.name == name))
                .flatten()
        })
    };
    let mut servers: Vec<McpServer<'a>> = draft
        .addons
        .mcp_servers
        .iter()
        .enumerate()
        .map(|(ix, server)| McpServer {
            name: &server.name,
            state: saved(&server.name),
            configured: Some((ix, server)),
            package: None,
        })
        .collect();
    if let Some(addons) = addons {
        servers.extend(package_servers(addons).map(|state| McpServer {
            name: &state.name,
            state: Some(state),
            configured: None,
            package: package_of(addons, &state.name),
        }));
    }
    servers
}

/// Servers of the last assembly that the saved profile does not list.
pub fn package_servers(addons: &AppAddonsSnapshot) -> impl Iterator<Item = &AppMcpServerState> {
    addons.mcp_servers.iter().filter(|state| {
        !addons
            .settings
            .mcp_servers
            .iter()
            .any(|server| server.name == state.name)
    })
}

/// Name of the loaded package whose `package:item` prefix `name` carries.
pub fn package_of<'a>(addons: &'a AppAddonsSnapshot, name: &str) -> Option<&'a str> {
    let (prefix, _) = name.split_once(':')?;
    addons
        .plugins
        .iter()
        .filter_map(|plugin| plugin.name.as_deref())
        .find(|package| *package == prefix)
}

/// MCP server that registered a tool in the last assembly.
pub fn tool_server<'a>(addons: &'a AppAddonsSnapshot, tool: &str) -> Option<&'a str> {
    addons
        .mcp_servers
        .iter()
        .find(|server| server.tools.iter().any(|name| name == tool))
        .map(|server| server.name.as_str())
}

/// Origin reported by the assembly, never inferred from a tool name prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolOrigin<'a> {
    Module {
        component: &'a str,
        module: &'a str,
    },
    Mcp {
        package: Option<&'a str>,
        server: &'a str,
    },
    Package(&'a str),
    Source(&'a str),
}

pub fn tool_origin<'a>(
    tool: &'a ConfigBuilderTool,
    addons: Option<&'a AppAddonsSnapshot>,
) -> ToolOrigin<'a> {
    if let Some(owner) = &tool.owner {
        return ToolOrigin::Module {
            component: &owner.component_id,
            module: &owner.module_id,
        };
    }
    if let Some(addons) = addons {
        if let Some(server) = tool_server(addons, &tool.name) {
            return ToolOrigin::Mcp {
                package: package_of(addons, server),
                server,
            };
        }
        if let Some(package) = package_of(addons, &tool.name) {
            return ToolOrigin::Package(package);
        }
    }
    ToolOrigin::Source(&tool.source)
}

#[cfg(test)]
mod tool_origin_tests {
    use super::*;
    use proteus_contracts::contracts::ProcessToolOwner;

    fn snapshot() -> AppAddonsSnapshot {
        AppAddonsSnapshot {
            reload_error: None,
            writable: true,
            settings: AppAddonsUpdate::default(),
            catalogs: Vec::new(),
            mcp_servers: vec![AppMcpServerState {
                name: "package:server".into(),
                enabled: true,
                tools: vec!["ordinary_name".into()],
                error: None,
            }],
            plugins: vec![AppAgentPluginState {
                path: "package".into(),
                name: Some("package".into()),
                version: None,
                description: None,
                enabled: true,
                warnings: Vec::new(),
                error: None,
            }],
        }
    }

    #[test]
    fn exact_mcp_membership_and_loaded_packages_supply_provenance() {
        let addons = snapshot();
        let mut tool = ConfigBuilderTool {
            name: "ordinary_name".into(),
            source: "opaque".into(),
            ..Default::default()
        };
        assert_eq!(
            tool_origin(&tool, Some(&addons)),
            ToolOrigin::Mcp {
                package: Some("package"),
                server: "package:server",
            }
        );
        tool.owner = Some(ProcessToolOwner {
            component_id: "component".into(),
            module_id: "module".into(),
        });
        assert_eq!(
            tool_origin(&tool, Some(&addons)),
            ToolOrigin::Module {
                component: "component",
                module: "module",
            }
        );
        tool.owner = None;
        tool.name = "package:action".into();
        assert_eq!(
            tool_origin(&tool, Some(&addons)),
            ToolOrigin::Package("package")
        );
        tool.name = "unknown:action".into();
        assert_eq!(
            tool_origin(&tool, Some(&addons)),
            ToolOrigin::Source("opaque")
        );
    }

    #[test]
    fn familiar_tool_prefixes_do_not_invent_origins() {
        for name in ["mcp__server__call", "package:action", "builtin_read"] {
            let tool = ConfigBuilderTool {
                name: name.into(),
                source: "declared-source".into(),
                ..Default::default()
            };
            assert_eq!(
                tool_origin(&tool, None),
                ToolOrigin::Source("declared-source")
            );
        }
    }
}

/// Every skill of the providers' catalogs with the provider that found it.
pub fn skills(addons: &AppAddonsSnapshot) -> Vec<(&str, &SkillDescriptor)> {
    addons
        .catalogs
        .iter()
        .filter_map(|catalog| Some((catalog.provider.as_str(), catalog.catalog.as_ref()?)))
        .flat_map(|(provider, catalog)| catalog.skills.iter().map(move |skill| (provider, skill)))
        .collect()
}

/// Disabled skill ids that no catalog lists. Unknown while a provider
/// failed, since its skills are not visible.
pub fn missing_skills<'a>(draft: &'a Draft, addons: &AppAddonsSnapshot) -> Vec<&'a str> {
    if addons
        .catalogs
        .iter()
        .any(|catalog| catalog.error.is_some())
    {
        return Vec::new();
    }
    let known = skills(addons);
    draft
        .addons
        .addons
        .disabled_skills
        .iter()
        .filter(|id| !known.iter().any(|(_, skill)| &skill.id == *id))
        .map(String::as_str)
        .collect()
}

/// Load state of a package of the saved profile, matched by its entry.
pub fn package_state<'a>(
    addons: &'a AppAddonsSnapshot,
    path: &Path,
) -> Option<&'a AppAgentPluginState> {
    let ix = addons
        .settings
        .addons
        .plugins
        .iter()
        .position(|plugin| plugin.path == path)?;
    addons.plugins.get(ix)
}

/// Skills and MCP servers a loaded package contributes.
pub fn package_contents(addons: &AppAddonsSnapshot, package: &str) -> (usize, usize) {
    let owned = |name: &str| {
        name.split_once(':')
            .is_some_and(|(prefix, _)| prefix == package)
    };
    (
        skills(addons)
            .iter()
            .filter(|(_, skill)| owned(&skill.id))
            .count(),
        addons
            .mcp_servers
            .iter()
            .filter(|server| owned(&server.name))
            .count(),
    )
}
