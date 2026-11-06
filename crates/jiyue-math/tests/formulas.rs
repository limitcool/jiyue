//! Coverage: 50+ formulas (inline and display) must render to valid, inert SVG.
#![cfg(feature = "raster")]

use jiyue_math::{MathTheme, render_math};

/// (latex, display)
const FORMULAS: &[(&str, bool)] = &[
    // Fractions, roots, scripts
    (r"\frac{a}{b}", false),
    (r"\frac{-b \pm \sqrt{b^2-4ac}}{2a}", true),
    (r"\frac{1}{1+\frac{1}{x}}", true),
    (r"\sqrt{x}", false),
    (r"\sqrt[3]{x}", false),
    (r"\sqrt{1+\sqrt{x}}", false),
    (r"x^2", false),
    (r"x_i^2", false),
    (r"\log_2 n", false),
    (r"e^{i\pi} + 1 = 0", false),
    // Big operators and limits
    (r"\sum_{i=1}^{n} i", true),
    (r"\sum_{k=0}^{\infty} \frac{x^k}{k!}", true),
    (r"\prod_{i=1}^n a_i", true),
    (r"\int_0^1 f(x)\,dx", true),
    (r"\oint_C \mathbf{F}\cdot d\mathbf{r}", true),
    (r"\lim_{x\to 0} \frac{\sin x}{x}", true),
    (r"\underbrace{a+b}_{c}", false),
    (r"\stackrel{def}{=}", false),
    // Greek, symbols, relations
    (r"\alpha + \beta = \gamma", false),
    (r"\hbar \omega", false),
    (r"a \le b \ne c \approx d", false),
    (r"\forall x \in \mathbb{R}, \exists y", false),
    (
        r"\nabla \cdot \mathbf{E} = \frac{\rho}{\varepsilon_0}",
        true,
    ),
    (r"\partial_t u = \nabla^2 u", false),
    (r"\sin\theta \cos\phi", false),
    (r"\mathrm{d}x", false),
    (r"\xrightarrow{f}", false),
    // Accents
    (r"\hat{x}", false),
    (r"\bar{y}", false),
    (r"\vec{v}", false),
    (r"\ddot{z}", false),
    (r"\dot{x}", false),
    (r"\hat{x} + \bar{y} + \vec{v} + \ddot{z}", false),
    (r"\overline{AB}", false),
    // Alphabets
    (r"\mathbb{R}^n", false),
    (r"\mathcal{L}", false),
    (r"\mathbf{v}", false),
    (r"\mathfrak{g}", false),
    (r"\mathscr{F}", false),
    // Delimiters
    (r"\left( \frac{a}{b} \right)", false),
    (r"\left\{ x \right\}", false),
    (r"\binom{n}{k}", false),
    (r"\boxed{x}", false),
    // Matrices and environments
    (r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}", true),
    (r"\begin{bmatrix} 1 & 0 \\ 0 & 1 \end{bmatrix}", true),
    (r"\begin{vmatrix} a & b \\ c & d \end{vmatrix}", true),
    (r"\begin{array}{cc} 1 & 2 \\ 3 & 4 \end{array}", true),
    (
        r"f(x)=\begin{cases} x & x>0 \\ 0 & \text{otherwise} \end{cases}",
        true,
    ),
    (r"\begin{aligned} a &= b + c \\ d &= e \end{aligned}", true),
    (r"\begin{align} x &= 1 \\ y &= 2 \end{align}", true),
    // Text
    (r"\text{if } x > 0", false),
    (r"\text{otherwise}", false),
    (r"\text{hello world}", false),
    // Spacing
    (r"a\,b\;c\quad d", false),
];

/// Formulas the renderer must reject, with the error family expected.
const INVALID: &[&str] = &[r"\frac{", r"\nosuchcommand{x}", r"}{", r"\begin{pmatrix} a"];

fn theme() -> MathTheme {
    MathTheme::default()
}

#[test]
fn coverage_set_has_at_least_forty_formulas() {
    assert!(FORMULAS.len() >= 40, "only {} formulas", FORMULAS.len());
}

#[test]
fn every_formula_renders_to_valid_inert_svg() {
    let mut failures = Vec::new();
    for (latex, display) in FORMULAS {
        match render_math(latex, *display, &theme()) {
            Ok(m) => {
                let parsed = resvg::usvg::Tree::from_str(&m.svg, &resvg::usvg::Options::default());
                if parsed.is_err() {
                    failures.push(format!("{latex}: SVG does not parse"));
                }
                if !(m.width > 0.0 && m.height >= m.baseline && m.baseline >= 0.0) {
                    failures.push(format!("{latex}: bad metrics {m:?}"));
                }
                if m.svg.contains("<script") || m.svg.contains("href") {
                    failures.push(format!("{latex}: unsafe SVG"));
                }
            }
            Err(e) => failures.push(format!("{latex} (display={display}): {e}")),
        }
    }
    assert!(failures.is_empty(), "failures:\n{}", failures.join("\n"));
}

#[test]
fn rendering_is_deterministic() {
    for (latex, display) in FORMULAS.iter().take(20) {
        let a = render_math(latex, *display, &theme()).expect("a");
        let b = render_math(latex, *display, &theme()).expect("b");
        assert_eq!(a, b, "{latex}");
    }
}

#[test]
fn invalid_formulas_are_errors_not_panics() {
    for latex in INVALID {
        assert!(
            render_math(latex, false, &theme()).is_err(),
            "{latex} should fail"
        );
    }
}

#[test]
fn display_is_not_smaller_than_inline_for_fractions() {
    let inline = render_math(r"\frac{a}{b}", false, &theme()).expect("inline");
    let display = render_math(r"\frac{a}{b}", true, &theme()).expect("display");
    assert!(display.height >= inline.height * 0.99);
}

#[test]
fn font_size_scales_metrics_linearly() {
    let small = MathTheme {
        color: "#000000".into(),
        font_size: 16.0,
    };
    let large = MathTheme {
        color: "#000000".into(),
        font_size: 32.0,
    };
    let a = render_math(r"\frac{a}{b}", false, &small).expect("small");
    let b = render_math(r"\frac{a}{b}", false, &large).expect("large");
    let ratio = b.width / a.width;
    assert!((ratio - 2.0).abs() < 0.01, "ratio {ratio}");
}

#[test]
fn colour_appears_in_output() {
    let t = MathTheme {
        color: "#336699".into(),
        font_size: 16.0,
    };
    let m = render_math("x", false, &t).expect("render");
    assert!(m.svg.to_ascii_lowercase().contains("#336699"), "{}", m.svg);
}
