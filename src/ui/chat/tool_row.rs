// Адаптировано из Zed: crates/agent_ui/src/conversation_view/thread_view.rs
// (bounded tool previews and output disclosure).
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::SelectableText;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use proteus_contracts::app_protocol::AppTranscriptTool;
use serde_json::Value;

use super::Chat;
use crate::tool_display::{self, PlanStep, StepStatus};
use crate::ui::fade::faded;
use crate::ui::format;
use crate::ui::kit::{self, Hint as _, Pressable as _};
use crate::ui::motion::{Spinner, chevron, preview_reveal, reveal};

/// Lines of a long block shown until the user asks for all of them.
const PEEK_LINES: usize = 12;
const PEEK_CHARS: usize = 2_000;
/// Even a block shown in full stops somewhere.
const FULL_LINES: usize = 400;
const FULL_CHARS: usize = 40_000;
/// Subjects longer than this are repeated in full when the row is expanded.
const SHORT_SUBJECT: usize = 100;

fn row_id(name: &'static str, key: u64) -> ElementId {
    ElementId::NamedInteger(name.into(), key)
}

pub(in crate::ui) fn tool_args(tool: &AppTranscriptTool) -> &Value {
    tool.effective_args.as_ref().unwrap_or(&tool.args)
}

pub(super) fn status_icon(status: &str, cx: &App) -> AnyElement {
    let theme = cx.theme();
    match status {
        "done" => Icon::new(IconName::Check)
            .xsmall()
            .text_color(theme.success)
            .into_any_element(),
        "failed" | "denied" => Icon::new(IconName::CircleX)
            .xsmall()
            .text_color(theme.danger)
            .into_any_element(),
        "waiting_approval" => Icon::new(IconName::CircleAlert)
            .xsmall()
            .text_color(theme.warning)
            .into_any_element(),
        status if is_running(status) => Spinner::new().xsmall().into_any_element(),
        // Interrupted, canceled or unknown: nothing will finish it any more.
        _ => Icon::new(Lucide::CircleStop)
            .xsmall()
            .text_color(theme.muted_foreground)
            .into_any_element(),
    }
}

pub(super) fn is_running(status: &str) -> bool {
    matches!(status, "running" | "approved")
}

/// One-line headline: status, action label, subject and patch stats.
pub(super) fn tool_line(tool: &AppTranscriptTool, cx: &App) -> Div {
    let theme = cx.theme();
    let headline = tool_display::headline(&tool.name, tool_args(tool));
    let subject = headline.subject.map(|subject| {
        div()
            .min_w_0()
            .font_family(theme.mono_font_family.clone())
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(faded(subject))
    });
    h_flex()
        .gap_2()
        .text_sm()
        .min_w_0()
        .child(status_icon(&tool.status, cx))
        .child(div().flex_none().child(headline.label))
        .children(subject)
        .children(
            headline
                .stats
                .filter(|stats| *stats != (0, 0))
                .map(|stats| diff_stats(stats, cx)),
        )
}

fn diff_stats((added, removed): (usize, usize), cx: &App) -> Div {
    crate::ui::diff::stat(added, removed, cx)
}

pub(super) fn render_tool(
    key: u64,
    tool: &AppTranscriptTool,
    expanded: bool,
    full: [bool; 2],
    diff: Option<Entity<crate::ui::diff::DiffView>>,
    cx: &mut Context<Chat>,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let fold = Fold {
        key,
        full,
        chat: cx.entity().downgrade(),
    };
    let app: &App = cx;
    // The gap above the details is part of them, so it opens and closes
    // with them instead of jumping at the end.
    let details = reveal(
        super::thread::reveal_key(cx.entity_id(), key),
        expanded,
        app,
        || {
            tool_details(&fold, tool, diff, app)
                .pt_1()
                .into_any_element()
        },
    );
    v_flex()
        .child(
            tool_line(tool, cx)
                .id(row_id("tool", key))
                .disclosure(cx)
                .child(chevron(row_id("tool-chevron", key), expanded, muted))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_expanded(key, cx))),
        )
        .children(details)
        .into_any_element()
}

