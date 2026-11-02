//! Visible SVG paint uses the native image primitive and the shared LRU.
//! Retained Canvas closures own parsed metadata, never prepared bitmap leases.
//! GPUI element state keeps a paint lease while its cached sprite is replayed.
use std::rc::Weak as RcWeak;
use std::sync::{Arc, Weak};

use gpui_kit::*;

use super::{Bitmap, Failures, Key, RASTERS};

pub(in super::super) struct Raster {
    key: Key,
    svg: Arc<ParsedSvg>,
    owner: Weak<()>,
    failures: RcWeak<Failures>,
}

impl Raster {
    pub(super) fn new(
        key: Key,
        svg: Arc<ParsedSvg>,
        owner: Weak<()>,
        failures: RcWeak<Failures>,
    ) -> Self {
        Self {
            key,
            svg,
            owner,
            failures,
        }
    }

    pub(in super::super) fn element(
        self,
        id: impl Into<ElementId>,
        view: EntityId,
    ) -> impl IntoElement + Styled {
        let id = id.into();
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                if !crate::ui::render_image::visible(bounds, window.content_mask().bounds) {
                    return;
                }
                let failed = self.failed();
                window.with_global_id(id, |id, window| {
                    crate::ui::render_image::with_lease(
                        id,
                        &self.key,
                        window,
                        cx,
                        |previous, _, cx| {
                            // A stale Canvas must not reuse another view's live
                            // bitmap after its own source/DPI owner has expired.
                            self.owner.upgrade()?;
                            previous.or_else(|| self.resolve(&cx.svg_renderer(), Some(cx)))
                        },
                        |bitmap, window, _| {
                            if let Some(bitmap) = bitmap {
                                let image = bitmap.image();
                                let image_bounds =
                                    ObjectFit::Contain.get_bounds(bounds, image.size(0));
                                if let Err(error) = window.paint_image(
                                    bounds,
                                    image_bounds,
                                    Corners::default(),
                                    image,
                                    0,
                                    false,
                                ) {
                                    self.fail(format!("SVG: {error}"));
                                }
                            }
                        },
                    );
                });
                if !failed && self.failed() {
                    cx.notify(view);
                }
            },
        )
    }

    fn failed(&self) -> bool {
        self.failures
            .upgrade()
            .is_some_and(|failures| failures.borrow().contains_key(&self.key))
    }

    fn fail(&self, error: String) {
        if let Some(failures) = self.failures.upgrade() {
            failures.borrow_mut().insert(self.key.clone(), error);
        }
    }

    pub(super) fn resolve(&self, renderer: &SvgRenderer, cx: Option<&App>) -> Option<Arc<Bitmap>> {
        let owner = self.owner.upgrade()?;
        if self.failed() {
            return None;
        }
        if let Some(image) = RASTERS.with(|cache| cache.borrow_mut().get(&self.key, &owner)) {
            return Some(image);
        }
        // A cache miss gets a new native image identity. Old paint-frame Arcs
        // may outlive eviction/atlas cleanup but are never rehydrated here.
        RASTERS.with(|cache| {
            cache
                .borrow_mut()
                .reserve(self.key.size.pixels(), self.key.source.len())
        });
        let image = match renderer.render_parsed(
            &self.svg,
            SvgSize::ExactSize(size(
                DevicePixels(self.key.size.width as i32),
                DevicePixels(self.key.size.height as i32),
            )),
        ) {
            Ok(image) => Bitmap::new(image, cx),
            Err(error) => {
                self.fail(format!("SVG: {error}"));
                return None;
            }
        };
        RASTERS.with(|cache| {
            cache.borrow_mut().insert(
                self.key.clone(),
                image.clone(),
                self.key.size.pixels(),
                self.key.source.len(),
                &owner,
            )
        });
        Some(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::render_image::visible;

    #[::core::prelude::v1::test]
    fn only_a_positive_visible_intersection_admits_a_raster() {
        let mask = Bounds::new(point(px(10.), px(20.)), size(px(100.), px(80.)));
        let bounds = |x, y, w, h| Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
        assert!(visible(bounds(0., 0., 11., 21.), mask));
        assert!(visible(bounds(20., 30., 40., 20.), mask));
        assert!(!visible(bounds(110., 30., 40., 20.), mask));
        assert!(!visible(bounds(20., 100., 40., 20.), mask));
        assert!(!visible(bounds(20., 30., 0., 20.), mask));
    }
}
