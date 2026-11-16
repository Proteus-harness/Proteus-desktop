// Адаптировано из Zed: crates/workspace/src/workspace.rs (native path prompts).
//! Visual-file actions affect one window, never its agent or live backend.

use crate::ui::prefs::{self, WindowVisualPreferences};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use std::path::PathBuf;

pub(in crate::ui) fn menu(
    mut menu: PopupMenu,
    _: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let current = cx.global::<WindowVisualPreferences>();
    let source = current.selection.path.as_ref().map_or_else(
        || "Встроенный default".into(),
        |path| path.display().to_string(),
    );
    let has_file = current.selection.path.is_some();
    let has_agent = current.agent_path.is_some();
    let bound = current.agent_path.as_ref().is_some_and(|path| {
        prefs::current(cx)
            .visual_configs
            .bindings
            .contains_key(path)
    });
    menu = menu
        .label(source)
        .item(
            PopupMenuItem::new("Открыть визуальный конфиг…")
                .on_click(|_, window, cx| open(window, cx)),
        )
        .item(
            PopupMenuItem::new("Сохранить визуальный конфиг как…")
                .on_click(|_, window, cx| save_as(window, cx)),
        )
        .item(
            PopupMenuItem::new("Использовать default")
                .on_click(|_, window, cx| prefs::reset_visual_default(window, cx)),
        )
        .item(
            PopupMenuItem::new("Следовать привязке агента / default")
                .on_click(|_, window, cx| prefs::reset_visual_auto(window, cx)),
        )
        .separator()
        .item(
            PopupMenuItem::new("Сделать default для новых окон")
                .disabled(!has_file)
                .on_click(|_, window, cx| remember(false, window, cx)),
        )
        .item(
            PopupMenuItem::new("Привязать к этому конфигу агента")
                .disabled(!has_file || !has_agent)
                .on_click(|_, window, cx| remember(true, window, cx)),
        )
        .item(
            PopupMenuItem::new("Убрать привязку к агенту")
                .disabled(!bound)
                .on_click(|_, _, cx| {
                    let agent = cx.global::<WindowVisualPreferences>().agent_path.clone();
                    if let Some(agent) = agent {
                        prefs::update(cx, |prefs| {
                            prefs.visual_configs.bindings.remove(&agent);
                        });
                    }
                }),
        );
    menu
}

fn open(window: &mut Window, cx: &mut App) {
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Открыть визуальный конфиг".into()),
    });
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let selected = match paths.await {
            Ok(Ok(Some(paths))) => paths.into_iter().next(),
            Ok(Ok(None)) => None,
            result => {
                handle
                    .update(cx, |_, _, cx| {
                        prefs::visual_error(
                            format!("Не удалось выбрать визуальный конфиг: {result:?}"),
                            cx,
                        )
                    })
                    .ok();
                None
            }
        };
        if let Some(path) = selected {
            handle
                .update(cx, |_, window, cx| prefs::select_visual(path, window, cx))
                .ok();
        }
    })
    .detach();
}

fn save_as(window: &mut Window, cx: &mut App) {
    let state = cx.global::<WindowVisualPreferences>();
    let value = state.value.clone();
    let agent = state.agent_path.clone();
    let intent = prefs::visual_intent(cx);
    let folder = state
        .selection
        .path
        .as_ref()
        .and_then(|path| path.parent().map(ToOwned::to_owned))
        .or_else(crate::preferences::config_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    let picked = cx.prompt_for_new_path(&folder, Some("desktop.json"));
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let path = match picked.await {
            Ok(Ok(Some(path))) => path,
            Ok(Ok(None)) => return,
            result => {
                handle
                    .update(cx, |_, _, cx| {
                        prefs::visual_error(format!("Не удалось выбрать файл: {result:?}"), cx)
                    })
                    .ok();
                return;
            }
        };
        let saved = cx
            .background_executor()
            .spawn(async move { crate::visual_config::save(&path, &value, agent.as_deref()) })
            .await;
        handle
            .update(cx, |_, window, cx| match saved {
                Ok(path) => {
                    prefs::select_visual_if_current(intent, path, window, cx);
                }
                Err(error) => {
                    prefs::visual_error(format!("Визуальный конфиг не сохранён: {error:#}"), cx)
                }
            })
            .ok();
    })
    .detach();
}

fn remember(binding: bool, window: &mut Window, cx: &mut App) {
    let state = cx.global::<WindowVisualPreferences>();
    let Some(path) = state.selection.path.clone() else {
        return;
    };
    let agent = state.agent_path.clone();
    if binding && agent.is_none() {
        return;
    }
    let mut registry = prefs::current(cx).visual_configs.clone();
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let checked = cx
            .background_executor()
            .spawn(async move {
                if binding {
                    registry.bind(agent.as_deref().unwrap(), &path)?;
                } else {
                    registry.set_default(Some(&path))?;
                }
                anyhow::Ok((registry, agent))
            })
            .await;
        handle
            .update(cx, |_, _, cx| match checked {
                Ok((registry, agent)) => prefs::update(cx, |prefs| {
                    if binding {
                        if let Some(agent) = agent {
                            if let Some(path) = registry.bindings.get(&agent) {
                                prefs.visual_configs.bindings.insert(agent, path.clone());
                            }
                        }
                    } else {
                        prefs.visual_configs.default_config = registry.default_config;
                    }
                }),
                Err(error) => prefs::visual_error(format!("Привязка не сохранена: {error:#}"), cx),
            })
            .ok();
    })
    .detach();
}
