use super::*;
use crate::settings::reconcile::CatalogChange;

impl SettingsView {
    /// Acknowledge only the saved revision; retain any newer editor buffers.
    /// Адаптировано из Zed: crates/language/src/buffer.rs (did_save).
    pub(super) fn accept_save(&mut self, snapshot: ConfigBuilderSnapshot, submitted_revision: u64) {
        self.catalog_conflict = None;
        if self.edit_revision == submitted_revision {
            self.accept(snapshot);
        } else {
            self.baseline = Draft::from_snapshot(&snapshot);
            self.state = LoadState::Ready(Box::new(snapshot));
        }
    }

    /// Catalog operations persist a delta against the saved profile, rather
    /// than the current draft. Replay that delta before acknowledging its
    /// revision so the next manual Save cannot accidentally undo it.
    pub(super) fn accept_catalog_save(
        &mut self,
        snapshot: ConfigBuilderSnapshot,
        submitted_revision: u64,
        change: &CatalogChange,
    ) {
        let conflicts = if self.edit_revision != submitted_revision {
            let before = self.draft.addons.mcp_servers.clone();
            let edited = self
                .editors
                .mcp
                .iter()
                .filter_map(|(ix, form)| {
                    form.edited
                        .then(|| before.get(*ix).map(|server| server.name.clone()))
                        .flatten()
                })
                .collect();
            let accepted = Draft::from_snapshot(&snapshot);
            let conflicts = change.rebase(&mut self.draft.addons, &accepted.addons, &edited);
            self.remap_mcp_editors(&before);
            conflicts
        } else {
            Vec::new()
        };
        self.accept_save(snapshot, submitted_revision);
        if !conflicts.is_empty() {
            self.clear_feedback();
            self.catalog_conflict = Some(format!("Каталог сохранён. Конфликт: {}. Ваши правки оставлены несохранёнными; проверьте их перед «Сохранить» или сбросьте к сохранённому профилю.", conflicts.join(", ")).into());
        }
    }

    pub(super) fn save(&mut self, cx: &mut Context<Self>) {
        if !self.can_save() {
            return;
        }
        let Some(context::Profile { api, dir }) = self.context.profile.clone() else {
            return;
        };
        let request = self.draft.changes(&self.baseline);
        let submitted_revision = self.edit_revision;
        self.saving = true;
        self.clear_feedback();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.save_config_builder(&dir, &request) })
                .await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(snapshot) => {
                        this.accept_save(snapshot, submitted_revision);
                        this.load_addons(cx);
                        this.refresh_history(cx);
                        let text = if this.dirty() {
                            "Сохранено · более новые изменения ещё не сохранены"
                        } else {
                            "Сохранено · новые запросы используют обновлённую сборку"
                        };
                        this.show_feedback(text, false, cx);
                        cx.emit(SettingsEvent::Saved);
                    }
                    Err(error) => {
                        this.show_feedback(format!("Не сохранено: {error:#}"), true, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
