// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs (render_settings_item_layout).
//! File selection is a settings concern. The same controls also work before
//! a project/backend exists; every visual action keeps its exact native owner.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::*;
use gpui_kit::*;

use super::SettingsView;
use super::widgets::{card, note, row, section};
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

/// Source labels and actions are shared with the startup settings dialog,
/// rather than retaining a second set of header/menu controls.
pub(in crate::ui) fn controls(
    agent: Option<PathBuf>,
    open_agent: OpenAgent,
    cx: &App,
) -> Vec<AnyElement> {
    let state = cx.global::<WindowVisualPreferences>();
    let agent = state.agent_path.as_ref().or(agent.as_ref());
    let source = path_label(agent, "Файл агента не выбран");
    let visual = path_label(state.selection.path.as_ref(), "Встроенный default");
    let default = prefs::current(cx)
        .visual_configs
        .default_config
        .as_ref()
        .or_else(|| {
            (state.selection.origin == SelectionOrigin::Default)
                .then_some(state.selection.path.as_ref())
                .flatten()
        });
    let binding = agent.and_then(|agent| prefs::current(cx).visual_configs.bindings.get(agent));
    let mode = match state.selection.origin {
        SelectionOrigin::Default => "Default",
        SelectionOrigin::Binding => "Привязка к агенту",
        SelectionOrigin::Manual => "Ручной выбор",
    };
    let has_file = state.selection.path.is_some();
    let has_agent = state.agent_path.is_some();
    let agent_controls = v_flex()
        .gap_2()
        .child(source_line("config-agent-source", source, cx))
        .child(
            kit::button("config-open-agent")
                .small()
                .faded_label("Открыть с другим конфигом…")
                .on_click(move |_, window, cx| open_agent(window, cx)),
        );
    let visual_controls = v_flex()
        .gap_2()
        .child(source_line("config-visual-source", visual, cx))
        .child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(action(
                    "config-open-visual",
                    "Открыть…",
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
    let rows = [
        row("Файл оформления", None, visual_controls, true, cx).into_any_element(),
        row(
            format!("Выбор для этого окна · {mode}"),
            None,
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(action(
                    "config-use-default",
                    "Использовать default",
                    false,
                    Action::UseDefault,
                ))
                .child(action(
                    "config-follow-agent",
                    "Следовать привязке",
                    false,
                    Action::FollowAgent,
                )),
            true,
            cx,
        )
        .into_any_element(),
    ];
    let remembered = [
        row(
            "Default для новых окон",
            None,
            v_flex()
                .gap_2()
                .child(source_line(
                    "config-default-source",
                    path_label(default, "Default приложения"),
                    cx,
                ))
                .child(action(
                    "config-set-default",
                    "Сделать текущий default",
                    !has_file,
                    Action::SetDefault,
                )),
            true,
            cx,
        )
        .into_any_element(),
        row(
            "Привязка к этому файлу агента",
            None,
            v_flex()
                .gap_2()
                .child(source_line(
                    "config-binding-source",
                    path_label(binding, "Не задана"),
                    cx,
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(action(
                            "config-bind-agent",
                            "Привязать текущий",
                            !has_file || !has_agent,
                            Action::BindAgent,
                        ))
                        .child(action(
                            "config-unbind-agent",
                            "Убрать привязку",
                            binding.is_none(),
                            Action::UnbindAgent,
                        )),
                ),
            true,
            cx,
        )
        .into_any_element(),
    ];
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
            Some(
                "Другой файл открывается в новом окне с новым чатом; это окно продолжает работу."
                    .into(),
            ),
            None,
            card(
                [row("Файл конфигурации", None, agent_controls, true, cx).into_any_element()],
                cx,
            ),
            cx,
        )
        .into_any_element(),
    );
    parts.push(
        section(
            "Desktop этого окна",
            Some(
                "Оформление и виджеты применяются сразу, независимо от агента и соседних окон."
                    .into(),
            ),
            None,
            card(rows, cx),
            cx,
        )
        .into_any_element(),
    );
    parts.push(
        section("Default и привязка", None, None, card(remembered, cx), cx).into_any_element(),
    );
    parts.push(note("Ручной выбор действует в этом окне. Default применяется новым окнам; привязка использует путь файла агента.", cx).into_any_element());
    parts
}

fn path_label(path: Option<&PathBuf>, missing: &str) -> String {
    path.map_or_else(|| missing.into(), |path| path.display().to_string())
}

fn source_line(id: &'static str, source: String, cx: &App) -> impl IntoElement + use<> {
    div()
        .id(id)
        .min_w_0()
        .w_full()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .tip_text(source.clone())
        .child(faded(source))
}

fn action(id: &'static str, label: &'static str, disabled: bool, action: Action) -> AnyElement {
    kit::button(id)
        .small()
        .faded_label(label)
        .disabled(disabled)
        .on_click(move |_, window, cx| visual::run(action, window, cx))
        .into_any_element()
}
