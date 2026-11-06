//! Inline rich text: `[[wikilink]]` chips, **bold**, `code`, and search-term highlights.
//! Built on GPUI's StyledText runs so that mixed text wraps naturally.

use std::ops::Range;

use gpui::{
    AnyElement, ClickEvent, Context, ElementId, FontWeight, Hsla, IntoElement, MouseButton,
    MouseDownEvent, Pixels, Point, Rgba, SharedString, StrikethroughStyle, StyledText, TextLayout,
    TextRun, div, font, prelude::*, px,
};

use super::MONO_FONT;
use crate::RustdianApp;
use crate::theme::ActiveTheme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Plain,
    Bold,
    Code,
    Chip,
    Mark,
}

struct Seg {
    text: String,
    style: Style,
    target: Option<String>,
    /// The whole link body (with `#anchor`) for navigation; `target` is the note title.
    link: Option<String>,
}

/// The note a wikilink body points at: `Title`, `Title|alias` and `Title#Heading` all give
/// `Title`. Surrounding spaces are dropped; the result may be empty.
pub fn wikilink_target(inner: &str) -> &str {
    let before_alias = inner.split('|').next().unwrap_or("");
    before_alias.split('#').next().unwrap_or("").trim()
}

/// The text a wikilink chip shows: the alias after `|` when there is one, else the link as
/// written before the `|`.
fn wikilink_label(inner: &str) -> String {
    let alias = inner
        .split_once('|')
        .map(|(_, alias)| alias.trim())
        .filter(|alias| !alias.is_empty());
    match alias {
        Some(alias) => alias.to_string(),
        None => inner.split('|').next().unwrap_or("").trim().to_string(),
    }
}

/// The note title of the `[[wikilink]]` that covers byte `offset` in `text`, looking only at the
/// line that holds the offset. `None` when the offset is outside every wikilink.
pub fn wikilink_at(text: &str, offset: usize) -> Option<String> {
    let inner = wikilink_inner_at(text, offset)?;
    let target = wikilink_target(&inner);
    (!target.is_empty()).then(|| target.to_string())
}

/// The body of the `[[wikilink]]` covering byte `offset`, as written between the brackets
/// (`笔记#标题`, `笔记|别名`, `#^id`). Keeps the anchor, so the caller can jump to it.
pub fn wikilink_inner_at(text: &str, offset: usize) -> Option<String> {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[offset..].find('\n').map_or(text.len(), |i| offset + i);
    let line = &text[line_start..line_end];
    let rel = offset - line_start;

    let mut search = 0;
    while let Some(p) = line[search..].find("[[") {
        let open = search + p;
        let body_start = open + 2;
        let Some(q) = line[body_start..].find("]]") else {
            break;
        };
        let close = body_start + q + 2;
        if (open..close).contains(&rel) {
            let inner = line[body_start..body_start + q].trim();
            return (!inner.is_empty()).then(|| inner.to_string());
        }
        search = close;
    }
    None
}

/// Whether `url` is a link the app may open: `http`, `https` or `mailto`, without spaces.
pub fn is_openable_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let scheme_ok = ["http://", "https://", "mailto:"]
        .iter()
        .any(|p| lower.starts_with(p) && lower.len() > p.len());
    scheme_ok && !url.chars().any(char::is_whitespace)
}

/// The link under byte `offset` that a click opens: a Markdown link `[text](url)` (on its text or
/// its URL), or a bare `http://`, `https://` or `mailto:` address. Looks only at the line that
/// holds the offset. `None` when the offset is outside such a link, on a `[[wikilink]]` (those
/// keep their own behaviour), or when the link is not one `is_openable_url` accepts.
pub fn url_at(text: &str, offset: usize) -> Option<String> {
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    if wikilink_inner_at(text, offset).is_some() {
        return None;
    }
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[offset..].find('\n').map_or(text.len(), |i| offset + i);
    let line = &text[line_start..line_end];
    let rel = offset - line_start;
    let url = markdown_link_at(line, rel).or_else(|| bare_url_at(line, rel))?;
    is_openable_url(&url).then_some(url)
}

