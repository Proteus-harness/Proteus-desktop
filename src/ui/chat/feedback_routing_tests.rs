use super::*;
use proteus_contracts::app_protocol::{
    AppRun, AppRunStatus, AppTranscriptMessage, AppTranscriptTool,
};
use proteus_contracts::domain::{EventContext, ModelRef, ToolResult};
use uuid::Uuid;

#[test]
fn first_turn_child_errors_and_tool_results_cannot_change_parent_thinking() {
    let root = Uuid::from_u128(10);
    let child = Uuid::from_u128(20);
    let mut session = Session::new("synthetic".into());
    session.connected = true;
    session.execution.active = Some(AppRun {
        run_id: "first".into(),
        options: Default::default(),
        status: AppRunStatus::Running,
        error: None,
    });
    session.feedback.execution(&session.execution);
    let envelope = |thread, event| {
        EventEnvelope::new(
            EventContext::new(Uuid::from_u128(1), thread, None),
            1,
            event,
        )
    };
    let started = |thread| Event::TurnStarted {
        session_id: Uuid::from_u128(1),
        thread_id: thread,
        turn_id: Uuid::from_u128(2),
    };
    session.runtime_feedback(&envelope(root, started(root)));
    assert_eq!(session.root_thread, Some(root));
    assert!(session.thinking());
    session.runtime_feedback(&envelope(
        child,
        Event::Error {
            message: "child failure".into(),
        },
    ));
    assert!(
        session.thinking(),
        "child errors must not clear parent activity"
    );
    session.runtime_feedback(&envelope(
        root,
        Event::AssistantTextDelta {
            offset: 0,
            message_id: Uuid::from_u128(3),
            phase: None,
            text: "answer".into(),
        },
    ));
    assert!(!session.thinking());
    session.runtime_feedback(&envelope(
        child,
        Event::ToolFinished {
            result: ToolResult::ok("child-tool".into(), ""),
        },
    ));
    assert!(
        !session.thinking(),
        "child results must not resurrect parent activity"
    );
    session.runtime_feedback(&envelope(child, started(child)));
    assert_eq!(session.root_thread, Some(root));
    session.runtime_feedback(&envelope(
        root,
        Event::ModelRequestPrepared {
            model: ModelRef::new("synthetic", "synthetic"),
        },
    ));
    assert!(session.thinking());
    session.stream_disconnected("synthetic offline".into());
    assert!(!session.thinking());
}

#[test]
fn reconnect_and_parallel_completion_yield_to_an_approved_running_tool() {
    let root = Uuid::from_u128(10);
    let mut session = Session::new("synthetic".into());
    session.connected = true;
    let execution = AppExecutionState {
        active: Some(AppRun {
            run_id: "active".into(),
            options: Default::default(),
            status: AppRunStatus::Running,
            error: None,
        }),
        last: None,
    };
    session.apply_snapshot(
        AppSessionSnapshot {
            session_id: Uuid::from_u128(1),
            stream_id: "synthetic".into(),
            seq: 1,
            root_thread_id: Some(root),
            history: None,
            execution,
            transcript: vec![AppTranscriptMessage {
                timestamp_ms: None,
                message_id: None,
                phase: None,
                role: "tool".into(),
                text: String::new(),
                images: vec![],
                subagent: None,
                streaming: false,
                tool: Some(AppTranscriptTool {
                    call_id: "still-executing".into(),
                    name: "shell".into(),
                    args: serde_json::json!({}),
                    effective_args: None,
                    status: "approved".into(),
                    result: None,
                    metadata: serde_json::Value::Null,
                }),
            }],
        },
        &mut History::default(),
    );
    assert!(
        !session.thinking(),
        "approved tool is executing after reconnect"
    );
    session.runtime_feedback(&EventEnvelope::new(
        EventContext::new(Uuid::from_u128(1), root, None),
        2,
        Event::ToolFinished {
            result: ToolResult::ok("other-tool".into(), "done"),
        },
    ));
    assert!(
        !session.thinking(),
        "another parallel tool is still executing"
    );
    session.transcript.reset(vec![]);
    assert!(
        session.thinking(),
        "model waiting resumes after the remaining tool finishes"
    );
}
