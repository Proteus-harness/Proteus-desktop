// Адаптировано из Zed: crates/editor/src/element.rs (visible rows, render_spacer_block)
//! A naturally tall block that lays out and paints only rows intersecting
//! the current content mask. Unlike a nested scrolling list it also stays
//! virtualized inside a disclosure during its height animation.

use crate::diff::File;
use crate::ui::prefs;
use gpui_kit::*;
use std::rc::Rc;

pub(super) struct Lines {
    file: Rc<File>,
    syntax: Rc<super::syntax::Syntax>,
    side: Option<bool>,
    code_width: Pixels,
}

impl Lines {
    pub fn new(
        file: Rc<File>,
        syntax: Rc<super::syntax::Syntax>,
        side: Option<bool>,
        code_width: Pixels,
        _: &App,
    ) -> Self {
        Self {
            file,
            syntax,
            side,
            code_width,
        }
    }
}

impl IntoElement for Lines {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Lines {
    type RequestLayoutState = ();
    type PrepaintState = Vec<AnyElement>;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, ()) {
        let gutter = prefs::scaled(if self.side.is_some() { 72. } else { 112. })
            .to_pixels(window.rem_size());
        let width = self.code_width + gutter;
        let count = if self.side.is_some() {
            self.file.pairs.len()
        } else {
            self.file.unified.len()
        };
        let height = prefs::scaled(super::LINE).to_pixels(window.rem_size()) * count as f32;
        let mut style = Style::default();
        style.min_size.width = width.into();
        style.size.height = height.into();
        (
            window.request_measured_layout(style, move |known, available, _, _| {
                let room = known.width.unwrap_or(match available.width {
                    AvailableSpace::Definite(room) => room,
                    _ => width,
                });
                size(room.max(width), height)
            }),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let mask = window.content_mask().bounds.intersect(&bounds);
        if mask.size.height <= px(0.) {
            return Vec::new();
        }
        let height = prefs::scaled(super::LINE).to_pixels(window.rem_size());
        let count = if self.side.is_some() {
            self.file.pairs.len()
        } else {
            self.file.unified.len()
        };
        let start = ((mask.top() - bounds.top()) / height).floor().max(0.) as usize;
        let end = (((mask.bottom() - bounds.top()) / height).ceil() as usize).min(count);
        let mut visible = Vec::with_capacity(end.saturating_sub(start));
        for index in start..end {
            let mut row =
                super::row::row(&self.file, &self.syntax, index, self.side, cx).into_any_element();
            row.layout_as_root(
                size(
                    AvailableSpace::Definite(bounds.size.width),
                    AvailableSpace::Definite(height),
                ),
                window,
                cx,
            );
            row.prepaint_at(
                bounds.origin + point(px(0.), height * index as f32),
                window,
                cx,
            );
            visible.push(row);
        }
        visible
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        rows: &mut Vec<AnyElement>,
        window: &mut Window,
        cx: &mut App,
    ) {
        for row in rows {
            row.paint(window, cx);
        }
    }
}
