use super::*;
use image::{Delay, Rgba, RgbaImage};

fn png(image: RgbaImage) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn gif(count: usize, width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        encoder
            .encode_frames((0..count).map(|index| {
                Frame::from_parts(
                    RgbaImage::from_pixel(
                        width,
                        height,
                        Rgba([
                            if index % 2 == 0 { 255 } else { 0 },
                            0,
                            if index % 2 == 0 { 0 } else { 255 },
                            255,
                        ]),
                    ),
                    0,
                    0,
                    Delay::from_numer_denom_ms((index as u32 + 1) * 10, 1),
                )
            }))
            .unwrap();
    }
    bytes
}

#[::core::prelude::v1::test]
fn scale_has_a_hard_device_pixel_cap() {
    assert_eq!(edge(1.), 96);
    assert_eq!(edge(2.), 192);
    assert_eq!(edge(1.25), 120);
    assert_eq!(edge(100.), 384);
    assert_eq!(edge(f32::NAN), 96);
}

#[::core::prelude::v1::test]
fn actual_png_is_resized_cover_cropped_and_bgra() {
    let image = RgbaImage::from_fn(1000, 100, |x, _| {
        if (450..550).contains(&x) {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([0, 0, 255, 255])
        }
    });
    let preview = prepare(png(image), 96).unwrap();
    assert_eq!(preview.image.size(0).width.0, 96);
    assert_eq!(preview.image.size(0).height.0, 96);
    assert_eq!(preview.pixels, 96 * 96);
    let bytes = preview.image.as_bytes(0).unwrap();
    assert_eq!(&bytes[(48 * 96 + 48) * 4..][..4], &[0, 0, 255, 255]);
    // The center strip fills the square; preserving the complete 10:1 source
    // then stretching would turn the red strip into only 10% of the square.
    assert_eq!(&bytes[(48 * 96 + 20) * 4..][..4], &[0, 0, 255, 255]);
}

#[::core::prelude::v1::test]
fn source_dimensions_and_decoded_bytes_rejected_before_decode() {
    assert!(check_dimensions(32769, 1, 1).is_err());
    assert!(check_dimensions(8000, 8000, 1).is_err());
    assert!(check_dimensions(100, 100, MAX_SOURCE_BYTES + 1).is_err());
    assert!(check_dimensions(4096, 4096, 4096 * 4096 * 4).is_ok());
    // Alter only the advertised GIF canvas: no image pixel decoder should run.
    let mut bytes = gif(1, 1, 1);
    bytes[6..8].copy_from_slice(&32768_u16.to_le_bytes());
    bytes[8..10].copy_from_slice(&32768_u16.to_le_bytes());
    let error = prepare(bytes, 96).err().unwrap().to_string();
    assert!(error.contains("лимит декодирования"), "{error}");
}

#[::core::prelude::v1::test]
fn gif_preserves_all_frames_and_delays_and_bounds_aggregate_pixels() {
    let preview = prepare(gif(40, 400, 400), 384).unwrap();
    assert_eq!(preview.image.frame_count(), 40);
    assert_eq!(preview.image.delay(0).numer_denom_ms(), (10, 1));
    assert_eq!(preview.image.delay(39).numer_denom_ms(), (400, 1));
    assert!(preview.pixels <= MAX_ANIMATION_PIXELS);
    assert!(preview.image.size(0).width.0 < 384);
    assert_eq!(&preview.image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
    assert_eq!(&preview.image.as_bytes(1).unwrap()[..4], &[255, 0, 0, 255]);
}

#[::core::prelude::v1::test]
fn overlong_gif_is_explicit_error_instead_of_frozen_or_truncated_preview() {
    let error = prepare(gif(MAX_FRAMES + 1, 1, 1), 1)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("256 кадров"), "{error}");
}

#[::core::prelude::v1::test]
fn jpeg_exif_orientation_is_applied_before_cover() {
    let source = RgbaImage::from_fn(64, 32, |x, y| {
        Rgba([
            if x < 32 { 240 } else { 20 },
            if y < 16 { 240 } else { 20 },
            0,
            255,
        ])
    });
    let mut jpeg = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(source)
        .write_to(&mut jpeg, ImageFormat::Jpeg)
        .unwrap();
    let original = jpeg.into_inner();
    let exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut tagged = original[..2].to_vec();
    tagged.extend_from_slice(&[0xff, 0xe1]);
    tagged.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    tagged.extend_from_slice(exif);
    tagged.extend_from_slice(&original[2..]);
    let preview = prepare(tagged, 32).unwrap();
    let mut expected = image::load_from_memory(&original).unwrap();
    expected.apply_orientation(image::metadata::Orientation::Rotate90);
    assert_eq!(
        preview.image.as_bytes(0).unwrap(),
        thumbnail(expected, 32).as_raw()
    );
}

#[::core::prelude::v1::test]
fn high_dpi_animation_has_geometric_resize_headroom() {
    let mut edge = 384;
    let mut passes = 0;
    for count in 1..=MAX_FRAMES as u64 {
        let next = animation_edge(edge, count);
        if next != edge {
            passes += 1;
        }
        edge = next;
        assert!(u64::from(edge) * u64::from(edge) * count <= MAX_ANIMATION_PIXELS);
    }
    assert_eq!(passes, 2);
    let preview = prepare(gif(MAX_FRAMES, 1, 1), 384).unwrap();
    assert_eq!(preview.image.frame_count(), MAX_FRAMES);
    assert_eq!(preview.image.delay(255).numer_denom_ms(), (2560, 1));
    assert!(preview.pixels <= MAX_ANIMATION_PIXELS);
    assert_eq!(preview.image.size(0), preview.image.size(255));
}

#[::core::prelude::v1::test]
fn small_rasters_and_animations_are_never_upscaled() {
    let still = prepare(
        png(RgbaImage::from_pixel(12, 8, Rgba([255, 0, 0, 255]))),
        384,
    )
    .unwrap();
    assert_eq!(still.image.size(0).width.0, 8);
    assert_eq!(still.pixels, 64);
    let animation = prepare(gif(3, 12, 8), 384).unwrap();
    assert_eq!(animation.image.frame_count(), 3);
    assert_eq!(animation.image.size(0).width.0, 8);
    assert_eq!(animation.image.size(2).width.0, 8);
    assert_eq!(animation.pixels, 3 * 64);
}

#[::core::prelude::v1::test]
fn animated_webp_preserves_frames_delays_colors_and_shared_pixel_budget() {
    let bytes = include_bytes!("testdata/animated-40.webp");
    let preview = prepare(bytes.to_vec(), 384).unwrap();
    assert_eq!(preview.image.frame_count(), 40);
    assert_eq!(preview.image.delay(0).numer_denom_ms(), (10, 1));
    assert_eq!(preview.image.delay(39).numer_denom_ms(), (400, 1));
    assert!(preview.pixels <= MAX_ANIMATION_PIXELS);
    assert!(preview.image.size(0).width.0 < 384);
    assert_eq!(preview.image.size(0), preview.image.size(39));
    assert_eq!(&preview.image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
    // Chained triangle downsampling can round a constant channel down by one.
    let blue = &preview.image.as_bytes(1).unwrap()[..4];
    assert!(
        blue[0] >= 254 && blue[1] == 0 && blue[2] == 0 && blue[3] == 255,
        "{blue:?}"
    );
}

#[::core::prelude::v1::test]
fn overlong_webp_is_an_explicit_error_without_silent_freezing() {
    let error = prepare(include_bytes!("testdata/overlong-257.webp").to_vec(), 96)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("WebP превышает лимит 256 кадров"), "{error}");
}
