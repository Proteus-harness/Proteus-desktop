//! Accepted SVG trees retain parsed metadata; bounded rasters are prepared only
//! while visible content paints. Retained closures own no bitmaps; GPUI's
//! painted-scene state leases the rasters until repaint or unmount.

mod budget;
mod cache;
mod raster;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::*;

use crate::declarative::widget::Node;
use crate::ui::render_image::Bitmap;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    source: Arc<str>,
    size: budget::RasterSize,
}

struct Asset {
    svg: Arc<ParsedSvg>,
    aspect: Option<f64>,
}

thread_local! {
    static RASTERS: RefCell<cache::Cache<Key, Arc<Bitmap>>> = RefCell::default();
}

type Failures = RefCell<HashMap<Key, String>>;

#[derive(Default)]
pub(super) struct Images {
    parsed: HashMap<Arc<str>, Asset>,
    // A key's owner expires on source/DPI replacement, even if an old Canvas
    // survives until the next frame. Such a Canvas cannot revive stale work.
    owners: RefCell<HashMap<Key, Arc<()>>>,
    failures: Rc<Failures>,
    scale: f32,
    error: Option<String>,
}

impl Images {
    pub(super) fn prepare(&mut self, root: &Node, cx: &App) {
        self.prepare_with(root, &cx.svg_renderer());
    }

    fn prepare_with(&mut self, root: &Node, renderer: &SvgRenderer) {
        self.error = None;
        let mut used = HashSet::new();
        root.visit(&mut |node| {
            if let Node::Svg { source, .. } = node {
                used.insert(source.clone());
                if !self.parsed.contains_key(source.as_str()) {
                    match renderer.parse_svg(source.as_bytes()) {
                        Ok(svg) => {
                            self.parsed.insert(
                                Arc::from(source.as_str()),
                                Asset {
                                    svg: Arc::new(svg),
                                    aspect: budget::intrinsic_aspect(source),
                                },
                            );
                        }
                        Err(error) => self.error = Some(format!("SVG: {error}")),
                    }
                }
            }
        });
        self.parsed
            .retain(|source, _| used.contains(source.as_ref()));
        self.owners
            .get_mut()
            .retain(|key, _| used.contains(key.source.as_ref()));
        self.failures
            .borrow_mut()
            .retain(|key, _| used.contains(key.source.as_ref()));
        RASTERS.with(|cache| cache.borrow_mut().prune());
    }

    /// Prunes old scale/dimension owners before any paint-time allocation.
    /// Other views still using the old DPI keep their own ownership.
    pub(super) fn rasterize(&mut self, root: &Node, scale: f32, _cx: &App) -> bool {
        self.rasterize_with(root, scale)
    }

    fn rasterize_with(&mut self, root: &Node, scale: f32) -> bool {
        let old_error = self.error();
        self.scale = scale;
        let mut used = HashSet::new();
        root.visit(&mut |node| {
            if let Node::Svg {
                source,
                width,
                height,
                ..
            } = node
            {
                if let Some(key) = self.key(source, *width, *height) {
                    used.insert(key);
                }
            }
        });
        self.owners.get_mut().retain(|key, _| used.contains(key));
        self.failures
            .borrow_mut()
            .retain(|key, _| used.contains(key));
        RASTERS.with(|cache| cache.borrow_mut().prune());
        old_error != self.error()
    }

    /// Returns metadata only. Offscreen/layout-only elements allocate no raster.
    pub(super) fn get(&self, source: &str, width: f32, height: f32) -> Option<raster::Raster> {
        let key = self.key(source, width, height)?;
        if self.failures.borrow().contains_key(&key) {
            return None;
        }
        let owner = self
            .owners
            .borrow_mut()
            .entry(key.clone())
            .or_default()
            .clone();
        Some(raster::Raster::new(
            key,
            self.parsed.get(source)?.svg.clone(),
            Arc::downgrade(&owner),
            Rc::downgrade(&self.failures),
        ))
    }

    pub(super) fn error(&self) -> Option<String> {
        self.error
            .clone()
            .or_else(|| self.failures.borrow().values().next().cloned())
    }

    fn key(&self, source: &str, width: f32, height: f32) -> Option<Key> {
        let (source, asset) = self.parsed.get_key_value(source)?;
        Some(Key {
            source: source.clone(),
            size: budget::raster_size(width, height, self.scale, asset.aspect)?,
        })
    }

    #[cfg(test)]
    fn get_with(
        &self,
        source: &str,
        width: f32,
        height: f32,
        renderer: &SvgRenderer,
        cx: Option<&App>,
    ) -> Option<Arc<Bitmap>> {
        self.get(source, width, height)?.resolve(renderer, cx)
    }
}

