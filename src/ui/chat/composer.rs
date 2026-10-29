// Адаптировано из Zed: crates/agent_ui/src/conversation_view/thread_view.rs
// (send readiness, retained queued drafts and focus after sending).
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::Textarea;
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
        h_flex()
            .w_full()
            .justify_center()
            .px_6()
            .pb_4()
            // This gap is outside the scroll viewport: it never becomes
            // extra scrollable room below the last message.
            .pt_6()
            .child(
                v_flex()
                    .on_children_prepainted(kit::record_first_child(
                        self.composer_strip_bounds.clone(),
                    ))
                    .w_full()
                    .max_w(prefs::chat_width(cx))
                    .child(
                        div()
                            .mx_3()
                            .pb_2()
                            .mb_neg_2()
                            .surface_frame(cx)
                            .rounded_b(px(0.))
                            .bg(theme.group_box)
                            .child(dock),
                    )
                    .child(
                        div()
                            .relative()
                            .surface_frame(cx)
                            .bg(theme.secondary)
                            .child(self.render_input_box(cx)),
                    ),
            )
    }

    fn render_input_box(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = &self.session;
        let running = session.running();
        let ready = session.connected;
        let queued = session
            .pending
            .as_ref()
            .map(|pending| pending.queued_user_messages.as_slice())
            .unwrap_or_default();

        let keys = match prefs::current(cx).send {
            crate::preferences::SendMode::Enter => "Enter",
            crate::preferences::SendMode::CtrlEnter => "Ctrl+Enter",
        };
        let send = kit::button("send")
            .icon(IconName::ArrowUp)
            .hint_keys(if running { "В очередь" } else { "Send" }, Some(keys), cx)
            .primary()
            .small()
            .disabled(!ready || !self.has_draft(cx) || self.attachments_loading)
            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)));
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
            .disabled(self.attachments_loading)
            .on_click(cx.listener(|this, _, window, cx| this.choose_images(window, cx)));
        let chat = cx.entity().downgrade();
        let stop = running.then(|| {
            kit::button("stop")
                .icon(IconName::Square)
                .hint("Остановить", cx)
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
            .child(kit::contain_scroll(
                Textarea::new(&self.composer)
                    .font_family(prefs::chat_font(cx))
                    .text_size(prefs::chat_text(cx))
                    .appearance(false)
                    .bordered(false)
                    .on_paste(move |item, _, cx| {
                        chat.update(cx, |this, cx| this.attach_clipboard(item, cx))
                            .unwrap_or(false)
                    }),
            ))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_x_2()
                    .gap_y_1()
                    .child(attach)
                    .child(h_flex().ml_auto().gap_2().children(stop).child(send)),
            )
    }
}

fn render_saved(draft: &SavedDraft, loading: bool, cx: &Context<Chat>) -> impl IntoElement {
    let theme = cx.theme();
    let restore_id = draft.id.clone();
    let copy = draft.text.clone();
    let images = draft.attachments.len();
    h_flex()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .bg(theme.muted)
        .text_xs()
        .child(
            Icon::new(IconName::Inbox)
                .xsmall()
                .text_color(theme.warning),
        )
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
                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
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
        .child(
            Icon::new(IconName::Inbox)
                .xsmall()
                .text_color(theme.muted_foreground),
        )
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
