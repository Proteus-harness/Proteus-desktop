//! A shortcut to the root chat from its documents and child chats.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::Button;
use gpui_kit::*;

use crate::ui::extensions::OpenDocument;
use crate::ui::kit::{self, ButtonLabel as _, Hint as _};
use crate::ui::workspace::Workspace;

impl Workspace {
    pub(super) fn render_root_chat_button(&self, cx: &mut Context<Self>) -> Option<Button> {
        let chat = self.space_root_key()?;
        let title = self.session_title(&chat, cx);
        Some(
            kit::icon_button("context-chat")
                .icon(Lucide::MessageSquare)
                .quiet(cx)
                .accessibility_label("Вернуться к основному чату")
                .hint(&title, cx)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_chat(
                        chat.clone(),
                        OpenDocument {
                            preview: false,
                            focus: true,
                        },
                        window,
                        cx,
                    );
                })),
        )
    }
}
