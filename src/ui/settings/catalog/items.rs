//! Entries of every loaded catalog with their install state, filtered by the
//! search and the toolbar.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::Icon;

use super::{SettingsView, Show};
use crate::catalog::{
    Installed, Source, install,
    schema::{Catalog, Entry, Kind},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Available,
    /// A package exists but its metadata or required files are broken.
    Broken,
    /// Files and receipt exist, the profile no longer references them.
    Detached,
    Update,
    Installed,
}

pub(super) struct Item<'a> {
    pub source: &'a Source,
    pub catalog: &'a Catalog,
    pub entry: &'a Entry,
    pub installed: Option<&'a Installed>,
    pub present: Option<&'a crate::catalog::inventory::Package>,
    pub status: Status,
}

impl Item<'_> {
    pub fn key(&self) -> (&Source, &str) {
        (self.source, &self.entry.id)
    }

    pub fn category(&self) -> Option<&str> {
        self.catalog
            .categories
            .iter()
            .find(|category| category.id == self.entry.category)
            .map(|category| category.title.as_str())
    }

    fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || [
                self.entry.id.as_str(),
                &self.entry.title,
                &self.entry.description,
                self.entry.kind.title(),
                self.category().unwrap_or_default(),
                &self.catalog.title,
            ]
            .iter()
            .any(|text| text.to_lowercase().contains(query))
    }
}

pub(super) const KINDS: [Kind; 5] = [
    Kind::ProcessModule,
    Kind::ClientExtension,
    Kind::AgentPlugin,
    Kind::Skill,
    Kind::McpServer,
];

pub(super) fn kind_icon(kind: Kind) -> Icon {
    Icon::new(match kind {
        Kind::ProcessModule => Lucide::Boxes,
        Kind::ClientExtension => Lucide::Blocks,
        Kind::AgentPlugin => Lucide::Puzzle,
        Kind::Skill => Lucide::Sparkles,
        Kind::McpServer => Lucide::Server,
    })
}

/// Version as people read it: a git tree id of a skill gets cut to the
/// usual seven characters.
pub(super) fn version_label(version: &str) -> String {
    if version.len() == 40 && version.bytes().all(|c| c.is_ascii_hexdigit()) {
        version[..7].to_string()
    } else {
        version.to_string()
    }
}

/// Short plural for the type filter.
pub(super) fn kind_filter(kind: Kind) -> &'static str {
    match kind {
        Kind::ProcessModule => "Модули",
        Kind::ClientExtension => "Extensions",
        Kind::AgentPlugin => "Agent Plugins",
        Kind::Skill => "Skills",
        Kind::McpServer => "MCP",
    }
}

impl SettingsView {
    /// Every entry of the loaded catalogs; `installed` are the receipts.
    pub(super) fn catalog_items<'a>(&'a self, installed: &'a [Installed]) -> Vec<Item<'a>> {
        let profile = self
            .snapshot()
            .and_then(|snapshot| snapshot.target_path.as_deref())
            .map(std::path::Path::new)
            .unwrap_or(std::path::Path::new(""));
        self.catalogs
            .loaded
            .iter()
            .flat_map(|(source, catalog)| {
                catalog.entries.iter().map(move |entry| {
                    let present = self
                        .catalogs
                        .inventory
                        .find(source, entry.kind, &entry.id)
                        .filter(|package| !installed_elsewhere(installed, entry, package));
                    let installed = install::receipt(installed, source, entry, profile);
                    let status = match installed {
                        None => presence_status(present, &entry.version),
                        Some(_)
                            if entry.kind == Kind::ClientExtension
                                && present.is_some_and(|package| package.problem.is_some()) =>
                        {
                            Status::Broken
                        }
                        Some(old) if !install::connected(old, &self.baseline) => Status::Detached,
                        Some(old) if !self.catalog_module_connected(old) => Status::Detached,
                        Some(old) if old.version != entry.version => Status::Update,
                        Some(_) => Status::Installed,
                    };
                    Item {
                        source,
                        catalog,
                        entry,
                        installed,
                        present,
                        status,
                    }
                })
            })
            .collect()
    }

    fn catalog_module_connected(&self, installed: &Installed) -> bool {
        let crate::catalog::Target::Component { command, .. } = &installed.target else {
            return true;
        };
        command.first().is_some_and(|command| {
            self.snapshot().is_some_and(|snapshot| {
                snapshot
                    .plugins
                    .iter()
                    .any(|plugin| plugin.command == *command)
            })
        })
    }

