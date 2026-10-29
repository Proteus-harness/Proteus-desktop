use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Chat;
use super::scroller::{ChatList, FooterState};
use crate::ui::kit::SelectionCopyMenu as _;
use crate::ui::{motion, prefs};

impl Render for Chat {
    /// The conversation: thread, pending requests and composer.
    /// Files dropped anywhere on it attach to the draft.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.display_transcript().entries().is_empty() {
            crate::startup_trace::first("chat.transcript.render");
        }
        // The chat is cached apart from the window, so it drives its own
        // disclosures.
        motion::drive(window, cx);
        self.prepare_tool_diffs(cx);
        let renderer = cx.processor(|this, ix: usize, window, cx| this.render_row(ix, window, cx));
        let live = !self.history.archived();
        let thinking = live && self.session.thinking();
        let footer_state = FooterState {
            thinking,
            receipt: live
                .then(|| self.session.feedback.latest_receipt())
                .flatten()
                .map(|(send, previous)| (send.id.clone(), send.status(), previous)),
        };
        let footer = cx.processor(|this, thinking: bool, _, cx| {
            this.render_feedback(thinking, cx)
                .unwrap_or_else(|| div().into_any_element())
        });
        let running = self.session.running();
        v_flex()
            .id("chat")
            .size_full()
            .drag_over::<ExternalPaths>(|chat, _, _, cx| chat.bg(cx.theme().drop_target))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.attach_files(paths.paths(), cx)
            }))
            .child(motion::fade_in_if(
                div()
                    .relative()
                    // The selection menu owns the parent's flex slot. Its
                    // block trigger must explicitly fill that slot, or the
                    // percentage-height ChatList collapses to zero height.
                    .size_full()
                    .min_h_0()
                    .child(
                        ChatList::new("thread", self.scroller.clone(), renderer)
                            .footer(footer_state, move |window, cx| footer(thinking, window, cx))
                            .running(running),
                    )
                    .children(self.render_notice(cx))
                    .selection_copy_menu()
                    .flex_1()
                    .min_h_0(),
                "thread-enter",
                true,
            ))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let keystroke = &event.keystroke;
                let handled = match keystroke.key.as_str() {
                    // A popup or a free-answer field owns its Escape. Only
                    // the composer offers the chat's deny shortcut.
                    "escape" if this.composer.focus_handle(cx).is_focused(window) => {
                        this.deny_pending(cx)
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
            .child(self.render_composer(window, cx))
    }
}

impl Chat {
    /// A local send receipt is visible in the first frame after Enter. The
    /// canonical thread stays server-owned and replaces it on its own echo.
    fn render_feedback(&self, thinking: bool, cx: &App) -> Option<AnyElement> {
        if self.history.archived() {
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
                                .child(div().text_xs().text_color(theme.muted_foreground).child(
                                    if previous == 0 {
                                        send.status().to_owned()
                                    } else {
                                        format!("{} · ещё сообщений: {previous}", send.status())
                                    },
                                ))
                        }))
                        .when(thinking, |footer| {
                            footer.child(
                                div()
                                    .pt_4()
                                    .pb_1()
                                    .text_size(prefs::chat_text(cx))
                                    .child(self.thinking_caption.clone()),
                            )
                        }),
                )
                .into_any_element(),
        )
    }
}
