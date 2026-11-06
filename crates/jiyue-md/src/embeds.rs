//! Links, embeds, headings and block references in Markdown notes.
//!
//! Supported syntax: `[[note]]`, `[[note|alias]]`, `[[note#Heading]]`, `[[note#^block]]`,
//! `[[#Heading]]`, `![[file.png|300]]`, `![[file.png|300x200]]`, `![[note#Heading]]`,
//! `[text](path/to/note.md#h "title")`, `![alt|300](<my file.png>)` and percent-encoded paths.
//!
//! All offsets are byte offsets into the original text and always fall on char boundaries.
//! Fenced code blocks, inline code spans and HTML comments are blanked out before scanning,
//! so nothing inside them is reported. The scanner is linear in the input size.

use std::collections::HashMap;
use std::ops::Range;

/// The syntactic form of a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// `[[target]]`
    Wiki,
    /// `![[target]]`
    Embed,
    /// `[text](target)`
    MdLink,
    /// `![alt](target)`
    MdImage,
}

/// The part after `#` in a link target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    /// `#Heading` (may contain `#` separated nested headings, e.g. `A#B`).
    Heading(String),
    /// `#^blockid`, stored without the caret.
    Block(String),
}

/// One link or embed found in a note.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    /// Byte range of the whole construct, including brackets and `!`.
    pub range: Range<usize>,
    pub kind: LinkKind,
    /// Decoded target. Empty for same-note links such as `[[#Heading]]`.
    /// For external URLs this is the full URL.
    pub target: String,
    /// Byte range of the raw (undecoded) target text inside the construct.
    pub target_range: Range<usize>,
    pub anchor: Option<Anchor>,
    /// Display text: wiki alias, link text, or image alt text (without a size suffix).
    pub alias: Option<String>,
    /// Display size from `|300` or `|300x200`. Only set for embeds and images.
    pub size: Option<(u32, Option<u32>)>,
    /// Markdown link title (`"title"`).
    pub title: Option<String>,
    /// True for `http(s)://`, `mailto:` and other URL-scheme targets. Such links are
    /// reported but never resolve to a vault file.
    pub external: bool,
}

/// Classification of a link target by file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedKind {
    /// Markdown note, or a target without an extension.
    Note,
    Image,
    Pdf,
    Audio,
    Video,
    /// Any other extension.
    Other,
}

/// A block reference (`^id`) and the block it labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockId {
    /// The id without the caret.
    pub id: String,
    /// 0-based index of the first line of the labelled block.
    pub line_start: usize,
    /// Byte range of the labelled paragraph, list item, quote or heading.
    /// The standalone `^id` line is not part of the range.
    pub range: Range<usize>,
}

/// An ATX heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// 1..=6.
    pub level: u8,
    /// Heading text without `#` markers and closing sequence.
    pub text: String,
    /// 0-based line index.
    pub line: usize,
    /// Byte range of the heading line (without its line terminator).
    pub range: Range<usize>,
}

// ---------------------------------------------------------------------------
// Line scanning and masking
// ---------------------------------------------------------------------------

/// One physical line. `end` excludes the `\n` and a preceding `\r`.
#[derive(Debug, Clone, Copy)]
struct LineSpan {
    start: usize,
    end: usize,
    fenced: bool,
}

fn line_text<'a>(text: &'a str, l: &LineSpan) -> &'a str {
    text.get(l.start..l.end).unwrap_or("")
}

/// Returns `(fence char, fence length)` when the line opens a fenced code block.
fn opening_fence(content: &str) -> Option<(u8, usize)> {
    let t = content.trim_start_matches([' ', '\t']);
    let ch = *t.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let n = t.bytes().take_while(|&c| c == ch).count();
    if n < 3 {
        return None;
    }
    if ch == b'`' && t[n..].contains('`') {
        return None;
    }
    Some((ch, n))
}

fn is_closing_fence(content: &str, ch: u8, n: usize) -> bool {
    let t = content.trim_start_matches([' ', '\t']);
    let m = t.bytes().take_while(|&c| c == ch).count();
    m >= n && t[m..].trim().is_empty()
}

/// Splits the text into lines and marks the lines that belong to fenced code blocks.
/// An unclosed fence runs to the end of the text.
fn scan_lines(text: &str) -> Vec<LineSpan> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut fence: Option<(u8, usize)> = None;
    while start < len {
        let nl = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(len, |p| start + p);
        let next = if nl < len { nl + 1 } else { len };
        let mut end = nl;
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        let content = text.get(start..end).unwrap_or("");
        let fenced = match fence {
            Some((ch, n)) => {
                if is_closing_fence(content, ch, n) {
                    fence = None;
                }
                true
            }
            None => match opening_fence(content) {
                Some(f) => {
                    fence = Some(f);
                    true
                }
                None => false,
            },
        };
        lines.push(LineSpan { start, end, fenced });
        start = next;
    }
    lines
}

fn fill(buf: &mut [u8], a: usize, b: usize) {
    if let Some(s) = buf.get_mut(a..b) {
        s.fill(b' ');
    }
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Blanks inline code spans on one line. A backtick run is closed by the next run of the same
/// length on that line; unmatched runs are literal.
fn mask_code_spans(text: &str, buf: &mut [u8], a: usize, b: usize) {
    let bytes = text.as_bytes().get(a..b).unwrap_or(&[]);
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let s = i;
            while i < bytes.len() && bytes[i] == b'`' {
                i += 1;
            }
            // A backslash-escaped opening backtick is literal.
            if s > 0 && bytes[s - 1] == b'\\' {
                continue;
            }
            runs.push((s, i - s));
        } else {
            i += 1;
        }
    }
    let n = runs.len();
    let mut next_same: Vec<Option<usize>> = vec![None; n];
    let mut last: HashMap<usize, usize> = HashMap::new();
    for k in (0..n).rev() {
        next_same[k] = last.get(&runs[k].1).copied();
        last.insert(runs[k].1, k);
    }
    let mut i = 0usize;
    while i < n {
        match next_same[i] {
            Some(j) => {
                let s = a + runs[i].0;
                let e = a + runs[j].0 + runs[j].1;
                fill(buf, s, e);
                i = j + 1;
            }
            None => i += 1,
        }
    }
}

/// Returns a byte buffer of the same length as `text` in which fenced code, inline code and
/// HTML comments are replaced by spaces. Replaced regions always begin and end at ASCII
/// boundaries, so the buffer stays valid UTF-8.
fn mask(text: &str) -> Vec<u8> {
    let mut buf = text.as_bytes().to_vec();
    let lines = scan_lines(text);
    for l in &lines {
        if l.fenced {
            fill(&mut buf, l.start, l.end);
        }
    }
    for l in &lines {
        if !l.fenced {
            mask_code_spans(text, &mut buf, l.start, l.end);
        }
    }
    let mut pos = 0usize;
    while let Some(p) = find_bytes(buf.get(pos..).unwrap_or(&[]), b"<!--").map(|p| p + pos) {
        match find_bytes(buf.get(p + 4..).unwrap_or(&[]), b"-->") {
            Some(q) => {
                let end = p + 4 + q + 3;
                fill(&mut buf, p, end);
                pos = end;
            }
            None => {
                let len = buf.len();
                fill(&mut buf, p, len);
                break;
            }
        }
    }
    buf
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Shrinks a range by ASCII spaces and tabs at both ends.
fn trim_range(text: &str, r: Range<usize>) -> Range<usize> {
    let s = text.get(r.clone()).unwrap_or("");
    let ws: &[char] = &[' ', '\t'];
    let t1 = s.trim_start_matches(ws);
    let lead = s.len() - t1.len();
    let t2 = t1.trim_end_matches(ws);
    let start = r.start + lead;
    start..start + t2.len()
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())
}

