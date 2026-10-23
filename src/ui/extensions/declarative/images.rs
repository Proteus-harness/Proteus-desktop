//! Native full-color SVG assets. Parsing is tied to the accepted tree;
//! raster images are cached per source, dimensions and effective DPI/zoom.
//! Scrolling and local tab/disclosure state reuse these assets.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::*;

use crate::declarative::widget::Node;

type Key = (String, u32, u32, u32);

#[derive(Default)]
pub(super) struct Images {
    parsed: HashMap<String, ParsedSvg>,
    images: HashMap<Key, Arc<RenderImage>>,
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
                if !self.parsed.contains_key(source) {
                    match renderer.parse_svg(source.as_bytes()) {
                        Ok(svg) => {
                            self.parsed.insert(source.clone(), svg);
                        }
                        Err(error) => self.error = Some(format!("SVG: {error}")),
                    }
                }
            }
        });
        self.parsed.retain(|source, _| used.contains(source));
        self.images.retain(|(source, ..), _| used.contains(source));
    }

    /// Returns whether the error changed, so the enclosing source fallback
    /// can remeasure itself even when only the device scale changed.
    pub(super) fn rasterize(&mut self, root: &Node, scale: f32, cx: &App) -> bool {
        let old_error = self.error.clone();
        let renderer = cx.svg_renderer();
        self.scale = scale;
        let mut used = HashSet::new();
        root.visit(&mut |node| {
            if let Node::Svg {
                source,
                width,
                height,
            } = node
            {
                let key = key(source, *width, *height, scale);
                used.insert(key.clone());
                if !self.images.contains_key(&key)
                    && let Some(svg) = self.parsed.get(source)
                {
                    match renderer.render_parsed(
                        svg,
                        SvgSize::Size(size(
                            DevicePixels((width * scale).ceil() as i32),
                            DevicePixels((height * scale).ceil() as i32),
                        )),
                    ) {
                        Ok(image) => {
                            self.images.insert(key, image);
                        }
                        Err(error) => self.error = Some(format!("SVG: {error}")),
                    }
                }
            }
        });
        self.images.retain(|key, _| used.contains(key));
        old_error != self.error
    }

    pub(super) fn get(&self, source: &str, width: f32, height: f32) -> Option<Arc<RenderImage>> {
        self.images
            .get(&key(source, width, height, self.scale))
            .cloned()
    }

    pub(super) fn error(&self) -> Option<String> {
        self.error.clone()
    }
}

fn key(source: &str, width: f32, height: f32, scale: f32) -> Key {
    (
        source.to_owned(),
        width.to_bits(),
        height.to_bits(),
        scale.to_bits(),
    )
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
        assert_ne!(key("same", 20., 10., 1.), key("same", 20., 10., 2.));
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
        };
        node.validate().unwrap();
        let mut images = Images::default();
        images.prepare_with(&node, &SvgRenderer::new(Arc::new(())));
        assert!(images.error().is_some());
        assert!(images.get(source, 640., 360.).is_none());
    }

    #[::core::prelude::v1::test]
    fn a_square_svg_keeps_its_aspect_inside_a_wide_viewport() {
        let renderer = SvgRenderer::new(Arc::new(()));
        let svg = renderer.parse_svg(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="red"/></svg>"#).unwrap();
        let image = renderer
            .render_parsed(
                &svg,
                SvgSize::Size(size(DevicePixels(640), DevicePixels(360))),
            )
            .unwrap();
        // The native image keeps 1:1 pixels; img.object_fit(Contain) fits
        // that image inside the authored viewport without stretching it.
        assert_eq!(image.size(0), size(DevicePixels(640), DevicePixels(640)));
    }
}
