//! One footer boundary and local timestamp per completed response turn.
// Local-day comparison follows Zed: crates/time_format/src/time_format.rs.

use chrono::{DateTime, Local, TimeZone as _};
use gpui_kit::component::*;
use gpui_kit::*;

use crate::transcript::{Entry, Role};
use crate::ui::tip::Tip as _;

pub(super) fn hidden(entries: &[Entry], entry: &Entry) -> bool {
    entry
        .tool
        .as_ref()
        .is_some_and(|tool| crate::transcript::subagent::progress_row(entries, tool).is_some())
}

/// `end` includes the whole represented tool chain. Hidden duplicate progress
/// rows do not become separate turns; an unloaded page edge is not an ending.
pub(super) fn ending(entries: &[Entry], end: usize, complete_tail: bool) -> bool {
    let Some(entry) = end.checked_sub(1).and_then(|row| entries.get(row)) else {
        return false;
    };
    if entry.role == Role::User || entry.streaming {
        return false;
    }
    let next = entries[end..].iter().find(|entry| !hidden(entries, entry));
    next.map_or(complete_tail, |entry| entry.role == Role::User)
}

pub(super) fn time_after(entries: &[Entry], end: usize, complete_tail: bool) -> Option<i64> {
    if !ending(entries, end, complete_tail) {
        return None;
    }
    entries[..end]
        .iter()
        .rev()
        .take_while(|entry| entry.role != Role::User)
        .find_map(|entry| entry.time)
}

/// The time of row `key` at `time` (Unix milliseconds), muted and small.
pub(super) fn turn_time(key: u64, time: Option<i64>, cx: &App) -> Option<AnyElement> {
    let time = Local.timestamp_millis_opt(time?).single()?;
    let element = div()
        .id(ElementId::NamedInteger("turn-time".into(), key))
        .flex_none()
        .px_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(short(time, Local::now()))
        .tip_text(full(time));
    Some(element.into_any_element())
}

fn short(time: DateTime<Local>, now: DateTime<Local>) -> String {
    time.format(if time.date_naive() == now.date_naive() {
        "%H:%M"
    } else {
        "%H:%M %d.%m.%Y"
    })
    .to_string()
}

/// Full source date and seconds remain available in the tooltip.
fn full(time: DateTime<Local>) -> String {
    time.format("%H:%M:%S %d.%m.%Y").to_string()
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Local, TimeZone as _};

    use super::{full, short};

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, 5)
            .single()
            .expect("unambiguous local time")
    }

    #[test]
    fn today_has_only_time_and_other_days_have_time_before_full_date() {
        let now = at(2026, 10, 22, 23, 59);
        assert_eq!(short(at(2026, 10, 22, 14, 32), now), "14:32");
        assert_eq!(short(at(2026, 10, 21, 14, 32), now), "14:32 21.10.2026");
        assert_eq!(short(at(2025, 10, 22, 14, 32), now), "14:32 22.10.2025");
        assert_eq!(short(at(2026, 10, 23, 0, 0), now), "00:00 23.10.2026");
    }

    #[test]
    fn full_time_has_seconds() {
        assert_eq!(full(at(2026, 10, 22, 14, 32)), "14:32:05 22.10.2026");
    }

    fn transcript(roles: &[&str]) -> crate::transcript::Transcript {
        let mut transcript = crate::transcript::Transcript::default();
        transcript.reset(
            roles
                .iter()
                .enumerate()
                .map(
                    |(ix, role)| proteus_contracts::app_protocol::AppTranscriptMessage {
                        timestamp_ms: Some(ix as i64 + 100),
                        message_id: Some(uuid::Uuid::from_u128(ix as u128 + 1)),
                        phase: None,
                        role: (*role).into(),
                        text: "fixture".into(),
                        images: vec![],
                        tool: (*role == "system").then(|| {
                            proteus_contracts::app_protocol::AppTranscriptTool {
                                call_id: format!("tool-{ix}"),
                                name: "read_file".into(),
                                args: serde_json::json!({"path": "fixture"}),
                                effective_args: None,
                                status: "done".into(),
                                result: Some("fixture".into()),
                                metadata: serde_json::Value::Null,
                                terminal: None,
                            }
                        }),
                        subagent: None,
                        streaming: false,
                    },
                )
                .collect(),
        );
        transcript
    }

    #[test]
    fn one_stamp_after_all_replies_and_tools_of_each_turn() {
        let transcript = transcript(&[
            "user",
            "assistant",
            "system",
            "assistant",
            "user",
            "assistant",
        ]);
        let rows = transcript.entries();
        let stamps: Vec<_> = (1..=rows.len())
            .filter_map(|end| super::time_after(rows, end, true).map(|time| (end, time)))
            .collect();
        assert_eq!(stamps, vec![(4, 103), (6, 105)]);
        assert_eq!(
            super::time_after(rows, 6, false),
            None,
            "running or unloaded tail"
        );
        assert_eq!(
            super::time_after(rows, 4, false),
            Some(103),
            "earlier complete turn"
        );
    }

    #[test]
    fn tool_only_turn_and_folded_run_share_one_ending() {
        let transcript = transcript(&["user", "system", "system", "system"]);
        let rows = transcript.entries();
        assert_eq!(crate::transcript::tool_chain(rows, 1), Some(1..4));
        assert_eq!(super::time_after(rows, 2, true), None);
        assert_eq!(super::time_after(rows, 3, true), None);
        assert_eq!(super::time_after(rows, 4, true), Some(103));
        assert_eq!(super::time_after(rows, 4, false), None);
    }

    #[test]
    fn duplicate_subagent_progress_does_not_hide_the_turn_ending() {
        let transcript = transcript(&["system", "system", "user"]);
        let mut rows = transcript.entries().to_vec();
        rows[0].tool = None;
        rows[0].subagent = Some(proteus_contracts::app_protocol::AppTranscriptSubagent {
            child_thread_id: "child".into(),
            session: None,
            role: "worker".into(),
            description: None,
            status: "completed".into(),
            iterations: None,
            tools: vec![],
        });
        rows[1].tool.as_mut().unwrap().metadata = serde_json::json!({
            "tool": "task", "child_thread_id": "child"
        });
        assert!(super::hidden(&rows, &rows[1]));
        assert_eq!(super::time_after(&rows, 1, false), Some(100));
    }

    #[test]
    fn streaming_and_unanswered_user_have_no_end_stamp() {
        let mut transcript = transcript(&["user", "assistant"]);
        assert_eq!(super::time_after(transcript.entries(), 1, true), None);
        // Streaming state comes from the same public snapshot used by render.
        let mut message = proteus_contracts::app_protocol::AppTranscriptMessage {
            timestamp_ms: Some(100),
            message_id: None,
            phase: None,
            role: "assistant".into(),
            text: "stream".into(),
            images: vec![],
            tool: None,
            subagent: None,
            streaming: true,
        };
        transcript.reset(vec![message.clone()]);
        assert_eq!(super::time_after(transcript.entries(), 1, true), None);
        message.streaming = false;
        message.timestamp_ms = None;
        message.text = "unknown time".into();
        transcript.reset(vec![message]);
        assert!(super::ending(transcript.entries(), 1, true));
        assert_eq!(super::time_after(transcript.entries(), 1, true), None);
    }
}
