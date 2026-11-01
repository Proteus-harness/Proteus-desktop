// Адаптировано из Zed: crates/agent_ui/src/agent_model_selector.rs
// (catalog-driven model selection in the conversation's input controls).
use super::*;
use crate::ui::kit::{self, ButtonLabel as _, Hint as _};
use gpui_kit::component::*;

impl Chat {
    pub(in crate::ui::chat) fn render_codex_dock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let binding = self.codex.as_ref()?;
        let selected = binding.model.clone();
        let title = binding
            .models
            .iter()
            .find(|model| Some(&model.model) == selected.as_ref())
            .map(|model| model.title.clone())
            .or(selected.clone())
            .or_else(|| {
                binding
                    .models
                    .iter()
                    .find(|model| model.default)
                    .map(|model| model.title.clone())
            })
            .unwrap_or_else(|| "Модель Codex".into());
        let models = binding.models.clone();
        let chat = cx.weak_entity();
        let busy = self.session.running();
        let model_menu = crate::ui::context::menu_button(
            "codex-model-menu",
            kit::button("codex-model")
                .faded_label(title)
                .hint("Модель Codex", cx)
                .quiet(cx)
                .small()
                .disabled(busy || models.is_empty()),
            true,
            move |menu, _, _| {
                models.iter().fold(menu, |menu, model| {
                    let chat = chat.clone();
                    let model = model.clone();
                    menu.item(
                        crate::ui::selection::menu_option(
                            model.title.clone(),
                            Some(&model.model) == selected.as_ref(),
                        )
                        .on_click(move |_, _, cx| {
                            chat.update(cx, |this, cx| {
                                if this.session.running() {
                                    return;
                                }
                                let binding = this.codex.as_mut().unwrap();
                                binding.model = Some(model.model.clone());
                                binding.effort = model
                                    .efforts
                                    .contains(&model.default_effort)
                                    .then(|| model.default_effort.clone());
                                this.remember_codex_selection(cx);
                                cx.notify();
                            })
                            .ok();
                        }),
                    )
                })
            },
        );
        let binding = self.codex.as_ref().unwrap();
        let efforts = binding
            .models
            .iter()
            .find(|model| Some(&model.model) == binding.model.as_ref())
            .or_else(|| {
                binding
                    .model
                    .is_none()
                    .then(|| binding.models.iter().find(|model| model.default))
                    .flatten()
            })
            .map(|model| model.efforts.clone())
            .unwrap_or_default();
        let selected = binding.effort.clone();
        let chat = cx.weak_entity();
        let effort_menu = crate::ui::context::menu_button(
            "codex-effort-menu",
            kit::button("codex-effort")
                .faded_label(selected.clone().unwrap_or_else(|| "По умолчанию".into()))
                .hint("Глубина рассуждений", cx)
                .quiet(cx)
                .small()
                .disabled(busy || efforts.is_empty()),
            true,
            move |menu, _, _| {
                efforts.iter().fold(menu, |menu, effort| {
                    let chat = chat.clone();
                    let effort = effort.clone();
                    menu.item(
                        crate::ui::selection::menu_option(
                            effort.clone(),
                            Some(&effort) == selected.as_ref(),
                        )
                        .on_click(move |_, _, cx| {
                            chat.update(cx, |this, cx| {
                                if !this.session.running() {
                                    let binding = this.codex.as_mut().unwrap();
                                    if binding.model.is_none() {
                                        binding.model = binding
                                            .models
                                            .iter()
                                            .find(|model| model.default)
                                            .map(|model| model.model.clone());
                                    }
                                    binding.effort = Some(effort.clone());
                                    this.remember_codex_selection(cx);
                                    cx.notify();
                                }
                            })
                            .ok();
                        }),
                    )
                })
            },
        );
        let pending = self.render_codex_requests(window, cx);
        let error = self.session.link_error.clone();
        let plan = self
            .codex
            .as_ref()
            .and_then(|binding| binding.plan.as_ref())
            .and_then(|value| value.get("plan"))
            .and_then(Value::as_array)
            .map(|steps| {
                steps
                    .iter()
                    .map(|step| {
                        let title = step.get("step").and_then(Value::as_str).unwrap_or("");
                        let status = step.get("status").and_then(Value::as_str).unwrap_or("");
                        let mark = if status == "completed" {
                            "✓"
                        } else if status == "inProgress" {
                            "•"
                        } else {
                            "·"
                        };
                        format!("{mark} {title}")
                    })
                    .collect::<Vec<_>>()
            });
        Some(
            v_flex()
                .w_full()
                .gap_2()
                .p_2()
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Codex"),
                        )
                        .child(model_menu)
                        .child(effort_menu),
                )
                .children(
                    error.map(|error| div().text_xs().text_color(cx.theme().danger).child(error)),
                )
                .children(plan.filter(|steps| !steps.is_empty()).map(|steps| {
                    v_flex()
                        .gap_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .children(steps)
                }))
                .children(pending)
                .into_any_element(),
        )
    }
}
