use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::{Arc, OnceLock};

use crate::error::DiagramError;
use crate::render::RenderedDiagram;

/// Largest accepted output, in pixels (width * height).
pub const MAX_RASTER_PIXELS: u64 = 16_000_000;

/// Straight (non-premultiplied) RGBA8 pixels, row-major, 4 bytes per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba8Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Font files that are loaded in addition to the system fonts when present.
/// Microsoft YaHei and SimHei ship with Windows and cover Chinese; Noto Sans SC is
/// picked up when installed. Nothing is bundled into the binary.
const EXTRA_FONT_FILES: &[&str] = &[
    r"C:\Windows\Fonts\msyh.ttc",
    r"C:\Windows\Fonts\msyh.ttf",
    r"C:\Windows\Fonts\simhei.ttf",
    r"C:\Windows\Fonts\simsun.ttc",
    "/System/Library/Fonts/PingFang.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
];

static SYSTEM_FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();

/// Shared font database: system fonts plus [`EXTRA_FONT_FILES`], loaded once.
pub(crate) fn font_database() -> Arc<fontdb::Database> {
    SYSTEM_FONTS
        .get_or_init(|| {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            for file in EXTRA_FONT_FILES {
                if Path::new(file).is_file() {
                    // A broken font file must not stop rendering; skip it.
                    let _ = db.load_font_file(file);
                }
            }
            db.set_sans_serif_family("Arial");
            db.set_serif_family("Times New Roman");
            db.set_monospace_family("Consolas");
            Arc::new(db)
        })
        .clone()
}

/// Rasterises `diagram` at `scale` times its natural size, on a transparent background.
///
/// `scale` must be finite and in `(0, 8]`. The output is `ceil(width * scale)` by
/// `ceil(height * scale)` pixels, and at most [`MAX_RASTER_PIXELS`] pixels in total.
/// Text uses system fonts, with per-character fallback so CJK glyphs render.
pub fn rasterize(diagram: &RenderedDiagram, scale: f32) -> Result<Rgba8Image, DiagramError> {
    if !(scale.is_finite() && scale > 0.0 && scale <= 8.0) {
        return Err(DiagramError::Render(format!(
            "invalid raster scale {scale}"
        )));
    }
    let width = (diagram.width * scale).ceil();
    let height = (diagram.height * scale).ceil();
    if !(width.is_finite() && height.is_finite() && width >= 1.0 && height >= 1.0) {
        return Err(DiagramError::Render("diagram has no pixels".into()));
    }
    let (w, h) = (width as u64, height as u64);
    if w * h > MAX_RASTER_PIXELS {
        return Err(DiagramError::TooLarge);
    }
    let (w, h) = (w as u32, h as u32);

    // The rasteriser is a third-party crate; a panic in it must not reach the UI.
    let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<Vec<u8>, DiagramError> {
        let options = resvg::usvg::Options {
            fontdb: font_database(),
            ..Default::default()
        };
        let tree = resvg::usvg::Tree::from_str(&diagram.svg, &options)
            .map_err(|e| DiagramError::Render(format!("SVG could not be loaded: {e}")))?;
        let tree_size = tree.size();
        if tree_size.width() <= 0.0 || tree_size.height() <= 0.0 {
            return Err(DiagramError::Render("SVG has no size".into()));
        }
        let mut pixmap = tiny_skia::Pixmap::new(w, h)
            .ok_or_else(|| DiagramError::Render("could not allocate the pixel buffer".into()))?;
        let transform = tiny_skia::Transform::from_scale(
            w as f32 / tree_size.width(),
            h as f32 / tree_size.height(),
        );
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        Ok(demultiply(pixmap.data()))
    }));
    match outcome {
        Ok(Ok(data)) => Ok(Rgba8Image {
            width: w,
            height: h,
            data,
        }),
        Ok(Err(err)) => Err(err),
        Err(_) => Err(DiagramError::Render(
            "the rasteriser hit an internal error".into(),
        )),
    }
}

/// Converts premultiplied RGBA to straight alpha.
fn demultiply(premultiplied: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(premultiplied.len());
    for px in premultiplied.as_chunks::<4>().0 {
        let a = px[3];
        if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
            out.extend_from_slice(px);
        } else {
            let scale = 255.0 / f32::from(a);
            for &c in &px[..3] {
                out.push((f32::from(c) * scale).round().min(255.0) as u8);
            }
            out.push(a);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiagramTheme, render_mermaid};

    fn sample(src: &str) -> RenderedDiagram {
        render_mermaid(src, &DiagramTheme::light()).unwrap_or_else(|e| panic!("render failed: {e}"))
    }

    #[test]
    fn size_at_scale_one_and_two() {
        let d = sample("flowchart LR\n A[Start] --> B[End]");
        let one = rasterize(&d, 1.0).unwrap_or_else(|e| panic!("{e}"));
        let two = rasterize(&d, 2.0).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(one.width, d.width.ceil() as u32);
        assert_eq!(one.height, d.height.ceil() as u32);
        assert_eq!(two.width, (d.width * 2.0).ceil() as u32);
        assert_eq!(two.height, (d.height * 2.0).ceil() as u32);
        assert_eq!(one.data.len(), (one.width * one.height * 4) as usize);
        assert_eq!(two.data.len(), (two.width * two.height * 4) as usize);
    }

    #[test]
    fn rasterised_pixels_are_not_blank() {
        let d = sample("pie title T\n \"a\" : 1\n \"b\" : 3");
        let img = rasterize(&d, 1.0).unwrap_or_else(|e| panic!("{e}"));
        let opaque = img
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 0)
            .count();
        assert!(opaque > 100, "only {opaque} opaque pixels");
    }

    #[test]
    fn transparent_background_has_transparent_corners() {
        let theme = DiagramTheme {
            background: "transparent".into(),
            ..DiagramTheme::light()
        };
        let d =
            render_mermaid("pie\n \"a\" : 1\n \"b\" : 2", &theme).unwrap_or_else(|e| panic!("{e}"));
        let img = rasterize(&d, 1.0).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(img.data[3], 0, "top-left pixel should be transparent");
    }

    #[test]
    fn rejects_bad_scales() {
        let d = sample("flowchart LR\n A-->B");
        assert!(rasterize(&d, 0.0).is_err());
        assert!(rasterize(&d, -1.0).is_err());
        assert!(rasterize(&d, f32::NAN).is_err());
        assert!(rasterize(&d, f32::INFINITY).is_err());
        assert!(rasterize(&d, 9.0).is_err());
    }

    #[test]
    fn huge_output_is_too_large() {
        let d = RenderedDiagram {
            svg: "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"5000\" height=\"5000\"/>"
                .into(),
            width: 5000.0,
            height: 5000.0,
        };
        assert_eq!(rasterize(&d, 1.0), Err(DiagramError::TooLarge));
    }

    #[test]
    fn demultiply_restores_straight_alpha() {
        // premultiplied half-transparent red: (128, 0, 0, 128) -> (255, 0, 0, 128)
        let out = demultiply(&[128, 0, 0, 128, 0, 0, 0, 0]);
        assert_eq!(&out[..4], &[255, 0, 0, 128]);
        assert_eq!(&out[4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn font_database_loads_system_fonts() {
        let db = font_database();
        assert!(db.faces().count() > 0, "no fonts found on this machine");
    }

    #[test]
    fn invalid_svg_is_render_error() {
        let d = RenderedDiagram {
            svg: "<svg".into(),
            width: 10.0,
            height: 10.0,
        };
        assert!(matches!(rasterize(&d, 1.0), Err(DiagramError::Render(_))));
    }
}
