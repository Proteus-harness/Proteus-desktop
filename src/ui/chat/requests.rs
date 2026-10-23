use crate::ui::kit::ButtonLabel as _;
use std::hash::{DefaultHasher, Hash, Hasher};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::app_protocol::{AppApprovalRequest, AppPendingRequests};
use proteus_contracts::contracts::{ApprovalCacheScope, UserInputRequest};
use proteus_contracts::domain::ToolSafety;
use serde_json::Value;

use super::Chat;
use crate::tool_display;
use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::prefs;
use crate::ui::{format, motion};

const PREVIEW_LINES: usize = 24;
const PREVIEW_CHARS: usize = 6_000;
const JUSTIFICATION: &str = "justification";

impl Chat {
    /// Approval and question cards the agent is waiting on, right above
    /// the composer.
    pub(super) fn render_requests(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let pending = self.session.pending.clone()?;
        if pending.approvals.is_empty() && pending.user_inputs.is_empty() {
            return None;
        }
        let mut cards = pending
            .approvals
            .iter()
            .map(|request| {
                let id = approval_card(request);
                let fresh = self.fresh_requests.is_fresh(request_key(&id));
                arrive(render_approval(request, cx), id, fresh)
            })
            .collect::<Vec<_>>();
        for request in &pending.user_inputs {
            let id = input_card(request);
            let fresh = self.fresh_requests.is_fresh(request_key(&id));
            cards.push(arrive(
                self.render_user_input(request, window, cx),
                id,
                fresh,
            ));
        }
        Some(
            v_flex()
                .id("requests")
                .w_full()
                .max_h(prefs::scaled(480.))
                .overflow_y_scroll()
                .gap_2()
                .children(cards)
                .into_any_element(),
        )
    }
}

/// A request card rises in once, when it arrives; shown again after a tab
/// switch it stays put.
fn arrive(card: AnyElement, id: String, fresh: bool) -> AnyElement {
    motion::rise_in_if(div().child(card), ElementId::Name(id.into()), fresh).into_any_element()
}

/// Keys of the cards in `pending`, for telling the newly arrived ones.
pub(super) fn request_keys(pending: &AppPendingRequests) -> impl Iterator<Item = u64> + '_ {
    let approvals = pending.approvals.iter().map(approval_card);
    let inputs = pending.user_inputs.iter().map(input_card);
    approvals.chain(inputs).map(|id| request_key(&id))
}

fn approval_card(request: &AppApprovalRequest) -> String {
    format!("approval-{}", request.approval_id)
}

fn input_card(request: &UserInputRequest) -> String {
    format!("user-input-{}", request.request_id)
}

fn request_key(id: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish()
}

/// A card of the zone above the composer, raised off the conversation like
/// the composer itself.
pub(super) fn panel_card(cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .p_3()
        .gap_3()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(theme.border)
        .bg(theme.background.blend(theme.foreground.opacity(0.04)))
}

/// What a tool asks to do, as the approval shows it.
fn approval_icon(request: &AppApprovalRequest) -> Lucide {
    let kind = request
        .preview
        .as_ref()
        .map(|preview| preview.kind.as_str());
    match (request.call.name.as_str(), kind) {
        (_, Some("command")) | ("exec_command" | "shell" | "write_stdin", _) => {
            Lucide::SquareTerminal
        }
        (_, Some("patch" | "write_file")) | ("apply_patch" | "write_file" | "edit_file", _) => {
            Lucide::FilePen
        }
        (name, _) if name.starts_with("web") => Lucide::Globe,
        _ => Lucide::ShieldAlert,
    }
}

/// Why the tool asks: the agent's own justification, else the policy reason.
fn approval_reason(request: &AppApprovalRequest) -> Option<String> {
    let reason = request
        .call
        .args
        .get(JUSTIFICATION)
        .and_then(Value::as_str)
        .unwrap_or(&request.reason)
        .trim();
    (!reason.is_empty()).then(|| reason.to_owned())
}

/// The text worth reading before allowing: the server preview, the
/// tool's main argument or the other arguments.
fn approval_body(request: &AppApprovalRequest) -> Option<String> {
    if let Some(preview) = &request.preview {
        return preview.body.clone();
    }
    let args = &request.call.args;
    let subject = tool_display::headline(&request.call.name, args)
        .subject_key
        .filter(|key| key.as_str() != JUSTIFICATION)
        .and_then(|key| args.get(&key).and_then(Value::as_str).map(str::to_owned));
    subject.or_else(|| {
        let mut rest = args.as_object()?.clone();
        rest.remove(JUSTIFICATION);
        (!rest.is_empty())
            .then(|| serde_json::to_string_pretty(&rest).ok())
            .flatten()
    })
}

