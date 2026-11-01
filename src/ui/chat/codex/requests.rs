//! Native Codex requests retain their raw RPC ids and exact response choices.
use super::*;
use crate::agents::codex::RpcError;
use crate::ui::kit::{self, ButtonLabel as _, Frame as _, Hint as _};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::*;
use serde_json::json;

#[derive(Clone)]
pub(in crate::ui::chat) struct Request {
    pub id: Value,
    pub method: String,
    pub params: Value,
}

impl Request {
    pub fn projected(id: Value, method: String, mut params: Value) -> Self {
        // Keep every actionable choice verbatim, but do not mirror optional
        // parsed-command metadata or unrelated multi-megabyte payloads.
        let fields: &[&str] = match method.as_str() {
            "item/commandExecution/requestApproval" => &[
                "threadId",
                "turnId",
                "itemId",
                "approvalId",
                "kind",
                "command",
                "cwd",
                "reason",
                "availableDecisions",
                "proposedExecpolicyAmendment",
                "proposedNetworkPolicyAmendments",
                "networkApprovalContext",
            ],
            "item/fileChange/requestApproval" => &[
                "threadId",
                "turnId",
                "itemId",
                "reason",
                "grantRoot",
                "availableDecisions",
            ],
            "item/permissions/requestApproval" => {
                &["threadId", "turnId", "itemId", "reason", "permissions"]
            }
            "item/tool/requestUserInput" => {
                &["threadId", "turnId", "itemId", "questions", "isBlocking"]
            }
            _ => &["threadId", "turnId", "itemId", "tool", "name"],
        };
        if let Value::Object(map) = &mut params {
            map.retain(|key, _| fields.contains(&key.as_str()));
        }
        Self { id, method, params }
    }
    pub fn retained_bytes(&self) -> usize {
        value_bytes(&self.id) + self.method.len() + value_bytes(&self.params)
    }
}
fn value_bytes(value: &Value) -> usize {
    std::mem::size_of::<Value>()
        + match value {
            Value::String(text) => text.len(),
            Value::Array(values) => values.iter().map(value_bytes).sum(),
            Value::Object(values) => values
                .iter()
                .map(|(key, value)| key.len() + value_bytes(value))
                .sum(),
            _ => 0,
        }
}
fn decision_caption(decision: &Value) -> String {
    match decision.as_str() {
        Some("accept") => "Разрешить".into(),
        Some("acceptForSession") => "Разрешить для сессии".into(),
        Some("decline") => "Отклонить".into(),
        Some("cancel") => "Отклонить и остановить".into(),
        Some(value) => value.into(),
        None if decision.get("acceptWithExecpolicyAmendment").is_some() => format!(
            "Разрешить и сохранить правило: {}",
            decision["acceptWithExecpolicyAmendment"]["execpolicy_amendment"]
        ),
        None if decision.get("applyNetworkPolicyAmendment").is_some() => format!(
            "Сохранить правило сети: {}",
            decision["applyNetworkPolicyAmendment"]["network_policy_amendment"]
        ),
        None => format!("{}", decision),
    }
}

