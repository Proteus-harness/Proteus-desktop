use gpui_kit::component::*;
use gpui_kit::*;

use super::Chat;
use super::scroller::ChatList;
use crate::ui::motion;

impl Render for Chat {
    /// The conversation: thread, pending requests and composer.
    /// Files dropped anywhere on it attach to the draft.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The chat is cached apart from the window, so it drives its own
        // disclosures.
        motion::drive(window, cx);
        self.prepare_tool_diffs(cx);
        let renderer = cx.processor(|this, ix: usize, window, cx| this.render_row(ix, window, cx));
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
                    .flex_1()
                    .min_h_0()
                    .child(ChatList::new("thread", self.scroller.clone(), renderer))
                    .child(fade(cx.theme().background))
                    .children(self.render_notice(cx)),
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

/// The thread fades out above the composer instead of ending at a hard
/// edge. Two layers ease the fade: a long faint one and a short one that
/// only closes the last pixels.
fn fade(background: Hsla) -> impl IntoElement {
    let layer = |height: f32, from: f32| {
        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .h(px(height))
            .bg(linear_gradient(
                180.,
                linear_color_stop(background.opacity(0.), 0.),
                linear_color_stop(background.opacity(from), 1.),
            ))
    };
    div()
        .absolute()
        .bottom_0()
        .left_0()
        .right_0()
        .child(layer(64., 0.6))
        .child(layer(20., 1.))
}
