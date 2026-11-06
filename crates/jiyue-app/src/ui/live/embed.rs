//! Images and note embeds of the live preview (pure logic, no gpui).
//!
//! An image or a note embed alone on its source line is a block widget (see `map::BlockKind`).
//! This module reads such a line ([`line_embed`]), sizes an image inside the column ([`image_box`]),
//! cuts the text of a note excerpt for `![[note]]`, `![[note#Heading]]` or `![[note#^block]]`
//! ([`excerpt`]) and shows the excerpt lines with their inline markers hidden ([`excerpt_pieces`]).
//! Decoding, the file index and the image cache live in `context.rs`.

use std::ops::Range;

use jiyue_md::embeds::{
    Anchor, EmbedKind, LinkKind, embed_kind, extract_block_text, extract_section, parse_links,
};
use jiyue_md::frontmatter;

use super::map;
use crate::ui::attach_ui::resolve::display_size;
use crate::ui::md_style;

/// Lines of a note shown by an embed excerpt before the cut-off mark.
pub const EXCERPT_LINES: usize = 12;
/// Tallest an image may be drawn, in logical pixels (a taller image is scaled down to it).
pub const IMAGE_MAX_H: f32 = 560.0;
/// Images over this many bytes on disk are not decoded.
pub const IMAGE_MAX_BYTES: u64 = 20 * 1024 * 1024;
/// Longest side of the pixels kept for the GPU. Larger images are downscaled once when decoded.
pub const DECODE_MAX_SIDE: u32 = 1600;

/// What a line holds: an image, a remote image (never fetched) or a note embed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedClass {
    /// `![alt](path)` or `![[file.png]]` naming a local image.
    Image,
    /// `![alt](https://...)`: a placeholder with the URL, nothing is fetched.
    Remote,
    /// `![[note]]`, `![[note#Heading]]`, `![[note#^block]]`.
    Note,
}

/// One embed line, read from its source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedLine {
    pub class: EmbedClass,
    /// The file path (image), the URL (remote) or the note title (note).
    pub target: String,
    /// The `#Heading` or `#^block` part of a note embed.
    pub anchor: Option<Anchor>,
    /// Alt text or wiki alias (the caption).
    pub alias: Option<String>,
    /// `|300` or `|300x200` (only images use it).
    pub size: Option<(u32, Option<u32>)>,
}

/// Reads a source line that holds exactly one image or note embed and nothing else. An indented
/// line (four spaces or a tab) is code, not an embed. Other embeds (PDF, audio, video, files) are
/// not widgets and give `None`.
pub fn line_embed(line: &str) -> Option<EmbedLine> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent >= 4 || line.starts_with('\t') {
        return None;
    }
    let trimmed = line.trim();
    if !trimmed.starts_with("![") {
        return None;
    }
    let links = parse_links(trimmed);
    let [link] = links.as_slice() else {
        return None;
    };
    if link.range != (0..trimmed.len()) {
        return None;
    }
    match link.kind {
        LinkKind::MdImage if link.external => Some(EmbedLine {
            class: EmbedClass::Remote,
            target: link.target.clone(),
            anchor: None,
            alias: link.alias.clone(),
            size: None,
        }),
        LinkKind::MdImage | LinkKind::Embed if link.external => None,
        LinkKind::MdImage if embed_kind(&link.target) == EmbedKind::Image => Some(EmbedLine {
            class: EmbedClass::Image,
            target: link.target.clone(),
            anchor: None,
            alias: link.alias.clone(),
            size: link.size,
        }),
        LinkKind::Embed if embed_kind(&link.target) == EmbedKind::Image => Some(EmbedLine {
            class: EmbedClass::Image,
            target: link.target.clone(),
            anchor: None,
            alias: link.alias.clone(),
            size: link.size,
        }),
        LinkKind::Embed
            if embed_kind(&link.target) == EmbedKind::Note && !link.target.trim().is_empty() =>
        {
            Some(EmbedLine {
                class: EmbedClass::Note,
                target: link.target.trim().to_string(),
                anchor: link.anchor.clone(),
                alias: link.alias.clone(),
                size: None,
            })
        }
        _ => None,
    }
}

/// The raw link body of a note embed as `[[...]]` would write it: `title`, `title#Heading` or
/// `title#^block`. Opening it goes through the same path as a wikilink.
pub fn note_link_body(title: &str, anchor: Option<&Anchor>) -> String {
    match anchor {
        None => title.to_string(),
        Some(Anchor::Heading(h)) => format!("{title}#{h}"),
        Some(Anchor::Block(id)) => format!("{title}#^{id}"),
    }
}

