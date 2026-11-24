//! Bounded display projection of the app-server protocol. No transcript cache.
use super::{Event, Item, ItemKind, Model, Summary};
use serde_json::{Value, json};
use std::fmt::{self, Write as _};
use std::path::PathBuf;

pub(super) const MAX_TEXT: usize = 128 * 1024;
const TRUNCATED: &str = "\n[… показаны первые 128 КиБ]";

pub(super) fn text(value: &Value, key: &str) -> String {
    clipped(value.get(key).and_then(Value::as_str).unwrap_or_default())
}

pub(super) fn clipped(value: &str) -> String {
    if value.len() <= MAX_TEXT {
        return value.to_owned();
    }
    let end = value.floor_char_boundary(MAX_TEXT);
    let mut result = String::with_capacity(end + TRUNCATED.len());
    result.push_str(&value[..end]);
    result.push_str(TRUNCATED);
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

/// Keep the same compact JSON prefix as `clipped(&value.to_string())`, but
/// stop the formatter as soon as the display limit is known to be exceeded.
fn json_preview(value: &Value) -> (String, bool) {
    struct Prefix(String);
    impl fmt::Write for Prefix {
        fn write_str(&mut self, chunk: &str) -> fmt::Result {
            let left = MAX_TEXT - self.0.len();
            if chunk.len() <= left {
                self.0.push_str(chunk);
                return Ok(());
            }
            self.0.push_str(&chunk[..chunk.floor_char_boundary(left)]);
            Err(fmt::Error)
        }
    }
    let mut prefix = Prefix(String::new());
    let truncated = write!(&mut prefix, "{value}").is_err();
    if truncated {
        prefix.0.reserve_exact(TRUNCATED.len());
        prefix.0.push_str(TRUNCATED);
    }
    (prefix.0, truncated)
}

/// Returned text is retained by the chat; discard spare formatter capacity.
fn json_text(value: &Value) -> String {
    let (mut text, _) = json_preview(value);
    text.shrink_to_fit();
    text
}

fn display_json(value: &Value) -> Value {
    let (preview, truncated) = json_preview(value);
    if truncated {
        json!({"preview":preview, "truncated":true})
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
    turn_id: impl Into<String>,
    completed: bool,
    timestamp_ms: Option<i64>,
) -> Option<Item> {
    item_with_primary_text(value, turn_id, completed, timestamp_ms, None)
}

/// These history fields already have the representation the UI needs. Move
/// tight, in-limit strings; other variants keep the shared clipping rules.
pub(super) fn history_item(
    mut value: Value,
    turn_id: &str,
    timestamp_ms: Option<i64>,
) -> Option<Item> {
    let primary_text = if matches!(
        value.get("type").and_then(Value::as_str),
        Some("agentMessage" | "plan")
    ) {
        value.get_mut("text").and_then(|field| match field {
            Value::String(text) if text.len() <= MAX_TEXT && text.capacity() == text.len() => {
                Some(std::mem::take(text))
            }
            _ => None,
        })
    } else {
        None
    };
    item_with_primary_text(&value, turn_id, true, timestamp_ms, primary_text)
}

fn item_with_primary_text(
    value: &Value,
    turn_id: impl Into<String>,
    completed: bool,
    timestamp_ms: Option<i64>,
    primary_text: Option<String>,
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
            text: primary_text.unwrap_or_else(|| text(value, "text")),
        },
        "reasoning" => ItemKind::Reasoning {
            text: joined(value.get("summary")),
        },
        "plan" => ItemKind::Plan {
            text: primary_text.unwrap_or_else(|| text(value, "text")),
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
            output: value
                .get("result")
                .or_else(|| value.get("contentItems"))
                .or_else(|| value.get("error"))
                .filter(|v| !v.is_null())
                .map(json_text)
                .unwrap_or_default(),
        },
        "collabAgentToolCall" => ItemKind::Tool {
            name: text(value, "tool"),
            input: json!({"prompt":text(value,"prompt"),"receiverThreadIds":display_json(&value["receiverThreadIds"])}),
            output: json_text(&value["agentsStates"]),
        },
        "webSearch" => ItemKind::Tool {
            name: "web_search".into(),
            input: json!({"query":text(value,"query")}),
            output: String::new(),
        },
        "functionCallOutput" => ItemKind::Tool {
            name: text(value, "name"),
            input: Value::Null,
            output: json_text(&value["output"]),
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
            turn_id(),
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
    fn bounded_json_matches_full_serialization_at_display_boundaries() {
        for value in [
            Value::Null,
            json!({"n": -123.5, "s": "quote\" tab\t newline\n", "v": [true, null]}),
            json!("x".repeat(MAX_TEXT - 3)), // one byte below, counting quotes
            json!("x".repeat(MAX_TEXT - 2)),
            json!("x".repeat(MAX_TEXT - 1)),
            json!(format!("{}🦀tail", "x".repeat(MAX_TEXT - 3))),
            json!(format!("{}\n", "x".repeat(MAX_TEXT - 2))),
            json!({"rows": vec![json!({"value": "я 🦀"}); MAX_TEXT / 8]}),
        ] {
            let full = value.to_string();
            let expected = clipped(&full);
            let truncated = full.len() > MAX_TEXT;
            assert_eq!(json_preview(&value), (expected.clone(), truncated));
            assert_eq!(json_text(&value), expected);
            assert_eq!(
                display_json(&value),
                if truncated {
                    json!({"preview": expected, "truncated": true})
                } else {
                    value
                }
            );
        }
    }

    #[test]
    fn upsert_keeps_the_same_bounded_turn_id_and_started_or_completed_stamp() {
        let turn = format!("{}🦀 tail", "a".repeat(MAX_TEXT - 2));
        let message = json!({
            "turnId": turn,
            "startedAtMs": 12,
            "completedAtMs": 34,
            "item": {"id": "synthetic", "type": "agentMessage", "text": "visible"}
        });
        let expected = format!("{}{}", "a".repeat(MAX_TEXT - 2), TRUNCATED);
        for (method, time, status) in [
            ("item/started", 12, "inProgress"),
            ("item/completed", 34, "completed"),
        ] {
            let Some(Event::ItemUpsert(item)) = notification(method, &message) else {
                panic!("expected projected item");
            };
            assert_eq!(item.turn_id, expected);
            assert_eq!(item.timestamp_ms, Some(time));
            assert_eq!(item.status, status);
        }
    }

    #[test]
    fn truncation_respects_utf8() {
        let s = "я".repeat(MAX_TEXT);
        let output = clipped(&s);
        assert!(output.len() < MAX_TEXT + 100);
    }
}