/// Ways to allow without being asked again, as the server takes them.
fn cache_choices(request: &AppApprovalRequest) -> Vec<(&'static str, ApprovalCacheScope)> {
    let metadata = request.tool_spec.as_ref().map(|spec| &spec.metadata);
    let cache = metadata.and_then(|metadata| metadata.pointer("/approval/cache"));
    if cache
        .and_then(|cache| cache.get("disabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Vec::new();
    }
    let command = matches!(
        request.call.name.as_str(),
        "exec_command" | "shell" | "write_stdin"
    ) || request
        .preview
        .as_ref()
        .is_some_and(|preview| preview.kind == "command");
    let mut choices = vec![if command {
        (
            "Больше не спрашивать про эту команду",
            ApprovalCacheScope::ExactCommand,
        )
    } else {
        (
            "Больше не спрашивать про этот вызов",
            ApprovalCacheScope::ExactCall,
        )
    }];
    let writes = request
        .tool_spec
        .as_ref()
        .is_some_and(|spec| spec.safety == ToolSafety::WritesFiles);
    let flagged = cache
        .and_then(|cache| cache.get("workspace_write"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let listed = ["/approval/cache", "/approval/cache_scopes"]
        .into_iter()
        .filter_map(|pointer| metadata.and_then(|metadata| metadata.pointer(pointer)))
        .filter_map(Value::as_array)
        .flatten()
        .any(|scope| scope.as_str() == Some("workspace_write"));
    if writes && (flagged || listed) && !request.call.name.eq_ignore_ascii_case("shell") {
        choices.push((
            "Разрешать запись в рабочей папке",
            ApprovalCacheScope::WorkspaceWrite,
        ));
    }
    choices
}

/// The key that presses a button, shown on it.
fn key_hint(key: &'static str) -> Div {
    div().ml_1().text_xs().opacity(0.6).child(key)
}

fn render_approval(request: &AppApprovalRequest, cx: &mut Context<Chat>) -> AnyElement {
    let theme = cx.theme();
    let summary = request
        .preview
        .as_ref()
        .map(|preview| preview.summary.clone())
        .filter(|summary| !summary.trim().is_empty());
    let body =
        approval_body(request).map(|body| format::clip_block(&body, PREVIEW_LINES, PREVIEW_CHARS));

    let id = request.approval_id.clone();
    let (approve_id, deny_id) = (id.clone(), id.clone());
    let allow = kit::button(SharedString::from(format!("approve-{id}")))
        .faded_label("Разрешить один раз")
        .child(key_hint("⏎"))
        .primary()
        .small()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.resolve_approval(approve_id.clone(), true, ApprovalCacheScope::None, cx)
        }));
    let choices = cache_choices(request);
    let allow = if choices.is_empty() {
        allow.into_any_element()
    } else {
        let chat = cx.entity().downgrade();
        let menu_id = id.clone();
        crate::ui::context::split_button(
            SharedString::from(format!("approve-menu-{id}")),
            allow,
            kit::button(SharedString::from(format!("approve-arrow-{id}")))
                .primary()
                .small(),
            false,
            move |menu, _, _| {
                choices.iter().fold(menu, |menu, &(label, scope)| {
                    let (chat, id) = (chat.clone(), menu_id.clone());
                    menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                        chat.update(cx, |chat, cx| {
                            chat.resolve_approval(id.clone(), true, scope, cx)
                        })
                        .ok();
                    }))
                })
            },
        )
        .into_any_element()
    };
    panel_card(cx)
        .gap_2()
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(
                    Icon::new(approval_icon(request))
                        .small()
                        .text_color(theme.muted_foreground),
                )
                .child(div().font_medium().child(request.call.name.clone()))
                .children(summary.map(|summary| {
                    div()
                        .min_w_0()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(faded(summary))
                })),
        )
        .children(approval_reason(request).map(|reason| div().text_sm().child(reason)))
        .children(body.map(|body| {
            div()
                .id(SharedString::from(format!("approval-body-{id}")))
                .max_h(prefs::scaled(280.))
                .overflow_y_scroll()
                .px_3()
                .py_2()
                .rounded(theme.radius)
                .bg(theme.muted)
                .text_color(theme.muted_foreground)
                .font_family(theme.mono_font_family.clone())
                .text_xs()
                .child(body)
        }))
        .child(
            h_flex()
                .justify_end()
                .gap_2()
                .child(
                    kit::button(SharedString::from(format!("deny-{deny_id}")))
                        .faded_label("Отклонить")
                        .child(key_hint("Esc"))
                        .outline()
                        .small()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.resolve_approval(
                                deny_id.clone(),
                                false,
                                ApprovalCacheScope::None,
                                cx,
                            )
                        })),
                )
                .child(allow),
        )
        .into_any_element()
}