/// The URL of the Markdown link `[text](url)` whose span covers byte `rel` of `line`. Images
/// (`![..](..)`) are not links here.
fn markdown_link_at(line: &str, rel: usize) -> Option<String> {
    let mut search = 0;
    while let Some(p) = line[search..].find("](") {
        let mid = search + p;
        let url_start = mid + 2;
        let Some(q) = line[url_start..].find(')') else {
            break;
        };
        let end = url_start + q + 1;
        search = end;
        let Some(open) = line[..mid].rfind('[') else {
            continue;
        };
        if line[open + 1..mid].contains(']') || line[..open].ends_with('!') {
            continue;
        }
        if (open..end).contains(&rel) {
            let inner = line[url_start..url_start + q]
                .trim()
                .trim_start_matches('<')
                .trim_end_matches('>');
            // A link title (`url "title"`) follows the URL.
            return Some(inner.split_whitespace().next().unwrap_or("").to_string());
        }
    }
    None
}

/// The bare `http://`, `https://` or `mailto:` address covering byte `rel` of `line`. The address
/// ends at a space or a bracket, and trailing punctuation or an unpaired `)` is not part of it.
fn bare_url_at(line: &str, rel: usize) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    for scheme in ["https://", "http://", "mailto:"] {
        let mut search = 0;
        while let Some(p) = lower[search..].find(scheme) {
            let start = search + p;
            let body = &line[start..];
            let stop = body
                .find(|c: char| c.is_whitespace() || "<>\"'`[]".contains(c))
                .unwrap_or(body.len());
            let mut token = &body[..stop];
            loop {
                let mut t = token.trim_end_matches(|c: char| ".,;:!?".contains(c));
                if t.ends_with(')') && t.matches(')').count() > t.matches('(').count() {
                    t = &t[..t.len() - 1];
                }
                if t.len() == token.len() {
                    break;
                }
                token = t;
            }
            let end = start + token.len();
            if (start..end).contains(&rel) {
                return Some(token.to_string());
            }
            search = start + scheme.len();
        }
    }
    None
}

/// Splits source text into styled segments.
fn parse(src: &str) -> Vec<Seg> {
    let mut segs: Vec<Seg> = Vec::new();
    let mut plain = String::new();
    let mut rest = src;

    while !rest.is_empty() {
        // Find the earliest opening marker: 0 = [[, 1 = **, 2 = `
        let mut best: Option<(usize, u8)> = None;
        for (kind, pat) in [(0u8, "[["), (1u8, "**"), (2u8, "`")] {
            if let Some(p) = rest.find(pat)
                && best.is_none_or(|(bp, _)| p < bp)
            {
                best = Some((p, kind));
            }
        }
        let Some((pos, kind)) = best else {
            plain.push_str(rest);
            break;
        };
        plain.push_str(&rest[..pos]);
        let after = &rest[pos..];

        let parsed: Option<(usize, Seg)> = match kind {
            0 => after[2..].find("]]").map(|end| {
                let inner = &after[2..2 + end];
                (
                    end + 4,
                    Seg {
                        text: format!("\u{2009}{}\u{2009}", wikilink_label(inner)),
                        style: Style::Chip,
                        target: Some(wikilink_target(inner).to_string()),
                        // The link body without the alias, so `#Heading` and `#^id` reach navigation.
                        link: Some(inner.split('|').next().unwrap_or("").trim().to_string()),
                    },
                )
            }),
            1 => after[2..].find("**").map(|end| {
                (
                    end + 4,
                    Seg {
                        text: after[2..2 + end].to_string(),
                        style: Style::Bold,
                        target: None,
                        link: None,
                    },
                )
            }),
            _ => after[1..].find('`').map(|end| {
                (
                    end + 2,
                    Seg {
                        text: after[1..1 + end].to_string(),
                        style: Style::Code,
                        target: None,
                        link: None,
                    },
                )
            }),
        };

        match parsed {
            Some((consumed, seg)) => {
                if !plain.is_empty() {
                    segs.push(Seg {
                        text: std::mem::take(&mut plain),
                        style: Style::Plain,
                        target: None,
                        link: None,
                    });
                }
                segs.push(seg);
                rest = &after[consumed..];
            }
            None => {
                // Unterminated marker: keep the marker character as plain text.
                let width = if kind == 0 || kind == 1 { 2 } else { 1 };
                plain.push_str(&after[..width]);
                rest = &after[width..];
            }
        }
    }

    if !plain.is_empty() {
        segs.push(Seg {
            text: plain,
            style: Style::Plain,
            target: None,
            link: None,
        });
    }
    segs
}

