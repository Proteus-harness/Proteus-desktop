use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::app_protocol::AppTranscriptTool;

use super::Chat;
use super::requests::panel_card;
use super::tool_row::{plan_list, tool_args};
use crate::tool_display::{self, StepStatus};
use crate::transcript::{Entry, Role};
use crate::ui::fade::faded;
use crate::ui::kit::Pressable as _;
use crate::ui::motion::{self, chevron, reveal};

impl Chat {
    /// The plan of the current turn above the composer: a line with the
    /// step at work, the whole list when opened. It goes away once the
    /// turn ends with every step done.
    pub(super) fn render_plan(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tool = current_plan(self.session.transcript.entries())?;
        let steps = tool_display::plan_steps(tool_args(tool));
        let done = steps
            .iter()
            .filter(|step| step.status == StepStatus::Completed)
            .count();
        if steps.is_empty() || (done == steps.len() && !self.session.running()) {
            return None;
        }
        let theme = cx.theme();
        let open = self.plan_open;
        let current = steps
            .iter()
            .find(|step| step.status == StepStatus::InProgress)
            .or_else(|| steps.iter().find(|step| step.status == StepStatus::Pending))
            .map(|step| step.step.clone());
        let key = plan_reveal_key(cx.entity_id());
        let header = h_flex()
            .id("plan-panel")
            .gap_2()
            .text_sm()
            .cursor_pointer()
            .pressable(cx)
            .child(
                Icon::new(Lucide::ListTodo)
                    .small()
                    .text_color(theme.muted_foreground),
            )
            .child(div().flex_none().font_medium().child("План"))
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{done}/{}", steps.len())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .children((!open).then(|| current.map(faded)).flatten()),
            )
            .child(chevron("plan-panel-chevron", open, theme.muted_foreground))
            .on_click(cx.listener(|this, _, _, cx| {
                this.plan_open = !this.plan_open;
                motion::toggled(plan_reveal_key(cx.entity_id()), this.plan_open, cx);
                cx.notify();
            }));
        let app: &App = cx;
        let list = reveal(key, open, app, || {
            plan_list(&steps, app).pt_2().pl_6().into_any_element()
        });
        Some(
            panel_card(cx)
                .gap_0()
                .py_2()
                .child(header)
                .children(list)
                .into_any_element(),
        )
    }
}

fn plan_reveal_key(chat: EntityId) -> SharedString {
    format!("plan-panel-{chat}").into()
}

/// The latest plan the agent made since the user last wrote.
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
