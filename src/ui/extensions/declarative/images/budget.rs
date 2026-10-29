//! Plan a bounded bitmap before asking resvg to allocate it.

pub(super) const MAX_IMAGE_PIXELS: u64 = 2 * 1024 * 1024;
const MAX_SIDE: f64 = 4096.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct RasterSize {
    pub width: u32,
    pub height: u32,
}

impl RasterSize {
    pub fn pixels(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

pub(super) fn raster_size(
    width: f32,
    height: f32,
    scale: f32,
    aspect: Option<f64>,
) -> Option<RasterSize> {
    if ![width, height, scale]
        .iter()
        .all(|value| value.is_finite() && *value > 0.)
    {
        return None;
    }
    let box_width = f64::from(width) * f64::from(scale);
    let box_height = f64::from(height) * f64::from(scale);
    let ratio = aspect
        .filter(|ratio| ratio.is_finite() && *ratio > 0.)
        .unwrap_or(f64::from(width) / f64::from(height));
    let mut w = box_width;
    let mut h = w / ratio;
    if h > box_height {
        h = box_height;
        w = h * ratio;
    }
    let shrink = 1_f64
        .min(MAX_SIDE / w)
        .min(MAX_SIDE / h)
        .min((MAX_IMAGE_PIXELS as f64 / (w * h)).sqrt());
    let width = (w * shrink).floor().max(1.) as u32;
    let mut height = (h * shrink).floor().max(1.) as u32;
    // Rounding an extremely thin dimension up to one pixel still obeys the area limit.
    height = height
        .min((MAX_IMAGE_PIXELS / u64::from(width)) as u32)
        .max(1);
    Some(RasterSize { width, height })
}

pub(super) fn intrinsic_aspect(source: &str) -> Option<f64> {
    let document = roxmltree::Document::parse(source).ok()?;
    let root = document.root_element();
    if let (Some(width), Some(height)) = (
        root.attribute("width").and_then(physical_length),
        root.attribute("height").and_then(physical_length),
    ) {
        return Some(width / height);
    }
    let values: Vec<_> = root
        .attribute("viewBox")?
        .split(|ch: char| ch.is_whitespace() || ch == ',')
        .filter(|part| !part.is_empty())
        .map(str::parse::<f64>)
        .collect::<Result<_, _>>()
        .ok()?;
    if values.len() == 4
        && values.iter().all(|value| value.is_finite())
        && values[2] > 0.
        && values[3] > 0.
    {
        Some(values[2] / values[3])
    } else {
        None
    }
}

fn physical_length(value: &str) -> Option<f64> {
    let value = value.trim();
    let units = [
        ("px", 1.),
        ("in", 96.),
        ("cm", 96. / 2.54),
        ("mm", 96. / 25.4),
        ("pt", 96. / 72.),
        ("pc", 16.),
        ("Q", 96. / 101.6),
    ];
    let (number, factor) = units
        .iter()
        .find_map(|(unit, factor)| value.strip_suffix(unit).map(|number| (number, *factor)))
        .unwrap_or((value, 1.));
    let length = number.trim().parse::<f64>().ok()? * factor;
    (length.is_finite() && length > 0.).then_some(length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_and_tall_sources_fit_the_authored_box() {
        assert_eq!(
            raster_size(640., 360., 1., Some(1.)).unwrap(),
            RasterSize {
                width: 360,
                height: 360
            }
        );
        assert_eq!(
            raster_size(640., 360., 1., Some(0.1)).unwrap(),
            RasterSize {
                width: 36,
                height: 360
            }
        );
        assert_eq!(
            raster_size(640., 360., 1., Some(10.)).unwrap(),
            RasterSize {
                width: 640,
                height: 64
            }
        );
    }

    #[test]
    fn high_dpi_and_extreme_aspect_obey_both_area_and_side_limits() {
        for scale in [1., 2., 4., 16.] {
            for aspect in [Some(1.), Some(0.001), Some(1000.), None] {
                let size = raster_size(4096., 4096., scale, aspect).unwrap();
                assert!(size.pixels() <= MAX_IMAGE_PIXELS);
                assert!(size.width <= 4096 && size.height <= 4096);
                assert!(size.width > 0 && size.height > 0);
            }
        }
        assert!(raster_size(640., 360., f32::NAN, None).is_none());
        assert!(raster_size(640., 360., 0., None).is_none());
    }

    #[test]
    fn root_physical_size_then_viewbox_then_authored_ratio() {
        assert_eq!(
            intrinsic_aspect(r#"<svg width="2in" height="96px" viewBox="0 0 1 1"/>"#),
            Some(2.)
        );
        assert_eq!(
            intrinsic_aspect(r#"<svg width="100%" viewBox="0,0,20,10"/>"#),
            Some(2.)
        );
        assert_eq!(intrinsic_aspect(r#"<svg viewBox="0 0 10 0"/>"#), None);
        assert_eq!(
            raster_size(600., 300., 1., None).unwrap(),
            RasterSize {
                width: 600,
                height: 300
            }
        );
    }
}
