use crate::ui::kit::Frame as _;
use std::ops::Range;

use crate::ui::kit::Clipboard;
use gpui_kit::base::SelectableText;
use gpui_kit::component::text::TextView;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use proteus_contracts::app_protocol::AppTranscriptSubagent;

use super::Chat;
use super::images::{SentImages, render_sent_images};
use super::stamp::message_time;
use super::tool_chain::{chain_reveal_key, render_chain_header};
use super::tool_row::{render_tool, status_icon, tool_line};
use crate::transcript::{Entry, Role};
use crate::ui::fade::faded;
use crate::ui::kit::Pressable as _;
use crate::ui::kit::{self, Hint as _};
use crate::ui::motion::{self, chevron, fade_in_if, reveal};
use crate::ui::{format, prefs};

impl Chat {
    pub(super) fn toggle_expanded(&mut self, key: u64, cx: &mut Context<Self>) {
        let open = !self.expanded.remove(&key);
        if open {
            self.expanded.insert(key);
        }
        motion::toggled(reveal_key(cx.entity_id(), key), open, cx);
        if let Some(row) = self.row_of(key) {
            self.scroller
                .update(cx, |scroller, cx| scroller.toggled(row..row + 1, cx));
        }
        cx.notify();
    }

    /// Opens just the chosen result/argument block, keeping its preview.
    pub(super) fn toggle_full(&mut self, key: u64, part: usize, cx: &mut Context<Self>) {
        let block = (key, part);
        let open = !self.full_blocks.remove(&block);
        if open {
            self.full_blocks.insert(block);
        }
        motion::toggled(
            super::tool_row::full_reveal_key(cx.entity_id(), key, part),
            open,
            cx,
        );
        if let Some(row) = self.row_of(key) {
            self.scroller
                .update(cx, |scroller, cx| scroller.toggled(row..row + 1, cx));
        }
        cx.notify();
    }

    /// Measures the row of entry `key` again on the next layout.
    pub(super) fn remeasure_row(&mut self, key: u64, cx: &mut Context<Self>) {
        if let Some(row) = self.row_of(key) {
            self.scroller.update(cx, |scroller, cx| {
                scroller.remeasure_items(row..row + 1, cx)
            });
        }
        cx.notify();
    }

    pub(super) fn row_of(&self, key: u64) -> Option<usize> {
        self.display_transcript()
            .entries()
            .iter()
            .position(|entry| entry.key == key)
    }

    pub(super) fn render_row(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entries = self.history.displayed(&self.session.transcript).entries();
        let Some(entry) = entries.get(ix) else {
            return div().into_any_element();
        };
        let compact = prefs::current(cx).view.compact_tools;
        let chain = self.chain_at(ix, compact);
        let chain_head = chain.as_ref().map(|chain| entries[chain.start].key);
        let chain_open = chain_head.is_some_and(|head| self.chain_open(head));
        let chain_reveal = chain_head.map(|head| chain_reveal_key(cx.entity_id(), head));
        // Calls after the head are rows of their own while the run rests
        // open; while it opens or closes the head draws the whole run as one
        // block, so it grows like any other disclosure.
        let chain_moving = chain_reveal
            .as_ref()
            .is_some_and(|key| motion::moving(key, chain_open, cx));
        let chain_tail = chain.as_ref().is_some_and(|chain| ix != chain.start);
        if chain_tail && (!chain_open || chain_moving) {
            return div().into_any_element();
        }
        let missing = self.sent_images.missing(&entry.images);
        let expanded = self.expanded.contains(&entry.key);
        let full = [0, 1].map(|part| self.full_blocks.contains(&(entry.key, part)));
        let fresh = self.fresh_rows.is_fresh(entry.key);
        let key = entry.key;
        // A run drawn by its head alone ends where its last call does.
        let last = match &chain {
            Some(chain) if !chain_open || chain_moving => chain.end == entries.len(),
            _ => ix + 1 == entries.len(),
        };
        let spacing = match (&entry.role, &entry.tool) {
            (Role::User, _) => Spacing {
                top: 4.,
                bottom: 0.,
            },
            (Role::Assistant, _) => Spacing {
                top: 2.,
                bottom: 0.,
            },
            (Role::System, Some(_)) => TOOL_SPACING,
            (Role::System, None) => Spacing {
                top: 2.,
                bottom: 2.,
            },
        };
        let body = match (&entry.role, &entry.tool, &entry.subagent) {
            (Role::User, _, _) => render_user(entry, &self.sent_images, cx),
            (Role::Assistant, _, _) => {
                let reply = self.replies.parts(
                    entry.key,
                    &entry.text,
                    self.workspace.clone(),
                    self.session.dir.clone(),
                    window,
                    cx,
                );
                render_assistant(entry, reply, cx)
            }
            (Role::System, Some(tool), _) => match &chain {
                Some(chain) if ix == chain.start => {
                    let tools: Vec<_> = entries[chain.clone()]
                        .iter()
                        .filter_map(|entry| entry.tool.as_ref())
                        .collect();
                    let header = render_chain_header(key, &tools, chain_open, cx);
                    let calls = if chain_moving {
                        let run = self.render_run(chain.clone(), cx);
                        reveal(chain_reveal.unwrap_or_default(), chain_open, cx, || run)
                    } else {
                        chain_open.then(|| {
                            let tool = render_tool(
                                entry.key,
                                tool,
                                expanded,
                                full,
                                self.tool_diffs.get(&entry.key).cloned(),
                                cx,
                            );
                            div().pt_1().child(tool).into_any_element()
                        })
                    };
                    v_flex().child(header).children(calls).into_any_element()
                }
                _ => render_tool(
                    entry.key,
                    tool,
                    expanded,
                    full,
                    self.tool_diffs.get(&entry.key).cloned(),
                    cx,
                ),
            },
            (Role::System, None, Some(agent)) => render_subagent(entry.key, agent, expanded, cx),
            (Role::System, None, None) => render_note(entry, cx),
        };
        if !missing.is_empty() {
            self.fetch_images(missing, cx);
        }
        h_flex()
            .w_full()
            .justify_center()
            .px_6()
            .pt(px(spacing.top * 4.))
            .pb(px(spacing.bottom * 4.))
            .when(ix == 0, |row| row.pt_6())
            .when(last, |row| row.pb_8())
            .child(fade_in_if(
                div().w_full().max_w(prefs::chat_width(cx)).child(body),
                row_id("enter", key),
                fresh,
            ))
            .into_any_element()
    }

