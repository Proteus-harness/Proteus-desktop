//! Real-window input regression probe for the production chat scroller.
//! Run inside an isolated native display; see chat_scrolling/README.md.
#![allow(dead_code)]

#[path = "chat_scrolling/ui.rs"]
mod ui;

fn main() {
    ui::chat::run();
}