fn choices(request: &Request) -> Vec<(String, Value)> {
    let mut choices = Vec::new();
    match request.method.as_str() {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            if let Some(available) = request
                .params
                .get("availableDecisions")
                .and_then(Value::as_array)
            {
                return available
                    .iter()
                    .map(|decision| (decision_caption(decision), json!({"decision":decision})))
                    .collect();
            }
            for (caption, decision) in [
                ("Разрешить", "accept"),
                ("Разрешить для сессии", "acceptForSession"),
                ("Отклонить", "decline"),
                ("Отклонить и остановить", "cancel"),
            ] {
                choices.push((caption.into(), json!({"decision":decision})));
            }
            if request.method == "item/commandExecution/requestApproval" {
                if let Some(amendment) = request
                    .params
                    .get("proposedExecpolicyAmendment")
                    .filter(|value| value.is_array())
                {
                    choices.push(("Разрешить и сохранить правило команды".into(), json!({"decision":{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":amendment}}})));
                }
                if let Some(amendments) = request
                    .params
                    .get("proposedNetworkPolicyAmendments")
                    .and_then(Value::as_array)
                {
                    for amendment in amendments {
                        let host = amendment.get("host").and_then(Value::as_str).unwrap_or("");
                        let action = amendment
                            .get("action")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        choices.push((format!("Сохранить правило сети: {action} {host}"), json!({"decision":{"applyNetworkPolicyAmendment":{"network_policy_amendment":amendment}}})));
                    }
                }
            }
        }
        "item/permissions/requestApproval" => {
            let permissions = request
                .params
                .get("permissions")
                .cloned()
                .unwrap_or_else(|| json!({}));
            choices.push((
                "Разрешить на этот ход".into(),
                json!({"permissions":permissions,"scope":"turn"}),
            ));
            choices.push((
                "Разрешить для сессии".into(),
                json!({"permissions":permissions,"scope":"session"}),
            ));
            choices.push(("Отклонить".into(), json!({"permissions":{},"scope":"turn"})));
        }
        _ => {}
    }
    choices
}

impl Chat {
    pub(in crate::ui::chat) fn render_codex_requests(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let requests = self.codex.as_ref().unwrap().pending.clone();
        requests
            .into_iter()
            .map(|request| {
                let rid = request.id.to_string();
                let responding = self
                    .codex
                    .as_ref()
                    .unwrap()
                    .responding
                    .contains(&request.id);
                let title = match request.method.as_str() {
                    "item/commandExecution/requestApproval" => "Codex · запуск команды",
                    "item/fileChange/requestApproval" => "Codex · изменение файлов",
                    "item/permissions/requestApproval" => "Codex · разрешения",
                    "item/tool/requestUserInput" => "Codex · вопрос",
                    _ => "Codex · запрос клиента",
                };
                let mut card = v_flex()
                    .id(SharedString::from(format!("codex-request-{rid}")))
                    .w_full()
                    .gap_2()
                    .p_3()
                    .surface_frame(cx)
                    .child(div().font_medium().text_sm().child(title));
                if request.method == "item/tool/requestUserInput" {
                    let questions = request
                        .params
                        .get("questions")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let mut answered = true;
                    for question in &questions {
                        let qid = question
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        let key = (rid.clone(), qid.clone());
                        let input = self
                            .codex
                            .as_mut()
                            .unwrap()
                            .answers
                            .entry(key.clone())
                            .or_insert_with(|| {
                                let input = cx.new(|cx| {
                                    InputState::new(window, cx)
                                        .placeholder("Свой ответ")
                                        .masked(
                                            question
                                                .get("isSecret")
                                                .and_then(Value::as_bool)
                                                .unwrap_or(false),
                                        )
                                });
                                cx.observe(&input, |_, _, cx| cx.notify()).detach();
                                input
                            })
                            .clone();
                        let selected = self.codex.as_ref().unwrap().selected.get(&key).cloned();
                        answered &= !input.read(cx).value().trim().is_empty() || selected.is_some();
                        let mut body = v_flex().gap_1().child(
                            div().text_sm().child(
                                question
                                    .get("question")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_owned(),
                            ),
                        );
                        if let Some(options) = question.get("options").and_then(Value::as_array) {
                            for (index, option) in options.iter().enumerate() {
                                let label = option
                                    .get("label")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_owned();
                                let description = option
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_owned();
                                let key = key.clone();
                                let chosen = label.clone();
                                body = body.child(
                                    kit::button(SharedString::from(format!(
                                        "codex-question-{rid}-{qid}-{index}"
                                    )))
                                    .faded_label(label.clone())
                                    .hint(&description, cx)
                                    .quiet(cx)
                                    .small()
                                    .selected(selected.as_ref() == Some(&label))
                                    .disabled(responding)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let binding = this.codex.as_mut().unwrap();
                                        binding.selected.insert(key.clone(), chosen.clone());
                                        if let Some(input) = binding.answers.get(&key) {
                                            input.update(cx, |state, cx| {
                                                state.set_value("", window, cx)
                                            });
                                        }
                                        cx.notify();
                                    })),
                                );
                            }
                        }
                        if question
                            .get("isOther")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                            || question
                                .get("options")
                                .is_none_or(|options| options.is_null())
                        {
                            body = body.child(Input::new(&input).small());
                        }
                        card = card.child(body);
                    }
                    let id = request.id.clone();
                    let questions_for_send = questions.clone();
                    let rid_for_send = rid.clone();
                    card = card.child(
                        kit::button(SharedString::from(format!("codex-answer-{rid}")))
                            .faded_label("Ответить")
                            .primary()
                            .small()
                            .disabled(responding || !answered)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let binding = this.codex.as_ref().unwrap();
                                let mut answers = serde_json::Map::new();
                                for question in &questions_for_send {
                                    let qid =
                                        question.get("id").and_then(Value::as_str).unwrap_or("");
                                    let key = (rid_for_send.clone(), qid.to_owned());
                                    let free = binding
                                        .answers
                                        .get(&key)
                                        .map(|input| input.read(cx).value().trim().to_owned())
                                        .unwrap_or_default();
                                    let answer = if free.is_empty() {
                                        binding.selected.get(&key).cloned().unwrap_or_default()
                                    } else {
                                        free
                                    };
                                    if !answer.is_empty() {
                                        answers.insert(qid.into(), json!({"answers":[answer]}));
                                    }
                                }
                                this.respond_codex(id.clone(), Ok(json!({"answers":answers})), cx);
                            })),
                    );
                    let id = request.id.clone();
                    card = card.child(
                        kit::button(SharedString::from(format!("codex-dismiss-{rid}")))
                            .faded_label("Закрыть без ответа")
                            .quiet(cx)
                            .small()
                            .disabled(responding)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.respond_codex(id.clone(), Ok(json!({"answers":{}})), cx)
                            })),
                    );
                } else {
                    let detail = request
                        .params
                        .get("command")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            request
                                .params
                                .get("permissions")
                                .or_else(|| request.params.get("changes"))
                                .map(Value::to_string)
                                .unwrap_or_default()
                        });
                    if !detail.is_empty() {
                        card =
                            card.child(div().text_sm().child(gpui_kit::base::SelectableText::new(
                                SharedString::from(format!("codex-detail-{rid}")),
                                SharedString::from(detail),
                            )));
                    }
                    if let Some(reason) = request.params.get("reason").and_then(Value::as_str) {
                        card = card.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(reason.to_owned()),
                        );
                    }
                    let choices = choices(&request);
                    let mut actions = h_flex().gap_2().flex_wrap();
                    for (index, (label, response)) in choices.iter().enumerate() {
                        let id = request.id.clone();
                        let response = response.clone();
                        actions = actions.child(
                            kit::button(SharedString::from(format!("codex-choice-{rid}-{index}")))
                                .faded_label(label.clone())
                                .outline()
                                .small()
                                .disabled(responding)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.respond_codex(id.clone(), Ok(response.clone()), cx)
                                })),
                        );
                    }
                    if choices.is_empty() {
                        card = card.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "Этот запрос ещё не поддерживается: {}",
                                    request.method
                                )),
                        );
                        let id = request.id.clone();
                        actions = actions.child(
                            kit::button(SharedString::from(format!("codex-unsupported-{rid}")))
                                .faded_label("Отклонить запрос")
                                .outline()
                                .small()
                                .disabled(responding)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.respond_codex(
                                        id.clone(),
                                        Err(RpcError {
                                            code: -32601,
                                            message:
                                                "This client does not support this server request"
                                                    .into(),
                                            data: None,
                                        }),
                                        cx,
                                    )
                                })),
                        );
                    }
                    card = card.child(actions);
                }
                card.into_any_element()
            })
            .collect()
    }

    pub(in crate::ui::chat) fn reject_codex_overflow(&mut self, id: Value, cx: &mut Context<Self>) {
        let handle = self.codex.as_ref().unwrap().handle.clone();
        self.show_notice(
            "Codex: запрос отклонён — превышен лимит ожидающих запросов (32 / 8 МиБ)",
            cx,
        );
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    handle.respond(
                        id,
                        Err(RpcError {
                            code: -32000,
                            message:
                                "Client pending request capacity exceeded (32 requests / 8 MiB)"
                                    .into(),
                            data: None,
                        }),
                    )
                })
                .await;
            if let Err(error) = result {
                this.update(cx, |this, cx| {
                    this.show_notice(format!("Не удалось отклонить запрос Codex: {error}"), cx)
                })
                .ok();
            }
        })
        .detach();
    }

    fn respond_codex(
        &mut self,
        id: Value,
        result: Result<Value, RpcError>,
        cx: &mut Context<Self>,
    ) {
        let binding = self.codex.as_mut().unwrap();
        if binding.responding.contains(&id)
            || !binding.pending.iter().any(|request| request.id == id)
        {
            return;
        }
        binding.responding.push(id.clone());
        let handle = binding.handle.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let request_id = id.clone();
            let result = cx
                .background_executor()
                .spawn(async move { handle.respond(request_id, result) })
                .await;
            this.update(cx, |this, cx| {
                let binding = this.codex.as_mut().unwrap();
                binding.responding.retain(|pending| pending != &id);
                match result {
                    Ok(()) => {
                        binding.pending.retain(|request| request.id != id);
                        let rid = id.to_string();
                        binding.answers.retain(|(request, _), _| request != &rid);
                        binding.selected.retain(|(request, _), _| request != &rid);
                        this.session.external.as_mut().unwrap().waiting =
                            !binding.pending.is_empty();
                    }
                    Err(error) => {
                        this.show_notice(format!("Не удалось ответить Codex: {error}"), cx)
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn session_and_persistent_approval_are_distinct_exact_wire_choices() {
        let request = Request {
            id: json!(17),
            method: "item/commandExecution/requestApproval".into(),
            params: json!({"proposedExecpolicyAmendment":["git","status"],"proposedNetworkPolicyAmendments":[{"action":"allow","host":"example.org"}]}),
        };
        let choices = choices(&request);
        assert!(
            choices
                .iter()
                .any(|(_, response)| response == &json!({"decision":"acceptForSession"}))
        );
        assert!(choices.iter().any(|(_, response)| response == &json!({"decision":{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["git","status"]}}})));
        assert!(choices.iter().any(|(_, response)| response == &json!({"decision":{"applyNetworkPolicyAmendment":{"network_policy_amendment":{"action":"allow","host":"example.org"}}}})));
    }
    #[::core::prelude::v1::test]
    fn permission_decline_grants_nothing_and_scope_is_explicit() {
        let request = Request {
            id: json!("opaque:rpc"),
            method: "item/permissions/requestApproval".into(),
            params: json!({"permissions":{"network":{"enabled":true}}}),
        };
        let choices = choices(&request);
        assert_eq!(choices[0].1["scope"], "turn");
        assert_eq!(choices[1].1["scope"], "session");
        assert_eq!(choices[2].1, json!({"permissions":{},"scope":"turn"}));
    }
    #[::core::prelude::v1::test]
    fn available_decisions_are_authoritative_and_keep_object_choices_exact() {
        let decision =
            json!({"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["cargo","test"]}});
        let request = Request {
            id: json!(1),
            method: "item/commandExecution/requestApproval".into(),
            params: json!({"availableDecisions":["accept",decision,"cancel"], "proposedExecpolicyAmendment":["sh"], "proposedNetworkPolicyAmendments":[{"host":"example.org","action":"allow"}]}),
        };
        let choices = choices(&request);
        assert_eq!(choices.len(), 3);
        assert_eq!(choices[1].1, json!({"decision":decision}));
        assert!(
            !choices
                .iter()
                .any(|(_, choice)| choice.to_string().contains("example.org"))
        );
        assert!(
            !choices
                .iter()
                .any(|(_, choice)| choice["decision"] == "acceptForSession")
        );
    }
    #[::core::prelude::v1::test]
    fn projection_removes_heavy_command_mirror_but_preserves_all_decisions() {
        let decisions =
            json!(["accept",{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["git"]}}]);
        let request = Request::projected(
            json!(1),
            "item/commandExecution/requestApproval".into(),
            json!({"availableDecisions":decisions, "commandActions":[{"command":"x".repeat(100_000)}],"command":"git status"}),
        );
        assert_eq!(request.params["availableDecisions"], decisions);
        assert!(request.params.get("commandActions").is_none());
        assert!(request.retained_bytes() < 1024);
    }
    #[::core::prelude::v1::test]
    fn available_decisions_restrict_simple_choices() {
        let request = Request {
            id: json!(1),
            method: "item/fileChange/requestApproval".into(),
            params: json!({"availableDecisions":["accept","cancel"]}),
        };
        assert_eq!(choices(&request).len(), 2);
    }
}