/// Size of an image box inside `column`: the natural size, scaled down to the `|300` / `|300x200`
/// box and to the column, never enlarged, and scaled down again when taller than [`IMAGE_MAX_H`].
/// `None` for an image with a zero dimension.
pub fn image_box(
    natural: (u32, u32),
    size: Option<(u32, Option<u32>)>,
    column: f32,
) -> Option<(f32, f32)> {
    let (w, h) = display_size(natural, size, column)?;
    if h > IMAGE_MAX_H {
        let s = IMAGE_MAX_H / h;
        Some((w * s, IMAGE_MAX_H))
    } else {
        Some((w, h))
    }
}

/// Pixel size to decode an image of `natural` size at: its longest side at most `max_side`, the
/// aspect ratio kept. Images that already fit are not changed.
pub fn decode_size(natural: (u32, u32), max_side: u32) -> (u32, u32) {
    let (w, h) = natural;
    let longest = w.max(h);
    if longest <= max_side || longest == 0 {
        return natural;
    }
    let s = f64::from(max_side) / f64::from(longest);
    let nw = ((f64::from(w) * s).round() as u32).max(1);
    let nh = ((f64::from(h) * s).round() as u32).max(1);
    (nw, nh)
}

/// The text of a note embed: the whole body (after the frontmatter), one heading section or one
/// block, cut to [`EXCERPT_LINES`] lines. Leading and trailing blank lines are dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excerpt {
    /// The shown text, lines joined with `\n`.
    pub text: String,
    /// Number of lines shown.
    pub lines: usize,
    /// True when the note has more lines than shown.
    pub more: bool,
}

/// Why an excerpt cannot be cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcerptError {
    NoHeading,
    NoBlock,
}

/// The excerpt of `note` (its full content) for `anchor`.
pub fn excerpt(note: &str, anchor: Option<&Anchor>) -> Result<Excerpt, ExcerptError> {
    let body: String = match anchor {
        None => {
            let (_, start) = frontmatter::split(note);
            note.get(start..).unwrap_or(note).to_string()
        }
        Some(Anchor::Heading(h)) => extract_section(note, h).ok_or(ExcerptError::NoHeading)?,
        Some(Anchor::Block(id)) => extract_block_text(note, id).ok_or(ExcerptError::NoBlock)?,
    };
    let all: Vec<&str> = body.lines().collect();
    let first = all
        .iter()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(all.len());
    let last = all
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map_or(first, |i| i + 1);
    let kept = &all[first.min(last)..last];
    let more = kept.len() > EXCERPT_LINES;
    let shown: Vec<&str> = kept.iter().take(EXCERPT_LINES).copied().collect();
    Ok(Excerpt {
        text: shown.join("\n"),
        lines: shown.len(),
        more,
    })
}

/// The pieces of each line of an excerpt (`text` joined with `\n`): inline markers hidden, block
/// records (code, tables, callouts, embeds) left out, so an excerpt shows its text only. One entry
/// per line, in order. Piece offsets are offsets of `text`.
pub fn excerpt_pieces(text: &str) -> Vec<Vec<map::Piece>> {
    let spans = md_style::compute_spans(text);
    let lines = md_style::split_lines(text);
    let segs = md_style::line_segments(&spans, &lines);
    let hides: Vec<map::Hide> = map::compute_hides(text, &spans)
        .into_iter()
        .filter(|h| !matches!(h.deco, map::Deco::Block(_)))
        .collect();
    // A selection far outside the text: nothing is revealed, every marker is hidden.
    let none = usize::MAX - 1..usize::MAX - 1;
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let line_range: Range<usize> = line.clone();
            let lo = hides.partition_point(|h| h.src.end <= line_range.start);
            let hi = hides.partition_point(|h| h.src.start < line_range.end);
            map::line_pieces(
                line_range,
                &segs[i],
                &hides[lo..hi.max(lo)],
                &none,
                None,
                true,
            )
        })
        .collect()
}

/// The shown text of an excerpt line, from its pieces.
pub fn excerpt_line_text(pieces: &[map::Piece], text: &str) -> String {
    map::display_text(pieces, text)
}

/// Font size of a block formula, in CSS pixels, and the scale its raster is made at (so it is
/// drawn crisp on a 2x display).
pub const MATH_FONT_PX: f32 = 18.0;
pub const MATH_RENDER_SCALE: f32 = 2.0;

