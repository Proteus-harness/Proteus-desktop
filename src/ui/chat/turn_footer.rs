//! One compact copy/time row for a complete assistant turn.
use super::{Chat, stamp};
use crate::ui::kit::{self, ButtonLabel as _, Hint as _};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::Duration;

pub(super) const GROUP: &str = "turn-footer-owner";

#[derive(Default)]
pub(super) struct CopyState {
    generation: u64,
    key: Option<u64>,
    pending: bool,
    copied: bool,
    task: Option<Task<()>>,
}

impl Chat {
    pub(super) fn turn_footer(
        &self,
        key: u64,
        end_key: u64,
        time: Option<i64>,
        copyable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.turn_copy.key == Some(key);
        let pending = selected && self.turn_copy.pending;
        let copied = selected && self.turn_copy.copied;
        h_flex()
            .h_5()
            .gap_1()
            .items_center()
            .invisible()
            .group_hover(GROUP, |style| style.visible())
            .text_color(cx.theme().muted_foreground)
            .when(copyable, |row| {
                row.child(
                    kit::button(ElementId::NamedInteger("copy-turn".into(), key))
                        .quiet(cx)
                        .xsmall()
                        .icon(if copied {
                            IconName::Check
                        } else {
                            IconName::Copy
                        })
                        .disabled(pending)
                        .hint(
                            if copied {
                                "Скопировано"
                            } else {
                                "Копировать ответ"
                            },
                            cx,
                        )
                        .on_click(cx.listener(move |chat, _, _, cx| {
                            chat.copy_turn(key, end_key, cx);
                        })),
                )
            })
            .children(stamp::turn_time(key, time, cx))
            .into_any_element()
    }

    fn copy_turn(&mut self, key: u64, end_key: u64, cx: &mut Context<Self>) {
        let plan = match self.turn_copy_plan(end_key) {
            Ok(plan) => plan,
            Err(error) => {
                self.show_notice(format!("Не удалось скопировать ответ: {error}"), cx);
                return;
            }
        };
        self.turn_copy.generation = self.turn_copy.generation.wrapping_add(1);
        let generation = self.turn_copy.generation;
        self.turn_copy.key = Some(key);
        self.turn_copy.pending = true;
        self.turn_copy.copied = false;
        let session = self.session.key.clone();
        let connection = self.api.as_ref().map(|api| api.connection_id());
        let navigation = self
            .workspace
            .upgrade()
            .map(|workspace| workspace.read(cx).navigation_generation);
        self.turn_copy.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { plan.read() })
                .await;
            let copied = this
                .update(cx, |chat, cx| {
                    if chat.turn_copy.generation != generation {
                        return false;
                    }
                    chat.turn_copy.pending = false;
                    if chat.session.key != session
                        || chat.api.as_ref().map(|api| api.connection_id()) != connection
                        || chat
                            .workspace
                            .upgrade()
                            .map(|workspace| workspace.read(cx).navigation_generation)
                            != navigation
                    {
                        cx.notify();
                        return false;
                    }
                    match result {
                        Ok(text) => {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                            chat.turn_copy.copied = true;
                            cx.notify();
                            true
                        }
                        Err(error) => {
                            chat.show_notice(format!("Не удалось скопировать ответ: {error}"), cx);
                            false
                        }
                    }
                })
                .unwrap_or(false);
            if copied {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                this.update(cx, |chat, cx| {
                    if chat.turn_copy.generation == generation {
                        chat.turn_copy.copied = false;
                        cx.notify();
                    }
                })
                .ok();
            }
        }));
        cx.notify();
    }
}
