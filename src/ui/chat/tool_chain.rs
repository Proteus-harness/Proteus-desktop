//! A folded run of tool calls: one line with the count, the distinct actions
//! and the most urgent status; a click shows every call again.

use std::ops::Range;

use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::app_protocol::AppTranscriptTool;

use super::Chat;
use super::tool_row::{is_running, status_icon, tool_args};
use crate::tool_display;
use crate::transcript::{affected_rows, tool_chain};
use crate::ui::fade::faded;
use crate::ui::kit::Pressable as _;
use crate::ui::motion::{self, chevron};

impl Chat {
    pub(super) fn chain_open(&self, head: u64) -> bool {
        self.open_chains.contains(&head)
    }

    fn toggle_chain(&mut self, head: u64, cx: &mut Context<Self>) {
        let open = !self.open_chains.remove(&head);
        if open {
            self.open_chains.insert(head);
        }
        motion::toggled(chain_reveal_key(cx.entity_id(), head), open, cx);
        let entries = self.display_transcript().entries();
        if let Some(row) = entries.iter().position(|entry| entry.key == head) {
            let rows = affected_rows(entries, row);
            self.scroller
                .update(cx, |scroller, cx| scroller.toggled(rows, cx));
        }
        cx.notify();
    }

    /// Folded run around `ix`, when compact chains are on.
    pub(super) fn chain_at(&self, ix: usize, compact: bool) -> Option<Range<usize>> {
        compact
            .then(|| tool_chain(self.display_transcript().entries(), ix))
            .flatten()
    }

    /// Remeasures rows a change of row `ix` may fold or unfold.
    pub(super) fn remeasure_chain(&mut self, ix: usize, cx: &mut Context<Self>) {
        let rows = affected_rows(self.display_transcript().entries(), ix);
        if rows.len() > 1 {
            self.scroller
                .update(cx, |scroller, cx| scroller.remeasure_items(rows, cx));
        }
    }
}

/// Motion key of the run starting at row `head`; row keys repeat across chats.
pub(super) fn chain_reveal_key(chat: EntityId, head: u64) -> SharedString {
    format!("tool-chain-{chat}-{head}").into()
}

pub(super) fn render_chain_header(
    head: u64,
    tools: &[&AppTranscriptTool],
    open: bool,
    cx: &mut Context<Chat>,
) -> AnyElement {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let mut labels: Vec<String> = Vec::new();
    for tool in tools {
        let label = tool_display::headline(&tool.name, tool_args(tool)).label;
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    let summary = labels.join(", ");
    h_flex()
        .id(ElementId::NamedInteger("tool-chain".into(), head))
        .gap_2()
        .text_sm()
        .min_w_0()
        .disclosure(cx)
        .child(status_icon(chain_status(tools), cx))
        .child(
            div()
                .flex_none()
                .child(format!("Инструменты · {}", tools.len())),
        )
        .child(div().min_w_0().text_color(muted).child(faded(summary)))
        .child(chevron(
            ElementId::NamedInteger("tool-chain-chevron".into(), head),
            open,
            muted,
        ))
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_chain(head, cx)))
        .into_any_element()
}

/// Running wins over waiting, waiting over failure, failure over an
/// interrupted call, and that over success.
fn chain_status(tools: &[&AppTranscriptTool]) -> &'static str {
    let has = |wanted: &[&str]| {
        tools
            .iter()
            .any(|tool| wanted.contains(&tool.status.as_str()))
    };
    if tools.iter().any(|tool| is_running(&tool.status)) {
        "running"
    } else if has(&["waiting_approval"]) {
        "waiting_approval"
    } else if has(&["failed", "denied"]) {
        "failed"
    } else if tools.iter().any(|tool| tool.status != "done") {
        "interrupted"
    } else {
        "done"
    }
}
