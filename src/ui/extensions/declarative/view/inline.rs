//! Atomic content shares the bounded SVG raster cache and native text layout.

use super::*;

impl DeclarativeView {
    /// Atomic content uses the existing bounded SVG raster cache at the
    /// surrounding text's actual font size and device scale.
    pub(in super::super) fn inline_element(
        &mut self,
        context: &gpui_kit::base::text::InlineRenderContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::base::text::InlineElement> {
        if self.accepted_source_revision != self.source_revision || !self.ready(cx) {
            return None;
        }
        let root = self.root.clone()?;
        let Node::Svg {
            source,
            width,
            height,
            baseline,
            em_size,
        } = root.as_ref()
        else {
            return None;
        };
        let geometry = geometry(
            *width,
            *height,
            *baseline,
            em_size.unwrap_or(16.),
            f32::from(context.font_size()),
            window.scale_factor(),
        )?;
        let scale = geometry.raster_scale;
        if self.preparation.for_frame(self.root_revision, scale).images {
            self.images.rasterize(&root, scale, cx);
        }
        let image = self.images.get(source, *width, *height, cx)?;
        let element = gpui_kit::base::text::InlineElement::new(
            img(image)
                .w(px(geometry.width))
                .h(px(geometry.height))
                .object_fit(ObjectFit::Contain),
        );
        Some(match geometry.baseline {
            Some(baseline) => element.with_baseline(px(baseline)),
            None => element,
        })
    }
}

#[derive(Debug, PartialEq)]
struct Geometry {
    width: f32,
    height: f32,
    baseline: Option<f32>,
    raster_scale: f32,
}

/// Protocol-valid values can still overflow when scaled to the text's em size.
/// Reject them before updating cache preparation or passing dimensions to GPUI.
fn geometry(
    width: f32,
    height: f32,
    baseline: Option<f32>,
    em_size: f32,
    font_size: f32,
    device_scale: f32,
) -> Option<Geometry> {
    let positive = |value: f32| value.is_finite() && value > 0.;
    if ![width, height, em_size, font_size, device_scale]
        .into_iter()
        .all(positive)
        || baseline.is_some_and(|value| !value.is_finite() || value < 0. || value > height)
    {
        return None;
    }
    let ratio = font_size / em_size;
    let geometry = Geometry {
        width: width * ratio,
        height: height * ratio,
        baseline: baseline.map(|value| value * ratio),
        raster_scale: device_scale * ratio,
    };
    if ![
        ratio,
        geometry.width,
        geometry.height,
        geometry.raster_scale,
    ]
    .into_iter()
    .all(positive)
        || geometry
            .baseline
            .is_some_and(|value| !value.is_finite() || value < 0. || value > geometry.height)
    {
        return None;
    }
    Some(geometry)
}

#[cfg(test)]
mod tests {
    use super::{Geometry, geometry};

    #[::core::prelude::v1::test]
    fn inline_geometry_preserves_text_scale_and_fractional_baseline_at_each_dpi() {
        for dpi in [1., 1.25, 2.] {
            assert_eq!(
                geometry(20., 13.4672, Some(13.2752), 16., 24., dpi),
                Some(Geometry {
                    width: 30.,
                    height: 13.4672 * 1.5,
                    baseline: Some(13.2752 * 1.5),
                    raster_scale: dpi * 1.5,
                })
            );
            assert_eq!(
                geometry(20., 16., Some(0.), 16., 16., dpi)
                    .unwrap()
                    .baseline,
                Some(0.)
            );
            assert_eq!(
                geometry(20., 16., None, 16., 16., dpi).unwrap().baseline,
                None
            );
        }
    }

    #[::core::prelude::v1::test]
    fn inline_geometry_rejects_overflow_even_when_ratio_is_finite() {
        // All SVG fields pass the protocol bounds, but logical width becomes infinity.
        assert!((16_f32 / 1e-35).is_finite());
        assert!(geometry(4096., 16., Some(12.), 1e-35, 16., 1.).is_none());
        assert!(geometry(16., 4096., None, 1e-35, 16., 1.).is_none());
        // The native dimensions can be finite while raster scale overflows.
        assert!(geometry(1., 1., Some(0.), 1., 2., f32::MAX).is_none());
        assert!(geometry(1., 1., None, f32::MIN_POSITIVE, f32::MAX, 1.).is_none());
        // Multiplication can also underflow an otherwise positive ratio.
        assert!(geometry(f32::MIN_POSITIVE, 1., None, f32::MAX, 1., 1.).is_none());
    }

    #[::core::prelude::v1::test]
    fn inline_geometry_rejects_invalid_inputs_and_baseline_overshoot() {
        for value in [0., -1., f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(geometry(value, 16., None, 16., 16., 1.).is_none());
            assert!(geometry(20., value, None, 16., 16., 1.).is_none());
            assert!(geometry(20., 16., None, value, 16., 1.).is_none());
            assert!(geometry(20., 16., None, 16., value, 1.).is_none());
            assert!(geometry(20., 16., None, 16., 16., value).is_none());
        }
        for baseline in [-1., 16.001, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(geometry(20., 16., Some(baseline), 16., 16., 1.).is_none());
        }
    }
}
