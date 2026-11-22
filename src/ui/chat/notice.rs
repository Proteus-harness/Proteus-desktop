// По образцу Zed: crates/agent_ui/src/conversation_view/thread_view.rs
// (ошибка хода остаётся доступной для чтения рядом с полем ввода).
//! Short notices, persistent errors and unresolved cancellation over the composer.

use crate::ui::kit::ButtonLabel as _;
use crate::ui::kit::Frame as _;
use std::time::{Duration, Instant};

use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::app_protocol::AppRunStatus;

use super::Chat;
use crate::ui::kit::Hint as _;
use crate::ui::{kit, motion, prefs};

const SHOWN: Duration = Duration::from_secs(4);
const PER_CHAR: Duration = Duration::from_millis(40);
const LONGEST: Duration = Duration::from_secs(12);
const CANCEL_WAIT: Duration = Duration::from_secs(10);
const CANCEL_PENDING: &str = "Запрос остановки принят. Core ещё не подтвердил завершение хода.";

#[derive(Default)]
pub(super) struct Notice {
    info: Option<Message>,
    error: Option<Message>,
    cancel: Option<CancelNotice>,
    /// Counts the notes, so each one fades in afresh.
    count: usize,
    hide: Option<Task<()>>,
}

struct Message {
    text: SharedString,
    sequence: usize,
    shown_at: Instant,
}

struct CancelNotice {
    run_id: String,
    message: Option<Message>,
    _timer: Task<()>,
}

#[derive(Clone, Copy)]
enum Kind {
    Info,
    Error,
    CancelPending,
}

impl Kind {
    fn id(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Error => "error",
            Self::CancelPending => "cancel-pending",
        }
    }
}

impl Notice {
    fn message(&mut self, text: SharedString) -> Message {
        self.count = self.count.wrapping_add(1);
        Message {
            text,
            sequence: self.count,
            shown_at: Instant::now(),
        }
    }
}

impl Chat {
    pub fn show_notice(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        let stay = (SHOWN + PER_CHAR * text.chars().count() as u32).min(LONGEST);
        let message = self.notice.message(text);
        self.notice.info = Some(message);
        self.notice.hide = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(stay).await;
            this.update(cx, |this, cx| {
                this.notice.info = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub(super) fn clear_notice(&mut self, cx: &mut Context<Self>) {
        self.notice.hide = None;
        if self.notice.info.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn show_error(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        if self
            .notice
            .error
            .as_ref()
            .is_some_and(|error| error.text == text)
        {
            return;
        }
        let message = self.notice.message(text);
        self.notice.error = Some(message);
        cx.notify();
    }

    /// Start once per authoritative CancelRequested run, including a reconnect
    /// baseline. The timer only changes presentation.
    pub(super) fn sync_cancel_notice(&mut self, cx: &mut Context<Self>) {
        let Some(run) = self
            .session
            .execution
            .active
            .as_ref()
            .filter(|run| run.status == AppRunStatus::CancelRequested)
        else {
            self.notice.cancel = None;
            return;
        };
        if self
            .notice
            .cancel
            .as_ref()
            .is_some_and(|notice| notice.run_id == run.run_id)
        {
            return;
        }
        let run_id = run.run_id.clone();
        let expected = run_id.clone();
        let timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CANCEL_WAIT).await;
            this.update(cx, |this, cx| {
                if !this
                    .notice
                    .cancel
                    .as_ref()
                    .is_some_and(|notice| notice.run_id == expected)
                {
                    return;
                }
                let message = this.notice.message(CANCEL_PENDING.into());
                this.notice
                    .cancel
                    .as_mut()
                    .expect("current cancellation")
                    .message = Some(message);
                cx.notify();
            })
            .ok();
        });
        self.notice.cancel = Some(CancelNotice {
            run_id,
            message: None,
            _timer: timer,
        });
    }

    fn dismiss_notice(&mut self, kind: Kind, cx: &mut Context<Self>) {
        match kind {
            Kind::Info => self.clear_notice(cx),
            Kind::Error => {
                self.notice.error = None;
                cx.notify();
            }
            Kind::CancelPending => {
                if let Some(notice) = &mut self.notice.cancel {
                    notice.message = None;
                    cx.notify();
                }
            }
        }
    }

    /// The note floats over the bottom of the thread, so the conversation
    /// does not jump when it comes and goes.
    pub(super) fn render_notice(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let messages = [
            (Kind::Error, self.notice.error.as_ref()),
            (
                Kind::CancelPending,
                self.notice
                    .cancel
                    .as_ref()
                    .and_then(|notice| notice.message.as_ref()),
            ),
            (Kind::Info, self.notice.info.as_ref()),
        ];
        if messages.iter().all(|(_, message)| message.is_none()) {
            return None;
        }
        Some(
            v_flex()
                .absolute()
                .bottom_2()
                .left_0()
                .right_0()
                .px_6()
                .gap_1()
                .items_center()
                .children(messages.into_iter().filter_map(|(kind, message)| {
                    message.map(|message| self.notice_row(kind, message, cx))
                })),
        )
    }

    fn notice_row(&self, kind: Kind, message: &Message, cx: &mut Context<Self>) -> AnyElement {
        let text = message.text.clone();
        let theme = cx.theme();
        let (color, border) = match kind {
            Kind::Info => (theme.muted_foreground, theme.transparent),
            Kind::Error => (theme.danger, theme.danger.opacity(0.4)),
            Kind::CancelPending => (theme.warning, theme.warning.opacity(0.4)),
        };
        let note = h_flex()
            .id(kind.id())
            .min_w_0()
            .max_w(prefs::chat_width(cx))
            .gap_1()
            .pl_3()
            .pr_1()
            .py_0p5()
            .frame(cx)
            .border_color(border)
            .bg(theme.popover)
            .text_xs()
            .text_color(color)
            .child(div().flex_1().min_w_0().child(text.clone()))
            .child(
                kit::Clipboard::new("notice-copy")
                    .value(text)
                    .hint("Копировать полный текст уведомления", cx)
                    .xsmall(),
            )
            .child(
                kit::button("notice-close")
                    .icon(IconName::Close)
                    .quiet(cx)
                    .xsmall()
                    .hint("Закрыть уведомление", cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.dismiss_notice(kind, cx))),
            );
        motion::fade_in_if(
            note,
            (kind.id(), message.sequence),
            message.shown_at.elapsed() < motion::BASE * 2,
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests;
