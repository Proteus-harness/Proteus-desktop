use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::Instant;

use super::Chat;
use super::row_layout::RowContext;
use super::scroller::{ChatList, ChatScroller, FooterState};
use crate::ui::{motion, prefs};

impl Render for Chat {
    /// The conversation: thread, pending requests and composer.
    /// Files dropped anywhere on it attach to the draft.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_readonly_history(cx);
        let thread_entering = self
            .transcript_entered_at
            .get_or_insert_with(Instant::now)
            .elapsed()
            < motion::BASE;
        if !self.display_transcript().entries().is_empty() {
            crate::startup_trace::first("chat.transcript.render");
            if self.api.is_none() && self.codex.is_none() && self.readonly.is_none() {
                crate::startup_trace::first("chat.saved_tail.rendered");
            }
        }
        // The chat is retained apart from the window and drives its own
        // disclosures; caption ticks update their retained child separately.
        motion::drive(window, cx);
        self.prepare_tool_diffs(cx);
        self.prepare_turn_stamp(cx);
        let live = !self.archived();
        let activity = live.then(|| self.session.activity()).flatten();
        let thinking = activity.is_some();
        if let Some(activity) = activity {
            self.thinking_caption.update(cx, |caption, cx| {
                caption.set_caption(activity.text, activity.animated, cx);
            });
        }
        let footer_state = FooterState {
            thinking,
            receipt: live
                .then(|| self.session.feedback.latest_receipt())
                .flatten()
                .map(|(send, previous)| (send.id.clone(), send.status(), previous)),
        };
        let running = self.session.connected && self.session.running();
        // The track spans the composer, while its handle measures the list.
        let scrollbar = Scrollbar::vertical(&self.scrollbar)
            .id("chat-scrollbar")
            .viewport_from_layout()
            .scroll_viewport_from_handle();
        let row_context = super::row_layout::row_context(
            self.display_transcript().entries(),
            prefs::visual(cx).view.compact_tools,
            self.history.range(&self.session.transcript),
        );
        self.thread_view.update(cx, |thread, cx| {
            thread.row_context = row_context;
            thread.footer = footer_state;
            thread.running = running;
            cx.notify();
        });
        v_flex()
            .id("chat")
            .relative()
            .when_some(
                self.readonly.as_ref().map(|binding| binding.focus.clone()),
                |view, focus| view.track_focus(&focus),
            )
            .size_full()
            .drag_over::<ExternalPaths>(|chat, _, _, cx| chat.bg(cx.theme().drop_target))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                if this.readonly.is_none() && this.session.connected {
                    this.attach_files(paths.paths(), cx)
                }
            }))
            .children(self.render_readonly_header(cx))
            .child(motion::fade_in_if(
                div()
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .child(self.thread_view.clone())
                    .children(self.render_notice(cx)),
                "thread-enter",
                thread_entering,
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::Copy, window, cx| {
                    if this.readonly.is_none()
                        && this.composer.focus_handle(cx).is_focused(window)
                        && this.composer.read(cx).selected_range().is_empty()
                        && this.stop_from_keyboard(cx)
                    {
                        cx.stop_propagation();
                    }
                },
            ))
            .capture_action(cx.listener(
                |this, _: &gpui_kit::component::input::Escape, window, cx| {
                    if this.readonly.is_none()
                        && this.composer.focus_handle(cx).is_focused(window)
                        && (this.deny_pending(cx)
                            || (!this.has_draft(cx) && this.stop_from_keyboard(cx)))
                    {
                        cx.stop_propagation();
                    }
                },
            ))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.readonly.is_some() {
                    return;
                }
                let keystroke = &event.keystroke;
                let handled = match keystroke.key.as_str() {
                    // A popup or a free-answer field owns its Escape. Only
                    // the composer offers the chat's deny shortcut.
                    "enter"
                        if keystroke.modifiers == Modifiers::secondary_key(cx)
                            && this.composer.focus_handle(cx).is_focused(window)
                            && !this.has_draft(cx) =>
                    {
                        this.approve_from_composer(event.is_held, cx)
                    }
                    "escape" if this.composer.focus_handle(cx).is_focused(window) => {
                        this.deny_pending(cx)
                            || (!this.has_draft(cx) && this.stop_from_keyboard(cx))
                    }
                    key if !keystroke.modifiers.modified() => key
                        .parse::<usize>()
                        .is_ok_and(|digit| this.pick_option(digit, window, cx)),
                    _ => false,
                };
                if handled {
                    cx.stop_propagation();
                }
            }))
            .when(self.readonly.is_none(), |view| {
                view.child(self.render_composer(window, cx))
            })
            .child(div().absolute().inset_0().child(scrollbar))
    }
}

