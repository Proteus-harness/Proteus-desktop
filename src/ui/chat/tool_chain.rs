//! A folded run of tool calls: one line with the count, the distinct actions
//! and the most urgent status; a click shows every call again.

use std::ops::Range;

use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::app_protocol::AppTranscriptTool;

use super::Chat;
use super::tool_row::{is_running, status_icon};
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
}

/// Motion key of the run starting at row `head`; row keys repeat across chats.
pub(super) fn chain_reveal_key(chat: EntityId, head: u64) -> SharedString {
    format!("tool-chain-{chat}-{head}").into()
}

/// Geometry only, bounded like the chat's scroll diagnostics. Comparing the
/// last combined row with the settled virtual rows catches endpoint jumps.
#[cfg(debug_assertions)]
pub(super) fn trace_bounds(
    head: u64,
    row: u64,
    moving: bool,
    frame: std::time::Instant,
    bounds: Bounds<Pixels>,
) {
    trace_geometry("row", head, row, moving, frame, bounds);
}

#[cfg(debug_assertions)]
pub(super) fn trace_run(head: u64, frame: std::time::Instant, bounds: Bounds<Pixels>) {
    trace_geometry("natural-run", head, head, true, frame, bounds);
}

#[cfg(debug_assertions)]
fn trace_geometry(
    kind: &'static str,
    head: u64,
    row: u64,
    moving: bool,
    frame: std::time::Instant,
    bounds: Bounds<Pixels>,
) {
    use std::sync::{
        LazyLock,
        atomic::{AtomicUsize, Ordering},
    };
    static ENABLED: LazyLock<bool> =
        LazyLock::new(|| std::env::var("PROTEUS_SCROLL_TRACE").as_deref() == Ok("1"));
    static START: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    if !*ENABLED || COUNT.fetch_add(1, Ordering::Relaxed) >= 6000 {
        return;
    }
    eprintln!(
        "PROTEUS_TOOL_CHAIN {}",
        serde_json::json!({
            "kind":kind, "head":head, "row":row, "moving":moving,
            "frame_ms":frame.saturating_duration_since(*START).as_secs_f64()*1000.,
            "top":f32::from(bounds.top()), "bottom":f32::from(bounds.bottom()),
            "height":f32::from(bounds.size.height),
        })
    );
}

pub(super) fn render_chain_header(
    head: u64,
    tools: &[&AppTranscriptTool],
    open: bool,
    connected: bool,
    cx: &mut Context<Chat>,
) -> AnyElement {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let mut labels: Vec<(&str, usize)> = Vec::new();
    for tool in tools {
        if let Some((_, count)) = labels.iter_mut().find(|(name, _)| *name == tool.name) {
            *count += 1;
        } else {
            labels.push((&tool.name, 1));
        }
    }
    let weak = cx.entity().downgrade();
    h_flex()
        .id(ElementId::NamedInteger("tool-chain".into(), head))
        .gap_2()
        .text_sm()
        .min_w_0()
        .inline_disclosure(cx)
        .child(status_icon(chain_status(tools), connected, cx))
        .child(
            div()
                .flex_none()
                .child(format!("Инструменты · {}", tools.len())),
        )
        .child(
            h_flex()
                .min_w_0()
                .gap_2()
                .overflow_hidden()
                .children(labels.into_iter().map(|(name, count)| {
                    let label = if count == 1 {
                        name.to_owned()
                    } else {
                        format!("{name} ×{count}")
                    };
                    super::tool_hint::group(
                        div()
                            .id(SharedString::from(format!("tool-chain-name-{head}-{name}")))
                            .flex_initial()
                            .min_w_0()
                            .text_color(muted)
                            .child(faded(label)),
                        weak.clone(),
                        head,
                        name.to_owned(),
                    )
                })),
        )
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
