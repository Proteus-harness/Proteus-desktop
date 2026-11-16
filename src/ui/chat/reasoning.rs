// Адаптировано из Zed: crates/agent_ui/src/conversation_view/thread_view.rs
// (thinking block disclosure, with the shared Proteus row motion).
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::SelectableText;
use gpui_kit::component::*;
use gpui_kit::*;

use super::Chat;
use crate::transcript::Entry;
use crate::ui::kit::Pressable as _;
use crate::ui::motion::{Spinner, chevron, reveal};
use crate::ui::prefs;

pub(super) fn render(
    entry: &Entry,
    expanded: bool,
    connected: bool,
    cx: &mut Context<Chat>,
) -> AnyElement {
    let key = entry.key;
    let muted = cx.theme().chat_secondary_foreground;
    let active = entry.streaming && connected;
    let icon = if active {
        Spinner::new().xsmall().into_any_element()
    } else {
        Icon::new(Lucide::Brain)
            .xsmall()
            .text_color(muted)
            .into_any_element()
    };
    let header = h_flex()
        .id(ElementId::NamedInteger("reasoning".into(), key))
        .gap_2()
        .text_sm()
        .text_color(muted)
        .chat_disclosure(cx)
        .child(icon)
        .child("Рассуждения")
        .child(chevron(
            ElementId::NamedInteger("reasoning-chevron".into(), key),
            expanded,
            muted,
            cx,
        ))
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_expanded(key, cx)));
    // The shared reveal does not construct the full text while collapsed.
    let details = reveal(
        super::thread::reveal_key(cx.entity_id(), key),
        expanded,
        cx,
        || {
            div()
                .w_full()
                .pl_3()
                .pt_1()
                .font_family(prefs::chat_font(cx))
                .text_size(prefs::chat_text(cx))
                .text_color(muted)
                .child(SelectableText::new(
                    ElementId::NamedInteger("reasoning-text".into(), key),
                    SharedString::from(&entry.text),
                ))
                .into_any_element()
        },
    );
    v_flex()
        .w_full()
        .child(header)
        .children(details)
        .into_any_element()
}
