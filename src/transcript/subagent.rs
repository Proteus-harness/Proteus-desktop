//! Only reserved, typed Core metadata may turn a tool result into a child link.
use proteus_contracts::app_protocol::{AppSubagentSession, AppTranscriptTool};
use serde::Deserialize;

/// The live overlay is the single visual entry while it exists. A conflicting
/// durable reference is not silently deduplicated into some other child.
pub fn progress_row(entries: &[super::Entry], tool: &AppTranscriptTool) -> Option<usize> {
    if !is_task(tool) {
        return None;
    }
    let child = tool.metadata.get("child_thread_id")?.as_str()?;
    let mut candidates = entries.iter().enumerate().filter_map(|(row, entry)| {
        let agent = entry.subagent.as_ref()?;
        (agent.child_thread_id == child).then_some((row, &agent.session))
    });
    // Saved task cards have no live progress row. Do not deserialize (and
    // allocate) a session path just to decide they are not duplicates.
    let (row, live) = candidates.next()?;
    let Some(value) = tool
        .metadata
        .get("subagent_session")
        .filter(|value| !value.is_null())
    else {
        return Some(row);
    };
    // An explicitly invalid binding never deduplicates, even if a live row
    // is unbound. A later compatible overlay can still follow a conflict.
    let bound = decode_session(value)?;
    std::iter::once((row, live))
        .chain(candidates)
        .find_map(|(row, live)| {
            live.as_ref()
                .is_none_or(|live| *live == bound)
                .then_some(row)
        })
}

pub fn status(status: &str) -> &str {
    match status {
        "done" | "completed" | "success" => "completed",
        "failed" | "errored" => "failed",
        "cancelled" | "canceled" | "interrupted" => "cancelled",
        other => other,
    }
}

pub fn is_task(tool: &AppTranscriptTool) -> bool {
    tool.metadata
        .get("tool")
        .and_then(|value| value.as_str())
        .is_some_and(|tool| matches!(tool, "task" | "spawn_agent" | "followup_task"))
        && tool
            .metadata
            .get("child_thread_id")
            .and_then(|value| value.as_str())
            .is_some_and(|id| !id.is_empty())
}

/// A background spawn/followup result acknowledges the call, not the child's
/// lifecycle. Only the synchronous task result can establish a final status.
pub fn task_status(tool: &AppTranscriptTool) -> &str {
    if tool.metadata.get("tool").and_then(|value| value.as_str()) == Some("task") {
        status(&tool.status)
    } else {
        "unknown"
    }
}

/// Human captions from the known tool schema. They never identify a session.
pub fn task_caption(tool: &AppTranscriptTool) -> (&str, Option<&str>) {
    let text = |key| {
        tool.args
            .get(key)
            .and_then(|value| value.as_str())
            .filter(|text| !text.is_empty())
    };
    match tool.metadata.get("tool").and_then(|value| value.as_str()) {
        Some("followup_task") => (text("target").unwrap_or("Продолжение"), text("message")),
        Some("spawn_agent") => (
            text("agent_type")
                .or_else(|| text("role"))
                .unwrap_or("Субагент"),
            text("task_name").or_else(|| text("description")),
        ),
        _ => (
            text("agent_type")
                .or_else(|| text("role"))
                .unwrap_or("Субагент"),
            text("description").or_else(|| text("prompt")),
        ),
    }
}

pub fn task_session(tool: &AppTranscriptTool) -> Option<AppSubagentSession> {
    if !is_task(tool) {
        return None;
    }
    decode_session(tool.metadata.get("subagent_session")?)
}

fn decode_session(value: &serde_json::Value) -> Option<AppSubagentSession> {
    let session = AppSubagentSession::deserialize(value).ok()?;
    session.session_dir.is_absolute().then_some(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_task_link_requires_typed_reserved_metadata_not_name_or_uuid() {
        let mut tool = AppTranscriptTool {
            terminal: None,
            call_id: "call".into(),
            name: "task".into(),
            args: json!({}),
            effective_args: None,
            status: "done".into(),
            result: None,
            metadata: json!({"child_thread_id": "looks-like-a-thread"}),
        };
        assert!(!is_task(&tool));
        assert!(task_session(&tool).is_none());
        tool.metadata["tool"] = json!("task");
        assert!(is_task(&tool));
        assert!(
            task_session(&tool).is_none(),
            "old task remains a disabled strip"
        );
        tool.metadata["subagent_session"] = json!({
            "session_id": "00000000-0000-0000-0000-000000000001", "session_dir": "/saved/exact-child",
        });
        assert_eq!(
            task_session(&tool).unwrap().session_dir.to_str(),
            Some("/saved/exact-child")
        );
        tool.metadata["subagent_session"]["session_dir"] = json!("relative/guess");
        assert!(task_session(&tool).is_none());
    }

    #[test]
    fn typed_child_link_still_rejects_malformed_metadata() {
        let session = AppSubagentSession {
            session_id: uuid::Uuid::from_u128(1),
            session_dir: std::env::current_dir().unwrap().join("synthetic-child"),
        };
        let mut tool = AppTranscriptTool {
            terminal: None,
            call_id: "call".into(),
            name: "task".into(),
            args: json!({}),
            effective_args: None,
            status: "done".into(),
            result: None,
            metadata: json!({"tool": "task", "child_thread_id": "child", "subagent_session": session}),
        };
        assert_eq!(task_session(&tool), Some(session));
        tool.metadata["subagent_session"]["extra"] = json!({"ignored": false});
        assert!(task_session(&tool).is_none());
        tool.metadata["subagent_session"]
            .as_object_mut()
            .unwrap()
            .remove("extra");
        tool.metadata["subagent_session"]["session_id"] = json!("not-a-uuid");
        assert!(task_session(&tool).is_none());
    }

    #[test]
    fn background_call_success_is_not_a_completed_child() {
        let mut tool = AppTranscriptTool {
            terminal: None,
            call_id: "spawn".into(),
            name: "anything".into(),
            args: json!({}),
            effective_args: None,
            status: "done".into(),
            result: None,
            metadata: json!({ "tool": "spawn_agent", "child_thread_id": "exact", "subagent_session": {
                "session_id": "00000000-0000-0000-0000-000000000001", "session_dir": "/saved/child",
            } }),
        };
        for kind in ["spawn_agent", "followup_task"] {
            tool.metadata["tool"] = json!(kind);
            assert!(is_task(&tool));
            assert!(task_session(&tool).is_some());
            assert_eq!(task_status(&tool), "unknown");
        }
        tool.metadata["tool"] = json!("spawn_agent");
        tool.args = json!({"agent_type":"explorer","task_name":"inspect cache"});
        assert_eq!(task_caption(&tool), ("explorer", Some("inspect cache")));
        tool.metadata["tool"] = json!("followup_task");
        tool.args = json!({"target":"/root/cache-audit","message":"check the retained window"});
        assert_eq!(
            task_caption(&tool),
            ("/root/cache-audit", Some("check the retained window"))
        );
        tool.metadata["tool"] = json!("task");
        tool.args = json!({"agent_type":"coder","description":"fix history"});
        assert_eq!(task_caption(&tool), ("coder", Some("fix history")));
        assert_eq!(task_status(&tool), "completed");
    }
}
