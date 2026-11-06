//! Small colour helpers used to derive a full Mermaid palette from a few theme colours.

pub(crate) type Rgb = (u8, u8, u8);

/// Parses `#RRGGBB` (case-insensitive). Anything else yields `None`.
pub(crate) fn parse_hex(value: &str) -> Option<Rgb> {
    let bytes = value.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' {
        return None;
    }
    let r = u8::from_str_radix(value.get(1..3)?, 16).ok()?;
    let g = u8::from_str_radix(value.get(3..5)?, 16).ok()?;
    let b = u8::from_str_radix(value.get(5..7)?, 16).ok()?;
    Some((r, g, b))
}

pub(crate) fn to_hex(c: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", c.0, c.1, c.2)
}

/// Linear interpolation between `a` (t = 0) and `b` (t = 1).
pub(crate) fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let lerp = |x: u8, y: u8| -> u8 {
        let v = f32::from(x) + (f32::from(y) - f32::from(x)) * t;
        v.round().clamp(0.0, 255.0) as u8
    };
    (lerp(a.0, b.0), lerp(a.1, b.1), lerp(a.2, b.2))
}

/// WCAG relative luminance in `[0, 1]`.
pub(crate) fn luminance(c: Rgb) -> f32 {
    let lin = |v: u8| -> f32 {
        let s = f32::from(v) / 255.0;
        if s <= 0.039_28 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2)
}

/// WCAG contrast ratio between two colours (>= 1).
pub(crate) fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Dark or white text, whichever reads better on `fill`.
pub(crate) fn text_on(fill: Rgb) -> String {
    let dark = (0x0B, 0x10, 0x20);
    let white = (0xFF, 0xFF, 0xFF);
    if contrast(fill, dark) >= contrast(fill, white) {
        to_hex(dark)
    } else {
        to_hex(white)
    }
}

fn rgb_to_hsl(c: Rgb) -> (f32, f32, f32) {
    let r = f32::from(c.0) / 255.0;
    let g = f32::from(c.1) / 255.0;
    let b = f32::from(c.2) / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d <= f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < f32::EPSILON {
        ((g - b) / d).rem_euclid(6.0)
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Rgb {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to_u8 = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(r), to_u8(g), to_u8(b))
}

/// `n` distinct hues spread with the golden angle, starting at the accent hue.
/// Saturation follows the accent (clamped to a readable range); lightness is
/// tuned so that dark text reads on every slice in both light and dark themes.
pub(crate) fn palette(accent: Rgb, n: usize, dark: bool) -> Vec<String> {
    let (h, s, _) = rgb_to_hsl(accent);
    let sat = s.clamp(0.45, 0.75);
    let light = if dark { 0.66 } else { 0.60 };
    (0..n)
        .map(|i| {
            let hue = h + (i as f32) * 137.508;
            to_hex(hsl_to_rgb(hue, sat, light))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colours() {
        assert_eq!(parse_hex("#1e90FF"), Some((0x1e, 0x90, 0xff)));
        assert_eq!(parse_hex("#fff"), None);
        assert_eq!(parse_hex("red"), None);
        assert_eq!(parse_hex("#GG0000"), None);
        assert_eq!(parse_hex("#ééé"), None);
    }

    #[test]
    fn mix_endpoints_and_midpoint() {
        assert_eq!(mix((0, 0, 0), (255, 255, 255), 0.0), (0, 0, 0));
        assert_eq!(mix((0, 0, 0), (255, 255, 255), 1.0), (255, 255, 255));
        assert_eq!(mix((0, 0, 0), (200, 100, 50), 0.5), (100, 50, 25));
    }

    #[test]
    fn text_on_picks_readable_text() {
        assert_eq!(text_on((255, 255, 255)), "#0B1020");
        assert_eq!(text_on((0, 0, 0)), "#FFFFFF");
    }

    #[test]
    fn palette_has_requested_size_and_valid_hex() {
        let p = palette((37, 99, 235), 12, false);
        assert_eq!(p.len(), 12);
        for c in &p {
            assert!(parse_hex(c).is_some(), "bad colour {c}");
        }
        // distinct hues
        let mut sorted = p.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 12);
    }

    #[test]
    fn hsl_roundtrip_is_close() {
        let c = (120, 60, 200);
        let (h, s, l) = rgb_to_hsl(c);
        let back = hsl_to_rgb(h, s, l);
        assert!((i32::from(back.0) - 120).abs() <= 1);
        assert!((i32::from(back.1) - 60).abs() <= 1);
        assert!((i32::from(back.2) - 200).abs() <= 1);
    }

    #[test]
    fn contrast_black_white_is_21() {
        let c = contrast((0, 0, 0), (255, 255, 255));
        assert!((c - 21.0).abs() < 0.01);
    }
}
