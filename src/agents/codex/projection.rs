//! Bounded display projection of the app-server protocol. No transcript cache.
use super::{Event, Item, ItemKind, Model, Summary};
use serde_json::{Value, json};
use std::path::PathBuf;

pub(super) const MAX_TEXT: usize = 128 * 1024;

pub(super) fn text(value: &Value, key: &str) -> String {
    clipped(value.get(key).and_then(Value::as_str).unwrap_or_default())
}

pub(super) fn clipped(value: &str) -> String {
    let mut end = value.len().min(MAX_TEXT);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut result = value[..end].to_owned();
    if end < value.len() {
        result.push_str("\n[… показаны первые 128 КиБ]");
    }
    result
}

fn joined(value: Option<&Value>) -> String {
    let Some(values) = value.and_then(Value::as_array) else {
        return String::new();
    };
    let mut result = String::new();
    for value in values {
        let chunk = value
            .as_str()
            .or_else(|| value.get("text").and_then(Value::as_str));
        if let Some(chunk) = chunk {
            if !result.is_empty() {
                result.push('\n');
            }
            let left = MAX_TEXT.saturating_sub(result.len());
            let mut end = left.min(chunk.len());
            while !chunk.is_char_boundary(end) {
                end -= 1;
            }
            result.push_str(&chunk[..end]);
            if result.len() >= MAX_TEXT {
                break;
            }
        }
    }
    result
}

fn display_json(value: &Value) -> Value {
    // Do not retain multi-megabyte tool arguments/results in a second mirror.
    let encoded = value.to_string();
    if encoded.len() > MAX_TEXT {
        json!({"preview":clipped(&encoded), "truncated":true})
    } else {
        value.clone()
    }
}

pub fn summary(value: &Value) -> Option<Summary> {
    Some(Summary {
        id: value.get("id")?.as_str()?.to_owned(),
        title: value
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(clipped)
            .unwrap_or_else(|| text(value, "preview")),
        cwd: PathBuf::from(value.get("cwd").and_then(Value::as_str).unwrap_or_default()),
        parent_thread_id: value
            .get("parentThreadId")
            .and_then(Value::as_str)
            .or_else(|| {
                value
                    .pointer("/source/subAgent/thread_spawn/parent_thread_id")
                    .and_then(Value::as_str)
            })
            .map(str::to_owned),
        created_at: value
            .get("createdAt")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        updated_at: value
            .get("updatedAt")
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        model: value
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_owned),
        effort: value
            .get("reasoningEffort")
            .and_then(Value::as_str)
            .map(str::to_owned),
        active: value.pointer("/status/type").and_then(Value::as_str) == Some("active"),
    })
}

pub fn item(
    value: &Value,
    turn_id: &str,
    completed: bool,
    timestamp_ms: Option<i64>,
) -> Option<Item> {
    let id = value.get("id")?.as_str()?.to_owned();
    let name = value.get("type")?.as_str()?;
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or(if completed { "completed" } else { "inProgress" })
        .to_owned();
    let kind = match name {
        "userMessage" => ItemKind::User {
            text: joined(value.get("content")),
        },
        "agentMessage" => ItemKind::Assistant {
            text: text(value, "text"),
        },
        "reasoning" => ItemKind::Reasoning {
            text: joined(value.get("summary")),
        },
        "plan" => ItemKind::Plan {
            text: text(value, "text"),
        },
        "commandExecution" => ItemKind::Tool {
            name: "shell".into(),
            input: json!({"command":text(value,"command"),"cwd":text(value,"cwd"),"processId":value.get("processId")}),
            output: text(value, "aggregatedOutput"),
        },
        "fileChange" => ItemKind::Tool {
            name: "apply_patch".into(),
            input: display_json(&value["changes"]),
            output: String::new(),
        },
        "mcpToolCall" | "dynamicToolCall" => ItemKind::Tool {
            name: if name == "mcpToolCall" {
                format!("{} / {}", text(value, "server"), text(value, "tool"))
            } else {
                text(value, "tool")
            },
            input: display_json(&value["arguments"]),
            output: clipped(
                &value
                    .get("result")
                    .or_else(|| value.get("contentItems"))
                    .or_else(|| value.get("error"))
                    .filter(|v| !v.is_null())
                    .map(Value::to_string)
                    .unwrap_or_default(),
            ),
        },
        "collabAgentToolCall" => ItemKind::Tool {
            name: text(value, "tool"),
            input: json!({"prompt":text(value,"prompt"),"receiverThreadIds":display_json(&value["receiverThreadIds"])}),
            output: clipped(&value["agentsStates"].to_string()),
        },
        "webSearch" => ItemKind::Tool {
            name: "web_search".into(),
            input: json!({"query":text(value,"query")}),
            output: String::new(),
        },
        "functionCallOutput" => ItemKind::Tool {
            name: text(value, "name"),
            input: Value::Null,
            output: clipped(&value["output"].to_string()),
        },
        _ => ItemKind::Other {
            name: name.into(),
            text: value
                .get("text")
                .or_else(|| value.get("review"))
                .and_then(Value::as_str)
                .map(clipped)
                .unwrap_or_default(),
        },
    };
    Some(Item {
        id,
        turn_id: turn_id.into(),
        kind,
        status,
        timestamp_ms,
    })
}

