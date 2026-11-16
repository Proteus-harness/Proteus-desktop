// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs (render_settings_item_layout).
//! Agent files, the current window and rules for future windows are distinct.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::SettingsView;
use super::widgets::{card, note, section};
use crate::ui::chrome::WorkspaceRequest;
use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::kit::ButtonLabel as _;
use crate::ui::launcher::visual::{self, Action};
use crate::ui::prefs::{self, WindowVisualPreferences};
use crate::ui::tip::Tip as _;
use crate::visual_config::SelectionOrigin;

pub(in crate::ui) type OpenAgent = Rc<dyn Fn(&mut Window, &mut App)>;

impl SettingsView {
    pub(super) fn configurations_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let workspace = self.workspace.clone();
        let open_agent: OpenAgent = Rc::new(move |_, cx| {
            workspace
                .update(cx, |_, cx| cx.emit(WorkspaceRequest::OpenConfig))
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
    let has_file = selected.is_some();
    let already_default = has_file && selected == default;
    let already_bound = has_file && selected == binding;
    let agent_name = file_name(agent.map(PathBuf::as_path), "этого конфига агента");

    let agent_file = h_flex()
        .gap_3()
        .items_center()
        .child(file_details(
            "config-agent-source",
            agent,
            "Конфиг по умолчанию",
            cx,
        ))
        .child(
            kit::button("config-open-agent")
                .small()
                .icon(Lucide::ChevronDown)
                .faded_label("Выбрать конфиг…")
                .on_click(move |_, window, cx| open_agent(window, cx)),
        );
    let visual_file = v_flex()
        .gap_3()
        .child(file_details(
            "config-visual-source",
            selected,
            "Оформление приложения",
            cx,
        ))
        .child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(action(
                    "config-open-visual",
                    "Выбрать файл…",
                    false,
                    Action::Open,
                ))
                .child(action(
                    "config-save-visual",
                    "Сохранить как…",
                    false,
                    Action::SaveAs,
                )),
        );
    let mode = match state.selection.origin {
        SelectionOrigin::Default => "Сейчас используется общее оформление.",
        SelectionOrigin::Binding => "Сейчас используется оформление для этого конфига агента.",
        SelectionOrigin::Manual => "Файл выбран вручную только для этого окна.",
    };
    let current = v_flex().gap_3().p_4().child(visual_file).child(
        h_flex()
            .gap_3()
            .items_center()
            .flex_wrap()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(mode),
            )
            .when(state.selection.origin == SelectionOrigin::Manual, |row| {
                row.child(action(
                    "config-follow-agent",
                    "Вернуть автовыбор",
                    false,
                    Action::FollowAgent,
                ))
            }),
    );

    let common = v_flex()
        .gap_2()
        .p_4()
        .child(div().text_sm().font_medium().child("Общее оформление"))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Для новых окон, у которых нет отдельного правила ниже."),
        )
        .child(file_details(
            "config-default-source",
            default,
            "Оформление приложения",
            cx,
        ))
        .child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(action(
                    "config-set-default",
                    "Использовать текущее",
                    !has_file || already_default,
                    Action::SetDefault,
                ))
                .child(action(
                    "config-use-default",
                    "Применить в этом окне",
                    false,
                    Action::UseDefault,
                )),
        );

    let separate = kit::switch("config-agent-visual-rule")
        .checked(binding.is_some())
        .disabled(!has_agent || (!has_file && binding.is_none()))
        .on_click(|checked: &bool, window, cx| {
            visual::run(
                if *checked {
                    Action::BindAgent
                } else {
                    Action::UnbindAgent
                },
                window,
                cx,
            );
        });
    let binding_title = format!("Отдельное оформление для {agent_name}");
    let binding_help = if !has_agent {
        "Сначала выберите файл конфигурации агента."
    } else if !has_file && binding.is_none() {
        "Сохраните оформление в файл, затем включите это правило."
    } else {
        "Новые окна с этим конфигом агента будут открываться с указанным оформлением."
    };
    let specific = v_flex()
        .gap_2()
        .p_4()
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .font_medium()
                        .child(faded(binding_title)),
                )
                .child(separate),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(binding_help),
        )
        .when_some(binding, |row, path| {
            row.child(file_details("config-binding-source", Some(path), "", cx))
                .when(!already_bound, |row| {
                    row.child(action(
                        "config-bind-agent",
                        "Заменить текущим оформлением",
                        !has_file,
                        Action::BindAgent,
                    ))
                })
        });

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
            Some("Выбор конфига открывает новое окно с новым чатом.".into()),
            None,
            card([div().p_4().child(agent_file).into_any_element()], cx),
            cx,
        )
        .into_any_element(),
    );
    parts.push(
        section(
            "Оформление этого окна",
            Some("Тема, шрифты и виджеты. Поведение агента не меняется.".into()),
            None,
            card([current.into_any_element()], cx),
            cx,
        )
        .into_any_element(),
    );
    parts.push(
        section(
            "При открытии новых окон",
            None,
            None,
            card([common.into_any_element(), specific.into_any_element()], cx),
            cx,
        )
        .into_any_element(),
    );
    parts.push(note("Правила для новых окон не меняют уже открытые. «Вернуть автовыбор» применяет правило агента, а если его нет — общее оформление.", cx).into_any_element());
    parts
}

fn file_name(path: Option<&Path>, missing: &str) -> String {
    path.and_then(Path::file_name).map_or_else(
        || missing.into(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The filename stays visible even when the directory is long; the tooltip
/// retains the complete path. No filesystem work happens while rendering.
fn file_details(id: &'static str, path: Option<&PathBuf>, missing: &str, cx: &App) -> impl IntoElement + use<> {
    let name = file_name(path.map(PathBuf::as_path), missing);
    let directory = path
        .and_then(|path| path.parent())
        .map(|path| path.display().to_string());
    let tooltip = path.map_or_else(|| missing.into(), |path| path.display().to_string());
    v_flex()
        .id(id)
        .flex_1()
        .min_w_0()
        .gap_0p5()
        .tip_text(tooltip)
        .child(div().text_sm().font_medium().child(faded(name)))
        .children(directory.map(|directory| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(faded(directory))
        }))
}

fn action(id: &'static str, label: &'static str, disabled: bool, action: Action) -> AnyElement {
    kit::button(id)
        .small()
        .faded_label(label)
        .disabled(disabled)
        .on_click(move |_, window, cx| visual::run(action, window, cx))
        .into_any_element()
}
