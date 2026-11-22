//! Paint short horizontal alpha ramps on text using the same atlas sprites
//! and clip masks as normal text. Only the fading columns need extra sprites.

use crate::{ContentMask, Pixels, Primitive, ScaledPixels, Scene, Window};

#[derive(Clone, Copy)]
pub(super) struct TextEdgeFade {
    transparent: f32,
    opaque: f32,
}

impl Window {
    /// Fades glyphs, emoji, and text decoration between two horizontal window
    /// coordinates, expressed in logical pixels. The caller owns the content
    /// mask. Use only for short ramps in the paint phase.
    pub fn with_text_edge_fade<R>(
        &mut self,
        transparent: Pixels,
        opaque: Pixels,
        paint: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.invalidator.debug_assert_paint();
        if transparent == opaque {
            return paint(self);
        }
        let scale = self.scale_factor();
        let previous = self.text_edge_fade.replace(TextEdgeFade {
            transparent: transparent.0 * scale,
            opaque: opaque.0 * scale,
        });
        let result = paint(self);
        self.text_edge_fade = previous;
        result
    }

    #[inline]
    pub(crate) fn paint_text_primitive(&mut self, primitive: impl Into<Primitive>) {
        if let Some(fade) = self.text_edge_fade {
            fade.paint(primitive.into(), &mut self.next_frame.scene);
        } else {
            self.next_frame.scene.insert_primitive(primitive);
        }
    }
}

impl TextEdgeFade {
    fn paint(self, primitive: Primitive, scene: &mut Scene) {
        let visible = primitive
            .bounds()
            .intersect(&primitive.content_mask().bounds);
        if visible.is_empty() {
            return;
        }
        let left = visible.left().0;
        let right = visible.right().0;
        let fades_left = self.transparent < self.opaque;
        if (fades_left && left >= self.opaque) || (!fades_left && right <= self.opaque) {
            scene.insert_primitive(primitive);
            return;
        }
        self.spans(left, right, |left, right, alpha| {
            let mut bounds = visible;
            bounds.origin.x = ScaledPixels(left);
            bounds.size.width = ScaledPixels(right - left);
            let content_mask = ContentMask { bounds };
            let mut piece = primitive.clone();
            match &mut piece {
                Primitive::MonochromeSprite(sprite) => {
                    sprite.content_mask = content_mask;
                    sprite.color = sprite.color.opacity(alpha);
                }
                Primitive::SubpixelSprite(sprite) => {
                    sprite.content_mask = content_mask;
                    sprite.color = sprite.color.opacity(alpha);
                }
                Primitive::PolychromeSprite(sprite) => {
                    sprite.content_mask = content_mask;
                    sprite.opacity *= alpha;
                }
                Primitive::Underline(underline) => {
                    underline.content_mask = content_mask;
                    underline.color = underline.color.opacity(alpha);
                }
                _ => unreachable!("only text primitives use the text edge fade"),
            }
            scene.insert_primitive(piece);
        });
    }

    /// Glyph and mask bounds are snapped to whole device pixels. Keep the
    /// fully visible part as a single span, and sample each fading pixel at
    /// its center, so adjacent characters share the same alpha ramp.
    fn spans(self, left: f32, right: f32, mut paint: impl FnMut(f32, f32, f32)) {
        let first = self.transparent.min(self.opaque).floor();
        let last = self.transparent.max(self.opaque).ceil();
        let (opaque_left, opaque_right) = if self.transparent < self.opaque {
            (left.max(last), right)
        } else {
            (left, right.min(first))
        };
        if opaque_left < opaque_right {
            paint(opaque_left, opaque_right, 1.);
        }
        let slope = (self.opaque - self.transparent).recip();
        for column in left.max(first) as i32..right.min(last) as i32 {
            let column = column as f32;
            let alpha = ((column + 0.5 - self.transparent) * slope).clamp(0., 1.);
            if alpha > 0. {
                paint(column, column + 1., alpha);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TextEdgeFade;

    #[::core::prelude::v1::test]
    fn fade_splits_glyphs_into_device_pixel_columns_at_both_edges() {
        let mut spans = Vec::new();
        TextEdgeFade {
            transparent: 6.,
            opaque: 2.,
        }
        .spans(0., 8., |left, right, alpha| {
            spans.push((left, right, alpha))
        });
        assert_eq!(
            spans,
            [
                (0., 2., 1.),
                (2., 3., 0.875),
                (3., 4., 0.625),
                (4., 5., 0.375),
                (5., 6., 0.125)
            ]
        );

        spans.clear();
        TextEdgeFade {
            transparent: 2.,
            opaque: 6.,
        }
        .spans(0., 8., |left, right, alpha| {
            spans.push((left, right, alpha))
        });
        assert_eq!(
            spans,
            [
                (6., 8., 1.),
                (2., 3., 0.125),
                (3., 4., 0.375),
                (4., 5., 0.625),
                (5., 6., 0.875)
            ]
        );
    }
}
