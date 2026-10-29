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
