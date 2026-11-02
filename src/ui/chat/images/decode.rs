//! Thumbnail preparation follows Zed's native image pipeline:
//! crates/language_model/src/request.rs. Apply orientation before resizing;
//! unlike model attachments, previews never retain encoded/full-size sources.
use std::io::Cursor;
use std::sync::Arc;

use anyhow::{Result, bail};
use gpui_kit::RenderImage;
use image::{
    AnimationDecoder, DynamicImage, Frame, ImageDecoder, ImageFormat, ImageReader, Limits,
};

pub(super) const MAX_SOURCE_PIXELS: u64 = 32 * 1024 * 1024;
pub(super) const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const MAX_FRAMES: usize = 256;
pub(super) const MAX_ANIMATION_PIXELS: u64 = 4 * 1024 * 1024;

pub(super) struct Preview {
    pub image: Arc<RenderImage>,
    pub pixels: u64,
}

pub(super) fn edge(scale: f32) -> u32 {
    if !scale.is_finite() || scale <= 0. {
        return 96;
    }
    (96. * scale).ceil().clamp(1., 384.) as u32
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    limits.max_alloc = Some(MAX_SOURCE_BYTES);
    limits
}

fn check_dimensions(width: u32, height: u32, bytes: u64) -> Result<()> {
    if width == 0
        || height == 0
        || width > 32768
        || height > 32768
        || u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS
        || bytes > MAX_SOURCE_BYTES
    {
        bail!("изображение превышает лимит декодирования (32 Мп / 128 МиБ)");
    }
    Ok(())
}

fn thumbnail(mut image: DynamicImage, edge: u32) -> image::RgbaImage {
    // Cover matches the existing square UI frame; cropping prevents extremely
    // wide sources from retaining large offscreen strips in the GPU atlas.
    let (width, height) = (image.width(), image.height());
    let side = width.min(height);
    let edge = edge.min(side);
    // Crop BEFORE resizing: resize_to_fill would create a gigantic intermediate
    // for e.g. a 32768x1 source displayed in a square.
    if width != height {
        let cropped = image.crop((width - side) / 2, (height - side) / 2, side, side);
        image = cropped;
    }
    let mut rgba = image
        .resize_exact(edge, edge, image::imageops::FilterType::Triangle)
        .into_rgba8();
    for pixel in rgba.pixels_mut() {
        pixel.0.swap(0, 2); // GPUI's RenderImage consumes BGRA, not RGBA.
    }
    rgba
}

pub(super) fn prepare(bytes: Vec<u8>, edge: u32) -> Result<Preview> {
    let edge = edge.clamp(1, 384);
    let format = image::guess_format(&bytes)?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP | ImageFormat::Gif
    ) {
        bail!("неизвестный формат изображения");
    }
    let frames = match format {
        ImageFormat::Gif => animation(
            image::codecs::gif::GifDecoder::new(Cursor::new(&bytes))?,
            edge,
            "GIF",
        )?,
        ImageFormat::WebP => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(&bytes))?;
            if decoder.has_animation() {
                decoder.set_background_color(image::Rgba([0, 0, 0, 0]))?;
                animation(decoder, edge, "WebP")?
            } else {
                still(decoder, edge)?
            }
        }
        _ => {
            let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
            reader.limits(limits());
            still(reader.into_decoder()?, edge)?
        }
    };
    let pixels = frames
        .iter()
        .map(|frame| u64::from(frame.buffer().width()) * u64::from(frame.buffer().height()))
        .sum();
    Ok(Preview {
        image: Arc::new(RenderImage::new(frames)),
        pixels,
    })
}

fn animation_edge(current: u32, count: u64) -> u32 {
    if u64::from(current) * u64::from(current) * count <= MAX_ANIMATION_PIXELS {
        return current;
    }
    let allowed = ((MAX_ANIMATION_PIXELS / count) as f64).sqrt().floor() as u32;
    // Geometric headroom bounds reprocessing to two passes at edge <=384 and
    // <=256 frames. Shrinking by a pixel on every next frame would be O(n^2).
    (current / 2).max(1).min(allowed)
}

fn still(mut decoder: impl ImageDecoder, edge: u32) -> Result<Vec<Frame>> {
    let (width, height) = decoder.dimensions();
    check_dimensions(width, height, decoder.total_bytes())?;
    decoder.set_limits(limits())?;
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(vec![Frame::new(thumbnail(image, edge))])
}

fn animation<'a>(
    mut decoder: impl ImageDecoder + AnimationDecoder<'a>,
    edge: u32,
    format: &str,
) -> Result<Vec<Frame>> {
    let (width, height) = decoder.dimensions();
    check_dimensions(width, height, decoder.total_bytes())?;
    decoder.set_limits(limits())?;
    let orientation = decoder.orientation()?;
    let mut frames: Vec<Frame> = Vec::new();
    let mut actual_edge = edge.min(width.min(height));
    // Iterate, never collect full-size frames. The decoder's disposal canvas
    // remains bounded by source dimensions; each returned full frame is moved
    // into resizing and freed before the following frame is requested.
    for frame in decoder.into_frames() {
        if frames.len() == MAX_FRAMES {
            bail!("{format} превышает лимит 256 кадров; анимация не усечена");
        }
        let frame = frame?;
        let count = frames.len() as u64 + 1;
        let next_edge = animation_edge(actual_edge, count);
        if next_edge < actual_edge {
            actual_edge = next_edge;
            for previous in &mut frames {
                let delay = previous.delay();
                // Already BGRA: resize channel-agnostically without swapping.
                let old = std::mem::replace(previous, Frame::new(image::RgbaImage::new(0, 0)));
                let resized = image::imageops::resize(
                    &old.into_buffer(),
                    actual_edge,
                    actual_edge,
                    image::imageops::FilterType::Triangle,
                );
                *previous = Frame::from_parts(resized, 0, 0, delay);
            }
        }
        let delay = frame.delay();
        let mut image = DynamicImage::ImageRgba8(frame.into_buffer());
        image.apply_orientation(orientation);
        let image = thumbnail(image, actual_edge);
        frames.push(Frame::from_parts(image, 0, 0, delay));
    }
    if frames.is_empty() {
        bail!("{format} не содержит кадров");
    }
    Ok(frames)
}

#[cfg(test)]
mod tests;
