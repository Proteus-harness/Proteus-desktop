use proteus_contracts::app_protocol::{
    AppExecutionState, AppQueuedUserMessage, AppRun, AppRunStatus,
};
use proteus_contracts::domain::{Event, ModelRef, RunOptions, new_message_id};

use super::Feedback;

fn running(id: &str) -> AppExecutionState {
    AppExecutionState {
        active: Some(AppRun {
            run_id: id.into(),
            options: RunOptions::default(),
            status: AppRunStatus::Running,
            error: None,
        }),
        last: None,
    }
}

#[test]
fn http_acceptance_keeps_receipt_until_the_server_echo() {
    let mut feedback = Feedback::default();
    feedback.begin("one".into(), "hello".into(), &[], &[]);
    assert!(!feedback.complete("one", true, None));
    assert_eq!(feedback.visible_sends().count(), 1);
    feedback.submitted("hello", &[]);
    assert_eq!(feedback.visible_sends().count(), 0);
    assert!(feedback.sends.is_empty());
}

#[test]
fn echo_before_http_failure_does_not_restore_a_delivered_draft() {
    let mut feedback = Feedback::default();
    feedback.begin("one".into(), "hello".into(), &[], &[]);
    feedback.submitted("hello", &[]);
    assert_eq!(feedback.visible_sends().count(), 0);
    assert!(!feedback.complete("one", false, None));
    assert!(feedback.sends.is_empty());
    feedback.begin("two".into(), "next".into(), &[], &[]);
    assert!(feedback.complete("two", false, None));
}

#[test]
fn identical_concurrent_sends_need_independent_echoes() {
    let mut feedback = Feedback::default();
    for id in ["one", "two"] {
        feedback.begin(id.into(), "same".into(), &[], &[]);
        feedback.complete(id, true, None);
    }
    feedback.submitted("same", &[]);
    assert_eq!(feedback.visible_sends().count(), 1);
    assert_eq!(feedback.visible_sends().next().unwrap().id, "two");
    feedback.submitted("same", &[]);
    assert_eq!(feedback.visible_sends().count(), 0);
}

#[test]
fn old_and_repeated_queue_snapshots_do_not_acknowledge_another_send() {
    let mut feedback = Feedback::default();
    let old = AppQueuedUserMessage::new(new_message_id(), "same");
    for id in ["one", "two"] {
        feedback.begin(id.into(), "same".into(), &[], std::slice::from_ref(&old));
        feedback.complete(id, true, None);
    }
    feedback.queued(std::slice::from_ref(&old));
    assert_eq!(feedback.visible_sends().count(), 2);
    let new = AppQueuedUserMessage::new(new_message_id(), "same");
    feedback.queued(std::slice::from_ref(&new));
    feedback.queued(std::slice::from_ref(&new));
    assert_eq!(feedback.visible_sends().count(), 1);
}

#[test]
fn reconnect_uses_run_or_queued_message_identity() {
    let mut feedback = Feedback::default();
    feedback.begin("one".into(), "same".into(), &[], &[]);
    feedback.complete("one", true, None);
    feedback.snapshot(&running("other"), false, &[]);
    assert_eq!(feedback.visible_sends().count(), 1);
    feedback.snapshot(&running("one"), false, &[]);
    assert_eq!(feedback.visible_sends().count(), 0);

    let message_id = new_message_id();
    feedback.begin("queued".into(), "same".into(), &[], &[]);
    feedback.complete("queued", true, Some(message_id));
    feedback.snapshot(&AppExecutionState::default(), false, &[message_id]);
    assert_eq!(feedback.visible_sends().count(), 0);
}

#[test]
fn thinking_yields_to_text_tools_questions_cancel_and_settlement() {
    let mut feedback = Feedback::default();
    let mut execution = running("one");
    feedback.execution(&execution);
    assert!(feedback.thinking(&execution, false));
    assert!(!feedback.thinking(&execution, true));
    feedback.runtime(&Event::AssistantToolArgsDelta {
        call_id: "tool".into(),
        args_delta: "{".into(),
    });
    assert!(!feedback.thinking(&execution, false));
    feedback.runtime(&Event::ModelRequestPrepared {
        model: ModelRef::new("fake", "fake"),
    });
    feedback.runtime(&Event::AssistantReasoningDelta {
        text: "summary".into(),
    });
    assert!(feedback.thinking(&execution, false));
    feedback.runtime(&Event::AssistantTextDelta {
        offset: 0,
        message_id: new_message_id(),
        phase: None,
        text: "answer".into(),
    });
    assert!(!feedback.thinking(&execution, false));
    feedback.runtime(&Event::ModelRequestPrepared {
        model: ModelRef::new("fake", "fake"),
    });
    execution.active.as_mut().unwrap().status = AppRunStatus::CancelRequested;
    feedback.execution(&execution);
    assert!(!feedback.thinking(&execution, false));
    feedback.execution(&AppExecutionState::default());
    assert!(!feedback.thinking(&AppExecutionState::default(), false));
}

#[test]
fn runtime_error_does_not_reappear_on_an_unchanged_execution_update() {
    let mut feedback = Feedback::default();
    let execution = running("one");
    feedback.execution(&execution);
    feedback.runtime(&Event::Error {
        message: "offline".into(),
    });
    feedback.execution(&execution);
    assert!(!feedback.thinking(&execution, false));
    feedback.execution(&running("two"));
    assert!(feedback.thinking(&running("two"), false));
    feedback.fail_activity();
    feedback.execution(&running("two"));
    assert!(!feedback.thinking(&running("two"), false));
}

#[test]
fn an_image_only_send_requires_the_image_echo_and_preserves_the_receipt() {
    use proteus_contracts::domain::{ImageAttachment, ImageRef};
    let image = ImageAttachment {
        name: "picture.png".into(),
        mime_type: "image/png".into(),
        data: "synthetic".into(),
    };
    let mut feedback = Feedback::default();
    feedback.begin("one".into(), String::new(), &[image], &[]);
    feedback.complete("one", true, None);
    feedback.submitted("", &[]);
    assert_eq!(feedback.visible_sends().count(), 1);
    let echoed = ImageRef {
        id: "picture".into(),
        name: "picture.png".into(),
        mime_type: "image/png".into(),
        path: "/synthetic/picture.png".into(),
    };
    feedback.submitted("", &[echoed]);
    assert_eq!(feedback.visible_sends().count(), 0);
}

#[test]
fn large_drafts_keep_the_pending_receipt_bounded_in_both_dimensions() {
    assert_eq!(
        super::preview("one\ntwo\nthree\nfour\nfive"),
        "one\ntwo\nthree\nfour"
    );
    let text = "я".repeat(1_000_000);
    let preview = super::preview(&text);
    assert_eq!(preview.chars().count(), 320);
    assert!(!preview.ends_with('…'));
}

#[test]
fn many_pending_sends_have_one_latest_receipt_without_losing_earlier_drafts() {
    let mut feedback = Feedback::default();
    for id in 0..100 {
        feedback.begin(id.to_string(), format!("message {id}"), &[], &[]);
    }
    let (latest, previous) = feedback.latest_receipt().unwrap();
    assert_eq!(latest.id, "99");
    assert_eq!(previous, 99);
    assert_eq!(feedback.visible_sends().count(), 100);
    assert!(feedback.complete("99", false, None));
    assert_eq!(feedback.latest_receipt().unwrap().0.id, "98");
}
