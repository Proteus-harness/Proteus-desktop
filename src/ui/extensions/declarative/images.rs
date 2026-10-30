//! Parsed SVGs belong to accepted trees. A shared bounded LRU owns rasters;
//! view-local Weak references cannot pin old DPI bitmaps indefinitely.

mod budget;
mod cache;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};

use gpui_kit::*;

use crate::declarative::widget::Node;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    source: Arc<str>,
    size: budget::RasterSize,
}

struct Asset {
    svg: ParsedSvg,
    aspect: Option<f64>,
}

thread_local! {
    static RASTERS: RefCell<cache::Cache<Key, Arc<RenderImage>>> = RefCell::default();
}

#[derive(Default)]
pub(super) struct Images {
    parsed: HashMap<Arc<str>, Asset>,
    images: RefCell<HashMap<Key, Weak<RenderImage>>>,
    failures: RefCell<HashMap<Key, String>>,
    owner: Arc<()>,
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
                                    svg,
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
        self.images
            .get_mut()
            .retain(|key, _| used.contains(key.source.as_ref()));
        self.failures
            .get_mut()
            .retain(|key, _| used.contains(key.source.as_ref()));
        RASTERS.with(|cache| {
            cache
                .borrow_mut()
                .retain_owner(&self.owner, |key| used.contains(key.source.as_ref()))
        });
    }

    /// Returns whether the error changed, so the enclosing source fallback
    /// can remeasure itself even when only the device scale changed.
    pub(super) fn rasterize(&mut self, root: &Node, scale: f32, _cx: &App) -> bool {
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
        // Release old scale/dimension entries BEFORE any lazy allocation.
        // Other views which still use that DPI retain their own ownership.
        RASTERS.with(|cache| {
            cache
                .borrow_mut()
                .retain_owner(&self.owner, |key| used.contains(key))
        });
        self.images.get_mut().retain(|key, _| used.contains(key));
        self.failures.get_mut().retain(|key, _| used.contains(key));
        old_error != self.error()
    }

    pub(super) fn get<T: 'static>(
        &self,
        source: &str,
        width: f32,
        height: f32,
        cx: &mut Context<T>,
    ) -> Option<Arc<RenderImage>> {
        let previous_error = self.error();
        let image = self.get_with(source, width, height, &cx.svg_renderer());
        if previous_error != self.error() {
            cx.notify(); // A newly failed raster reaches the source fallback once.
        }
        image
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

    fn get_with(
        &self,
        source: &str,
        width: f32,
        height: f32,
        renderer: &SvgRenderer,
    ) -> Option<Arc<RenderImage>> {
        let key = self.key(source, width, height)?;
        if self.failures.borrow().contains_key(&key) {
            return None;
        }
        if let Some(image) = RASTERS.with(|cache| cache.borrow_mut().get(&key, &self.owner)) {
            self.images.borrow_mut().insert(key, Arc::downgrade(&image));
            return Some(image);
        }
        let image = match self.images.borrow().get(&key).and_then(Weak::upgrade) {
            Some(image) => image,
            None => {
                RASTERS.with(|cache| cache.borrow_mut().reserve(key.size.pixels(), source.len()));
                // SvgSize::Size only respects width in GPUI. ExactSize uses our
                // intrinsic-aspect/contain plan and cannot allocate past 2 MP.
                let result = renderer.render_parsed(
                    &self.parsed.get(source)?.svg,
                    SvgSize::ExactSize(size(
                        DevicePixels(key.size.width as i32),
                        DevicePixels(key.size.height as i32),
                    )),
                );
                match result {
                    Ok(image) => image,
                    Err(error) => {
                        self.failures
                            .borrow_mut()
                            .insert(key, format!("SVG: {error}"));
                        return None;
                    }
                }
            }
        };
        RASTERS.with(|cache| {
            cache.borrow_mut().insert(
                key.clone(),
                image.clone(),
                key.size.pixels(),
                source.len(),
                &self.owner,
            )
        });
        self.images.borrow_mut().insert(key, Arc::downgrade(&image));
        Some(image)
    }
}

impl Drop for Images {
    fn drop(&mut self) {
        // try_with also permits normal UI-thread TLS teardown.
        let _ = RASTERS.try_with(|cache| {
            if let Ok(mut cache) = cache.try_borrow_mut() {
                cache.retain_owner(&self.owner, |_| false);
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
                .get_with(source, 640., 360., &SvgRenderer::new(Arc::new(())))
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
        let image = images.get_with(source, 640., 360., &renderer).unwrap();
        assert_eq!(image.size(0), size(DevicePixels(360), DevicePixels(360)));
        assert!(Arc::ptr_eq(
            &image,
            &images.get_with(source, 640., 360., &renderer).unwrap()
        ));
    }

    #[::core::prelude::v1::test]
    fn weak_local_entries_rehydrate_after_global_eviction_and_drop_releases_the_cache() {
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
        let image = images.get_with(source, 20., 20., &renderer).unwrap();
        let old = Arc::downgrade(&image);
        drop(image);
        RASTERS.with(|cache| cache.borrow_mut().reserve(cache::MAX_CACHE_PIXELS, 0));
        assert!(
            old.upgrade().is_none(),
            "a view-local map must not pin an evicted bitmap"
        );
        let reloaded = images.get_with(source, 20., 20., &renderer).unwrap();
        assert_eq!(reloaded.size(0), size(DevicePixels(20), DevicePixels(20)));
        let live = Arc::downgrade(&reloaded);
        drop(reloaded);
        drop(images);
        assert!(
            live.upgrade().is_none(),
            "closing the last owner releases the cache Arc"
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
        let image = images.get_with(source, 4096., 4096., &renderer).unwrap();
        let dimensions = image.size(0);
        assert!(i32::from(dimensions.width) <= 5);
        assert!(i32::from(dimensions.height) <= 4096);
        assert!(
            u64::from(i32::from(dimensions.width) as u32)
                * u64::from(i32::from(dimensions.height) as u32)
                <= budget::MAX_IMAGE_PIXELS
        );
    }
}