    /// Whether the toolbar and the search leave the entry visible.
    pub(super) fn catalog_item_shown(&self, item: &Item) -> bool {
        (match self.catalogs.show {
            Show::Overview | Show::All => true,
            Show::Installed => matches!(
                item.status,
                Status::Installed | Status::Update | Status::Broken
            ),
            Show::Updates => item.status == Status::Update,
        }) && self
            .catalogs
            .kind
            .is_none_or(|kind| item.entry.kind == kind)
            && self.catalogs.scope.as_ref().is_none_or(|scope| {
                *item.source == scope.source
                    && scope
                        .category
                        .as_ref()
                        .is_none_or(|category| item.entry.category == *category)
            })
            && item.matches(self.catalogs.query.trim())
    }

    /// The overview stands in for the list until something narrows it.
    pub(super) fn catalog_browsing(&self) -> bool {
        self.catalogs.show != Show::Overview
            || self.catalogs.kind.is_some()
            || self.catalogs.scope.is_some()
            || !self.catalogs.query.trim().is_empty()
    }

    /// Rows the arrows walk: the list, or the rows of the overview.
    pub(super) fn catalog_listed<'a, 'b>(&self, items: &'b [Item<'a>]) -> Vec<&'b Item<'a>> {
        if self.catalog_browsing() {
            return items
                .iter()
                .filter(|item| self.catalog_item_shown(item))
                .collect();
        }
        let (updates, installed) = overview_rows(items);
        updates.into_iter().chain(installed).collect()
    }
}

/// A receipted third-party override is not the official bundled package,
/// even when it deliberately uses the same manifest id.
fn installed_elsewhere(
    receipts: &[Installed],
    entry: &Entry,
    package: &crate::catalog::inventory::Package,
) -> bool {
    entry.kind == Kind::ClientExtension
        && receipts.iter().any(|receipt| {
            receipt.kind == entry.kind
                && receipt.entry == entry.id
                && !crate::catalog::open::OFFICIAL[1].matches(&receipt.source)
                && matches!(&receipt.target, crate::catalog::Target::Folder { path } if *path == package.path)
        })
}

fn presence_status(package: Option<&crate::catalog::inventory::Package>, version: &str) -> Status {
    match package {
        None => Status::Available,
        Some(package) if package.problem.is_some() => Status::Broken,
        Some(package) if package.version.as_deref().is_some_and(|old| old != version) => {
            Status::Update
        }
        Some(_) => Status::Installed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::inventory::Package;

    #[test]
    fn unknown_extension_version_is_installed_without_fabricating_update() {
        let mut package = Package {
            version: None,
            path: "extension".into(),
            problem: None,
        };
        assert_eq!(presence_status(Some(&package), "2.0"), Status::Installed);
        package.version = Some("1.0".into());
        assert_eq!(presence_status(Some(&package), "2.0"), Status::Update);
        package.problem = Some("broken".into());
        assert_eq!(presence_status(Some(&package), "2.0"), Status::Broken);
        assert_eq!(presence_status(None, "2.0"), Status::Available);
    }

    #[test]
    fn third_party_override_does_not_claim_official_installation() {
        let package = Package {
            version: None,
            path: "/extensions/terminal".into(),
            problem: None,
        };
        let entry: Entry = serde_json::from_value(serde_json::json!({
            "id":"terminal", "kind":"client_extension", "title":"Terminal", "category":"panels",
            "version":"1.0", "path":"terminal"
        }))
        .unwrap();
        let mut receipt = Installed {
            source: Source {
                url: "https://example.invalid/other/extensions".into(),
                branch: None,
            },
            entry: entry.id.clone(),
            version: "1.0".into(),
            kind: entry.kind,
            profile: None,
            target: crate::catalog::Target::Folder {
                path: package.path.clone(),
            },
        };
        assert!(installed_elsewhere(&[receipt.clone()], &entry, &package));
        receipt.source = crate::catalog::open::OFFICIAL[1].source();
        assert!(!installed_elsewhere(&[receipt], &entry, &package));
    }
}

/// Rows the overview lists in full before linking to the rest.
pub(super) const OVERVIEW_ROWS: usize = 5;

/// The first updates and installed entries the overview shows.
pub(super) fn overview_rows<'a, 'b>(
    items: &'b [Item<'a>],
) -> (Vec<&'b Item<'a>>, Vec<&'b Item<'a>>) {
    let pick = |status| {
        items
            .iter()
            .filter(|item| item.status == status)
            .take(OVERVIEW_ROWS)
            .collect()
    };
    (pick(Status::Update), pick(Status::Installed))
}
