use crate::ui::kit::ButtonLabel as _;
use std::borrow::Cow;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

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
use super::activity_dock::ActivityId;
use crate::tool_display;
use crate::ui::fade::faded;
use crate::ui::format;
use crate::ui::kit::{self, Hint as _};
use crate::ui::prefs;

const PREVIEW_LINES: usize = 24;
const PREVIEW_CHARS: usize = 6_000;
const JUSTIFICATION: &str = "justification";

impl Chat {
    /// The selected approval keeps its action bar when its preview is folded.
    pub(super) fn render_pending_approval(
        &self,
        activity: &ActivityId,
        expanded: bool,
        preview_visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(Option<AnyElement>, AnyElement)> {
        let ActivityId::Approval(id) = activity else {
            return None;
        };
        let pending = self.session.pending.as_ref()?;
        let request = pending
            .approvals
            .iter()
            .find(|item| &item.approval_id == id)?;
        let identical = pending
            .approvals
            .iter()
            .filter(|other| {
                request.call.name == other.call.name
                    && request.call.args == other.call.args
                    && request.preview == other.preview
            })
            .map(|other| other.approval_id.clone())
            .collect();
        let actions = render_approval_actions(request, identical, expanded, cx);
        let preview = preview_visible.then(|| render_approval_preview(request, window, cx));
        Some((preview, actions))
    }

    pub(super) fn render_question(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let request = self
            .session
            .pending
            .as_ref()?
            .user_inputs
            .iter()
            .find(|request| request.request_id == id)?;
        Some(self.user_input.render_user_input(request, window, cx))
    }
}

pub(super) fn request_fresh(activity: &ActivityId, chat: &Chat) -> bool {
    let id = match activity {
        ActivityId::Approval(id) => format!("approval-{id}"),
        ActivityId::Question(id) => format!("user-input-{id}"),
        ActivityId::Extension(_) => return false,
    };
    chat.fresh_requests.is_fresh(request_key(&id))
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

/// Details sit inside the shared composer surface, without nested frames.
pub(super) fn panel_card(_: &App) -> Div {
    v_flex().p_3().gap_3()
}

/// What a tool asks to do, as the approval shows it.
pub(super) fn approval_icon(request: &AppApprovalRequest) -> Lucide {
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
    if reason.starts_with("codex policy requires approval for tool") {
        return None;
    }
    (!reason.is_empty()).then(|| reason.to_owned())
}

/// The text worth reading before allowing: the server preview, the
/// tool's main argument or the other arguments.
fn approval_body(request: &AppApprovalRequest) -> Option<Cow<'_, str>> {
    if request.call.name == tool_display::APPLY_PATCH
        && let Some(patch) = tool_display::patch_text(&request.call.args)
    {
        return Some(Cow::Borrowed(patch));
    }
    if let Some(preview) = &request.preview {
        return preview.body.as_deref().map(Cow::Borrowed);
    }
    let args = &request.call.args;
    let subject = tool_display::headline(&request.call.name, args)
        .subject_key
        .filter(|key| key.as_str() != JUSTIFICATION)
        .and_then(|key| args.get(&key).and_then(Value::as_str))
        .map(Cow::Borrowed);
    subject.or_else(|| {
        let mut rest = args.as_object()?.clone();
        rest.remove(JUSTIFICATION);
        (!rest.is_empty())
            .then(|| serde_json::to_string_pretty(&rest).ok())
            .flatten()
            .map(Cow::Owned)
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

fn render_approval_actions(
    request: &AppApprovalRequest,
    identical: Vec<String>,
    expanded: bool,
    cx: &mut Context<Chat>,
) -> AnyElement {
    let shortcut = if cx.platform_info().is_mac() {
        "⌘Enter"
    } else {
        "Ctrl+Enter"
    };
    let id = request.approval_id.clone();
    let (approve_id, deny_id) = (id.clone(), id.clone());
    let allow = kit::button(SharedString::from(format!("approve-{id}")))
        .faded_label("Разрешить один раз")
        .children(expanded.then(|| key_hint(shortcut)))
        .primary()
        .small()
        .on_click(cx.listener(move |this, _, window, cx| {
            this.resolve_approval(approve_id.clone(), true, ApprovalCacheScope::None, cx);
            this.focus(window, cx);
        }));
    let choices = cache_choices(request);
    let always = (!choices.is_empty()).then(|| {
        let chat = cx.entity().downgrade();
        let menu_id = id.clone();
        crate::ui::context::menu_button(
            SharedString::from(format!("approve-menu-{id}")),
            kit::button(SharedString::from(format!("approve-always-{id}")))
                .faded_label("Разрешать всегда")
                .hint("Выбрать, какие действия разрешать автоматически", cx)
                .small(),
            true,
            move |menu, _, _| {
                choices.iter().fold(menu, |menu, &(label, scope)| {
                    let (chat, id) = (chat.clone(), menu_id.clone());
                    menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        chat.update(cx, |chat, cx| {
                            chat.resolve_approval(id.clone(), true, scope, cx);
                            chat.focus(window, cx);
                        })
                        .ok();
                    }))
                })
            },
        )
        .align_end()
    });
    // The identical requests describe the same operation, including its
    // arguments and scope. The bulk action submits only the IDs shown here.
    let all = (identical.len() > 1).then(|| {
        kit::button(SharedString::from(format!("approve-all-{id}")))
            .faded_label(format!("Все {}", identical.len()))
            .hint("Разрешить одинаковые запросы один раз", cx)
            .small()
            .on_click(cx.listener(move |chat, _, window, cx| {
                chat.resolve_approvals(&identical, true, ApprovalCacheScope::None, cx);
                chat.focus(window, cx);
            }))
    });
    let card_id = id.clone();
    h_flex()
        .id(SharedString::from(format!("approval-actions-{id}")))
        .px_3()
        .py_2()
        .justify_end()
        .flex_wrap()
        .gap_2()
        .capture_key_down(cx.listener(move |chat, event: &KeyDownEvent, _, cx| {
            if event.keystroke.key == "enter"
                && (event.is_held || request_fresh(&ActivityId::Approval(card_id.clone()), chat))
            {
                cx.stop_propagation();
            }
        }))
        .child(
            kit::button(SharedString::from(format!("deny-{deny_id}")))
                .faded_label("Отклонить")
                .children(expanded.then(|| key_hint("Esc")))
                .small()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.resolve_approval(deny_id.clone(), false, ApprovalCacheScope::None, cx);
                    this.focus(window, cx);
                })),
        )
        .child(allow)
        .children(always)
        .children(all)
        .into_any_element()
}