pub(super) struct ChatListView {
    chat: WeakEntity<Chat>,
    scroller: Entity<ChatScroller>,
    row_context: RowContext,
    footer: FooterState,
    running: bool,
    _scroll: Subscription,
}

impl ChatListView {
    pub(super) fn new(
        chat: WeakEntity<Chat>,
        scroller: Entity<ChatScroller>,
        cx: &mut Context<Self>,
    ) -> Self {
        let scroll = cx.observe(&scroller, |_, _, cx| cx.notify());
        Self {
            chat,
            scroller,
            row_context: RowContext::default(),
            footer: FooterState::default(),
            running: false,
            _scroll: scroll,
        }
    }
}

impl Render for ChatListView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let Some(chat) = self.chat.upgrade() else {
            return Empty.into_any_element();
        };
        let renderer = super::thread::thread_renderer(chat.clone(), Instant::now());
        let thinking = self.footer.thinking;
        ChatList::new("thread", self.scroller.clone(), renderer)
            .scrollbar(false)
            .step_on_frame()
            .row_context(self.row_context.clone())
            .footer(self.footer.clone(), move |_, cx| {
                chat.read(cx)
                    .render_feedback(thinking, cx)
                    .unwrap_or_else(|| Empty.into_any_element())
            })
            .running(self.running)
            .into_any_element()
    }
}

impl Chat {
    /// A local send receipt is visible in the first frame after Enter. The
    /// canonical thread stays server-owned and replaces it on its own echo.
    fn render_feedback(&self, thinking: bool, cx: &App) -> Option<AnyElement> {
        if self.archived() {
            return None;
        }
        let receipt = self.session.feedback.latest_receipt();
        if receipt.is_none() && !thinking {
            return None;
        }
        let theme = cx.theme();
        Some(
            h_flex()
                .w_full()
                .justify_center()
                .px_6()
                .child(
                    v_flex()
                        .w_full()
                        .max_w(prefs::chat_width(cx))
                        .max_h(prefs::scaled(176.))
                        .overflow_hidden()
                        .font_family(prefs::chat_font(cx))
                        .gap_2()
                        .children(receipt.map(|(send, previous)| {
                            let preview = send.preview();
                            let clipped = preview.len() < send.text.len();
                            v_flex()
                                .id(SharedString::from(format!("sending-{}", send.id)))
                                .w_full()
                                .items_end()
                                .gap_0p5()
                                .child(
                                    v_flex()
                                        .font_family(prefs::user_chat_font(cx))
                                        .max_w(relative(0.85))
                                        .px_3()
                                        .py_2()
                                        .gap_1()
                                        .rounded(theme.radius_lg)
                                        .bg(theme.secondary)
                                        .text_color(theme.secondary_foreground)
                                        .when(!preview.is_empty(), |bubble| {
                                            bubble.child(
                                                div()
                                                    .relative()
                                                    .max_h(prefs::scaled(96.))
                                                    .overflow_hidden()
                                                    .text_size(prefs::chat_text(cx))
                                                    .child(preview)
                                                    .when(clipped, |block| {
                                                        block.child(crate::ui::fade::bottom(
                                                            theme.secondary,
                                                            prefs::scaled(20.),
                                                        ))
                                                    }),
                                            )
                                        })
                                        .when(!send.images.is_empty(), |bubble| {
                                            bubble.child(div().text_sm().child(format!(
                                                "Изображений: {}",
                                                send.images.len()
                                            )))
                                        }),
                                )
                                .child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "sending-status-{}",
                                            send.id
                                        )))
                                        .flex_initial()
                                        .max_w_full()
                                        .text_xs()
                                        .text_color(theme.chat_secondary_foreground)
                                        .hover(|style| style.text_color(theme.foreground))
                                        .child(if previous == 0 {
                                            send.status().to_owned()
                                        } else {
                                            format!("{} · ещё сообщений: {previous}", send.status())
                                        }),
                                )
                        }))
                        .when(thinking, |footer| {
                            footer.child(
                                div()
                                    .pt_4()
                                    .pb_1()
                                    .truncate()
                                    .text_size(prefs::chat_text(cx))
                                    .child(h_flex().min_w_0().child(self.thinking_caption.clone())),
                            )
                        }),
                )
                .into_any_element(),
        )
    }
}