    /// Every call of a run as one block, spaced as their rows are at rest,
    /// so the block ends exactly where the rows will.
    fn render_run(&self, run: Range<usize>, cx: &mut Context<Self>) -> AnyElement {
        let entries = self.history.displayed(&self.session.transcript).entries();
        let gap = px((TOOL_SPACING.bottom + TOOL_SPACING.top) * 4.);
        let mut block = v_flex();
        for (n, entry) in entries[run].iter().enumerate() {
            let Some(tool) = &entry.tool else {
                continue;
            };
            let expanded = self.expanded.contains(&entry.key);
            let full = [0, 1].map(|part| self.full_blocks.contains(&(entry.key, part)));
            let call = div().child(render_tool(
                entry.key,
                tool,
                expanded,
                full,
                self.tool_diffs.get(&entry.key).cloned(),
                cx,
            ));
            block = block.child(if n == 0 { call.pt_1() } else { call.pt(gap) });
        }
        block.into_any_element()
    }

    pub(super) fn prepare_tool_diffs(&mut self, cx: &mut Context<Self>) {
        for entry in self.history.displayed(&self.session.transcript).entries() {
            if !self.expanded.contains(&entry.key) {
                continue;
            }
            let Some(tool) = &entry.tool else {
                continue;
            };
            if tool.name != crate::tool_display::APPLY_PATCH {
                continue;
            }
            let Some(patch) = crate::tool_display::patch_text(super::tool_row::tool_args(tool))
            else {
                continue;
            };
            if let Some(diff) = self.tool_diffs.get(&entry.key) {
                diff.update(cx, |diff, cx| diff.set_patch(patch, &[], cx));
            } else {
                let diff =
                    cx.new(|cx| crate::ui::diff::DiffView::new(patch.to_owned(), Vec::new(), cx));
                self.tool_diffs.insert(entry.key, diff);
            }
        }
    }
}

/// Vertical padding of a row in 4px units: tool calls sit tightly together,
/// a user turn opens with extra space.
struct Spacing {
    top: f32,
    bottom: f32,
}

const TOOL_SPACING: Spacing = Spacing {
    top: 1.,
    bottom: 1.,
};

fn row_id(name: &'static str, key: u64) -> ElementId {
    ElementId::NamedInteger(name.into(), key)
}

/// Motion key of a row's expandable details; row keys repeat across chats.
pub(super) fn reveal_key(chat: EntityId, key: u64) -> SharedString {
    format!("thread-row-{chat}-{key}").into()
}

fn copy_button(name: &'static str, key: u64, text: SharedString, cx: &App) -> Clipboard {
    Clipboard::new(row_id(name, key))
        .value(text)
        .hint("Копировать", cx)
        .xsmall()
}

