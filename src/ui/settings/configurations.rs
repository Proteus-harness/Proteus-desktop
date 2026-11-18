// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs (render_settings_item_layout).
//! Current files and remembered appearance choices share the settings row layout.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::SettingsView;
use super::widgets::{card, row, section};
use crate::ui::chrome::WorkspaceRequest;
use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::kit::ButtonLabel as _;
use crate::ui::launcher::config::config_selector;
use crate::ui::launcher::visual::{self, Action};
use crate::ui::prefs::{self, WindowVisualPreferences};
use crate::ui::tip::Tip as _;
use crate::visual_config::SelectionOrigin;

pub(in crate::ui) type OpenAgent = Rc<dyn Fn(Option<PathBuf>, &mut Window, &mut App)>;

impl SettingsView {
    pub(super) fn configurations_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let workspace = self.workspace.clone();
        let open_agent: OpenAgent = Rc::new(move |path, _, cx| {
            workspace
                .update(cx, |_, cx| cx.emit(WorkspaceRequest::OpenConfig(path)))
                .ok();
        });
        let agent = self
            .snapshot()
            .and_then(|snapshot| snapshot.config_path.clone().map(PathBuf::from));
        controls(agent, open_agent, cx)
    }
}

/// Also used before a project/backend exists, in the startup settings dialog.
pub(in crate::ui) fn controls(
    agent: Option<PathBuf>,
    open_agent: OpenAgent,
    cx: &App,
) -> Vec<AnyElement> {
    let state = cx.global::<WindowVisualPreferences>();
    let agent = state.agent_path.as_ref().or(agent.as_ref());
    let selected = state.selection.path.as_ref();
    let registry = &prefs::current(cx).visual_configs;
    let default = registry.default_config.as_ref().or_else(|| {
        (state.selection.origin == SelectionOrigin::Default)
            .then_some(selected)
            .flatten()
    });
    let binding = agent.and_then(|agent| registry.bindings.get(agent));
    let has_agent = state.agent_path.is_some();
    let origin = state.selection.origin;

    let agent_file = crate::ui::kit::settings_row(
        file_label(agent, "По умолчанию"),
        config_selector(agent.cloned(), open_agent, cx),
        false,
    );
    let visual_name = file_name(selected.map(PathBuf::as_path), "Встроенное");
    let visual_tooltip = selected.map_or_else(
        || "Оформление этого окна".into(),
        |path| path.display().to_string(),
    );
    let visual_choice = crate::ui::context::menu_button(
        "config-visual-choice",
        kit::button("config-visual-file")
            .small()
            .min_w_0()
            .max_w_full()
            .faded_label(visual_name)
            .tip_text(visual_tooltip),
        true,
        move |menu: PopupMenu, _, _| {
            menu.item(
                crate::ui::selection::menu_option("Выбрать файл…", false)
                    .on_click(|_, window, cx| visual::run(Action::Open, window, cx)),
            )
            .item(
                crate::ui::selection::menu_option("Сохранить оформление как…", false)
                    .on_click(|_, window, cx| visual::run(Action::SaveAs, window, cx)),
            )
            .item(
                crate::ui::selection::menu_option(
                    "По умолчанию",
                    origin == SelectionOrigin::Default,
                )
                .on_click(|_, window, cx| visual::run(Action::UseDefault, window, cx)),
            )
            .when(has_agent, |menu| {
                menu.item(
                    crate::ui::selection::menu_option(
                        "Для этого агента",
                        origin == SelectionOrigin::Binding,
                    )
                    .on_click(|_, window, cx| visual::run(Action::FollowAgent, window, cx)),
                )
            })
        },
    )
    .settings_selector()
    .min_w_0()
    .max_w_full();
    let current = row("Это окно", None, visual_choice, false, cx);
    let common = row(
        "Для новых окон",
        None,
        remembered_choice(
            "config-default-choice",
            default,
            selected,
            "Встроенное",
            Action::SetDefault,
            Action::ClearDefault,
            false,
        ),
        false,
        cx,
    );
    let specific = row(
        "Для этого агента",
        agent.map(|path| path.display().to_string().into()),
        remembered_choice(
            "config-agent-choice",
            binding,
            selected,
            "Как для остальных",
            Action::BindAgent,
            Action::UnbindAgent,
            !has_agent,
        ),
        false,
        cx,
    );

    let mut parts = [
        cx.global::<prefs::ClientPreferences>().problem.clone(),
        state.problem.clone(),
    ]
    .into_iter()
    .flatten()
    .map(|problem| {
        div()
            .text_sm()
            .text_color(cx.theme().danger)
            .child(problem)
            .into_any_element()
    })
    .collect::<Vec<_>>();
    parts.push(
        section(
            "Агент",
            None,
            None,
            card([agent_file.into_any_element()], cx),
            cx,
        )
        .into_any_element(),
    );
    parts.push(
        section(
            "Оформление",
            None,
            None,
            card([current.into_any_element()], cx),
            cx,
        )
        .into_any_element(),
    );
    parts.push(
        section(
            "Запомнить оформление",
            None,
            None,
            card([common.into_any_element(), specific.into_any_element()], cx),
            cx,
        )
        .into_any_element(),
    );
    parts
}

/// One choice per destination: the current file, the remembered file, or no
/// override. Changing these choices never applies a file to an existing window.
fn remembered_choice(
    id: &'static str,
    saved: Option<&PathBuf>,
    current: Option<&PathBuf>,
    empty: &'static str,
    remember: Action,
    clear: Action,
    disabled: bool,
) -> AnyElement {
    let label = file_name(saved.map(PathBuf::as_path), empty);
    let tooltip = saved.map_or_else(|| empty.into(), |path| path.display().to_string());
    let saved = saved.cloned();
    let current = current.cloned();
    crate::ui::context::menu_button(
        id,
        kit::button(ElementId::Name(format!("{id}-value").into()))
            .small()
            .min_w_0()
            .max_w_full()
            .faded_label(label)
            .disabled(disabled)
            .tip_text(tooltip),
        true,
        move |menu: PopupMenu, _, _| {
            let menu = menu.item(
                crate::ui::selection::menu_option(empty, saved.is_none())
                    .on_click(move |_, window, cx| visual::run(clear, window, cx)),
            );
            let menu = menu.when_some(current.as_ref(), |menu, path| {
                menu.item(
                    crate::ui::selection::menu_option(
                        format!("Текущее · {}", file_name(Some(path), "")),
                        saved.as_ref() == Some(path),
                    )
                    .on_click(move |_, window, cx| visual::run(remember, window, cx)),
                )
            });
            menu.when_some(
                saved
                    .as_ref()
                    .filter(|path| Some(*path) != current.as_ref()),
                |menu, path| {
                    menu.item(crate::ui::selection::menu_option(
                        file_name(Some(path), ""),
                        true,
                    ))
                },
            )
        },
    )
    .settings_selector()
    .min_w_0()
    .max_w_full()
    .into_any_element()
}

fn file_name(path: Option<&Path>, missing: &str) -> String {
    path.and_then(Path::file_name).map_or_else(
        || missing.into(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Paths are available on hover; the primary row only needs the filename.
fn file_label(path: Option<&PathBuf>, missing: &str) -> impl IntoElement + use<> {
    let name = file_name(path.map(PathBuf::as_path), missing);
    let tooltip = path.map_or_else(|| missing.into(), |path| path.display().to_string());
    div()
        .flex_1()
        .min_w_0()
        .text_sm()
        .tip_text(tooltip)
        .child(faded(name))
}