impl Drop for Images {
    fn drop(&mut self) {
        self.owners.get_mut().clear();
        // try_with also permits normal UI-thread TLS teardown.
        let _ = RASTERS.try_with(|cache| {
            if let Ok(mut cache) = cache.try_borrow_mut() {
                cache.prune();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn svg_raster_preserves_color_instead_of_using_an_icon_mask() {
        let renderer = SvgRenderer::new(Arc::new(()));
        let svg = renderer.parse_svg(br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><rect width="1" height="1" fill="#ff0000"/><rect x="1" width="1" height="1" fill="#0000ff"/></svg>"##).unwrap();
        let image = renderer
            .render_parsed(
                &svg,
                SvgSize::ExactSize(size(DevicePixels(2), DevicePixels(1))),
            )
            .unwrap();
        assert_eq!(
            image.as_bytes(0).unwrap(),
            &[0, 0, 255, 255, 255, 0, 0, 255]
        );
        assert_ne!(
            budget::raster_size(20., 10., 1., None),
            budget::raster_size(20., 10., 2., None)
        );
    }

    #[::core::prelude::v1::test]
    fn well_formed_but_unrenderable_svg_reaches_the_content_error() {
        let source = r#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"/>"#;
        // XML/schema acceptance does not imply that the native SVG parser
        // can render it. The shell must still fall back to original source.
        let node = Node::Svg {
            source: source.into(),
            width: 640.,
            height: 360.,
            baseline: None,
            em_size: None,
        };
        node.validate().unwrap();
        let mut images = Images::default();
        images.prepare_with(&node, &SvgRenderer::new(Arc::new(())));
        assert!(images.error().is_some());
        assert!(
            images
                .get_with(source, 640., 360., &SvgRenderer::new(Arc::new(())), None)
                .is_none()
        );
    }

    #[::core::prelude::v1::test]
    fn a_square_svg_keeps_its_aspect_inside_a_wide_viewport() {
        let renderer = SvgRenderer::new(Arc::new(()));
        let source = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="red"/></svg>"#;
        let node = Node::Svg {
            source: source.into(),
            width: 640.,
            height: 360.,
            baseline: None,
            em_size: None,
        };
        let mut images = Images::default();
        images.scale = 1.;
        images.prepare_with(&node, &renderer);
        let image = images
            .get_with(source, 640., 360., &renderer, None)
            .unwrap();
        assert_eq!(
            image.image().size(0),
            size(DevicePixels(360), DevicePixels(360))
        );
        assert!(Arc::ptr_eq(
            &image,
            &images
                .get_with(source, 640., 360., &renderer, None)
                .unwrap()
        ));
    }

    #[::core::prelude::v1::test]
    fn eviction_and_last_owner_invalidate_loaders_even_while_raw_frames_survive() {
        let renderer = SvgRenderer::new(Arc::new(()));
        let source = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="red"/></svg>"#;
        let node = Node::Svg {
            source: source.into(),
            width: 20.,
            height: 20.,
            baseline: None,
            em_size: None,
        };
        let mut images = Images::default();
        images.scale = 1.;
        images.prepare_with(&node, &renderer);
        let image = images.get_with(source, 20., 20., &renderer, None).unwrap();
        let old = Arc::downgrade(&image);
        let old_frame = image.image();
        drop(image);
        RASTERS.with(|cache| cache.borrow_mut().reserve(cache::MAX_CACHE_PIXELS, 0));
        assert!(
            old.upgrade().is_none(),
            "a view-local map must not pin an evicted bitmap"
        );
        assert!(crate::ui::render_image::load(&old).is_none());
        let reloaded = images.get_with(source, 20., 20., &renderer, None).unwrap();
        assert_eq!(
            reloaded.image().size(0),
            size(DevicePixels(20), DevicePixels(20))
        );
        assert!(
            !Arc::ptr_eq(&old_frame, &reloaded.image()),
            "a retained raw frame cannot rehydrate the evicted image"
        );
        let live = Arc::downgrade(&reloaded);
        let last_frame = reloaded.image();
        drop(reloaded);
        drop(images);
        assert!(
            live.upgrade().is_none(),
            "closing the last owner releases the cache Arc"
        );
        assert!(crate::ui::render_image::load(&live).is_none());
        assert_eq!(last_frame.size(0), old_frame.size(0));
    }

    #[::core::prelude::v1::test]
    fn source_and_dpi_changes_release_only_the_changing_views_ownership() {
        let renderer = SvgRenderer::new(Arc::new(()));
        let red = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="red"/></svg>"#;
        let blue = red.replace("red", "blue");
        let node = |source: &str| Node::Svg {
            source: source.into(),
            width: 20.,
            height: 20.,
            baseline: None,
            em_size: None,
        };
        let mut first = Images::default();
        let mut second = Images::default();
        for images in [&mut first, &mut second] {
            images.prepare_with(&node(red), &renderer);
            images.rasterize_with(&node(red), 1.);
        }
        let bitmap = first.get_with(red, 20., 20., &renderer, None).unwrap();
        let old_canvas = first.get(red, 20., 20.).unwrap();
        let old = Arc::downgrade(&bitmap);
        let old_frame = bitmap.image();
        assert!(Arc::ptr_eq(
            &bitmap,
            &second.get_with(red, 20., 20., &renderer, None).unwrap()
        ));
        drop(bitmap);

        first.rasterize_with(&node(red), 2.);
        assert!(
            old_canvas.resolve(&renderer, None).is_none(),
            "an old-DPI Canvas cannot reuse another view's live cache entry"
        );
        assert!(crate::ui::render_image::load(&old).is_some());
        let high_dpi = first.get_with(red, 20., 20., &renderer, None).unwrap();
        let high_canvas = first.get(red, 20., 20.).unwrap();
        let high_loader = Arc::downgrade(&high_dpi);
        let high_frame = high_dpi.image();
        assert_eq!(high_frame.size(0), size(DevicePixels(40), DevicePixels(40)));
        assert!(!Arc::ptr_eq(&old_frame, &high_frame));
        drop(high_dpi);

        first.prepare_with(&node(&blue), &renderer);
        assert!(high_canvas.resolve(&renderer, None).is_none());
        assert!(crate::ui::render_image::load(&high_loader).is_none());
        assert!(crate::ui::render_image::load(&old).is_some());
        let different_source = first.get_with(&blue, 20., 20., &renderer, None).unwrap();
        assert!(!Arc::ptr_eq(&high_frame, &different_source.image()));
        assert_eq!(
            &different_source.image().as_bytes(0).unwrap()[..4],
            &[255, 0, 0, 255]
        );
        drop(second);
        assert!(crate::ui::render_image::load(&old).is_none());
        assert_eq!(old_frame.size(0), size(DevicePixels(20), DevicePixels(20)));
    }

    #[::core::prelude::v1::test]
    fn a_tree_larger_than_the_raster_budget_resolves_every_visible_paint() {
        let renderer = SvgRenderer::new(Arc::new(()));
        let children: Vec<_> = (0..20)
            .map(|index| Node::Svg {
                source: format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><title>large-root-{index}</title><rect width="10" height="10" fill="red"/></svg>"#),
                width: 1024.,
                height: 1024.,
                baseline: None,
                em_size: None,
            })
            .collect();
        let root = Node::Stack {
            direction: Default::default(),
            gap: Default::default(),
            transition: None,
            children: children.clone(),
        };
        let mut images = Images::default();
        images.prepare_with(&root, &renderer);
        images.rasterize_with(&root, 1.);
        let requests: Vec<_> = children
            .iter()
            .map(|node| {
                let Node::Svg {
                    source,
                    width,
                    height,
                    ..
                } = node
                else {
                    unreachable!()
                };
                images.get(source, *width, *height).unwrap()
            })
            .collect();
        assert_eq!(
            RASTERS.with(|cache| cache.borrow().usage()),
            (0, 0, 0),
            "constructing all retained elements must not rasterize any image"
        );

        let first = requests[0].resolve(&renderer, None).unwrap();
        let first_frame = first.image();
        let first_loader = Arc::downgrade(&first);
        drop(first);
        for request in &requests[1..] {
            let bitmap = request.resolve(&renderer, None).unwrap();
            assert_eq!(
                bitmap.image().size(0),
                size(DevicePixels(1024), DevicePixels(1024))
            );
        }
        assert!(crate::ui::render_image::load(&first_loader).is_none());
        let again = requests[0].resolve(&renderer, None).unwrap();
        assert!(
            !Arc::ptr_eq(&first_frame, &again.image()),
            "an evicted first source must rerasterize at paint, never stay blank"
        );
        assert_eq!(again.image().size(0), first_frame.size(0));
        assert!(RASTERS.with(|cache| cache.borrow().usage().1) <= cache::MAX_CACHE_PIXELS);
        drop(images);
        assert!(
            requests[0].resolve(&renderer, None).is_none(),
            "a retained Canvas must not revive a closed view"
        );
    }

    #[::core::prelude::v1::test]
    fn exact_raster_stays_bounded_for_a_tall_source_in_a_wide_high_dpi_box() {
        let renderer = SvgRenderer::new(Arc::new(()));
        let source = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1000"><rect width="1" height="1000" fill="red"/></svg>"#;
        let node = Node::Svg {
            source: source.into(),
            width: 4096.,
            height: 4096.,
            baseline: None,
            em_size: None,
        };
        let mut images = Images::default();
        images.scale = 4.;
        images.prepare_with(&node, &renderer);
        let image = images
            .get_with(source, 4096., 4096., &renderer, None)
            .unwrap();
        let dimensions = image.image().size(0);
        assert!(i32::from(dimensions.width) <= 5);
        assert!(i32::from(dimensions.height) <= 4096);
        assert!(
            u64::from(i32::from(dimensions.width) as u32)
                * u64::from(i32::from(dimensions.height) as u32)
                <= budget::MAX_IMAGE_PIXELS
        );
    }
}
