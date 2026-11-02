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
        // Img installs the decoded raster's aspect during request_layout.
        // Canvas needs the same metadata before width/max-width constraints
        // can derive its height; this does not prepare or allocate a raster.
        let aspect = self.key.size.width as f32 / self.key.size.height as f32;
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
        .aspect_ratio(aspect)
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
    use std::cell::Cell;
    use std::rc::Rc;

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

    #[::core::prelude::v1::test]
    fn a_width_clamped_svg_canvas_matches_the_original_img_layout() {
        let mut cx = ::gpui::TestAppContext::single();
        let window = cx.add_empty_window();
        let view = window.update(|window, _| window.root::<Empty>().flatten().unwrap().entity_id());
        let renderer = SvgRenderer::new(Arc::new(()));
        let source = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1280 280"><rect width="1280" height="280" fill="red"/></svg>"#;
        let node = crate::declarative::widget::Node::Svg {
            source: source.into(),
            width: 1280.,
            height: 280.,
            baseline: None,
            em_size: None,
        };
        let mut images = super::super::Images::default();
        images.prepare_with(&node, &renderer);
        let measure = |window: &mut ::gpui::VisualTestContext, width, element: AnyElement| {
            let measured = Rc::new(Cell::new(None));
            let bounds = measured.clone();
            window.draw(
                point(px(0.), px(0.)),
                size(
                    AvailableSpace::Definite(px(width)),
                    AvailableSpace::MaxContent,
                ),
                move |_, _| {
                    div()
                        .w(px(width))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .on_children_prepainted(move |children, _, _| {
                                    bounds.set(children.first().copied());
                                })
                                .child(element),
                        )
                        .into_any_element()
                },
            );
            measured.get().expect("SVG child laid out")
        };
        for dpi in [1., 2.] {
            images.rasterize_with(&node, dpi);
            let old = renderer
                .render_parsed(
                    &images.parsed.get(source).unwrap().svg,
                    SvgSize::ExactSize(size(
                        DevicePixels((1280. * dpi) as i32),
                        DevicePixels((280. * dpi) as i32),
                    )),
                )
                .unwrap();
            for width in [320., 820., 1280.] {
                let old_bounds = measure(
                    window,
                    width,
                    img(old.clone())
                        .w(px(1280.))
                        .h(px(280.))
                        .max_w_full()
                        .object_fit(ObjectFit::Contain)
                        .into_any_element(),
                );
                let new_bounds = measure(
                    window,
                    width,
                    images
                        .get(source, 1280., 280.)
                        .unwrap()
                        .element("layout-svg", view)
                        .w(px(1280.))
                        .h(px(280.))
                        .max_w_full()
                        .into_any_element(),
                );
                assert_eq!(
                    new_bounds, old_bounds,
                    "same finite-width layout at {dpi} DPI and width {width}"
                );
                assert_eq!(new_bounds.size.width, px(width));
                // When a parent derives height from the constrained width,
                // Canvas must expose the same intrinsic aspect as Img.
                let old_auto = measure(
                    window,
                    width,
                    img(old.clone())
                        .w(px(width))
                        .max_w_full()
                        .object_fit(ObjectFit::Contain)
                        .into_any_element(),
                );
                let new_auto = measure(
                    window,
                    width,
                    images
                        .get(source, 1280., 280.)
                        .unwrap()
                        .element("auto-height-svg", view)
                        .w(px(width))
                        .max_w_full()
                        .into_any_element(),
                );
                assert_eq!(
                    new_auto, old_auto,
                    "same intrinsic auto-height at {dpi} DPI and width {width}"
                );
                assert!(
                    (f32::from(new_auto.size.height) - width * 280. / 1280.).abs() <= 1.,
                    "device-pixel-rounded intrinsic geometry: {new_auto:?} at {dpi} DPI, width {width}"
                );
            }
        }
    }
}