pub fn notification(method: &str, params: &Value) -> Option<Event> {
    let turn_id = || text(params, "turnId");
    let id = || text(params, "itemId");
    Some(match method {
        "item/started" | "item/completed" => Event::ItemUpsert(item(
            &params["item"],
            &turn_id(),
            method.ends_with("completed"),
            params
                .get(if method.ends_with("completed") {
                    "completedAtMs"
                } else {
                    "startedAtMs"
                })
                .and_then(Value::as_i64),
        )?),
        "item/agentMessage/delta" | "item/plan/delta" => Event::TextDelta {
            id: id(),
            turn_id: turn_id(),
            delta: text(params, "delta"),
        },
        "item/reasoning/summaryTextDelta" => Event::ReasoningDelta {
            id: id(),
            turn_id: turn_id(),
            delta: text(params, "delta"),
        },
        "item/reasoning/summaryPartAdded" => Event::ReasoningDelta {
            id: id(),
            turn_id: turn_id(),
            delta: "\n".into(),
        },
        "item/commandExecution/outputDelta" | "item/fileChange/outputDelta" => {
            Event::ToolOutputDelta {
                id: id(),
                turn_id: turn_id(),
                delta: text(params, "delta"),
            }
        }
        "turn/started" => Event::TurnStarted {
            id: text(&params["turn"], "id"),
        },
        "turn/completed" => Event::TurnCompleted {
            id: text(&params["turn"], "id"),
            status: text(&params["turn"], "status"),
            error: params
                .pointer("/turn/error/message")
                .and_then(Value::as_str)
                .map(clipped),
        },
        "serverRequest/resolved" => Event::RequestResolved {
            id: params.get("requestId")?.clone(),
        },
        "turn/plan/updated" => Event::Plan(display_json(params)),
        "thread/tokenUsage/updated" => Event::Usage(display_json(params)),
        // `error` may describe an automatic retry (`willRetry: true`). Even a
        // non-retry notice is not the authoritative turn lifecycle boundary.
        // Only turn/completed settles a run and releases send/interrupt state.
        "error" => return None,
        _ => return None,
    })
}

pub(super) fn model(value: &Value) -> Option<Model> {
    Some(Model {
        id: value.get("id")?.as_str()?.into(),
        model: text(value, "model"),
        title: text(value, "displayName"),
        description: text(value, "description"),
        default: value
            .get("isDefault")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        default_effort: text(value, "defaultReasoningEffort"),
        efforts: value
            .get("supportedReasoningEfforts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| {
                v.get("reasoningEffort")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spawned_thread_uses_real_source_lineage_when_top_level_parent_is_absent() {
        let value = json!({"id":"child:opaque", "parentThreadId":null, "source":{"subAgent":{"thread_spawn":{
            "parent_thread_id":"parent:opaque", "depth":1, "agent_role":"worker"
        }}}});
        assert_eq!(
            summary(&value).unwrap().parent_thread_id.as_deref(),
            Some("parent:opaque")
        );
        assert_eq!(
            summary(&json!({"id":"main","source":"appServer"}))
                .unwrap()
                .parent_thread_id,
            None
        );
    }
    #[test]
    fn error_notice_is_nonterminal_even_when_no_retry_is_planned() {
        for will_retry in [true, false] {
            assert_eq!(
                notification(
                    "error",
                    &json!({"threadId":"thread", "turnId":"turn", "willRetry":will_retry,
                "error":{"message":"synthetic error", "codexErrorInfo":"streamDisconnected"}})
                ),
                None
            );
        }
    }
    #[test]
    fn interrupted_is_not_success_and_ids_remain_opaque() {
        assert_eq!(
            notification(
                "turn/completed",
                &json!({"threadId":"thread:external","turn":{"id":"opaque-turn","status":"interrupted"}})
            ),
            Some(Event::TurnCompleted {
                id: "opaque-turn".into(),
                status: "interrupted".into(),
                error: None
            })
        );
    }
    #[test]
    fn collaboration_projects_as_tool_without_losing_receivers() {
        let value = json!({"id":"collab","type":"collabAgentToolCall","tool":"spawnAgent","receiverThreadIds":["child:1"],"agentsStates":{"child:1":{"status":"running"}},"prompt":"job","status":"inProgress"});
        let item = item(&value, "turn", false, None).unwrap();
        match item.kind {
            ItemKind::Tool { name, input, .. } => {
                assert_eq!(name, "spawnAgent");
                assert_eq!(input["receiverThreadIds"][0], "child:1");
            }
            _ => panic!(),
        }
    }
    #[test]
    fn historical_reasoning_uses_summary_not_private_content() {
        let item = item(
            &json!({"id":"r","type":"reasoning","summary":["visible"],"content":["private"]}),
            "t",
            true,
            None,
        )
        .unwrap();
        assert_eq!(
            item.kind,
            ItemKind::Reasoning {
                text: "visible".into()
            }
        );
    }
    #[test]
    fn truncation_respects_utf8() {
        let s = "я".repeat(MAX_TEXT);
        let output = clipped(&s);
        assert!(output.len() < MAX_TEXT + 100);
    }
}