fn render_approval_preview(
    request: &AppApprovalRequest,
    window: &mut Window,
    cx: &mut Context<Chat>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let summary = request
        .preview
        .as_ref()
        .map(|preview| {
            if preview.affected_files.is_empty() {
                preview.summary.clone()
            } else {
                preview.affected_files.join(", ")
            }
        })
        .filter(|summary| !summary.trim().is_empty());
    let full_body = approval_body(request);
    let body = full_body
        .as_ref()
        .map(|body| format::clip_block(body, PREVIEW_LINES, PREVIEW_CHARS));
    let truncated = full_body
        .as_ref()
        .is_some_and(|body| body.len() > PREVIEW_CHARS || body.lines().count() > PREVIEW_LINES);

    let id = request.approval_id.clone();
    let patch = full_body.as_deref().filter(|patch| {
        (request.call.name == tool_display::APPLY_PATCH
            || request
                .preview
                .as_ref()
                .is_some_and(|preview| preview.kind == "patch"))
            && patch.len() <= 64 * 1024
    });
    let diff = patch.and_then(|patch| {
        let diff = window.use_keyed_state(
            SharedString::from(format!("approval-diff-{id}")),
            cx,
            |_, cx| crate::ui::diff::DiffView::new(Arc::new(patch.to_owned()), Vec::new(), cx),
        );
        diff.read(cx).has_files().then_some(diff)
    });
    panel_card(cx)
        .gap_2()
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(
                    Icon::new(approval_icon(request))
                        .small()
                        .tone(IconTone::Muted),
                )
                .child(div().font_medium().child(match approval_icon(request) {
                    Lucide::FilePen => "Нужно разрешение на изменение файлов",
                    Lucide::SquareTerminal => "Нужно разрешение на выполнение команды",
                    _ => "Нужно разрешение на вызов инструмента",
                }))
                .children(summary.map(|summary| {
                    div()
                        .min_w_0()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(faded(summary))
                })),
        )
        .children(approval_reason(request).map(|reason| div().text_sm().child(reason)))
        .children(
            diff.clone()
                .map(|diff| div().w_full().h(prefs::scaled(220.)).child(diff)),
        )
        .children(body.filter(|_| diff.is_none()).map(|body| {
            kit::code_surface(cx)
                .id(SharedString::from(format!("approval-body-{id}")))
                .max_h(prefs::scaled(280.))
                .overflow_y_scroll()
                .text_color(theme.muted_foreground)
                .child(body)
        }))
        .children(full_body.filter(|_| truncated).map(|body| {
            let body: SharedString = match body {
                Cow::Borrowed(text) => text.into(),
                Cow::Owned(text) => text.into(),
            };
            kit::button(SharedString::from(format!("approval-full-{id}")))
                .small()
                .faded_label("Показать действие полностью…")
                .on_click(move |_, window, cx| {
                    use gpui_kit::component::input::{Editor, EditorState};
                    let source = body.clone();
                    let editor = cx.new(|cx| {
                        let mut editor = EditorState::new(window, cx).rows(16).soft_wrap(true);
                        editor.set_value(source, window, cx);
                        editor.set_readonly(true, cx);
                        editor
                    });
                    let copy = body.clone();
                    window.open_dialog(cx, move |dialog, _, cx| {
                        let copy = copy.clone();
                        dialog
                            .title("Полное действие для подтверждения")
                            .w(px(900. * prefs::scale(cx)))
                            .child(Editor::new(&editor).readonly(true).w_full())
                            .child(
                                kit::button("approval-copy-full")
                                    .faded_label("Скопировать полностью")
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy.to_string(),
                                        ))
                                    }),
                            )
                    });
                })
        }))
        .into_any_element()
}
