// Адаптировано из Zed: crates/workspace/src/workspace.rs (prompt_for_open_path).
//! Config-file opening follows Zed's native path prompt/new-window flow
//! (crates/workspace/src/workspace.rs::prompt_for_open_path). A different
//! config owns a different backend; the current window keeps its session.

use super::*;
use crate::conversations::AgentKind;

impl Launcher {
    pub(super) fn open_configuration_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let agent = match &self.target {
            Target::Spawn {
                profile: Some(profile),
            } => Some(PathBuf::from(profile)),
            _ => None,
        };
        if let Some(path) = agent.as_ref().filter(|path| path.is_file()) {
            prefs::set_visual_agent(path, window, cx);
        }
        let launcher = cx.weak_entity();
        let open_agent: crate::ui::settings::configurations::OpenAgent =
            std::rc::Rc::new(move |window, cx| {
                launcher
                    .update(cx, |this, cx| this.choose_config(window, cx))
                    .ok();
            });
        window.open_dialog(cx, move |dialog, _, cx| {
            dialog
                .title("Конфигурации")
                .w(px(680.))
                .child(
                    v_flex()
                        .gap_6()
                        .children(crate::ui::settings::configurations::controls(
                            agent.clone(),
                            open_agent.clone(),
                            cx,
                        )),
                )
        });
    }

    pub(super) fn choose_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = match &self.stage {
            Stage::Ready(view) => Some(view.read(cx).active_project_path().to_path_buf()),
            Stage::Picking { .. } => None,
        };
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Открыть конфиг".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let paths = match paths.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) => return,
                result => {
                    this.update(cx, |this, cx| {
                        this.config_error(format!("Не удалось выбрать конфиг: {result:?}"), cx)
                    })
                    .ok();
                    return;
                }
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let selected = cx
                .background_executor()
                .spawn(async move { selected_config(path) })
                .await;
            this.update(cx, |this, cx| {
                let profile = match selected {
                    Ok(path) => path.display().to_string(),
                    Err(error) => return this.config_error(error, cx),
                };
                let opened = gpui_kit::open_window(crate::window_options(cx), cx, |window, cx| {
                    cx.new(|cx| {
                        Self::with_startup(
                            workspace,
                            Target::Spawn {
                                profile: Some(profile),
                            },
                            AgentKind::Proteus,
                            None,
                            StartupIntent::Fresh,
                            window,
                            cx,
                        )
                    })
                });
                if let Err(error) = opened {
                    this.config_error(format!("Не удалось открыть окно: {error:#}"), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn config_error(&mut self, message: String, cx: &mut Context<Self>) {
        match &mut self.stage {
            Stage::Picking { error } => *error = Some(message.into()),
            Stage::Ready(view) => view.update(cx, |view, cx| view.report(message, cx)),
        }
        cx.notify();
    }
}

fn selected_config(path: PathBuf) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("Конфиг недоступен: {}: {error}", path.display()))?;
    if !canonical.is_file()
        || !matches!(
            canonical
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("toml" | "json")
        )
    {
        return Err("Выберите файл конфигурации TOML или JSON".into());
    }
    Ok(canonical)
}
