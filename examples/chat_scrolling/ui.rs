//! Keep production visibility boundaries while replacing unrelated chrome.
//! The list, wheel queue, middle gesture, scrollbar and tail floor are real.
#[path = "../../src/ui/floor.rs"]
mod floor;

pub(super) mod chat;

mod prefs {
    pub fn scaled(value: f32) -> gpui_kit::Rems {
        gpui_kit::rems(value / 16.)
    }
}

mod motion {
    // This probe exercises the production scroller, while unrelated motion
    // chrome remains static (as with rise_in below).
    #[derive(gpui_kit::IntoElement)]
    pub struct WaveDots;
    impl gpui_kit::RenderOnce for WaveDots {
        fn render(
            self,
            _: &mut gpui_kit::Window,
            _: &mut gpui_kit::App,
        ) -> impl gpui_kit::IntoElement {
            gpui_kit::component::Icon::new(gpui_kit::component::IconName::Ellipsis)
        }
    }
    pub const SLOW: std::time::Duration = std::time::Duration::from_millis(260);
    pub fn plays() -> bool {
        false
    }
    pub fn rise_in<E>(element: E, _: impl Into<gpui_kit::ElementId>) -> E {
        element
    }
}

mod kit {
    use gpui_kit::{App, ElementId, component::button::Button};
    pub fn button(id: impl Into<ElementId>) -> Button {
        Button::new(id)
    }
    pub trait Hint: Sized {
        fn hint(self, _: &'static str, _: &App) -> Self {
            self
        }
    }
    impl Hint for Button {}
}