/// Splits plain segments around every occurrence of `needle` and marks them.
fn apply_mark(segs: Vec<Seg>, needle: &str) -> Vec<Seg> {
    if needle.is_empty() {
        return segs;
    }
    let mut out = Vec::new();
    for seg in segs {
        if seg.style != Style::Plain {
            out.push(seg);
            continue;
        }
        let mut rest = seg.text.as_str();
        while let Some(pos) = rest.find(needle) {
            if pos > 0 {
                out.push(Seg {
                    text: rest[..pos].to_string(),
                    style: Style::Plain,
                    target: None,
                    link: None,
                });
            }
            out.push(Seg {
                text: needle.to_string(),
                style: Style::Mark,
                target: None,
                link: None,
            });
            rest = &rest[pos + needle.len()..];
        }
        if !rest.is_empty() {
            out.push(Seg {
                text: rest.to_string(),
                style: Style::Plain,
                target: None,
                link: None,
            });
        }
    }
    out
}

/// Text plus the runs and click targets derived from a source string.
pub struct Rich {
    pub text: String,
    pub runs: Vec<TextRun>,
    pub links: Vec<(Range<usize>, String)>,
}

fn hsla(c: Rgba) -> Hsla {
    c.into()
}

fn run(
    len: usize,
    family: &str,
    weight: FontWeight,
    color: Rgba,
    bg: Option<Rgba>,
    strike: bool,
) -> TextRun {
    let mut f = font(family.to_string());
    f.weight = weight;
    TextRun {
        len,
        font: f,
        color: hsla(color),
        background_color: bg.map(hsla),
        underline: None,
        strikethrough: strike.then(|| StrikethroughStyle {
            thickness: px(1.0),
            color: Some(hsla(color)),
        }),
    }
}

/// Builds the text, runs and link ranges for `src`.
///
/// `color` is the body colour, `family` the UI font family the paragraph uses, `mark` an
/// optional search term to highlight (used by backlink snippets), and `strike` draws a
/// line through plain text (completed tasks).
pub fn build(
    src: &str,
    t: &ActiveTheme,
    color: Rgba,
    family: &str,
    mark: Option<&str>,
    strike: bool,
) -> Rich {
    let mut segs = parse(src);
    if let Some(needle) = mark {
        segs = apply_mark(segs, needle);
    }

    let mut text = String::new();
    let mut runs = Vec::new();
    let mut links = Vec::new();

    for seg in segs {
        if seg.text.is_empty() {
            continue;
        }
        let start = text.len();
        text.push_str(&seg.text);
        let len = seg.text.len();
        let r = match seg.style {
            Style::Plain => run(len, family, FontWeight::NORMAL, color, None, strike),
            Style::Bold => run(len, family, FontWeight::SEMIBOLD, color, None, strike),
            Style::Code => run(
                len,
                MONO_FONT,
                FontWeight::NORMAL,
                t.tx,
                Some(t.hover),
                strike,
            ),
            Style::Chip => run(
                len,
                family,
                FontWeight::MEDIUM,
                t.ac,
                Some(t.ac_soft),
                false,
            ),
            Style::Mark => run(
                len,
                family,
                FontWeight::MEDIUM,
                t.ac,
                Some(t.ac_soft),
                false,
            ),
        };
        runs.push(r);
        if let Some(target) = seg.link.or(seg.target) {
            links.push((start..start + len, target));
        }
    }

    Rich { text, runs, links }
}

