//! Native controls whose hints share the application's anchored tooltip
//! provider. The wrapper leaves their state, focus and keyboard handling native.

use gpui_kit::component::Size as ControlSize;
use gpui_kit::component::clipboard::Clipboard as NativeClipboard;
use gpui_kit::component::switch::Switch as NativeSwitch;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ui::tip::Tip as _;

#[derive(IntoElement)]
pub(crate) struct Hinted<T: IntoElement + 'static> {
    id: ElementId,
    inner: T,
    hint: Option<SharedString>,
}

pub(crate) type Switch = Hinted<NativeSwitch>;
pub(crate) type Clipboard = Hinted<NativeClipboard>;

impl<T: IntoElement + 'static> Hinted<T> {
    pub fn tooltip(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl<T: IntoElement + Sizable + 'static> Sizable for Hinted<T> {
    fn with_size(mut self, size: impl Into<ControlSize>) -> Self {
        self.inner = self.inner.with_size(size);
        self
    }
}

impl<T: IntoElement + Styled + 'static> Styled for Hinted<T> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.inner.style()
    }
}

impl<T: IntoElement + Disableable + 'static> Disableable for Hinted<T> {
    fn disabled(mut self, disabled: bool) -> Self {
        self.inner = self.inner.disabled(disabled);
        self
    }
}

impl Hinted<NativeSwitch> {
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            inner: NativeSwitch::new(id.clone()),
            id,
            hint: None,
        }
    }

    pub fn focus_ring(mut self, show: bool) -> Self {
        self.inner = self.inner.focus_ring(show);
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.inner = self.inner.checked(checked);
        self
    }

    pub fn on_click(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.inner = self.inner.on_click(handler);
        self
    }
}

impl Hinted<NativeClipboard> {
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            inner: NativeClipboard::new(id.clone()),
            id,
            hint: None,
        }
    }

    pub fn value(mut self, value: impl Into<SharedString>) -> Self {
        self.inner = self.inner.value(value);
        self
    }
}

impl<T: IntoElement + 'static> RenderOnce for Hinted<T> {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        h_flex()
            .id(self.id)
            .flex_none()
            .child(self.inner)
            .when_some(self.hint, |control, hint| control.tip_text(hint))
    }
}