/// The LaTeX of a block formula: the first display span (`$$...$$`) of the block's source.
/// `None` when the block holds no display span.
pub fn math_latex(block: &str) -> Option<String> {
    jiyue_math::find_math_spans(block)
        .into_iter()
        .find(|s| s.display)
        .map(|s| s.latex)
        .filter(|s| !s.is_empty())
}

/// Size of a formula drawn in a column of `column` width: its CSS size, scaled down when it is
/// wider than the column (never enlarged).
pub fn math_box(css: (f32, f32), column: f32) -> (f32, f32) {
    let (w, h) = css;
    if w <= 0.0 || h <= 0.0 || column <= 0.0 || w <= column {
        return (w.max(0.0), h.max(0.0));
    }
    let s = column / w;
    (column, h * s)
}

/// A short Chinese reason for a formula the renderer refused, shown in the card title. The
/// renderer's own message (which can hold Debug text such as `Unknown("...")`) is never shown.
pub fn math_error_reason(err: &jiyue_math::MathError) -> &'static str {
    use jiyue_math::MathError;
    match err {
        MathError::Parse { .. } => "语法错误",
        MathError::Unsupported(_) => "不支持的写法",
        MathError::TooLarge => "公式太长",
        MathError::Timeout => "渲染超时",
        MathError::Render(_) => "渲染失败",
    }
}

