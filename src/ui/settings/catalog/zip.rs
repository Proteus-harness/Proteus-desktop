//! Agent Plugins archive import uses the same file handling as extension ZIPs.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use gpui_kit::component::*;
use gpui_kit::*;

use super::super::SettingsView;
use crate::catalog::install::ZipImport;

impl SettingsView {
    pub(in crate::ui::settings) fn choose_package_zip(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Установить ZIP".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let packages = this
                .update(cx, |this, cx| {
                    if this.catalogs.busy || this.saving {
                        return None;
                    }
                    this.catalogs.busy = true;
                    cx.notify();
                    Some((
                        this.draft.addons.addons.plugins.clone(),
                        this.context.profile.clone(),
                    ))
                })
                .ok()
                .flatten();
            let Some((packages, profile)) = packages else {
                return;
            };
            let archive = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let cwd = if packages.iter().any(|package| package.path.is_relative()) {
                        let profile = profile.context(
                            "Для относительных папок пакетов нужен открытый чат Proteus",
                        )?;
                        Some(PathBuf::from(profile.api.config(&profile.dir)?.cwd))
                    } else {
                        None
                    };
                    crate::catalog::install::import_zip(
                        &crate::catalog::Locations::local()?,
                        &archive,
                        packages,
                        cwd.as_deref(),
                    )
                })
                .await;
            let imported = match result {
                Ok(imported) => imported,
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.catalogs.busy = false;
                        this.show_feedback(format!("ZIP не установлен: {error:#}"), true, cx);
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            if imported.previous.is_some() {
                let answer =
                    cx.update(|window, cx| confirm_replacement(&imported, &path, window, cx));
                let replace = match answer {
                    Ok(answer) => answer.recv().await.unwrap_or(false),
                    Err(_) => false,
                };
                if !replace {
                    this.update(cx, |this, cx| {
                        this.catalogs.busy = false;
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            }
            this.update(cx, |this, cx| {
                this.catalogs.busy = false;
                let mut result = Ok(());
                this.update_draft(cx, |draft| result = imported.connect(draft));
                match result {
                    Ok(()) => {
                        this.show_feedback("Пакет распакован · нажмите «Сохранить»", false, cx);
                    }
                    Err(error) => {
                        this.show_feedback(format!("ZIP не установлен: {error:#}"), true, cx)
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// Адаптировано из Zed: crates/workspace/src/workspace.rs (prepare_to_close).
/// Use the client's existing alert surface for the explicit replacement choice.
fn confirm_replacement(
    imported: &ZipImport,
    archive: &Path,
    window: &mut Window,
    cx: &mut App,
) -> async_channel::Receiver<bool> {
    let previous = imported.previous.as_ref().expect("replacement candidate");
    let title = format!("Заменить пакет «{}»?", imported.name);
    let detail = format!(
        "В профиле уже подключён пакет:\n{}\n\nЗаменить его пакетом из ZIP:\n{}",
        previous.path.display(),
        archive.display()
    );
    let (sender, receiver) = async_channel::bounded(1);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let confirm = sender.clone();
        let closed = sender.clone();
        alert
            .title(title.clone())
            .description(detail.clone())
            .ok_text("Заменить")
            .show_cancel(true)
            .cancel_text("Отмена")
            .on_ok(move |_, _, _| {
                let _ = confirm.try_send(true);
                true
            })
            .on_close(move |_, _, _| {
                let _ = closed.try_send(false);
            })
    });
    receiver
}
