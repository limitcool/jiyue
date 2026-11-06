//! Rasterises a set of formulas to PNG for visual review.
//!
//! Usage: cargo run -p jiyue-math --example shots -- <output-dir>

use std::path::PathBuf;

use jiyue_math::{MathTheme, rasterize, render_math};

/// (name, latex, display)
const SHOTS: &[(&str, &str, bool)] = &[
    ("inline_fraction", r"\frac{a}{b} + x^2", false),
    ("inline_greek", r"\alpha + \beta = \gamma", false),
    ("inline_scripts", r"x_i^2 + \sqrt[3]{x}", false),
    (
        "display_quadratic",
        r"\frac{-b \pm \sqrt{b^2-4ac}}{2a}",
        true,
    ),
    ("display_sum", r"\sum_{i=1}^{n} i", true),
    ("display_integral", r"\int_0^1 f(x)\,dx", true),
    ("display_limit", r"\lim_{x\to 0} \frac{\sin x}{x}", true),
    (
        "display_pmatrix",
        r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}",
        true,
    ),
    (
        "display_bmatrix",
        r"\begin{bmatrix} 1 & 0 \\ 0 & 1 \end{bmatrix}",
        true,
    ),
    (
        "display_cases",
        r"f(x)=\begin{cases} x & x>0 \\ 0 & \text{otherwise} \end{cases}",
        true,
    ),
    (
        "display_aligned",
        r"\begin{aligned} a &= b + c \\ d &= e \end{aligned}",
        true,
    ),
    (
        "inline_accents",
        r"\hat{x} + \bar{y} + \vec{v} + \ddot{z}",
        false,
    ),
    (
        "inline_alphabets",
        r"\mathbb{R}^n \mathcal{L} \mathbf{v}",
        false,
    ),
    (
        "inline_delims",
        r"\left( \frac{a}{b} \right) \binom{n}{k}",
        false,
    ),
    (
        "display_maxwell",
        r"\nabla \cdot \mathbf{E} = \frac{\rho}{\varepsilon_0}",
        true,
    ),
    ("display_nested_frac", r"\frac{1}{1+\frac{1}{x}}", true),
];

fn main() {
    let out_dir: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("cannot create {}: {e}", out_dir.display());
        std::process::exit(1);
    }
    let theme = MathTheme::default();
    let mut failed = 0;
    for (name, latex, display) in SHOTS {
        let rendered = match render_math(latex, *display, &theme) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{name}: render failed: {e}");
                failed += 1;
                continue;
            }
        };
        let raster = match rasterize(&rendered, 2.0) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{name}: raster failed: {e}");
                failed += 1;
                continue;
            }
        };
        // Composite over white so the PNG is readable in any viewer.
        let mut rgba = image::RgbaImage::new(raster.width + 40, raster.height + 40);
        for p in rgba.pixels_mut() {
            *p = image::Rgba([255, 255, 255, 255]);
        }
        for y in 0..raster.height {
            for x in 0..raster.width {
                let i = ((y * raster.width + x) * 4) as usize;
                let a = f32::from(raster.data[i + 3]) / 255.0;
                let blend = |c: u8| (f32::from(c) * a + 255.0 * (1.0 - a)).round() as u8;
                rgba.put_pixel(
                    x + 20,
                    y + 20,
                    image::Rgba([
                        blend(raster.data[i]),
                        blend(raster.data[i + 1]),
                        blend(raster.data[i + 2]),
                        255,
                    ]),
                );
            }
        }
        let path = out_dir.join(format!("{name}.png"));
        if let Err(e) = rgba.save(&path) {
            eprintln!("{}: save failed: {e}", path.display());
            failed += 1;
            continue;
        }
        println!(
            "{} ({}x{}, baseline {:.1}px)",
            path.display(),
            raster.width,
            raster.height,
            rendered.baseline
        );
    }
    if failed > 0 {
        std::process::exit(1);
    }
}