/// Long blocks of one row are shown cut, with a fade, until the user asks
/// for all of them.
struct Fold {
    key: u64,
    full: [bool; 2],
    chat: WeakEntity<Chat>,
}

impl Fold {
    /// `block` built from `text`; a long one is cut short unless shown in
    /// full, with a button switching between the two. `part` tells the
    /// blocks of a row apart.
    fn block(&self, part: usize, text: &str, build: impl Fn(String) -> Div, cx: &App) -> Div {
        let lines = text.lines().count();
        let long = lines > PEEK_LINES || text.len() > PEEK_CHARS;
        if !long {
            return build(text.trim_end().to_owned());
        }
        let full = self.full.get(part).copied().unwrap_or(false);
        let preview = format::peek_block(text, PEEK_LINES, PEEK_CHARS);
        let peeked = preview.lines().count();
        let theme = cx.theme();
        let chat = self.chat.clone();
        let copy_chat = self.chat.clone();
        let key = self.key;
        let label = if full {
            "Свернуть".to_owned()
        } else {
            match lines.saturating_sub(peeked) {
                0 => "Показать больше".to_owned(),
                hidden => format!("Показать больше · ещё строк: {hidden}"),
            }
        };
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .id(ElementId::NamedInteger(
                                format!("tool-more-{part}").into(),
                                key,
                            ))
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .text_xs()
                            .disclosure(cx)
                            .child(chevron(
                                ElementId::NamedInteger(
                                    format!("tool-more-chevron-{part}").into(),
                                    key,
                                ),
                                full,
                                theme.muted_foreground,
                            ))
                            .child(faded(label))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                chat.update(cx, |chat, cx| chat.toggle_full(key, part, cx))
                                    .ok();
                            }),
                    )
                    .child(
                        kit::button(ElementId::NamedInteger(
                            format!("tool-copy-{part}").into(),
                            key,
                        ))
                        .icon(IconName::Copy)
                        .hint("Копировать весь текст", cx)
                        .ghost()
                        .xsmall()
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            copy_chat
                                .update(cx, |chat, cx| chat.copy_tool_block(key, part, cx))
                                .ok();
                        }),
                    ),
            )
            // Keep the trigger above the changing height. A bottom control
            // disappears above the viewport when a long output collapses,
            // even though the scroller correctly retains the freed room.
            .child(preview_reveal(
                full_reveal_key(self.chat.entity_id(), key, part),
                full,
                cx,
                div()
                    .relative()
                    .child(build(preview))
                    .child(bottom_fade(theme.muted, theme.radius)),
                build(format::clip_block(text, FULL_LINES, FULL_CHARS)),
            ))
    }
}

pub(super) fn full_reveal_key(chat: EntityId, key: u64, part: usize) -> SharedString {
    format!("tool-preview-{chat}-{key}-{part}").into()
}

impl Chat {
    /// Copy the source instead of the bounded preview; fetch it at click
    /// time, so large tool results are not cloned for a toolbar every frame.
    fn copy_tool_block(&self, key: u64, part: usize, cx: &mut Context<Self>) {
        let Some(tool) = self
            .session
            .transcript
            .entries()
            .iter()
            .find(|entry| entry.key == key)
            .and_then(|entry| entry.tool.as_ref())
        else {
            return;
        };
        let text = match part {
            0 => tool.result.as_deref(),
            1 => {
                let args = tool_args(tool);
                tool_display::headline(&tool.name, args)
                    .subject_key
                    .as_deref()
                    .and_then(|key| args.get(key))
                    .and_then(Value::as_str)
            }
            _ => None,
        };
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        }
    }
}

/// The end of a cut block melts into its background.
fn bottom_fade(background: Hsla, radius: Pixels) -> Div {
    crate::ui::fade::bottom(background, crate::ui::prefs::scaled(56.)).rounded_b(radius)
}

