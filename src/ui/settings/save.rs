use super::*;

impl SettingsView {
    /// Acknowledge only the saved revision; retain any newer editor buffers.
    /// Адаптировано из Zed: crates/language/src/buffer.rs (did_save).
    pub(super) fn accept_save(&mut self, snapshot: ConfigBuilderSnapshot, submitted_revision: u64) {
        if self.edit_revision == submitted_revision {
            self.accept(snapshot);
        } else {
            self.baseline = Draft::from_snapshot(&snapshot);
            self.state = LoadState::Ready(Box::new(snapshot));
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