/// Renders inline text as an element. Wikilink chips are clickable and open their note.
///
/// `key` must be stable between frames (for example the source line number) so that
/// click state survives re-rendering.
// The render entry points take the text, theme and styling flags separately; a parameter struct
// would only move the same eight values into one more type at every call site.
#[allow(clippy::too_many_arguments)]
pub fn rich_text(
    src: &str,
    t: &ActiveTheme,
    color: Rgba,
    family: &str,
    mark: Option<&str>,
    strike: bool,
    key: u64,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    rich_text_geo(src, t, color, family, mark, strike, key, cx).0
}

/// Like `rich_text`, and also returns the layout of the rendered text. Blocks use the layout to
/// find the source column of a click (see `ui::editor`).
///
/// A press on a wikilink chip stops there: the blocks around the chip do not see that click, so
/// clicking a link never switches the preview to the editor.
#[allow(clippy::too_many_arguments)]
pub fn rich_text_geo(
    src: &str,
    t: &ActiveTheme,
    color: Rgba,
    family: &str,
    mark: Option<&str>,
    strike: bool,
    key: u64,
    cx: &mut Context<RustdianApp>,
) -> (AnyElement, TextLayout) {
    let rich = build(src, t, color, family, mark, strike);
    let styled = StyledText::new(SharedString::from(rich.text)).with_runs(rich.runs);
    let layout = styled.layout().clone();
    if rich.links.is_empty() {
        return (styled.into_any_element(), layout);
    }

    let ranges: Vec<Range<usize>> = rich.links.iter().map(|(r, _)| r.clone()).collect();
    let targets: Vec<String> = rich.links.into_iter().map(|(_, target)| target).collect();
    let entity = cx.entity();
    let press_ranges = ranges.clone();
    let press_layout = layout.clone();
    let click_layout = layout.clone();

    // A plain div with a click handler, not InteractiveText: the press and the release are both
    // hit-tested against the chip ranges here, so a link is opened on the same click that
    // presses it, with no paint-time state in between.
    let element = div()
        .id(ElementId::NamedInteger(SharedString::from("rich"), key))
        .child(styled)
        .on_mouse_down(
            MouseButton::Left,
            move |ev: &MouseDownEvent, _window, cx| {
                if link_index(&press_layout, &press_ranges, ev.position).is_some() {
                    cx.stop_propagation();
                }
            },
        )
        .on_click(move |ev: &ClickEvent, _window, cx| {
            let ClickEvent::Mouse(click) = ev else {
                return;
            };
            let down = link_index(&click_layout, &ranges, click.down.position);
            let up = link_index(&click_layout, &ranges, click.up.position);
            if let (Some(a), Some(b)) = (down, up)
                && a == b
            {
                let target = targets[a].clone();
                entity.update(cx, |this, cx| {
                    crate::ui::attach_ui::navigate_link(this, &target, cx);
                    cx.notify();
                });
            }
        })
        .into_any_element();
    (element, layout)
}

