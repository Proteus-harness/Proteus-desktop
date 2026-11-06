//! Source-only entries are downloaded to an explicitly selected folder.

use gpui_kit::*;

use super::super::SettingsView;
use crate::catalog::{Locations, Source, download, schema::Entry};

impl SettingsView {
    pub(super) fn download_catalog_source(
        &mut self,
        source: Source,
        entry: Entry,
        cx: &mut Context<Self>,
    ) {
        if !self.catalog_idle() || self.catalogs.loading.contains(&source) {
            return;
        }
        let Some((_, catalog)) = self.catalogs.loaded.iter().find(|(old, _)| *old == source) else {
            return;
        };
        let catalog = catalog.clone();
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Скачать исходники сюда".into()),
        });
        self.catalogs.busy = true;
        self.catalogs.working = Some((source.clone(), entry.id.clone()));
        self.catalogs.message = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = match picked.await {
                Ok(Ok(Some(paths))) if !paths.is_empty() => {
                    let parent = paths[0].clone();
                    Some(
                        cx.background_executor()
                            .spawn(async move {
                                download::source(
                                    &Locations::local()?,
                                    &source,
                                    &catalog,
                                    &entry,
                                    &parent,
                                )
                            })
                            .await,
                    )
                }
                Ok(Err(error)) => Some(Err(anyhow::anyhow!("Не удалось выбрать папку: {error}"))),
                _ => None,
            };
            this.update(cx, |this, cx| {
                this.catalogs.busy = false;
                this.catalogs.working = None;
                if let Some(result) = result {
                    this.catalogs.message = Some(match result {
                        Ok(path) => (
                            false,
                            format!(
                                "Исходники сохранены: {}. Команда сборки — в BUILD.md.",
                                path.display()
                            ),
                        ),
                        Err(error) => (true, format!("{error:#}")),
                    });
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
