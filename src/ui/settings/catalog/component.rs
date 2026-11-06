//! A prepared module becomes installed only after Core accepts its package.

use std::path::Path;

use gpui_kit::*;
use proteus_contracts::app_protocol::http::{
    ComponentPackageInstall, ComponentReplacement, SetConfigBuilderRequest,
};

use super::super::SettingsView;
use crate::{
    catalog::{Installed, Target, open::OFFICIAL},
    settings::reconcile::CatalogChange,
    ui::prefs,
};

impl SettingsView {
    pub(super) fn connect_catalog_component(
        &mut self,
        installed: Installed,
        previous: Option<&Installed>,
        cx: &mut Context<Self>,
    ) {
        if let Some(reason) = self.profile_blocked() {
            return self.catalog_failed(anyhow::anyhow!(reason), cx);
        }
        let Target::Component {
            command, exports, ..
        } = &installed.target
        else {
            return self.catalog_failed(anyhow::anyhow!("Неверный пакет модуля"), cx);
        };
        let previous_command = previous.and_then(|old| match &old.target {
            Target::Component { command, .. } => command.first(),
            _ => None,
        });
        let filename = command
            .first()
            .and_then(|command| Path::new(command).file_name());
        let bundled =
            self.catalogs
                .inventory
                .find(&installed.source, installed.kind, &installed.entry);
        let bundled_executable = bundled.and_then(|package| {
            let root = package.path.parent()?.parent()?.parent()?;
            Some(root.join("bin").join(filename?))
        });
        let replace = self
            .snapshot()
            .into_iter()
            .flat_map(|snapshot| &snapshot.plugins)
            .filter(|plugin| {
                previous_command.is_some_and(|old| plugin.command == *old)
                    || (OFFICIAL[0].matches(&installed.source)
                        && bundled.is_some()
                        && (Some(Path::new(&plugin.command)) == bundled_executable.as_deref()
                            || (Path::new(&plugin.command).components().count() == 1
                                && Path::new(&plugin.command).file_name() == filename)))
            })
            .map(|plugin| ComponentReplacement {
                id: plugin.id.clone(),
                command: plugin.command.clone(),
                expected_executable: if Path::new(&plugin.command).is_absolute() {
                    Some(plugin.command.clone().into())
                } else {
                    bundled_executable.clone()
                },
            })
            .collect();
        let request = SetConfigBuilderRequest {
            component_packages: vec![ComponentPackageInstall {
                id: installed.entry.clone(),
                command: command.clone(),
                exports: exports.clone(),
                replace,
            }],
            ..Default::default()
        };
        let submitted_revision = self.edit_revision;
        let change = CatalogChange {
            before: self.baseline.addons.clone(),
            replacement: None,
        };
        let Some(super::super::context::Profile { api, dir }) = self.context.profile.clone() else {
            return self
                .catalog_failed(anyhow::anyhow!("Для этого нужен открытый чат Proteus"), cx);
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.save_config_builder(&dir, &request) })
                .await;
            this.update(cx, |this, cx| {
                this.finish_catalog_profile_write(
                    result,
                    submitted_revision,
                    &change,
                    move |cx| prefs::update(cx, |prefs| prefs.catalogs.remember(installed)),
                    "Установлено · модуль доступен в профиле",
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }
}