/// Parses `300` or `300x200`.
fn parse_size(s: &str) -> Option<(u32, Option<u32>)> {
    let s = s.trim();
    let (w, h) = match s.split_once('x') {
        Some((w, h)) => (w, Some(h)),
        None => (s, None),
    };
    if !all_digits(w) || w.len() > 5 {
        return None;
    }
    let w: u32 = w.parse().ok()?;
    let h = match h {
        Some(h) => {
            if !all_digits(h) || h.len() > 5 {
                return None;
            }
            Some(h.parse().ok()?)
        }
        None => None,
    };
    Some((w, h))
}

/// Decodes `%XX` sequences. Returns the input unchanged if the result is not valid UTF-8.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    if !bytes.contains(&b'%') {
        return s.to_string();
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// True for targets with a URL scheme such as `https:`, `mailto:` or `obsidian:`.
fn is_external(dest: &str) -> bool {
    let Some(colon) = dest.find(':') else {
        return false;
    };
    let scheme = &dest[..colon];
    scheme.len() >= 2
        && scheme.as_bytes()[0].is_ascii_alphabetic()
        && scheme
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'.' || c == b'-')
}

fn parse_anchor(s: &str) -> Option<Anchor> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    match s.strip_prefix('^') {
        Some("") => None,
        Some(rest) => Some(Anchor::Block(rest.to_string())),
        None => Some(Anchor::Heading(s.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Link parsing
// ---------------------------------------------------------------------------

/// Finds every link and embed in `text`, in source order. Ranges never overlap.
pub fn parse_links(text: &str) -> Vec<Link> {
    let buf = mask(text);
    let Ok(m) = std::str::from_utf8(&buf) else {
        return Vec::new();
    };
    let b = m.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < n {
        let found = match b[i] {
            b'!' if i + 2 < n && b[i + 1] == b'[' && b[i + 2] == b'[' => {
                parse_wiki(text, b, i, true)
            }
            b'[' if i + 1 < n && b[i + 1] == b'[' => parse_wiki(text, b, i, false),
            b']' if i + 1 < n && b[i + 1] == b'(' => parse_md(text, b, i),
            _ => None,
        };
        match found {
            Some(link) => {
                i = link.range.end;
                out.push(link);
            }
            None => i += 1,
        }
    }
    out
}

fn parse_wiki(text: &str, b: &[u8], start: usize, embed: bool) -> Option<Link> {
    let n = b.len();
    let cs = start + if embed { 3 } else { 2 };
    let mut j = cs;
    while j < n && !matches!(b[j], b'[' | b']' | b'\n') {
        j += 1;
    }
    if j + 1 >= n || b[j] != b']' || b[j + 1] != b']' {
        return None;
    }
    let inner = text.get(cs..j)?;
    if inner.trim().is_empty() {
        return None;
    }
    let (mut left_end, right) = match inner.find('|') {
        Some(p) => (cs + p, Some(cs + p + 1..j)),
        None => (j, None),
    };
    // `\|` inside a table cell is an escaped pipe, not a separator.
    if right.is_some() && text.get(cs..left_end)?.ends_with('\\') {
        left_end -= 1;
    }
    let left = cs..left_end;
    let (target_part, anchor_part) = match text.get(left.clone())?.find('#') {
        Some(h) => (cs..cs + h, Some(cs + h + 1..left_end)),
        None => (left.clone(), None),
    };
    let target_range = trim_range(text, target_part);
    let target = text.get(target_range.clone())?.to_string();
    let anchor = match anchor_part {
        Some(r) => {
            let r = trim_range(text, r);
            parse_anchor(text.get(r)?)
        }
        None => None,
    };
    let (size, alias) = match right {
        Some(r) => {
            let r = trim_range(text, r);
            let s = text.get(r)?;
            if embed && parse_size(s).is_some() {
                (parse_size(s), None)
            } else if s.is_empty() {
                (None, None)
            } else {
                (None, Some(s.to_string()))
            }
        }
        None => (None, None),
    };
    Some(Link {
        range: start..j + 2,
        kind: if embed {
            LinkKind::Embed
        } else {
            LinkKind::Wiki
        },
        target,
        target_range,
        anchor,
        alias,
        size,
        title: None,
        external: false,
    })
}

/// Parses `[text](dest "title")` where `j` is the index of the `]` in `](`.
fn parse_md(text: &str, b: &[u8], j: usize) -> Option<Link> {
    let n = b.len();
    // Find the opening bracket without crossing another bracket or a line break.
    let mut k = j;
    while k > 0 && !matches!(b[k - 1], b'[' | b']' | b'\n') {
        k -= 1;
    }
    if k == 0 || b[k - 1] != b'[' {
        return None;
    }
    let open = k - 1;
    let text_range = k..j;
    let is_image = open > 0 && b[open - 1] == b'!';
    let start = if is_image { open - 1 } else { open };

    let mut p = j + 2;
    while p < n && matches!(b[p], b' ' | b'\t') {
        p += 1;
    }
    if p >= n {
        return None;
    }
    let (dest_range, after) = if b[p] == b'<' {
        let mut q = p + 1;
        while q < n && !matches!(b[q], b'>' | b'<' | b'\n') {
            q += 1;
        }
        if q >= n || b[q] != b'>' {
            return None;
        }
        (p + 1..q, q + 1)
    } else {
        let mut q = p;
        while q < n
            && !matches!(
                b[q],
                b' ' | b'\t' | b'\r' | b'\n' | b'(' | b')' | b'[' | b']'
            )
        {
            q += 1;
        }
        (p..q, q)
    };
    if dest_range.is_empty() {
        return None;
    }
    let mut p = after;
    while p < n && matches!(b[p], b' ' | b'\t') {
        p += 1;
    }
    let mut title = None;
    if p < n && matches!(b[p], b'"' | b'\'') {
        let quote = b[p];
        let mut q = p + 1;
        while q < n && b[q] != quote && b[q] != b'\n' {
            q += 1;
        }
        if q >= n || b[q] != quote {
            return None;
        }
        title = text.get(p + 1..q).map(str::to_string);
        p = q + 1;
        while p < n && matches!(b[p], b' ' | b'\t') {
            p += 1;
        }
    }
    if p >= n || b[p] != b')' {
        return None;
    }
    let end = p + 1;

    let dest_raw = text.get(dest_range.clone())?;
    let alt_raw = text.get(text_range)?;
    let kind = if is_image {
        LinkKind::MdImage
    } else {
        LinkKind::MdLink
    };

    let (target, target_range, anchor, external) = if is_external(dest_raw) {
        (dest_raw.to_string(), dest_range.clone(), None, true)
    } else {
        let (path_range, anchor_range) = match dest_raw.find('#') {
            Some(h) => (
                dest_range.start..dest_range.start + h,
                Some(dest_range.start + h + 1..dest_range.end),
            ),
            None => (dest_range.clone(), None),
        };
        let raw_path = text.get(path_range.clone())?;
        let target = percent_decode(raw_path);
        let anchor = match anchor_range {
            Some(r) => parse_anchor(&percent_decode(text.get(r)?)),
            None => None,
        };
        (target, path_range, anchor, false)
    };

    let (alias, size) = if is_image {
        match alt_raw.rfind('|') {
            Some(pos) if parse_size(&alt_raw[pos + 1..]).is_some() => {
                let alt = alt_raw[..pos].trim();
                let alias = if alt.is_empty() {
                    None
                } else {
                    Some(alt.to_string())
                };
                (alias, parse_size(&alt_raw[pos + 1..]))
            }
            _ => {
                let alt = alt_raw.trim();
                (
                    if alt.is_empty() {
                        None
                    } else {
                        Some(alt.to_string())
                    },
                    None,
                )
            }
        }
    } else {
        let t = alt_raw.trim();
        (
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            },
            None,
        )
    };

    Some(Link {
        range: start..end,
        kind,
        target,
        target_range,
        anchor,
        alias,
        size,
        title,
        external,
    })
}

// ---------------------------------------------------------------------------
// Embed classification
// ---------------------------------------------------------------------------

/// Classifies a link target by its extension (case-insensitive).
/// Targets without an extension, or whose "extension" is not a plain word (such as
/// `v1.2 notes`), are notes.
pub fn embed_kind(target: &str) -> EmbedKind {
    let base = target.rsplit(['/', '\\']).next().unwrap_or(target);
    let Some(dot) = base.rfind('.') else {
        return EmbedKind::Note;
    };
    let ext = &base[dot + 1..];
    if ext.is_empty() || ext.len() > 8 || !ext.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return EmbedKind::Note;
    }
    match ext.to_ascii_lowercase().as_str() {
        "md" | "markdown" => EmbedKind::Note,
        "png" | "jpg" | "jpeg" | "jpe" | "gif" | "webp" | "bmp" | "svg" | "avif" | "tif"
        | "tiff" | "heic" | "ico" => EmbedKind::Image,
        "pdf" => EmbedKind::Pdf,
        "mp3" | "wav" | "ogg" | "oga" | "m4a" | "m4b" | "flac" | "aac" | "opus" => EmbedKind::Audio,
        "mp4" | "m4v" | "webm" | "mov" | "mkv" | "avi" | "ogv" => EmbedKind::Video,
        _ => EmbedKind::Other,
    }
}

// ---------------------------------------------------------------------------
// Headings
// ---------------------------------------------------------------------------

/// Parses an ATX heading prefix. Returns the level and the text after the markers.
fn atx_level(content: &str) -> Option<(u8, &str)> {
    let indent = content.len() - content.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let t = &content[indent..];
    let hashes = t.bytes().take_while(|&c| c == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &t[hashes..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some((hashes as u8, rest))
}

/// Removes a closing `#` sequence (only when preceded by whitespace or the whole text).
fn strip_closing(s: &str) -> &str {
    let t = s.trim_end();
    let without = t.trim_end_matches('#');
    if without.len() == t.len() {
        return t;
    }
    if without.is_empty() {
        return "";
    }
    if without.ends_with([' ', '\t']) {
        return without.trim_end();
    }
    t
}

fn normalize_heading(s: &str) -> String {
    strip_closing(s.trim())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// All ATX headings outside fenced code, in document order.
pub fn headings(text: &str) -> Vec<Heading> {
    let lines = scan_lines(text);
    let mut out = Vec::new();
    for (idx, l) in lines.iter().enumerate() {
        if l.fenced {
            continue;
        }
        let content = line_text(text, l);
        if let Some((level, rest)) = atx_level(content) {
            out.push(Heading {
                level,
                text: strip_closing(rest.trim()).to_string(),
                line: idx,
                range: l.start..l.end,
            });
        }
    }
    out
}

/// Index one past the last heading of the section that starts at `idx`.
fn section_end_index(hs: &[Heading], idx: usize) -> usize {
    let level = hs[idx].level;
    (idx + 1..hs.len())
        .find(|&j| hs[j].level <= level)
        .unwrap_or(hs.len())
}

/// Resolves `A#B#C` against the heading tree. Returns the headings and the index of the
/// last component. Each component is searched inside the section of the previous one.
fn resolve_heading(text: &str, path: &str) -> Option<(Vec<Heading>, usize)> {
    let hs = headings(text);
    let comps: Vec<String> = path
        .split('#')
        .map(normalize_heading)
        .filter(|s| !s.is_empty())
        .collect();
    if comps.is_empty() {
        return None;
    }
    if comps.len() > 8 {
        return None;
    }
    let idx = resolve_path(&hs, &comps, 0, hs.len())?;
    Some((hs, idx))
}

/// Finds the last component of `comps` inside the heading index range `lo..hi`, trying every
/// candidate for each component (backtracking), so `B#C` can match a later `# B` section.
fn resolve_path(hs: &[Heading], comps: &[String], lo: usize, hi: usize) -> Option<usize> {
    let (first, rest) = comps.split_first()?;
    for i in lo..hi {
        if normalize_heading(&hs[i].text) != *first {
            continue;
        }
        if rest.is_empty() {
            return Some(i);
        }
        let sub_hi = section_end_index(hs, i);
        if let Some(found) = resolve_path(hs, rest, i + 1, sub_hi) {
            return Some(found);
        }
    }
    None
}

/// Byte range of the heading line that matches `heading` (case-insensitive, closing `#`s
/// ignored, nested paths such as `A#B` supported).
pub fn find_heading(text: &str, heading: &str) -> Option<Range<usize>> {
    let (hs, i) = resolve_heading(text, heading)?;
    Some(hs[i].range.clone())
}

/// Byte range of a whole section: from the matching heading line up to (not including) the
/// next heading of the same or higher level, or the end of the text.
pub fn heading_section(text: &str, heading: &str) -> Option<Range<usize>> {
    let (hs, i) = resolve_heading(text, heading)?;
    let end = match hs.get(section_end_index(&hs, i)) {
        Some(next) => next.range.start,
        None => text.len(),
    };
    Some(hs[i].range.start..end)
}

/// Text of a heading section, for display in an embed. Includes the heading line and drops
/// trailing whitespace.
pub fn extract_section(text: &str, heading: &str) -> Option<String> {
    let r = heading_section(text, heading)?;
    Some(text.get(r)?.trim_end().to_string())
}

/// Lower-case, hyphen-separated slug. Letters and digits (including CJK) are kept, spaces and
/// hyphens become single hyphens, other punctuation is dropped.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
        } else if (c.is_whitespace() || c == '-') && !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

// ---------------------------------------------------------------------------
// Block references
// ---------------------------------------------------------------------------

fn valid_block_id(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

/// Recognises a block marker on a line. Returns the id and whether the line is a standalone
/// `^id` line (which labels the block above it) rather than a trailing marker.
fn block_marker(line: &str) -> Option<(String, bool)> {
    let t = line.trim_end_matches([' ', '\t']);
    if let Some(rest) = t.strip_prefix('^') {
        return if valid_block_id(rest) {
            Some((rest.to_string(), true))
        } else {
            None
        };
    }
    let pos = t.rfind([' ', '\t'])?;
    let tok = &t[pos + 1..];
    let rest = tok.strip_prefix('^')?;
    if valid_block_id(rest) {
        Some((rest.to_string(), false))
    } else {
        None
    }
}

/// Removes a trailing ` ^id` marker from a line.
fn strip_marker(line: &str) -> &str {
    let t = line.trim_end_matches([' ', '\t']);
    match t.rfind([' ', '\t']) {
        Some(pos) if t[pos + 1..].starts_with('^') => t[..pos].trim_end_matches([' ', '\t']),
        _ => line,
    }
}

fn is_list_item(content: &str) -> bool {
    let t = content.trim_start_matches([' ', '\t']);
    if matches!(t.as_bytes().first(), Some(b'-' | b'*' | b'+')) {
        return t.len() == 1 || matches!(t.as_bytes().get(1), Some(b' ' | b'\t'));
    }
    let digits = t.bytes().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 9 {
        return false;
    }
    let after = &t[digits..];
    (after.starts_with('.') || after.starts_with(')'))
        && (after.len() == 1 || after[1..].starts_with([' ', '\t']))
}

/// A line that can continue a paragraph (not blank, fenced, a heading or a list item start).
fn paragraph_line(text: &str, lines: &[LineSpan], i: usize) -> bool {
    let l = &lines[i];
    if l.fenced {
        return false;
    }
    let c = line_text(text, l);
    if matches!(block_marker(c), Some((_, true))) {
        return false;
    }
    !c.trim().is_empty() && atx_level(c).is_none() && !is_list_item(c)
}

/// The inclusive line range of the block that contains line `idx`.
fn block_span(text: &str, lines: &[LineSpan], idx: usize) -> Option<(usize, usize)> {
    let l = lines.get(idx)?;
    let c = line_text(text, l);
    if l.fenced || c.trim().is_empty() {
        return None;
    }
    if atx_level(c).is_some() {
        return Some((idx, idx));
    }
    if is_list_item(c) {
        let mut last = idx;
        while last + 1 < lines.len() {
            let nx = &lines[last + 1];
            let nc = line_text(text, nx);
            if nx.fenced || nc.trim().is_empty() || !nc.starts_with([' ', '\t']) {
                break;
            }
            last += 1;
        }
        return Some((idx, last));
    }
    let mut first = idx;
    while first > 0 && paragraph_line(text, lines, first - 1) {
        first -= 1;
    }
    let mut last = idx;
    while last + 1 < lines.len() && paragraph_line(text, lines, last + 1) {
        last += 1;
    }
    Some((first, last))
}

/// Every block reference in the text, in document order.
pub fn parse_block_ids(text: &str) -> Vec<BlockId> {
    let lines = scan_lines(text);
    let mut out = Vec::new();
    for (idx, l) in lines.iter().enumerate() {
        if l.fenced {
            continue;
        }
        let Some((id, standalone)) = block_marker(line_text(text, l)) else {
            continue;
        };
        let target = if standalone {
            if idx == 0 {
                continue;
            }
            block_span(text, &lines, idx - 1)
        } else {
            block_span(text, &lines, idx)
        };
        let Some((first, last)) = target else {
            continue;
        };
        out.push(BlockId {
            id,
            line_start: first,
            range: lines[first].start..lines[last].end,
        });
    }
    out
}

/// The block labelled `id`, if any. Ids are case-sensitive.
pub fn find_block(text: &str, id: &str) -> Option<BlockId> {
    parse_block_ids(text).into_iter().find(|b| b.id == id)
}

/// Text of the block labelled `id`, with its `^id` marker removed. Used for block embeds.
pub fn extract_block_text(text: &str, id: &str) -> Option<String> {
    let b = find_block(text, id)?;
    let mut parts = Vec::new();
    for l in scan_lines(text) {
        if l.start < b.range.start || l.end > b.range.end {
            continue;
        }
        let content = line_text(text, &l);
        let shown = match block_marker(content) {
            Some((mid, false)) if mid == id => strip_marker(content),
            _ => content,
        };
        parts.push(shown);
    }
    Some(parts.join("\n"))
}

fn splitmix64(state: u64) -> u64 {
    let mut z = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A new 6-character lowercase alphanumeric block id that is not in `existing`
/// (compared case-insensitively). Deterministic for a given seed.
pub fn generate_block_id(existing: &[&str], seed: u64) -> String {
    const ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut state = seed;
    loop {
        let mut s = String::with_capacity(6);
        for _ in 0..6 {
            state = splitmix64(state);
            let idx = (state % 36) as usize;
            if let Some(&c) = ALPHABET.get(idx) {
                s.push(c as char);
            }
        }
        if !existing.iter().any(|e| e.eq_ignore_ascii_case(&s)) {
            return s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Link {
        let v = parse_links(text);
        assert_eq!(v.len(), 1, "expected one link in {text:?}, got {v:?}");
        v.into_iter().next().unwrap_or_else(|| unreachable!())
    }

    fn none(text: &str) {
        let v = parse_links(text);
        assert!(v.is_empty(), "expected no links in {text:?}, got {v:?}");
    }

    fn check_invariants(text: &str, links: &[Link]) {
        let mut prev_end = 0usize;
        for l in links {
            assert!(l.range.start >= prev_end, "overlap in {text:?}");
            assert!(l.range.start < l.range.end);
            assert!(l.range.end <= text.len());
            assert!(text.is_char_boundary(l.range.start));
            assert!(text.is_char_boundary(l.range.end));
            assert!(l.target_range.start >= l.range.start);
            assert!(l.target_range.end <= l.range.end);
            assert!(l.target_range.start <= l.target_range.end);
            assert!(text.is_char_boundary(l.target_range.start));
            assert!(text.is_char_boundary(l.target_range.end));
            prev_end = l.range.end;
        }
    }

    #[test]
    fn plain_wiki_link() {
        let t = "see [[note]] here";
        let l = one(t);
        assert_eq!(l.kind, LinkKind::Wiki);
        assert_eq!(l.target, "note");
        assert_eq!(&t[l.range.clone()], "[[note]]");
        assert_eq!(&t[l.target_range.clone()], "note");
        assert!(l.anchor.is_none() && l.alias.is_none() && l.size.is_none());
        assert!(!l.external);
    }

    #[test]
    fn wiki_alias() {
        let l = one("[[note|Shown Name]]");
        assert_eq!(l.target, "note");
        assert_eq!(l.alias.as_deref(), Some("Shown Name"));
        assert_eq!(l.kind, LinkKind::Wiki);
    }

    #[test]
    fn wiki_heading_anchor() {
        let l = one("[[note#Intro]]");
        assert_eq!(l.target, "note");
        assert_eq!(l.anchor, Some(Anchor::Heading("Intro".into())));
    }

    #[test]
    fn wiki_block_anchor() {
        let l = one("[[note#^abc123]]");
        assert_eq!(l.anchor, Some(Anchor::Block("abc123".into())));
    }

    #[test]
    fn wiki_same_note_heading() {
        let l = one("[[#Heading two]]");
        assert_eq!(l.target, "");
        assert_eq!(l.anchor, Some(Anchor::Heading("Heading two".into())));
        assert!(l.target_range.is_empty());
    }

    #[test]
    fn wiki_nested_heading_path_kept_raw() {
        let l = one("[[note#A#B]]");
        assert_eq!(l.anchor, Some(Anchor::Heading("A#B".into())));
    }

    #[test]
    fn wiki_anchor_and_alias() {
        let l = one("[[note#Sec|Alias]]");
        assert_eq!(l.target, "note");
        assert_eq!(l.anchor, Some(Anchor::Heading("Sec".into())));
        assert_eq!(l.alias.as_deref(), Some("Alias"));
    }

    #[test]
    fn wiki_escaped_pipe_in_table() {
        let l = one(r"| [[note\|Alias]] |");
        assert_eq!(l.target, "note");
        assert_eq!(l.alias.as_deref(), Some("Alias"));
    }

    #[test]
    fn wiki_whitespace_trimmed() {
        let l = one("[[  spaced note  |  alias  ]]");
        assert_eq!(l.target, "spaced note");
        assert_eq!(l.alias.as_deref(), Some("alias"));
    }

    #[test]
    fn wiki_cjk_target() {
        let t = "[[中文 笔记#标题|别名]]";
        let l = one(t);
        assert_eq!(l.target, "中文 笔记");
        assert_eq!(l.anchor, Some(Anchor::Heading("标题".into())));
        assert_eq!(l.alias.as_deref(), Some("别名"));
        assert_eq!(&t[l.range.clone()], t);
    }

    #[test]
    fn wiki_emoji_target() {
        let t = "x [[🙂 note]] y";
        let l = one(t);
        assert_eq!(l.target, "🙂 note");
        check_invariants(t, &[l]);
    }

    #[test]
    fn image_embed_with_width() {
        let l = one("![[a.png|300]]");
        assert_eq!(l.kind, LinkKind::Embed);
        assert_eq!(l.target, "a.png");
        assert_eq!(l.size, Some((300, None)));
        assert!(l.alias.is_none());
    }

    #[test]
    fn image_embed_with_width_and_height() {
        let l = one("![[a.png|300x200]]");
        assert_eq!(l.size, Some((300, Some(200))));
    }

    #[test]
    fn embed_with_non_size_alias() {
        let l = one("![[a.png|my picture]]");
        assert_eq!(l.size, None);
        assert_eq!(l.alias.as_deref(), Some("my picture"));
    }

    #[test]
    fn wiki_link_never_takes_size() {
        let l = one("[[a.png|300]]");
        assert_eq!(l.kind, LinkKind::Wiki);
        assert_eq!(l.size, None);
        assert_eq!(l.alias.as_deref(), Some("300"));
    }

    #[test]
    fn note_embed_with_heading() {
        let l = one("![[note#Heading]]");
        assert_eq!(l.kind, LinkKind::Embed);
        assert_eq!(l.target, "note");
        assert_eq!(l.anchor, Some(Anchor::Heading("Heading".into())));
    }

    #[test]
    fn embed_with_block_id() {
        let l = one("![[note#^k9x2qa]]");
        assert_eq!(l.anchor, Some(Anchor::Block("k9x2qa".into())));
    }

    #[test]
    fn size_overflow_is_not_a_size() {
        let l = one("![[a.png|999999999999]]");
        assert_eq!(l.size, None);
    }

    #[test]
    fn oversized_digit_count_is_alias() {
        let l = one("![[a.png|123456]]");
        assert_eq!(l.size, None);
        assert_eq!(l.alias.as_deref(), Some("123456"));
    }

    #[test]
    fn md_link_basic() {
        let t = "[t](path/to/note.md#h)";
        let l = one(t);
        assert_eq!(l.kind, LinkKind::MdLink);
        assert_eq!(l.target, "path/to/note.md");
        assert_eq!(l.anchor, Some(Anchor::Heading("h".into())));
        assert_eq!(l.alias.as_deref(), Some("t"));
        assert!(!l.external);
        assert_eq!(&t[l.range.clone()], t);
    }

    #[test]
    fn md_link_with_title() {
        let l = one(r#"[t](a.md "Title here")"#);
        assert_eq!(l.title.as_deref(), Some("Title here"));
        assert_eq!(l.target, "a.md");
    }

    #[test]
    fn md_link_single_quote_title() {
        let l = one("[t](a.md 'T')");
        assert_eq!(l.title.as_deref(), Some("T"));
    }

    #[test]
    fn md_image_with_size_in_alt() {
        let t = "![alt|300](pic.png)";
        let l = one(t);
        assert_eq!(l.kind, LinkKind::MdImage);
        assert_eq!(l.size, Some((300, None)));
        assert_eq!(l.alias.as_deref(), Some("alt"));
        assert_eq!(&t[l.range.clone()], t);
    }

    #[test]
    fn md_image_size_with_height() {
        let l = one("![|120x80](pic.png)");
        assert_eq!(l.size, Some((120, Some(80))));
        assert!(l.alias.is_none());
    }

    #[test]
    fn md_image_plain_alt() {
        let l = one("![a cat](cat.jpg)");
        assert_eq!(l.kind, LinkKind::MdImage);
        assert_eq!(l.alias.as_deref(), Some("a cat"));
        assert_eq!(l.size, None);
    }

    #[test]
    fn md_percent_encoded_path_is_decoded() {
        let t = "[x](my%20note%E4%B8%AD.md)";
        let l = one(t);
        assert_eq!(l.target, "my note中.md");
        assert_eq!(&t[l.target_range.clone()], "my%20note%E4%B8%AD.md");
    }

    #[test]
    fn md_invalid_percent_left_alone() {
        let l = one("[x](a%zzb.png)");
        assert_eq!(l.target, "a%zzb.png");
    }

    #[test]
    fn md_percent_invalid_utf8_left_alone() {
        let l = one("[x](a%FFb.png)");
        assert_eq!(l.target, "a%FFb.png");
    }

    #[test]
    fn md_angle_bracket_path_with_spaces() {
        let t = "![x](<my file.png>)";
        let l = one(t);
        assert_eq!(l.target, "my file.png");
        assert_eq!(&t[l.target_range.clone()], "my file.png");
        assert_eq!(l.kind, LinkKind::MdImage);
    }

    #[test]
    fn md_angle_bracket_with_title() {
        let l = one("[x](<a b.md> \"T\")");
        assert_eq!(l.target, "a b.md");
        assert_eq!(l.title.as_deref(), Some("T"));
    }

    #[test]
    fn md_angle_bracket_unclosed_is_not_link() {
        none("[x](<a b.md)");
    }

    #[test]
    fn md_external_http_is_flagged() {
        let l = one("[site](https://example.com/a?b=c)");
        assert!(l.external);
        assert_eq!(l.kind, LinkKind::MdLink);
        assert_eq!(l.target, "https://example.com/a?b=c");
        assert!(l.anchor.is_none());
    }

    #[test]
    fn md_external_image() {
        let l = one("![p](http://x.y/z.png)");
        assert!(l.external);
        assert_eq!(l.kind, LinkKind::MdImage);
    }

    #[test]
    fn md_mailto_is_external() {
        let l = one("[mail](mailto:a@b.c)");
        assert!(l.external);
    }

    #[test]
    fn windows_drive_path_not_external() {
        // A single-letter scheme is a drive letter, not a URL scheme.
        let l = one("[x](C:/notes/a.md)");
        assert!(!l.external);
        assert_eq!(l.target, "C:/notes/a.md");
    }

    #[test]
    fn md_same_note_anchor_only() {
        let l = one("[back](#Top)");
        assert_eq!(l.target, "");
        assert_eq!(l.anchor, Some(Anchor::Heading("Top".into())));
    }

    #[test]
    fn md_empty_destination_is_not_link() {
        none("[t]()");
    }

    #[test]
    fn md_empty_text_allowed() {
        let l = one("[](a.md)");
        assert_eq!(l.alias, None);
        assert_eq!(l.target, "a.md");
    }

    #[test]
    fn md_link_text_with_wiki_is_not_double_counted() {
        let links = parse_links("[[a]](b.md)");
        check_invariants("[[a]](b.md)", &links);
        assert!(!links.is_empty());
    }

    #[test]
    fn brackets_in_md_text_not_supported_are_skipped() {
        // Nested brackets in link text are not links; the inner part still is.
        let t = "[a [b](c.md) d](e.md)";
        let links = parse_links(t);
        assert!(links.iter().any(|l| l.target == "c.md"));
        check_invariants(t, &links);
    }

    #[test]
    fn link_missing_close_paren() {
        none("[t](a.md");
    }

    #[test]
    fn link_with_space_before_paren_is_not_link() {
        none("[t] (a.md)");
    }

    #[test]
    fn unclosed_wiki_is_ignored() {
        none("[[note");
        none("[[note]");
    }

    #[test]
    fn wiki_across_newline_is_ignored() {
        none("[[no\nte]]");
    }

    #[test]
    fn empty_wiki_is_ignored() {
        none("[[]]");
        none("[[   ]]");
    }

    #[test]
    fn wiki_with_nested_open_bracket_recovers() {
        let t = "[[a [[b]]";
        let l = one(t);
        assert_eq!(l.target, "b");
        assert_eq!(&t[l.range.clone()], "[[b]]");
    }

    #[test]
    fn triple_brackets() {
        let l = one("[[[x]]]");
        assert_eq!(l.target, "x");
    }

    #[test]
    fn fenced_backtick_block_is_ignored() {
        let t = "```\n[[hidden]]\n```\n[[shown]]";
        let links = parse_links(t);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "shown");
    }

    #[test]
    fn fenced_tilde_block_is_ignored() {
        let t = "~~~md\n![[hidden]]\n~~~\n![[shown]]";
        let links = parse_links(t);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "shown");
    }

    #[test]
    fn longer_closing_fence_needed() {
        let t = "````\n```\n[[hidden]]\n```\n````\n[[shown]]";
        let links = parse_links(t);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "shown");
    }

    #[test]
    fn unclosed_fence_runs_to_end() {
        let links = parse_links("```\n[[hidden]]\n[[also]]");
        assert!(links.is_empty());
    }

    #[test]
    fn fence_with_info_string_and_crlf() {
        let t = "```rust\r\n[[hidden]]\r\n```\r\n[[shown]]\r\n";
        let links = parse_links(t);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "shown");
        check_invariants(t, &links);
    }

    #[test]
    fn backtick_info_string_cannot_open_fence() {
        // "``` a ```" is an inline code span, not a fence.
        let t = "``` [[x]] ```";
        assert!(parse_links(t).is_empty());
        let t2 = "```a``` [[y]]";
        let l = parse_links(t2);
        assert_eq!(l.len(), 1);
    }

    #[test]
    fn inline_code_span_hides_link() {
        none("`[[note]]`");
    }

    #[test]
    fn double_backtick_span_hides_link() {
        none("`` ` [[note]] ` ``");
    }

    #[test]
    fn unmatched_backtick_is_literal() {
        let l = one("` oops [[note]]");
        assert_eq!(l.target, "note");
    }

    #[test]
    fn escaped_backtick_does_not_open_span() {
        let l = one(r"\`[[note]] `x`");
        assert_eq!(l.target, "note");
    }

    #[test]
    fn html_comment_hides_link() {
        none("<!-- [[note]] -->");
    }

    #[test]
    fn html_comment_multiline_hides_links() {
        let t = "a\n<!--\n[[x]]\n-->\n[[y]]";
        let links = parse_links(t);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "y");
    }

    #[test]
    fn unclosed_html_comment_hides_rest() {
        none("<!-- [[note]]");
    }

    #[test]
    fn link_after_closed_comment_on_same_line() {
        let l = one("<!-- x --> [[note]]");
        assert_eq!(l.target, "note");
    }

    #[test]
    fn code_span_masking_preserves_offsets_with_cjk() {
        let t = "中文 `代码 [[no]]` [[yes]] 🙂";
        let links = parse_links(t);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "yes");
        assert_eq!(&t[links[0].range.clone()], "[[yes]]");
        check_invariants(t, &links);
    }

    #[test]
    fn multiple_links_in_order() {
        let t = "[[a]] and ![[b.png|10]] and [c](d.md)";
        let links = parse_links(t);
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].target, "a");
        assert_eq!(links[1].target, "b.png");
        assert_eq!(links[2].target, "d.md");
        check_invariants(t, &links);
    }

    #[test]
    fn crlf_text_ranges_are_valid() {
        let t = "line one\r\n[[a]]\r\n![[b]]\r\n";
        let links = parse_links(t);
        assert_eq!(links.len(), 2);
        check_invariants(t, &links);
    }

    #[test]
    fn no_links_plain_text() {
        none("just words, [brackets] and (parens)");
    }

    #[test]
    fn exclamation_alone_not_embed() {
        let l = one("!a [[x]]");
        assert_eq!(l.kind, LinkKind::Wiki);
    }

    #[test]
    fn embed_kind_table() {
        let cases: &[(&str, EmbedKind)] = &[
            ("note", EmbedKind::Note),
            ("note.md", EmbedKind::Note),
            ("note.MD", EmbedKind::Note),
            ("a.png", EmbedKind::Image),
            ("a.PNG", EmbedKind::Image),
            ("photo.JPEG", EmbedKind::Image),
            ("pic.webp", EmbedKind::Image),
            ("doc.pdf", EmbedKind::Pdf),
            ("doc.PDF", EmbedKind::Pdf),
            ("song.mp3", EmbedKind::Audio),
            ("voice.M4A", EmbedKind::Audio),
            ("clip.mp4", EmbedKind::Video),
            ("clip.webm", EmbedKind::Video),
            ("data.zip", EmbedKind::Other),
            ("folder/sub/a.png", EmbedKind::Image),
            ("folder\\sub\\a.gif", EmbedKind::Image),
            ("v1.2 notes", EmbedKind::Note),
            ("中文.png", EmbedKind::Image),
            ("中文笔记", EmbedKind::Note),
            ("name.", EmbedKind::Note),
            ("", EmbedKind::Note),
            ("file.waytoolongextension", EmbedKind::Note),
        ];
        for (t, want) in cases {
            assert_eq!(embed_kind(t), *want, "target {t:?}");
        }
    }

    #[test]
    fn headings_basic_levels() {
        let t = "# One\n## Two\n### Three\n#### Four\n##### Five\n###### Six\n####### Seven";
        let hs = headings(t);
        assert_eq!(hs.len(), 6);
        for (i, h) in hs.iter().enumerate() {
            assert_eq!(h.level as usize, i + 1);
        }
        assert_eq!(hs[0].text, "One");
        assert_eq!(hs[0].line, 0);
    }

    #[test]
    fn headings_closing_hashes_removed() {
        let hs = headings("## Title ##\n# C#\n# #\n#   Spaced   ");
        assert_eq!(hs[0].text, "Title");
        assert_eq!(hs[1].text, "C#");
        assert_eq!(hs[2].text, "");
        assert_eq!(hs[3].text, "Spaced");
    }

    #[test]
    fn headings_need_space_after_hashes() {
        assert!(headings("#tag\n##no").is_empty());
    }

    #[test]
    fn headings_indent_up_to_three() {
        assert_eq!(headings("   # ok").len(), 1);
        assert!(headings("    # code").is_empty());
    }

    #[test]
    fn headings_ignore_fences() {
        let hs = headings("```\n# not heading\n```\n# real");
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].text, "real");
        assert_eq!(hs[0].line, 3);
    }

    #[test]
    fn headings_range_excludes_newline() {
        let t = "# A\r\nbody\n# B";
        let hs = headings(t);
        assert_eq!(&t[hs[0].range.clone()], "# A");
        assert_eq!(&t[hs[1].range.clone()], "# B");
    }

    #[test]
    fn headings_cjk() {
        let hs = headings("# 标题 一\n## 子标题");
        assert_eq!(hs[0].text, "标题 一");
        assert_eq!(hs[1].text, "子标题");
    }

    #[test]
    fn find_heading_case_insensitive() {
        let t = "# Intro\n\ntext\n## Details\n";
        let r = find_heading(t, "intro");
        assert_eq!(r.as_ref().map(|r| &t[r.clone()]), Some("# Intro"));
        let r2 = find_heading(t, "DETAILS");
        assert_eq!(r2.as_ref().map(|r| &t[r.clone()]), Some("## Details"));
    }

    #[test]
    fn find_heading_ignores_closing_hashes_in_query() {
        let t = "## Title\n";
        assert!(find_heading(t, "Title ##").is_some());
    }

    #[test]
    fn find_heading_nested_path() {
        let t = "# A\n## B\nx\n# B\n## C\n";
        let r = find_heading(t, "A#B");
        assert_eq!(
            r.map(|r| r.start),
            Some(t.find("## B").unwrap_or(usize::MAX))
        );
        let r2 = find_heading(t, "B#C");
        assert_eq!(
            r2.map(|r| r.start),
            Some(t.find("## C").unwrap_or(usize::MAX))
        );
        assert!(find_heading(t, "C#B").is_none());
        let r3 = find_heading(t, "A#C");
        assert!(r3.is_none(), "C is outside section A");
    }

    #[test]
    fn find_heading_deep_path() {
        let t = "# A\n## B\n### C\n";
        assert!(find_heading(t, "A#B#C").is_some());
        assert!(find_heading(t, "A#C").is_some());
    }

    #[test]
    fn find_heading_missing_returns_none() {
        assert!(find_heading("# A", "B").is_none());
        assert!(find_heading("# A", "").is_none());
        assert!(find_heading("# A", "#").is_none());
    }

    #[test]
    fn find_heading_skips_fenced_duplicate() {
        let t = "```\n# X\n```\n# X\n";
        let r = find_heading(t, "x");
        assert_eq!(
            r.map(|r| r.start),
            Some(t.rfind("# X").unwrap_or(usize::MAX))
        );
    }

    #[test]
    fn heading_section_extent() {
        let t = "# A\na\n## B\nb\n# C\nc\n";
        let sec = extract_section(t, "A").unwrap_or_default();
        assert_eq!(sec, "# A\na\n## B\nb");
        let sec_b = extract_section(t, "B").unwrap_or_default();
        assert_eq!(sec_b, "## B\nb");
        let sec_c = extract_section(t, "C").unwrap_or_default();
        assert_eq!(sec_c, "# C\nc");
    }

    #[test]
    fn heading_section_to_eof() {
        let t = "# Only";
        assert_eq!(heading_section(t, "Only"), Some(0..t.len()));
    }

    #[test]
    fn slugify_table() {
        let cases: &[(&str, &str)] = &[
            ("Hello World", "hello-world"),
            ("  Multiple   spaces ", "multiple-spaces"),
            ("C++ & Rust!", "c-rust"),
            ("中文 标题", "中文-标题"),
            ("a--b", "a-b"),
            ("under_score", "under_score"),
            ("", ""),
            ("---", ""),
        ];
        for (input, want) in cases {
            assert_eq!(slugify(input), *want, "input {input:?}");
        }
    }

    #[test]
    fn block_id_trailing_paragraph() {
        let t = "First line\nsecond line ^abc123\n\nNext";
        let ids = parse_block_ids(t);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].id, "abc123");
        assert_eq!(ids[0].line_start, 0);
        assert_eq!(&t[ids[0].range.clone()], "First line\nsecond line ^abc123");
    }

    #[test]
    fn block_id_standalone_line_labels_block_above() {
        let t = "Para one\nstill one\n^xyz9\n\nAfter";
        let b = find_block(t, "xyz9");
        assert!(b.is_some());
        let b = b.unwrap_or_else(|| unreachable!());
        assert_eq!(&t[b.range.clone()], "Para one\nstill one");
        assert_eq!(b.line_start, 0);
    }

    #[test]
    fn block_id_standalone_after_blank_labels_nothing() {
        let t = "Para\n\n^lost1\n";
        assert!(find_block(t, "lost1").is_none());
    }

    #[test]
    fn block_id_standalone_as_first_line_labels_nothing() {
        assert!(find_block("^first\ntext", "first").is_none());
    }

    #[test]
    fn block_id_list_item() {
        let t = "- one\n- two ^itm02\n- three";
        let b = find_block(t, "itm02").unwrap_or_else(|| unreachable!());
        assert_eq!(&t[b.range.clone()], "- two ^itm02");
        assert_eq!(b.line_start, 1);
    }

    #[test]
    fn block_id_list_item_with_continuation() {
        let t = "- item ^c1\n  continued\n- next";
        let b = find_block(t, "c1").unwrap_or_else(|| unreachable!());
        assert_eq!(&t[b.range.clone()], "- item ^c1\n  continued");
    }

    #[test]
    fn block_id_heading_line() {
        let t = "# Title ^hd1\nbody";
        let b = find_block(t, "hd1").unwrap_or_else(|| unreachable!());
        assert_eq!(&t[b.range.clone()], "# Title ^hd1");
    }

    #[test]
    fn block_id_quote() {
        let t = "> quoted ^q1\n> more";
        let b = find_block(t, "q1").unwrap_or_else(|| unreachable!());
        assert_eq!(&t[b.range.clone()], "> quoted ^q1\n> more");
    }

    #[test]
    fn block_id_in_fence_ignored() {
        let t = "```\nfake ^nope\n```\n";
        assert!(parse_block_ids(t).is_empty());
    }

    #[test]
    fn block_id_invalid_characters_rejected() {
        assert!(parse_block_ids("text ^bad_id!").is_empty());
        assert!(parse_block_ids("text ^ spaced").is_empty());
        assert!(parse_block_ids("text ^").is_empty());
        assert!(parse_block_ids("text^nospace").is_empty());
    }

    #[test]
    fn block_id_hyphen_allowed() {
        assert!(find_block("text ^a-b-c", "a-b-c").is_some());
    }

    #[test]
    fn block_id_case_sensitive_lookup() {
        assert!(find_block("text ^Abc", "abc").is_none());
        assert!(find_block("text ^Abc", "Abc").is_some());
    }

    #[test]
    fn extract_block_text_strips_marker() {
        let t = "alpha beta ^kk12\n\nomega";
        assert_eq!(extract_block_text(t, "kk12").as_deref(), Some("alpha beta"));
    }

    #[test]
    fn extract_block_text_standalone_marker_not_included() {
        let t = "gamma\ndelta\n^mm34\n";
        assert_eq!(
            extract_block_text(t, "mm34").as_deref(),
            Some("gamma\ndelta")
        );
    }

    #[test]
    fn extract_block_text_missing() {
        assert!(extract_block_text("x", "nope").is_none());
    }

    #[test]
    fn generate_block_id_shape() {
        let id = generate_block_id(&[], 42);
        assert_eq!(id.len(), 6);
        assert!(
            id.bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }

    #[test]
    fn generate_block_id_is_deterministic() {
        assert_eq!(generate_block_id(&[], 7), generate_block_id(&[], 7));
        assert_ne!(generate_block_id(&[], 7), generate_block_id(&[], 8));
    }

    #[test]
    fn generate_block_id_avoids_existing_case_insensitive() {
        let first = generate_block_id(&[], 99);
        let upper = first.to_ascii_uppercase();
        let existing = [upper.as_str()];
        let second = generate_block_id(&existing, 99);
        assert_ne!(second, first);
        assert!(!second.eq_ignore_ascii_case(&upper));
    }

    #[test]
    fn generate_block_id_many_unique() {
        let mut seen: Vec<String> = Vec::new();
        for seed in 0..500u64 {
            let refs: Vec<&str> = seen.iter().map(String::as_str).collect();
            let id = generate_block_id(&refs, seed);
            assert!(!seen.contains(&id));
            seen.push(id);
        }
    }

    #[test]
    fn pathological_open_brackets_are_linear() {
        let t = "[[".repeat(200_000);
        assert!(parse_links(&t).is_empty());
    }

    #[test]
    fn pathological_close_sequences_are_linear() {
        let t = "](".repeat(200_000);
        assert!(parse_links(&t).is_empty());
        let t2 = "[a](".repeat(100_000);
        let _ = parse_links(&t2);
    }

    #[test]
    fn one_megabyte_of_links() {
        let unit = "text [[note#Head|alias]] ![[pic.png|200x100]] [x](a%20b.md) ";
        let mut t = String::new();
        while t.len() < 1_000_000 {
            t.push_str(unit);
        }
        let links = parse_links(&t);
        assert!(links.len() > 40_000);
        check_invariants(&t, &links);
    }

    #[test]
    fn one_megabyte_single_line_no_newline() {
        let t = "a".repeat(1_000_000);
        assert!(parse_links(&t).is_empty());
        assert!(headings(&t).is_empty());
        assert!(parse_block_ids(&t).is_empty());
    }

    #[test]
    fn one_megabyte_of_backticks() {
        let mut t = String::new();
        for i in 0..200_000 {
            t.push('`');
            if i % 3 == 0 {
                t.push('x');
            }
        }
        let _ = parse_links(&t);
        let _ = parse_block_ids(&t);
    }

    #[test]
    fn one_megabyte_of_paragraph_block_ids() {
        let mut t = String::new();
        while t.len() < 1_000_000 {
            t.push_str("line of text\n");
        }
        t.push_str("^end1\n");
        let ids = parse_block_ids(&t);
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn one_megabyte_of_headings() {
        let mut t = String::new();
        let mut i = 0usize;
        while t.len() < 1_000_000 {
            t.push_str(&format!("## Section {i}\n"));
            i += 1;
        }
        let hs = headings(&t);
        assert_eq!(hs.len(), i);
        assert!(find_heading(&t, "Section 5").is_some());
    }

    #[test]
    fn one_megabyte_unclosed_comment_is_linear() {
        let mut t = String::from("<!-- ");
        while t.len() < 1_000_000 {
            t.push_str("[[x]] ");
        }
        assert!(parse_links(&t).is_empty());
    }

    #[test]
    fn robustness_random_inputs_keep_invariants() {
        let alphabet: Vec<&str> = vec![
            "[", "]", "[[", "]]", "(", ")", "!", "![[", "![", "#", "|", "^", "`", "```", "~~~",
            "\n", "\r\n", "-", " ", "\t", "a", "b", "中", "🙂", "<", ">", "<!--", "-->", "%",
            "%20", "\"", "'", ".", "md", "png", "300", "x", "http://", "\\", "- ", "1. ", "## ",
            "> ",
        ];
        let mut state: u64 = 0x1234_5678_9abc_def0;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as usize
        };
        for _ in 0..2000 {
            let parts = next() % 60;
            let mut s = String::new();
            for _ in 0..parts {
                s.push_str(alphabet[next() % alphabet.len()]);
            }
            let links = parse_links(&s);
            check_invariants(&s, &links);
            for h in headings(&s) {
                assert!(s.is_char_boundary(h.range.start) && s.is_char_boundary(h.range.end));
            }
            for b in parse_block_ids(&s) {
                assert!(s.is_char_boundary(b.range.start) && s.is_char_boundary(b.range.end));
                assert!(b.range.start <= b.range.end);
            }
            let _ = find_heading(&s, "a#b");
            let _ = extract_section(&s, "a");
            let _ = extract_block_text(&s, "abc");
        }
    }

    #[test]
    fn invalid_heading_queries_do_not_panic() {
        let t = "# A\n## B";
        for q in ["", "#", "##", "A##B", "  ", "#A#", "\u{feff}"] {
            let _ = find_heading(t, q);
            let _ = heading_section(t, q);
        }
    }
}
