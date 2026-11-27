// Адаптировано из Zed: crates/agent_ui/src/conversation_view/thread_view.rs
// (send readiness, retained queued drafts and focus after sending).
use std::rc::Rc;

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use proteus_contracts::app_protocol::AppQueuedUserMessage;

use super::Chat;
use super::draft::SavedDraft;
use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::kit::ButtonLabel as _;
use crate::ui::kit::Frame as _;
use crate::ui::kit::Hint as _;
use crate::ui::motion::Spinner;
use crate::ui::{format, prefs};

mod gap;
pub(super) use gap::INPUT_GAP;

impl Chat {
    /// The whole framed strip, including padding and expanded activity details.
    pub(in crate::ui) fn composer_strip_bounds(&self) -> Bounds<Pixels> {
        self.composer_strip_bounds.get()
    }

    /// The composer with what the agent waits on and its plan right above.
    pub(super) fn render_composer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme().clone();
        let dock = self.render_activity_dock(window, cx);
        let dock_present = dock.is_some();
        if !dock_present {
            self.composer_strip_bounds.set(Bounds::default());
        }
        h_flex().w_full().justify_center().px_6().pb_4().child(
            v_flex()
                .when(dock_present, |composer| {
                    composer.on_children_prepainted(kit::record_first_child(
                        self.composer_strip_bounds.clone(),
                    ))
                })
                .w_full()
                .max_w(prefs::chat_width(cx))
                .children(self.render_connection_banner(cx))
                .children(dock.map(|dock| {
                    div()
                        .mx_3()
                        .pb_2()
                        .mb_neg_2()
                        .surface_frame(cx)
                        .rounded_b(px(0.))
                        .bg(kit::composer_dock_background(&theme))
                        .child(dock)
                }))
                .child(
                    div()
                        .relative()
                        .surface_frame(cx)
                        .bg(kit::composer_background(&theme))
                        .child(self.render_input_box(cx)),
                ),
        )
    }

    fn render_connection_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.session.connected || self.session.link_error.is_none() {
            return None;
        }
        let theme = cx.theme();
        let can_restart = self.core_connection().filter(|_| {
            self.backend_stopped
                && self
                    .workspace
                    .upgrade()
                    .is_some_and(|workspace| workspace.read(cx).can_restart_core())
        });
        let workspace = self.workspace.clone();
        Some(
            h_flex()
                .min_h(prefs::scaled(36.))
                .mb_2()
                .px_3()
                .gap_2()
                .justify_between()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.warning.opacity(0.4))
                .bg(theme.warning.opacity(0.12))
                .text_color(crate::ui::theme::warning_text(theme))
                .text_sm()
                .child(if self.restarting_backend {
                    "Запускаю агента…"
                } else {
                    "Нет связи с агентом"
                })
                .children(can_restart.map(|connection| {
                    kit::button("restart-agent")
                        .faded_label("Перезапустить")
                        .small()
                        .disabled(self.restarting_backend)
                        .on_click(move |_, _, cx| {
                            workspace
                                .update(cx, |workspace, cx| workspace.restart_core(connection, cx))
                                .ok();
                        })
                }))
                .into_any_element(),
        )
    }

    fn render_input_box(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_send_button(cx);
        let session = &self.session;
        if self.composer_input.read(cx).connected != session.connected {
            self.composer_input.update(cx, |input, cx| {
                input.connected = session.connected;
                cx.notify();
            });
        }
        let running = session.running();
        let queued = session
            .pending
            .as_ref()
            .map(|pending| pending.queued_user_messages.as_slice())
            .unwrap_or_default();

        let attach = kit::button("attach")
            .when(!self.attachments_loading, |button| {
                button.icon(IconName::Plus)
            })
            .when(self.attachments_loading, |button| {
                button.child(Spinner::new().small())
            })
            .hint(
                if self.attachments_loading {
                    "Загружаю изображения"
                } else {
                    "Прикрепить изображения"
                },
                cx,
            )
            .quiet(cx)
            .small()
            .disabled(!session.connected || self.attachments_loading)
            .on_click(cx.listener(|this, _, window, cx| this.choose_images(window, cx)));
        let stop = (session.connected && running).then(|| {
            kit::icon_button("stop")
                .icon(IconName::Square)
                .hint_keys(
                    "Остановить",
                    Some(if cx.platform_info().is_mac() {
                        "Esc / ⌘C"
                    } else {
                        "Esc / Ctrl+C"
                    }),
                    cx,
                )
                .danger()
                .small()
                .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
        });

        v_flex()
            .w_full()
            .gap_1()
            .p_2()
            .children(
                (!self.saved_drafts.is_empty() || !queued.is_empty()).then(|| {
                    v_flex()
                        .id("unsent-messages")
                        .max_h(prefs::scaled(160.))
                        .overflow_y_scrollbar()
                        .gap_1()
                        .children(
                            self.saved_drafts
                                .iter()
                                .map(|draft| render_saved(draft, self.attachments_loading, cx)),
                        )
                        .children(queued.iter().map(|message| render_queued(message, cx)))
                }),
            )
            .children(self.render_attachments(cx))
            .child(self.composer_input.clone())
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_x_2()
                    .gap_y_1()
                    .children(self.supports_core().then_some(attach))
                    .child(
                        h_flex()
                            .ml_auto()
                            .gap_2()
                            .children(stop)
                            .child(self.send_button.clone()),
                    ),
            )
    }
}

