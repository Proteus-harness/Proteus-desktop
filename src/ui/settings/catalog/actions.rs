use gpui_kit::*;

use super::super::SettingsView;
use crate::catalog::{
    Locations, Source, git,
    inventory::Inventory,
    schema::{self, Catalog, Entry},
};
use crate::ui::prefs;

impl SettingsView {
    pub(in crate::ui::settings) fn open_catalogs(&mut self, cx: &mut Context<Self>) {
        if self.catalogs.opened {
            self.refresh_catalog_inventory(cx);
            return;
        }
        self.catalogs.opened = true;
        let initializing = !prefs::current(cx).catalogs.official_sources_initialized;
        if initializing {
            prefs::update(cx, |prefs| prefs.catalogs.ensure_official_sources());
        }
        let sources = prefs::current(cx).catalogs.sources.clone();
        self.catalogs.busy = true;
        self.catalogs.working = None;
        self.catalogs.inventory_generation += 1;
        let inventory_generation = self.catalogs.inventory_generation;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let locations = Locations::local()?;
                    let inventory = Inventory::local();
                    Ok::<_, anyhow::Error>((
                        sources
                            .into_iter()
                            .map(|source| {
                                let catalog = git::cached(&locations.cache, &source);
                                (source, catalog)
                            })
                            .collect::<Vec<_>>(),
                        inventory,
                    ))
                })
                .await;
            this.update(cx, |this, cx| {
                this.catalogs.busy = false;
                match result {
                    Ok((catalogs, inventory)) => {
                        if this.catalogs.inventory_generation == inventory_generation {
                            this.catalogs.inventory = inventory;
                        }
                        let mut absent = Vec::new();
                        for (source, catalog) in catalogs {
                            if !prefs::current(cx).catalogs.sources.contains(&source) {
                                continue;
                            }
                            if initializing {
                                absent.push(source.clone());
                            }
                            let fresh = this.catalogs.loaded.iter().any(|(old, _)| *old == source);
                            match (catalog, fresh) {
                                (Ok(catalog), false) => {
                                    this.catalogs.loaded.push((source, catalog))
                                }
                                (Err(_), false) if !absent.contains(&source) => absent.push(source),
                                (Err(_), false) => {}
                                (_, true) => {}
                            }
                        }
                        this.order_catalogs(cx);
                        // Lists that were never downloaded come without a click.
                        this.refresh_catalogs(absent, cx);
                    }
                    Err(error) => this.catalog_message(Err(error)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Re-read installation state after an install/removal or revisiting the
    /// page. A slow earlier scan cannot overwrite a more recent inventory.
    pub(super) fn refresh_catalog_inventory(&mut self, cx: &mut Context<Self>) {
        self.catalogs.inventory_generation += 1;
        let generation = self.catalogs.inventory_generation;
        cx.spawn(async move |this, cx| {
            let inventory = cx
                .background_executor()
                .spawn(async { Inventory::local() })
                .await;
            this.update(cx, |this, cx| {
                if this.catalogs.inventory_generation == generation {
                    this.catalogs.inventory = inventory;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn add_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.catalogs.url.read(cx).value().trim().to_string();
        let branch = self.catalogs.branch.read(cx).value().trim().to_string();
        let source = Source {
            url,
            branch: (!branch.is_empty()).then_some(branch),
        };
        if let Err(error) = git::validate(&source) {
            self.catalog_message(Err(error));
            cx.notify();
            return;
        }
        for input in [&self.catalogs.url, &self.catalogs.branch] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.add_catalog_sources(vec![source], cx);
    }

    /// Remembers the sources and downloads them.
    pub(super) fn add_catalog_sources(&mut self, sources: Vec<Source>, cx: &mut Context<Self>) {
        prefs::update(cx, |prefs| {
            for source in &sources {
                if !prefs.catalogs.sources.contains(source) {
                    prefs.catalogs.sources.push(source.clone());
                }
            }
        });
        self.refresh_catalogs(sources, cx);
    }

    /// Downloads the sources side by side; each list shows up when ready.
    pub(super) fn refresh_catalogs(&mut self, sources: Vec<Source>, cx: &mut Context<Self>) {
        let sources = sources
            .into_iter()
            .filter(|source| !self.catalogs.loading.contains(source))
            .collect::<Vec<_>>();
        if sources.is_empty() {
            return;
        }
        if self.catalogs.loading.is_empty() {
            self.catalogs.failures.clear();
            self.catalogs.message = None;
        }
        for source in sources {
            self.catalogs.loading.push(source.clone());
            cx.spawn(async move |this, cx| {
                let task = source.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move { git::refresh(&Locations::local()?.cache, &task) })
                    .await;
                this.update(cx, |this, cx| this.catalog_loaded(source, result, cx))
                    .ok();
            })
            .detach();
        }
        cx.notify();
    }

    fn catalog_loaded(
        &mut self,
        source: Source,
        result: anyhow::Result<Catalog>,
        cx: &mut Context<Self>,
    ) {
        self.catalogs.loading.retain(|old| *old != source);
        // A source removed meanwhile stays removed.
        if prefs::current(cx).catalogs.sources.contains(&source) {
            match result {
                Ok(catalog) => {
                    self.catalogs.loaded.retain(|(old, _)| *old != source);
                    self.catalogs.loaded.push((source, catalog));
                    self.order_catalogs(cx);
                }
                Err(error) => self
                    .catalogs
                    .failures
                    .push(format!("{}: {error:#}", source.name())),
            }
        }
        if self.catalogs.loading.is_empty() {
            let failures = std::mem::take(&mut self.catalogs.failures);
            self.catalog_message(if failures.is_empty() {
                Ok("Каталог обновлён")
            } else {
                Err(anyhow::anyhow!(failures.join("\n")))
            });
        }
        cx.notify();
    }

    /// Lists follow the order the sources were added in.
    fn order_catalogs(&mut self, cx: &App) {
        let sources = &prefs::current(cx).catalogs.sources;
        self.catalogs
            .loaded
            .sort_by_key(|(old, _)| sources.iter().position(|known| known == old));
    }

    pub(super) fn forget_catalog(&mut self, source: &Source, cx: &mut Context<Self>) {
        prefs::update(cx, |prefs| {
            prefs.catalogs.sources.retain(|old| old != source)
        });
        self.catalogs.loaded.retain(|(old, _)| old != source);
        self.catalogs.loading.retain(|old| old != source);
        self.forget_catalog_scope(source);
        if self
            .catalogs
            .selected
            .as_ref()
            .is_some_and(|(old, _)| old == source)
        {
            self.deselect_catalog_entry(cx);
        }
        cx.notify();
    }

    pub(super) fn select_catalog_entry(
        &mut self,
        source: Source,
        entry: Entry,
        cx: &mut Context<Self>,
    ) {
        let Some((_, catalog)) = self.catalogs.loaded.iter().find(|(old, _)| *old == source) else {
            return;
        };
        let catalog = catalog.clone();
        self.catalogs.selected = Some((source.clone(), entry.id.clone()));
        self.catalogs.closing = None;
        self.catalogs.readme = None;
        self.catalogs.readme_changes = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let id = entry.id.clone();
            let task = source.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut files = vec![entry.path.join("README.md"), entry.path.join("SKILL.md")];
                    if let Some(source) = &entry.source {
                        files.push(source.path.join("README.md"));
                    }
                    git::with_files(&Locations::local()?.cache, &task, &files, || {
                        schema::readme(&catalog, &entry)
                    })
                })
                .await;
            this.update(cx, |this, cx| {
                if this.catalogs.selected.as_ref() != Some(&(source, id)) {
                    return;
                }
                match result {
                    Ok(Some(text)) => {
                        let state = cx.new(|_| {
                            crate::ui::extensions::content::FormattedText::new(
                                this.workspace.clone(),
                                "text/markdown",
                                text,
                            )
                        });
                        this.catalogs.readme_changes =
                            Some(cx.observe(&state, |_, _, cx| cx.notify()));
                        this.catalogs.readme = Some(state);
                    }
                    Ok(None) => {}
                    Err(error) => this.catalog_message(Err(error)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(in crate::ui::settings) fn deselect_catalog_entry(&mut self, cx: &mut Context<Self>) {
        if let Some(selected) = self.catalogs.selected.take() {
            self.catalogs.closing = Some(selected);
        }
        cx.notify();
    }

    pub(in crate::ui::settings) fn catalog_message(&mut self, result: anyhow::Result<&str>) {
        self.catalogs.message = Some(match result {
            Ok(text) => (false, text.to_string()),
            Err(error) => (true, format!("{error:#}")),
        });
    }
}