fn render_user(entry: &Entry, images: &SentImages, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let text = SharedString::from(&entry.text);
    let has_text = !entry.text.trim().is_empty();
    v_flex()
        .group(ROW_GROUP)
        .w_full()
        .items_end()
        .gap_0p5()
        .children(render_sent_images(&entry.images, images, cx))
        .when(has_text, |row| {
            row.child(
                v_flex()
                    .max_w(relative(0.85))
                    .px_3()
                    .py_2()
                    .gap_1()
                    .rounded(theme.radius_lg)
                    .bg(theme.secondary)
                    .text_color(theme.secondary_foreground)
                    .child(
                        div()
                            .text_size(prefs::chat_text(cx))
                            .child(SelectableText::new(row_id("user", entry.key), text.clone())),
                    ),
            )
        })
        .when(has_text, |row| {
            row.child(
                row_actions()
                    .children(message_time(entry.key, entry.time, cx))
                    .child(copy_button("copy-user", entry.key, text, cx)),
            )
        })
        .into_any_element()
}

fn render_assistant(entry: &Entry, reply: Vec<super::replies::Part>, cx: &App) -> AnyElement {
    let text = SharedString::from(&entry.text);
    let actions = (!entry.streaming && !entry.text.trim().is_empty()).then(|| {
        row_actions()
            .child(copy_button("copy-assistant", entry.key, text.clone(), cx))
            .children(message_time(entry.key, entry.time, cx))
    });
    let copy_hint = kit::hint_text("Копировать код", None, cx);
    v_flex()
        .group(ROW_GROUP)
        .w_full()
        .text_size(prefs::chat_text(cx))
        .children(reply.into_iter().map(|part| match part {
            super::replies::Part::Content(view) => view.into_any_element(),
            super::replies::Part::Markdown(state) => {
                let copy_hint = copy_hint.clone();
                TextView::new(&state)
                    .selectable(true)
                    .stream_fade(true)
                    .code_block_actions(move |block, _, _| {
                        let code = block.code();
                        let copy =
                            Clipboard::new(row_id("copy-code", hash_code(&code))).value(code);
                        match &copy_hint {
                            Some(hint) => copy.tooltip(hint.clone()),
                            None => copy,
                        }
                        .xsmall()
                    })
                    .into_any_element()
            }
        }))
        .children(actions)
        .into_any_element()
}

const ROW_GROUP: &str = "thread-row";

/// Row actions stay laid out so revealing them on hover does not shift rows.
fn row_actions() -> Div {
    h_flex()
        .h_5()
        .gap_1()
        .invisible()
        .group_hover(ROW_GROUP, |style| style.visible())
}

fn hash_code(code: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    code.hash(&mut hasher);
    hasher.finish()
}

fn render_note(entry: &Entry, cx: &App) -> AnyElement {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .min_w_0()
        .child(faded(format::clip(&entry.text, 200)))
        .into_any_element()
}

fn render_subagent(
    key: u64,
    agent: &AppTranscriptSubagent,
    expanded: bool,
    cx: &mut Context<Chat>,
) -> AnyElement {
    let theme = cx.theme();
    let mut meta = format!("инструментов: {}", agent.tools.len());
    if let Some(iterations) = agent.iterations {
        meta.push_str(&format!(" · шагов: {iterations}"));
    }
    // Agent lifecycle statuses mapped onto the tool status icons.
    let status = match agent.status.as_str() {
        "running" => "running",
        "completed" | "done" => "done",
        "errored" | "failed" => "failed",
        _ => "interrupted",
    };
    let app: &App = cx;
    let tools = reveal(reveal_key(cx.entity_id(), key), expanded, app, || {
        v_flex()
            .pt_2()
            .pl_5()
            .gap_1()
            .children(agent.tools.iter().map(|tool| tool_line(tool, app)))
            .into_any_element()
    });
    v_flex()
        .px_3()
        .py_2()
        .surface_frame(cx)
        .child(
            h_flex()
                .id(ElementId::NamedInteger("subagent".into(), key))
                .gap_2()
                .text_sm()
                .disclosure(cx)
                .child(status_icon(status, cx))
                .child(Icon::new(IconName::Bot).small())
                .child(div().flex_none().font_semibold().child(agent.role.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(theme.muted_foreground)
                        .child(faded(agent.description.clone().unwrap_or_default())),
                )
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(meta),
                )
                .child(chevron(
                    row_id("subagent-chevron", key),
                    expanded,
                    theme.muted_foreground,
                ))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_expanded(key, cx))),
        )
        .children(tools)
        .into_any_element()
}