/// InputBase reads its state while building its frame. Keep that dependency
/// inside this view so cursor and text edits do not invalidate the thread.
pub(super) struct ComposerInput {
    chat: WeakEntity<Chat>,
    state: Entity<TextareaState>,
    connected: bool,
}

impl ComposerInput {
    pub(super) fn new(chat: WeakEntity<Chat>, state: Entity<TextareaState>) -> Self {
        Self {
            chat,
            state,
            connected: false,
        }
    }
}

impl Render for ComposerInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chat = self.chat.clone();
        kit::contain_scroll(
            Textarea::new(&self.state)
                .font_family(prefs::user_chat_font(cx))
                .text_size(prefs::chat_text(cx))
                .disabled(!self.connected)
                .appearance(false)
                .bordered(false)
                .on_paste(move |item, _, cx| {
                    chat.update(cx, |this, cx| this.attach_clipboard(item, cx))
                        .unwrap_or(false)
                }),
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SendButtonState {
    enabled: bool,
    queues: bool,
    enter_sends: bool,
}

pub(super) struct SendButton {
    chat: WeakEntity<Chat>,
    state: Option<SendButtonState>,
}

impl SendButton {
    pub(super) fn new(chat: WeakEntity<Chat>) -> Self {
        Self { chat, state: None }
    }
}

impl Chat {
    pub(super) fn update_send_button(&self, cx: &mut Context<Self>) {
        let running = self.session.running();
        let state = SendButtonState {
            enabled: self.session.connected
                && (!self.composer_blank || !self.attachments.is_empty())
                && !self.attachments_loading
                && !(self.codex.is_some() && running),
            queues: running && self.supports_core(),
            enter_sends: self.enter_sends,
        };
        if self.send_button.read(cx).state != Some(state) {
            self.send_button.update(cx, |button, cx| {
                button.state = Some(state);
                cx.notify();
            });
        }
    }
}

impl Render for SendButton {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self
            .state
            .expect("composer sets send state before rendering");
        let text = if state.queues {
            "В очередь"
        } else {
            "Отправить"
        };
        let keys = if state.enter_sends {
            "Enter"
        } else {
            "Ctrl+Enter"
        };
        let chat = self.chat.clone();
        kit::icon_button("send")
            .icon(IconName::ArrowUp)
            .hint_keys(text, Some(keys), cx)
            .primary()
            .small()
            .disabled(!state.enabled)
            .on_click(move |_, window, cx| {
                chat.update(cx, |this, cx| this.send(window, cx)).ok();
            })
    }
}

fn render_saved(draft: &SavedDraft, loading: bool, cx: &Context<Chat>) -> impl IntoElement {
    let theme = cx.theme();
    let restore_id = draft.id.clone();
    let copy = Rc::clone(&draft.text);
    let images = draft.attachments.len();
    h_flex()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .bg(theme.muted)
        .text_xs()
        .child(Icon::new(IconName::Inbox).xsmall().tone(IconTone::Warning))
        .child(
            div()
                .flex_none()
                .text_color(theme.muted_foreground)
                .child("Черновик"),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(faded(format::clip(&draft.text, 200))),
        )
        .when(images > 0, |row| {
            row.child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(format!("+{images} img")),
            )
        })
        .child(
            kit::button(SharedString::from(format!("copy-draft-{}", draft.id)))
                .icon(IconName::Copy)
                .hint("Копировать текст черновика", cx)
                .quiet(cx)
                .xsmall()
                .disabled(copy.is_empty())
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.as_ref().clone()))
                }),
        )
        .child(
            kit::button(SharedString::from(format!("restore-draft-{}", draft.id)))
                .icon(IconName::Undo2)
                .hint("Вернуть в поле; текущий черновик сохранится отдельно", cx)
                .quiet(cx)
                .xsmall()
                .disabled(loading)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.restore_draft(&restore_id, window, cx)
                })),
        )
}

fn render_queued(message: &AppQueuedUserMessage, cx: &Context<Chat>) -> impl IntoElement {
    let theme = cx.theme();
    let message_id = message.message_id;
    let images = (message.image_count > 0).then(|| format!("+{} img", message.image_count));
    h_flex()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .bg(theme.muted)
        .text_xs()
        .child(Icon::new(IconName::Inbox).xsmall().tone(IconTone::Muted))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(faded(format::clip(&message.text, 200))),
        )
        .when_some(images, |row, images| {
            row.child(div().text_color(theme.muted_foreground).child(images))
        })
        .child(
            kit::button(SharedString::from(format!("unqueue-{message_id}")))
                .icon(IconName::Close)
                .hint("Убрать из очереди", cx)
                .quiet(cx)
                .xsmall()
                .on_click(cx.listener(move |this, _, _, cx| this.delete_queued(message_id, cx))),
        )
}