/// Index of the link range that holds the character at `position`, if any.
fn link_index(
    layout: &TextLayout,
    ranges: &[Range<usize>],
    position: Point<Pixels>,
) -> Option<usize> {
    match layout.index_for_position(position) {
        Ok(ix) => ranges.iter().position(|r| r.contains(&ix)),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_drops_alias_and_heading() {
        assert_eq!(wikilink_target("知识图谱"), "知识图谱");
        assert_eq!(wikilink_target("知识图谱|图谱"), "知识图谱");
        assert_eq!(wikilink_target("知识图谱#相关笔记"), "知识图谱");
        assert_eq!(wikilink_target("Title#Heading|alias"), "Title");
        assert_eq!(wikilink_target(" 空格 "), "空格");
        assert_eq!(wikilink_target("|只有别名"), "");
    }

    #[test]
    fn chip_label_shows_alias_or_written_link() {
        assert_eq!(wikilink_label("A|别名"), "别名");
        assert_eq!(wikilink_label("A#H"), "A#H");
        assert_eq!(wikilink_label("A|"), "A");
    }

    #[test]
    fn wikilink_at_finds_span_under_offset() {
        let text = "前文 [[读书笔记]] 后文\n第二行 [[X|y]]";
        let inside = text.find("读书").unwrap();
        assert_eq!(wikilink_at(text, inside), Some("读书笔记".to_string()));
        // The brackets belong to the link too.
        let open = text.find("[[").unwrap();
        assert_eq!(wikilink_at(text, open), Some("读书笔记".to_string()));
        // Plain text before the link.
        assert_eq!(wikilink_at(text, 0), None);
        // Second line, alias form.
        let second = text.find("[[X").unwrap() + 2;
        assert_eq!(wikilink_at(text, second), Some("X".to_string()));
    }

    #[test]
    fn wikilink_at_ignores_other_lines_and_bad_offsets() {
        let text = "[[A]]\nplain text here";
        assert_eq!(wikilink_at(text, text.find("plain").unwrap()), None);
        assert_eq!(wikilink_at(text, text.len() + 5), None);
        // An unterminated link is not a link.
        assert_eq!(wikilink_at("[[open only", 3), None);
        // Empty target is not a link.
        assert_eq!(wikilink_at("[[|x]]", 3), None);
    }

    #[test]
    fn wikilink_inner_at_keeps_heading_and_block_anchors() {
        let text = "见 [[笔记#标题]] 与 [[笔记#^abc]] 和 [[笔记|别名]]";
        let heading = text.find("标题").unwrap();
        assert_eq!(
            wikilink_inner_at(text, heading),
            Some("笔记#标题".to_string())
        );
        let block = text.find("^abc").unwrap();
        assert_eq!(
            wikilink_inner_at(text, block),
            Some("笔记#^abc".to_string())
        );
        let alias = text.find("别名").unwrap();
        assert_eq!(
            wikilink_inner_at(text, alias),
            Some("笔记|别名".to_string())
        );
        // The title-only view still drops the anchor.
        assert_eq!(wikilink_at(text, heading), Some("笔记".to_string()));
        assert_eq!(wikilink_inner_at(text, 0), None);
    }

    #[test]
    fn url_at_finds_markdown_links_on_text_and_url() {
        let text = "见 [文档](https://example.com/a) 后";
        let on_text = text.find("文档").unwrap();
        let on_url = text.find("example").unwrap();
        assert_eq!(
            url_at(text, on_text),
            Some("https://example.com/a".to_string())
        );
        assert_eq!(
            url_at(text, on_url),
            Some("https://example.com/a".to_string())
        );
        assert_eq!(url_at(text, 0), None);
        assert_eq!(url_at(text, text.find("后").unwrap()), None);
    }

    #[test]
    fn url_at_finds_bare_addresses_and_trims_punctuation() {
        let text = "go https://a.b/c?x=1). then mailto:me@x.org, end";
        let bare = text.find("a.b").unwrap();
        assert_eq!(url_at(text, bare), Some("https://a.b/c?x=1".to_string()));
        let mail = text.find("me@").unwrap();
        assert_eq!(url_at(text, mail), Some("mailto:me@x.org".to_string()));
        // The comma after the address is not part of it.
        assert_eq!(url_at(text, text.find(", end").unwrap()), None);
    }

    #[test]
    fn url_at_rejects_other_schemes_images_and_wikilinks() {
        assert_eq!(url_at("[x](javascript:alert(1))", 1), None);
        assert_eq!(url_at("[x](note.md)", 1), None);
        assert_eq!(url_at("![img](https://a.b/p.png)", 3), None);
        assert_eq!(url_at("[[https://a.b]]", 4), None);
        assert_eq!(url_at("https://", 2), None);
        assert!(is_openable_url("HTTPS://a.b"));
        assert!(!is_openable_url("https://"));
        assert!(!is_openable_url("file:///etc/passwd"));
    }

    #[test]
    fn parse_keeps_chip_targets_for_alias_and_heading() {
        let segs = parse("见 [[A|别名]] 与 [[B#章节]]。");
        let chips: Vec<(&str, Option<&str>)> = segs
            .iter()
            .filter(|s| s.style == Style::Chip)
            .map(|s| (s.text.as_str(), s.target.as_deref()))
            .collect();
        assert_eq!(chips.len(), 2);
        assert!(chips[0].0.contains("别名"));
        assert_eq!(chips[0].1, Some("A"));
        assert!(chips[1].0.contains("B#章节"));
        assert_eq!(chips[1].1, Some("B"));
    }
}
