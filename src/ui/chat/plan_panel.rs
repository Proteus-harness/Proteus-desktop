// Адаптировано из Zed: crates/agent_ui/src/conversation_view/thread_view.rs
// (activity bar's plan summary and retained disclosure above the editor).
use proteus_contracts::app_protocol::AppTranscriptTool;

use super::Chat;
use super::tool_row::tool_args;
use crate::tool_display::{self, PlanStep, StepStatus};
use crate::transcript::{Entry, Role};

/// A projection of the real update_plan transcript DTO, never a client-made
/// workflow. The strip and its details share this exact snapshot.
pub(super) struct ActivePlan {
    pub steps: Vec<PlanStep>,
    pub done: usize,
}

impl ActivePlan {
    pub fn current(&self) -> Option<&str> {
        self.steps
            .iter()
            .find(|step| step.status == StepStatus::InProgress)
            .or_else(|| {
                self.steps
                    .iter()
                    .find(|step| step.status == StepStatus::Pending)
            })
            .map(|step| step.step.as_str())
    }
}

impl Chat {
    pub(super) fn active_plan(&self) -> Option<ActivePlan> {
        let tool = current_plan(self.session.transcript.entries())?;
        let steps = tool_display::plan_steps(tool_args(tool));
        let done = steps
            .iter()
            .filter(|step| step.status == StepStatus::Completed)
            .count();
        if steps.is_empty() || (done == steps.len() && !self.session.running()) {
            return None;
        }
        Some(ActivePlan { steps, done })
    }
}

/// The latest plan the agent made since the user last wrote. Old tool rows
/// stay in the thread; only the current turn contributes an active strip.
fn current_plan(entries: &[Entry]) -> Option<&AppTranscriptTool> {
    entries
        .iter()
        .rev()
        .take_while(|entry| entry.role != Role::User)
        .filter_map(|entry| entry.tool.as_ref())
        .find(|tool| {
            tool.name == tool_display::UPDATE_PLAN
                && !matches!(tool.status.as_str(), "failed" | "denied")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(id: &str, status: &str) -> Entry {
        Entry {
            key: 0,
            message_id: None,
            role: Role::Assistant,
            text: "".into(),
            images: Vec::new(),
            tool: Some(AppTranscriptTool {
                call_id: id.into(),
                name: tool_display::UPDATE_PLAN.into(),
                args: serde_json::json!({ "plan": [{ "step": "Current step", "status": "in_progress" }] }),
                effective_args: None,
                status: status.into(),
                result: None,
                metadata: serde_json::Value::Null,
            }),
            subagent: None,
            streaming: false,
            time: None,
        }
    }

    #[test]
    fn active_plan_comes_from_the_current_turn_and_preserves_the_last_accepted_update() {
        let mut entries = vec![tool("accepted", "completed"), tool("rejected", "denied")];
        assert_eq!(current_plan(&entries).unwrap().call_id, "accepted");
        let mut user = tool("unused", "completed");
        user.role = Role::User;
        user.tool = None;
        entries.push(user);
        assert!(current_plan(&entries).is_none());
        entries.push(tool("next-turn", "completed"));
        assert_eq!(current_plan(&entries).unwrap().call_id, "next-turn");
    }
}