/// `#RRGGBB` of a colour with channels in `0..=1`.
pub fn hex_colour(r: f32, g: f32, b: f32) -> String {
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}", c(r), c(g), c(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_latex_is_the_display_span_inside_the_block() {
        assert_eq!(
            math_latex("$$\n\\frac{a}{b}\n$$"),
            Some("\\frac{a}{b}".to_string())
        );
        assert_eq!(
            math_latex("$$  $$"),
            None,
            "an empty formula is not a block"
        );
        assert_eq!(math_latex("plain text"), None);
    }

    #[test]
    fn math_box_scales_down_to_the_column_only() {
        assert_eq!(math_box((100.0, 40.0), 620.0), (100.0, 40.0));
        assert_eq!(math_box((1240.0, 80.0), 620.0), (620.0, 40.0));
        assert_eq!(math_box((0.0, 40.0), 620.0), (0.0, 40.0));
    }

    #[test]
    fn hex_colour_rounds_channels() {
        assert_eq!(hex_colour(0.0, 0.5, 1.0), "#0080FF");
        assert_eq!(hex_colour(1.5, -1.0, 0.0), "#FF0000");
    }

    #[test]
    fn local_image_on_its_own_line() {
        let e = line_embed("![alt text](figs/pic.png)").unwrap();
        assert_eq!(e.class, EmbedClass::Image);
        assert_eq!(e.target, "figs/pic.png");
        assert_eq!(e.alias.as_deref(), Some("alt text"));
    }

    #[test]
    fn wiki_image_with_size_and_caption() {
        let e = line_embed("![[pic.png|300x200]]").unwrap();
        assert_eq!(e.class, EmbedClass::Image);
        assert_eq!(e.size, Some((300, Some(200))));
        let w = line_embed("![[pic.png|300]]").unwrap();
        assert_eq!(w.size, Some((300, None)));
        // Text after the first pipe is the alias (the caption); a size is only `|300` or `|300x200`.
        let cap = line_embed("![[pic.png|caption]]").unwrap();
        assert_eq!(cap.alias.as_deref(), Some("caption"));
        assert_eq!(cap.size, None);
    }

    #[test]
    fn remote_image_is_a_placeholder_class() {
        let e = line_embed("![](https://example.com/a.png)").unwrap();
        assert_eq!(e.class, EmbedClass::Remote);
        assert_eq!(e.target, "https://example.com/a.png");
    }

    #[test]
    fn note_embeds_keep_anchors() {
        let plain = line_embed("![[Plan]]").unwrap();
        assert_eq!(plain.class, EmbedClass::Note);
        assert_eq!(plain.anchor, None);
        let h = line_embed("![[Plan#Goals]]").unwrap();
        assert_eq!(h.anchor, Some(Anchor::Heading("Goals".into())));
        let b = line_embed("![[Plan#^abc12]]").unwrap();
        assert_eq!(b.anchor, Some(Anchor::Block("abc12".into())));
        assert_eq!(note_link_body("Plan", b.anchor.as_ref()), "Plan#^abc12");
    }

    #[test]
    fn only_a_whole_line_embed_counts() {
        assert!(line_embed("see ![[pic.png]] here").is_none());
        assert!(line_embed("> ![[pic.png]]").is_none());
        assert!(line_embed("- ![[pic.png]]").is_none());
        assert!(line_embed("![[a.png]] ![[b.png]]").is_none());
        assert!(line_embed("    ![[pic.png]]").is_none(), "indented is code");
        assert!(line_embed("\t![[pic.png]]").is_none());
    }

    #[test]
    fn other_files_and_empty_targets_are_not_widgets() {
        assert!(line_embed("![[report.pdf]]").is_none());
        assert!(line_embed("![[movie.mp4]]").is_none());
        assert!(line_embed("![[#Heading]]").is_none());
        assert!(line_embed("![[]]").is_none());
        assert!(
            line_embed("[[Plan]]").is_none(),
            "a plain wikilink is not an embed"
        );
    }

    #[test]
    fn image_box_fits_the_column_and_the_size_box() {
        assert_eq!(image_box((1000, 500), None, 620.0), Some((620.0, 310.0)));
        assert_eq!(
            image_box((1000, 500), Some((300, None)), 620.0),
            Some((300.0, 150.0))
        );
        // Width and height box: the tighter one wins, aspect kept.
        assert_eq!(
            image_box((1000, 500), Some((300, Some(200))), 620.0),
            Some((300.0, 150.0))
        );
        // Never enlarged.
        assert_eq!(image_box((100, 50), None, 620.0), Some((100.0, 50.0)));
        assert_eq!(
            image_box((100, 50), Some((400, None)), 620.0),
            Some((100.0, 50.0))
        );
        // Tall images are capped in height.
        let (w, h) = image_box((400, 2000), None, 620.0).unwrap();
        assert!((h - IMAGE_MAX_H).abs() < 0.01);
        assert!((w - 112.0).abs() < 0.5);
        assert_eq!(image_box((0, 10), None, 620.0), None);
    }

    #[test]
    fn decode_size_keeps_aspect_and_never_grows() {
        assert_eq!(decode_size((800, 600), 1600), (800, 600));
        assert_eq!(decode_size((4000, 2000), 1600), (1600, 800));
        assert_eq!(decode_size((2000, 4000), 1000), (500, 1000));
        assert_eq!(decode_size((0, 0), 1000), (0, 0));
    }

    #[test]
    fn excerpt_of_a_whole_note_skips_frontmatter() {
        let note = "---\ntitle: x\n---\nfirst\n\nsecond\n";
        let e = excerpt(note, None).unwrap();
        assert_eq!(e.text, "first\n\nsecond");
        assert_eq!(e.lines, 3);
        assert!(!e.more);
    }

    #[test]
    fn excerpt_of_a_heading_section_and_a_block() {
        let note = "# Top\nintro\n## Goals\nalpha\nbeta\n## Other\nno\n\nclaim text ^abc12\n";
        let h = excerpt(note, Some(&Anchor::Heading("Goals".into()))).unwrap();
        assert!(h.text.starts_with("## Goals"));
        assert!(h.text.contains("beta"));
        assert!(!h.text.contains("Other"));
        let b = excerpt(note, Some(&Anchor::Block("abc12".into()))).unwrap();
        assert_eq!(b.text, "claim text");
        assert_eq!(
            excerpt(note, Some(&Anchor::Heading("Missing".into()))),
            Err(ExcerptError::NoHeading)
        );
        assert_eq!(
            excerpt(note, Some(&Anchor::Block("zzzzz".into()))),
            Err(ExcerptError::NoBlock)
        );
    }

    #[test]
    fn excerpt_is_cut_after_twelve_lines() {
        let note: String = (1..=20).map(|i| format!("line {i}\n")).collect();
        let e = excerpt(&note, None).unwrap();
        assert_eq!(e.lines, EXCERPT_LINES);
        assert!(e.more);
        assert!(e.text.ends_with("line 12"));
    }

    #[test]
    fn excerpt_lines_hide_inline_markers() {
        let text = "a **bold** and [[Other|shown]]\n\n- [ ] task\n# Head";
        let lines = excerpt_pieces(text);
        assert_eq!(lines.len(), 4);
        assert_eq!(excerpt_line_text(&lines[0], text), "a bold and shown");
        assert_eq!(
            excerpt_line_text(&lines[2], text),
            format!(" {} task", map::CHECKBOX_SPACE)
        );
        assert_eq!(excerpt_line_text(&lines[3], text), "Head");
    }

    #[test]
    fn excerpt_shows_code_as_source_text() {
        // A fenced block inside an excerpt is not a widget there: its markers stay as source.
        let text = "```\n**x**\n```";
        let lines = excerpt_pieces(text);
        assert_eq!(excerpt_line_text(&lines[1], text), "**x**");
    }
}