fn tool_details(
    fold: &Fold,
    tool: &AppTranscriptTool,
    diff: Option<Entity<crate::ui::diff::DiffView>>,
    cx: &App,
) -> Div {
    let key = fold.key;
    let args = tool_args(tool);
    let details = v_flex().pl_5().gap_2();
    let patch = (tool.name == tool_display::APPLY_PATCH)
        .then(|| tool_display::patch_text(args))
        .flatten()
        .map(tool_display::patch_files)
        .filter(|files| !files.is_empty());
    let plan = (tool.name == tool_display::UPDATE_PLAN)
        .then(|| tool_display::plan_steps(args))
        .filter(|steps| !steps.is_empty());
    let details = match (patch, plan) {
        (Some(_), _) => details.children(diff.map(|diff| {
            div()
                .w_full()
                .min_w_0()
                .h(crate::ui::prefs::scaled(diff.read(cx).embedded_height()))
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .overflow_hidden()
                .child(diff)
        })),
        (None, Some(steps)) => details.child(plan_list(&steps, cx)),
        (None, None) => details.children(argument_details(fold, tool, cx)),
    };
    let failed = matches!(tool.status.as_str(), "failed" | "denied");
    let show_result = failed
        || !matches!(
            tool.name.as_str(),
            tool_display::APPLY_PATCH | tool_display::UPDATE_PLAN
        );
    let result = tool
        .result
        .as_deref()
        .filter(|result| show_result && !result.trim().is_empty())
        .map(|result| {
            fold.block(
                0,
                result,
                |text| {
                    code_block(row_id("tool-result", key), text, cx).when(failed, |block| {
                        block
                            .border_1()
                            .border_color(cx.theme().danger.opacity(0.5))
                    })
                },
                cx,
            )
        });
    details.children(result)
}

/// The full subject when the headline had to cut it, and the remaining
/// arguments as `key: value` lines.
fn argument_details(fold: &Fold, tool: &AppTranscriptTool, cx: &App) -> Vec<AnyElement> {
    let key = fold.key;
    let theme = cx.theme();
    let args = tool_args(tool);
    let headline = tool_display::headline(&tool.name, args);
    let subject_key = headline.subject_key.as_deref();
    let full_subject = subject_key
        .and_then(|key| args.get(key))
        .and_then(Value::as_str)
        .filter(|text| text.contains('\n') || text.chars().count() > SHORT_SUBJECT)
        .map(|text| {
            fold.block(
                1,
                text,
                |text| code_block(row_id("tool-subject", key), text, cx),
                cx,
            )
            .into_any_element()
        });
    let mono = theme.mono_font_family.clone();
    let rows = tool_display::other_args(args, subject_key)
        .into_iter()
        .map(|(name, value)| {
            h_flex()
                .gap_2()
                .min_w_0()
                .text_xs()
                .font_family(mono.clone())
                .child(
                    div()
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child(format!("{name}:")),
                )
                .child(div().min_w_0().child(faded(value)))
                .into_any_element()
        });
    full_subject.into_iter().chain(rows).collect()
}

pub(super) fn plan_list(steps: &[PlanStep], cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .gap_1()
        .text_sm()
        .children(steps.iter().map(|step| {
            let (icon, color) = match step.status {
                StepStatus::Completed => (Lucide::CircleCheck, theme.success),
                StepStatus::InProgress => (Lucide::CircleDot, theme.warning),
                StepStatus::Pending => (Lucide::Minus, theme.muted_foreground),
            };
            h_flex()
                .gap_2()
                .items_start()
                .child(Icon::new(icon).xsmall().text_color(color).mt_0p5())
                .child(
                    div()
                        .min_w_0()
                        .when(step.status == StepStatus::Completed, |text| {
                            text.text_color(theme.muted_foreground)
                        })
                        .child(step.step.clone()),
                )
        }))
}

pub(super) fn code_block(id: ElementId, text: String, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .w_full()
        .px_3()
        .py_2()
        .rounded(theme.radius)
        .bg(theme.muted)
        .font_family(theme.mono_font_family.clone())
        .text_xs()
        .child(SelectableText::new(id, text))
}
