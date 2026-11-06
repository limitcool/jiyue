//! SVG to RGBA rasteriser (resvg / tiny-skia). Compiled with feature `raster`.

use resvg::{tiny_skia, usvg};

use crate::{MathError, RenderedMath};

/// Largest accepted raster side in pixels.
pub const MAX_RASTER_SIDE: u32 = 8192;
/// Largest accepted raster area in pixels (about 64 MB of RGBA).
pub const MAX_RASTER_PIXELS: u64 = 16 * 1024 * 1024;

/// Straight (non-premultiplied) RGBA8 pixels, row-major, 4 bytes per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba8Image {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub data: Vec<u8>,
}

/// Rasterise a rendered formula at `scale` times its CSS-pixel size.
///
/// The background is transparent. `scale` must be finite and in `(0, 8]`.
pub fn rasterize(m: &RenderedMath, scale: f32) -> Result<Rgba8Image, MathError> {
    if !scale.is_finite() || scale <= 0.0 || scale > 8.0 {
        return Err(MathError::Render(format!("invalid raster scale {scale}")));
    }
    let tree = usvg::Tree::from_str(&m.svg, &usvg::Options::default())
        .map_err(|e| MathError::Render(format!("svg parse: {e}")))?;
    let size = tree.size();
    let w = (size.width() * scale).ceil();
    let h = (size.height() * scale).ceil();
    if !w.is_finite() || !h.is_finite() || w < 1.0 || h < 1.0 {
        return Err(MathError::Render("empty raster size".to_string()));
    }
    let (w, h) = (w as u32, h as u32);
    if w > MAX_RASTER_SIDE || h > MAX_RASTER_SIDE || u64::from(w) * u64::from(h) > MAX_RASTER_PIXELS
    {
        return Err(MathError::TooLarge);
    }
    let mut pixmap = tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| MathError::Render("cannot allocate pixmap".to_string()))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(Rgba8Image {
        width: w,
        height: h,
        data: pixmap.take_demultiplied(),
    })
}
