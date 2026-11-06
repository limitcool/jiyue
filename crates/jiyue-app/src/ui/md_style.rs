//! Markdown syntax spans for the source editor (live preview stage 1). Owned by W1-F.
//!
//! [`compute_spans`] finds the syntax structure of a document. Block structure and inline
//! emphasis come from `pulldown-cmark` (offset iterator; tables, strikethrough, task lists and
//! footnotes enabled, heading attributes off). The Obsidian parts that pulldown does not know
//! (wikilinks and embeds, `#tags`, `> [!note]` callouts, `^block-id` suffixes, `==highlight==`)
//! come from small scanners. The scanners ignore code spans, code blocks and frontmatter.
//!
//! Every span is validated (non-empty, inside the text, on char boundaries), so malformed or
//! unfinished input can only lose styling; it never yields a bad range.
//!
//! Output invariants: spans are sorted by `(start, end)` and no two spans are identical. Spans
//! may overlap (bold text that contains a link). [`line_segments`] resolves the overlaps into a
//! tiling of each logical line, which is what the editor paints.
//!
//! Dependency: `pulldown-cmark` 0.11.3 (no default features, about 0.45 MB of source; it pulls in
//! `bitflags`, `memchr` and `unicase`).

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag};

/// Documents larger than this are not styled; the editor shows them as plain text.
pub const MAX_STYLED_BYTES: usize = 600 * 1024;
/// A line overlapped by more spans than this is left unstyled (keeps pathological input cheap).
const MAX_SPANS_PER_LINE: usize = 512;
/// Longest `[[...]]` or `==...==` content the scanners accept.
const MAX_INLINE_SCAN: usize = 300;
/// Longest `^block-id` the scanner accepts.
const MAX_BLOCK_ID: usize = 64;

/// The style of one span: a set of flags plus the heading level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpanStyle {
    bits: u32,
    /// Heading level 1..=6, or 0 when the span is not part of a heading.
    pub heading: u8,
}

impl SpanStyle {
    pub const BOLD: u32 = 1 << 0;
    pub const ITALIC: u32 = 1 << 1;
    pub const STRIKE: u32 = 1 << 2;
    /// Inline code content (the backticks are `MARKER`).
    pub const CODE: u32 = 1 << 3;
    pub const LINK_TEXT: u32 = 1 << 4;
    /// Link destination, autolink or image source. Dimmed like a marker.
    pub const LINK_URL: u32 = 1 << 5;
    pub const WIKILINK: u32 = 1 << 6;
    pub const TAG: u32 = 1 << 7;
    pub const QUOTE: u32 = 1 << 8;
    pub const LIST_MARKER: u32 = 1 << 9;
    pub const TASK_MARKER: u32 = 1 << 10;
    pub const CODE_FENCE: u32 = 1 << 11;
    pub const CODE_BLOCK: u32 = 1 << 12;
    pub const HR: u32 = 1 << 13;
    /// Syntax characters: `#`, `**`, `_`, backticks, `[ ] ( )`, `>`, `|`, `^`.
    pub const MARKER: u32 = 1 << 14;
    pub const CALLOUT_TITLE: u32 = 1 << 15;
    pub const HIGHLIGHT: u32 = 1 << 16;
    pub const BLOCK_ID: u32 = 1 << 17;
    pub const FRONTMATTER: u32 = 1 << 18;
    /// Reserved for `$...$` and `$$...$$`. Not produced yet.
    pub const MATH: u32 = 1 << 19;

    /// A style with only the given flags.
    pub fn of(bits: u32) -> Self {
        Self { bits, heading: 0 }
    }

    /// A heading style of the given level (clamped to 1..=6; 0 gives a plain style).
    pub fn heading(level: u8) -> Self {
        Self {
            bits: 0,
            heading: level.min(6),
        }
    }

    pub fn has(self, flag: u32) -> bool {
        self.bits & flag != 0
    }

    pub fn bits(self) -> u32 {
        self.bits
    }

    pub fn is_plain(self) -> bool {
        self.bits == 0 && self.heading == 0
    }

    /// Combines two styles: flags are or-ed, the heading level is the larger one.
    pub fn union(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
            heading: self.heading.max(other.heading),
        }
    }
}

/// One styled byte range of the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyleSpan {
    pub range: Range<usize>,
    pub style: SpanStyle,
}

/// One piece of a logical line with a single combined style. The pieces of a line tile it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub range: Range<usize>,
    pub style: SpanStyle,
}

fn flag(bits: u32) -> SpanStyle {
    SpanStyle::of(bits)
}

fn marker() -> SpanStyle {
    SpanStyle::of(SpanStyle::MARKER)
}

struct Builder<'a> {
    text: &'a str,
    out: Vec<StyleSpan>,
}

impl Builder<'_> {
    /// Adds a span if it is non-empty, inside the text and on char boundaries.
    fn push(&mut self, range: Range<usize>, style: SpanStyle) {
        let t = self.text;
        if style.is_plain()
            || range.start >= range.end
            || range.end > t.len()
            || !t.is_char_boundary(range.start)
            || !t.is_char_boundary(range.end)
        {
            return;
        }
        self.out.push(StyleSpan { range, style });
    }
}

/// Computes the syntax spans of a Markdown document. Never panics, for any input.
pub fn compute_spans(text: &str) -> Vec<StyleSpan> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut b = Builder {
        text,
        out: Vec::new(),
    };
    // Bytes that the scanners must not look at (code and frontmatter).
    let mut skip = vec![false; text.len()];
    let fm_end = frontmatter_end(text);
    if let Some(end) = fm_end {
        b.push(0..end, flag(SpanStyle::FRONTMATTER));
        mark(&mut skip, 0..end);
    }

    let mut code_ranges: Vec<Range<usize>> = Vec::new();
    let mut code_blocks: Vec<(Range<usize>, bool)> = Vec::new();
    let mut quotes: Vec<Range<usize>> = Vec::new();
    let mut tables: Vec<Range<usize>> = Vec::new();
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    for (event, range) in Parser::new_ext(text, opts).into_offset_iter() {
        if range.start >= range.end || range.end > text.len() {
            continue;
        }
        if fm_end.is_some_and(|end| range.start < end) {
            continue;
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    // pulldown can include the newline that ends the heading line.
                    let range = trim_newlines(text, range);
                    b.push(range.clone(), SpanStyle::heading(heading_number(level)));
                    heading_markers(&mut b, range);
                }
                Tag::BlockQuote(_) => quotes.push(range),
                Tag::CodeBlock(kind) => {
                    code_blocks.push((range, matches!(kind, CodeBlockKind::Fenced(_))))
                }
                Tag::Item => list_item(&mut b, range),
                Tag::Emphasis => delimited(&mut b, range, 1, b"*_", flag(SpanStyle::ITALIC)),
                Tag::Strong => delimited(&mut b, range, 2, b"*_", flag(SpanStyle::BOLD)),
                Tag::Strikethrough => {
                    let k = if text.as_bytes().get(range.start + 1) == Some(&b'~') {
                        2
                    } else {
                        1
                    };
                    delimited(&mut b, range, k, b"~", flag(SpanStyle::STRIKE));
                }
                Tag::Link { .. } => link(&mut b, range, false),
                Tag::Image { .. } => link(&mut b, range, true),
                Tag::Table(_) => tables.push(range),
                _ => {}
            },
            Event::Code(_) => {
                inline_code(&mut b, range.clone());
                code_ranges.push(range);
            }
            Event::Rule => b.push(range, flag(SpanStyle::HR)),
            Event::FootnoteReference(_) => b.push(range, flag(SpanStyle::LINK_TEXT)),
            _ => {}
        }
    }

    for r in &code_ranges {
        mark(&mut skip, r.clone());
    }
    for (r, fenced) in &code_blocks {
        mark(&mut skip, r.clone());
        code_block_lines(&mut b, r.clone(), *fenced);
    }
    for r in &quotes {
        quote_markers(&mut b, &skip, r.clone());
        b.push(r.clone(), flag(SpanStyle::QUOTE));
    }
    for r in &tables {
        table_markers(&mut b, &skip, r.clone());
    }
    scan_callouts(&mut b, &skip);
    scan_wikilinks(&mut b, &skip);
    scan_tags(&mut b, &skip);
    scan_block_ids(&mut b, &skip);
    scan_highlights(&mut b, &skip);

    let mut out = b.out;
    out.sort_unstable_by_key(|s| (s.range.start, s.range.end, s.style.bits(), s.style.heading));
    out.dedup();
    out
}

/// Splits a text into its logical lines (the same pieces as `text.split('\n')`).
pub fn split_lines(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in text.bytes().enumerate() {
        if c == b'\n' {
            out.push(start..i);
            start = i + 1;
        }
    }
    out.push(start..text.len());
    out
}

/// Splits every line (given as byte ranges) into segments that tile it exactly. Empty lines get
/// no segments; a line with no spans gets one plain segment. Spans must be sorted by start, as
/// `compute_spans` returns them, and `lines` must be in document order.
pub fn line_segments(spans: &[StyleSpan], lines: &[Range<usize>]) -> Vec<Vec<Segment>> {
    let mut out = Vec::with_capacity(lines.len());
    let mut next = 0;
    // Reused buffers: spans overlapping the current line, and the cut points inside it.
    let mut active: Vec<usize> = Vec::new();
    let mut cuts: Vec<usize> = Vec::new();
    for line in lines {
        while next < spans.len() && spans[next].range.start < line.end {
            active.push(next);
            next += 1;
        }
        active.retain(|&k| spans[k].range.end > line.start);
        if line.start >= line.end {
            out.push(Vec::new());
            continue;
        }
        if active.is_empty() || active.len() > MAX_SPANS_PER_LINE {
            out.push(vec![Segment {
                range: line.clone(),
                style: SpanStyle::default(),
            }]);
            continue;
        }
        cuts.clear();
        cuts.push(line.start);
        cuts.push(line.end);
        for &k in &active {
            let r = &spans[k].range;
            cuts.push(r.start.clamp(line.start, line.end));
            cuts.push(r.end.clamp(line.start, line.end));
        }
        cuts.sort_unstable();
        cuts.dedup();
        let mut segs = Vec::with_capacity(cuts.len().saturating_sub(1));
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let style = active
                .iter()
                .map(|&k| &spans[k])
                .filter(|s| s.range.start <= a && s.range.end >= b)
                .fold(SpanStyle::default(), |acc, s| acc.union(s.style));
            segs.push(Segment { range: a..b, style });
        }
        out.push(segs);
    }
    out
}

// ----- block structure ----------------------------------------------------------------

fn heading_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// The `#` prefix and closing `#` run of an ATX heading, or the underline of a setext heading.
fn heading_markers(b: &mut Builder, range: Range<usize>) {
    let text = b.text;
    let Some(src) = text.get(range.clone()) else {
        return;
    };
    if let Some(nl) = src.rfind('\n') {
        // Setext heading: the underline (`===` or `---`) is the marker.
        let under = src[nl + 1..].trim();
        if !under.is_empty() && under.bytes().all(|c| c == b'=' || c == b'-') {
            b.push(range.start + nl + 1..range.end, marker());
        }
        return;
    }
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() && i < 3 && bytes[i] == b' ' {
        i += 1;
    }
    let hash_start = i;
    while i < bytes.len() && bytes[i] == b'#' {
        i += 1;
    }
    if i == hash_start {
        return;
    }
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    b.push(range.start..range.start + i, marker());

    let body = src.trim_end();
    let body_bytes = body.as_bytes();
    let mut j = body.len();
    while j > i && body_bytes[j - 1] == b'#' {
        j -= 1;
    }
    if j == body.len() {
        return;
    }
    if j == i {
        // The whole rest of the line is a closing run, as in `# ###`.
        b.push(range.start + i..range.start + body.len(), marker());
        return;
    }
    if body_bytes[j - 1] == b' ' || body_bytes[j - 1] == b'\t' {
        let mut k = j;
        while k > i && (body_bytes[k - 1] == b' ' || body_bytes[k - 1] == b'\t') {
            k -= 1;
        }
        b.push(range.start + k..range.start + body.len(), marker());
    }
}

/// The bullet or number of a list item, and a following `[ ]` / `[x]` task box.
fn list_item(b: &mut Builder, range: Range<usize>) {
    let text = b.text;
    let Some(src) = text.get(range.clone()) else {
        return;
    };
    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    let bullet_start = i;
    if i < len && matches!(bytes[i], b'-' | b'*' | b'+') {
        i += 1;
    } else {
        let digits = bytes[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || digits > 9 || i + digits >= len {
            return;
        }
        if !matches!(bytes[i + digits], b'.' | b')') {
            return;
        }
        i += digits + 1;
    }
    b.push(
        range.start + bullet_start..range.start + i,
        flag(SpanStyle::LIST_MARKER),
    );
    let mut j = i;
    while j < len && (bytes[j] == b' ' || bytes[j] == b'\t') {
        j += 1;
    }
    if j + 3 <= len
        && bytes[j] == b'['
        && bytes[j + 2] == b']'
        && matches!(bytes[j + 1], b' ' | b'x' | b'X')
        && (j + 3 == len || matches!(bytes[j + 3], b' ' | b'\t'))
    {
        b.push(
            range.start + j..range.start + j + 3,
            flag(SpanStyle::TASK_MARKER),
        );
    }
}

/// Fence lines and content lines of a code block. Indented blocks are all content.
fn code_block_lines(b: &mut Builder, range: Range<usize>, fenced: bool) {
    let text = b.text;
    let lines = line_ranges(text, range);
    let n = lines.len();
    for (idx, lr) in lines.iter().enumerate() {
        let Some(line) = text.get(lr.clone()) else {
            continue;
        };
        let is_fence = fenced
            && if idx == 0 {
                fence_char(line.trim_start()).is_some()
            } else {
                idx + 1 == n && n >= 2 && is_closing_fence(line)
            };
        let style = if is_fence {
            flag(SpanStyle::CODE_FENCE)
        } else {
            flag(SpanStyle::CODE_BLOCK)
        };
        b.push(lr.clone(), style);
    }
}

fn fence_char(trimmed: &str) -> Option<u8> {
    if trimmed.starts_with("```") {
        Some(b'`')
    } else if trimmed.starts_with("~~~") {
        Some(b'~')
    } else {
        None
    }
}

fn is_closing_fence(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3 && (t.bytes().all(|c| c == b'`') || t.bytes().all(|c| c == b'~'))
}

/// The `>` markers at the start of each line of a blockquote (nested markers included).
fn quote_markers(b: &mut Builder, skip: &[bool], range: Range<usize>) {
    let text = b.text;
    let bytes = text.as_bytes();
    for lr in line_ranges(text, range) {
        let mut p = lr.start;
        loop {
            while p < lr.end && (bytes[p] == b' ' || bytes[p] == b'\t') {
                p += 1;
            }
            if p < lr.end && bytes[p] == b'>' && !skip.get(p).copied().unwrap_or(true) {
                let mut q = p + 1;
                if q < lr.end && bytes[q] == b' ' {
                    q += 1;
                }
                b.push(p..q, marker());
                p = q;
            } else {
                break;
            }
        }
    }
}

/// Pipes of a table are markers, and so is a delimiter row (`| --- | :-: |`).
fn table_markers(b: &mut Builder, skip: &[bool], range: Range<usize>) {
    let text = b.text;
    let bytes = text.as_bytes();
    for lr in line_ranges(text, range) {
        let line = &text[lr.clone()];
        let t = line.trim();
        let is_delimiter = t.contains('-')
            && t.bytes()
                .all(|c| matches!(c, b'|' | b'-' | b':' | b' ' | b'\t'));
        if is_delimiter {
            b.push(lr, marker());
            continue;
        }
        for p in lr {
            if bytes[p] == b'|' && !skip.get(p).copied().unwrap_or(true) {
                b.push(p..p + 1, marker());
            }
        }
    }
}

/// Line ranges of `range` without the newline and a trailing CR; empty lines are dropped.
fn line_ranges(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let end = range.end.min(bytes.len());
    let mut out = Vec::new();
    let mut start = range.start;
    for i in range.start..end {
        if bytes[i] == b'\n' {
            push_line(&mut out, bytes, start, i);
            start = i + 1;
        }
    }
    push_line(&mut out, bytes, start, end);
    out
}

fn push_line(out: &mut Vec<Range<usize>>, bytes: &[u8], start: usize, end: usize) {
    let end = if end > start && bytes[end - 1] == b'\r' {
        end - 1
    } else {
        end
    };
    if start < end {
        out.push(start..end);
    }
}

// ----- inline structure ---------------------------------------------------------------

/// Emphasis, strong or strikethrough: the delimiter runs at both ends are markers. The
/// delimiter bytes are checked, so a node whose ends do not look like delimiters gets no style.
fn delimited(b: &mut Builder, range: Range<usize>, k: usize, allowed: &[u8], style: SpanStyle) {
    let text = b.text;
    let bytes = text.as_bytes();
    let (s, e) = (range.start, range.end);
    if k == 0 || e > bytes.len() || e - s <= 2 * k {
        return;
    }
    let ch = bytes[s];
    if !allowed.contains(&ch) || !(0..k).all(|i| bytes[s + i] == ch && bytes[e - 1 - i] == ch) {
        return;
    }
    b.push(s..s + k, marker());
    b.push(e - k..e, marker());
    b.push(s + k..e - k, style);
}

/// Inline code: the backtick runs at both ends are markers, the content is code.
fn inline_code(b: &mut Builder, range: Range<usize>) {
    let text = b.text;
    let bytes = text.as_bytes();
    let (s, e) = (range.start, range.end);
    if e > bytes.len() || e - s < 2 {
        return;
    }
    let k = bytes[s..e].iter().take_while(|&&c| c == b'`').count();
    let k_end = bytes[s..e].iter().rev().take_while(|&&c| c == b'`').count();
    let k = k.min(k_end);
    if k == 0 || 2 * k >= e - s {
        return;
    }
    b.push(s..s + k, marker());
    b.push(e - k..e, marker());
    b.push(s + k..e - k, flag(SpanStyle::CODE));
}

/// Links and images: `[text](url)`, `[text][ref]`, `<autolink>` and their `!` forms.
fn link(b: &mut Builder, range: Range<usize>, image: bool) {
    let text = b.text;
    let Some(src) = text.get(range.clone()) else {
        return;
    };
    let bytes = src.as_bytes();
    let base = range.start;
    let len = bytes.len();
    if !image && len > 2 && bytes[0] == b'<' && bytes[len - 1] == b'>' {
        b.push(base..base + 1, marker());
        b.push(base + 1..base + len - 1, flag(SpanStyle::LINK_URL));
        b.push(base + len - 1..base + len, marker());
        return;
    }
    let mut p = 0;
    if image {
        if bytes.first() != Some(&b'!') {
            return;
        }
        b.push(base..base + 1, marker());
        p = 1;
    }
    if bytes.get(p) != Some(&b'[') {
        return;
    }
    // The bracket that closes the link text, honouring nesting and backslash escapes.
    let mut depth = 0usize;
    let mut close = None;
    let mut i = p;
    while i < len {
        match bytes[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'[' => depth += 1,
            b']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let Some(close) = close else {
        return;
    };
    b.push(base + p..base + p + 1, marker());
    b.push(base + p + 1..base + close, flag(SpanStyle::LINK_TEXT));
    b.push(base + close..base + close + 1, marker());
    let rest = close + 1;
    if rest + 2 <= len && matches!(bytes[rest], b'(' | b'[') {
        let closer = if bytes[rest] == b'(' { b')' } else { b']' };
        if bytes[len - 1] == closer {
            b.push(base + rest..base + rest + 1, marker());
            b.push(base + rest + 1..base + len - 1, flag(SpanStyle::LINK_URL));
            b.push(base + len - 1..base + len, marker());
        }
    }
}

// ----- Obsidian scanners ----------------------------------------------------------------

/// `> [!note] Title` on a blockquote line: the `[!note] Title` part is the callout title.
fn scan_callouts(b: &mut Builder, skip: &[bool]) {
    let text = b.text;
    let bytes = text.as_bytes();
    for lr in line_ranges(text, 0..text.len()) {
        let mut p = lr.start;
        let mut lead = 0;
        while p < lr.end && bytes[p] == b' ' && lead < 3 {
            p += 1;
            lead += 1;
        }
        if p >= lr.end || bytes[p] != b'>' || skip.get(p).copied().unwrap_or(true) {
            continue;
        }
        let mut q = p + 1;
        if q < lr.end && bytes[q] == b' ' {
            q += 1;
        }
        if q + 1 >= lr.end || bytes[q] != b'[' || bytes[q + 1] != b'!' {
            continue;
        }
        let Some(rel) = text[q..lr.end].find(']') else {
            continue;
        };
        let name = &text[q + 2..q + rel];
        if name.is_empty()
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            continue;
        }
        b.push(p..q, marker());
        b.push(q..lr.end, flag(SpanStyle::CALLOUT_TITLE));
    }
}

/// `[[target]]`, `[[target|alias]]`, `[[note#heading]]` and the `![[embed]]` form. The
/// brackets are markers, the content is a wikilink. Content may not span lines.
fn scan_wikilinks(b: &mut Builder, skip: &[bool]) {
    let text = b.text;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] != b'[' || bytes[i + 1] != b'[' || skip[i] || skip[i + 1] {
            i += 1;
            continue;
        }
        let limit = (i + 2 + MAX_INLINE_SCAN).min(bytes.len());
        let mut j = i + 2;
        let mut close = None;
        while j + 1 < limit {
            let c = bytes[j];
            if c == b'\n' || (c == b'[' && bytes[j + 1] == b'[') {
                break;
            }
            if c == b']' && bytes[j + 1] == b']' {
                close = Some(j);
                break;
            }
            j += 1;
        }
        let Some(close) = close else {
            i += 1;
            continue;
        };
        if close == i + 2 || skip[i..close + 2].iter().any(|&s| s) {
            i += 1;
            continue;
        }
        if i > 0 && bytes[i - 1] == b'!' && !skip[i - 1] {
            b.push(i - 1..i, marker());
        }
        b.push(i..i + 2, marker());
        b.push(i + 2..close, flag(SpanStyle::WIKILINK));
        b.push(close..close + 2, marker());
        i = close + 2;
    }
}

/// `#tag` in running text. The `#` must start the line or follow whitespace or punctuation (so
/// `C#`, `a#b` and URL fragments are not tags). Tags need at least one non-digit character, and
/// headings (`# Title`) and code are not tags.
fn scan_tags(b: &mut Builder, skip: &[bool]) {
    let text = b.text;
    let mut prev: Option<char> = None;
    let mut heading_line = false;
    let mut line_begin = true;
    for (i, ch) in text.char_indices() {
        if line_begin {
            heading_line = is_heading_line(line_at(text, i));
            line_begin = false;
        }
        if ch == '\n' {
            prev = Some(ch);
            line_begin = true;
            continue;
        }
        if ch == '#' && !skip[i] && !heading_line && prev_allows_tag(prev) {
            let mut end = i + 1;
            let mut has_letter = false;
            for (j, c) in text[i + 1..].char_indices() {
                let at = i + 1 + j;
                if !(c.is_alphanumeric() || c == '_' || c == '-' || c == '/') || skip[at] {
                    break;
                }
                if !c.is_ascii_digit() {
                    has_letter = true;
                }
                end = at + c.len_utf8();
            }
            if has_letter {
                b.push(i..end, flag(SpanStyle::TAG));
            }
        }
        prev = Some(ch);
    }
}

fn prev_allows_tag(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(c) => c.is_whitespace() || (!c.is_alphanumeric() && !"#/&\\([{<=".contains(c)),
    }
}

/// The line that starts at byte `i` (without its newline).
fn line_at(text: &str, i: usize) -> &str {
    text.get(i..).unwrap_or("").split('\n').next().unwrap_or("")
}

fn is_heading_line(line: &str) -> bool {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return false;
    }
    let hashes = t.bytes().take_while(|&c| c == b'#').count();
    if !(1..=6).contains(&hashes) {
        return false;
    }
    match t[hashes..].chars().next() {
        None => true,
        Some(c) => c == ' ' || c == '\t',
    }
}

/// A trailing ` ^block-id` on a line: the `^id` is styled as a block id.
fn scan_block_ids(b: &mut Builder, skip: &[bool]) {
    let text = b.text;
    for lr in line_ranges(text, 0..text.len()) {
        let line = &text[lr.clone()];
        let t = line.trim_end();
        let Some(ws) = t.rfind(char::is_whitespace) else {
            continue;
        };
        let ws_len = t[ws..].chars().next().map_or(1, char::len_utf8);
        let tok_start = ws + ws_len;
        let Some(tok) = t.get(tok_start..) else {
            continue;
        };
        let Some(id) = tok.strip_prefix('^') else {
            continue;
        };
        if id.is_empty()
            || id.len() > MAX_BLOCK_ID
            || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            || skip.get(lr.start + tok_start).copied().unwrap_or(true)
        {
            continue;
        }
        b.push(
            lr.start + tok_start..lr.start + t.len(),
            flag(SpanStyle::BLOCK_ID),
        );
    }
}

/// `==highlight==` within a line.
fn scan_highlights(b: &mut Builder, skip: &[bool]) {
    let text = b.text;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let opens = bytes[i] == b'='
            && bytes[i + 1] == b'='
            && !skip[i]
            && (i == 0 || bytes[i - 1] != b'=')
            && bytes.get(i + 2).is_some_and(|&c| c != b'=' && c != b'\n');
        if !opens {
            i += 1;
            continue;
        }
        let limit = (i + 2 + MAX_INLINE_SCAN).min(bytes.len());
        let mut j = i + 2;
        let mut close = None;
        while j + 1 < limit {
            if bytes[j] == b'\n' {
                break;
            }
            if bytes[j] == b'=' && bytes[j + 1] == b'=' && bytes[j - 1] != b'=' {
                close = Some(j);
                break;
            }
            j += 1;
        }
        let Some(close) = close else {
            i += 1;
            continue;
        };
        if skip[i..close + 2].iter().any(|&s| s) {
            i += 1;
            continue;
        }
        b.push(i..i + 2, marker());
        b.push(i + 2..close, flag(SpanStyle::HIGHLIGHT));
        b.push(close..close + 2, marker());
        i = close + 2;
    }
}

/// End of a `---` ... `---` frontmatter block at the start of the text, if there is one. The
/// returned offset is the end of the closing line (without its newline).
pub(crate) fn frontmatter_end(text: &str) -> Option<usize> {
    let first_nl = text.find('\n')?;
    if text[..first_nl].trim_end_matches('\r') != "---" {
        return None;
    }
    let mut pos = first_nl + 1;
    loop {
        let line_end = text[pos..].find('\n').map_or(text.len(), |k| pos + k);
        if text[pos..line_end].trim_end_matches('\r') == "---" {
            return Some(line_end);
        }
        if line_end >= text.len() {
            return None;
        }
        pos = line_end + 1;
    }
}

/// The range without trailing `\n` and `\r` bytes (both are ASCII, so the cut is a boundary).
fn trim_newlines(text: &str, range: Range<usize>) -> Range<usize> {
    let bytes = text.as_bytes();
    let mut end = range.end.min(bytes.len());
    while end > range.start && matches!(bytes[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    range.start..end
}

fn mark(skip: &mut [bool], range: Range<usize>) {
    if let Some(s) = skip.get_mut(range) {
        for x in s {
            *x = true;
        }
    }
}

// ----- tests ----------------------------------------------------------------------------

/// The lines around an edit that form one paragraph: from the blank line (or the start) before the
/// line of `prefix` to the blank line (or the end) after the line of `new_end`. Returns the byte
/// range `(start, end)` of those lines in `text`, without the newline at the end.
fn paragraph_around(text: &str, prefix: usize, new_end: usize) -> (usize, usize) {
    let b = text.as_bytes();
    let p = prefix.min(b.len());
    let mut rs = b[..p]
        .iter()
        .rposition(|&c| c == b'\n')
        .map_or(0, |i| i + 1);
    while rs > 0 {
        let prev_end = rs - 1;
        let prev_start = b[..prev_end]
            .iter()
            .rposition(|&c| c == b'\n')
            .map_or(0, |i| i + 1);
        if text
            .get(prev_start..prev_end)
            .is_none_or(|l| l.trim().is_empty())
        {
            break;
        }
        rs = prev_start;
    }
    let ne = new_end.min(b.len());
    let mut le = b[ne..]
        .iter()
        .position(|&c| c == b'\n')
        .map_or(b.len(), |i| ne + i);
    while le < b.len() {
        let next_start = le + 1;
        let next_end = b[next_start..]
            .iter()
            .position(|&c| c == b'\n')
            .map_or(b.len(), |i| next_start + i);
        if text
            .get(next_start..next_end)
            .is_none_or(|l| l.trim().is_empty())
        {
            break;
        }
        le = next_end;
    }
    (rs, le)
}

/// Whether a paragraph's text is parsed the same alone as inside the whole note: no line that could
/// open or continue a block (indentation, list, quote, heading, fence, table, rule, html, digits),
/// no blank line and no bracket (a reference link may name a definition elsewhere).
fn plain_paragraph(region: &str) -> bool {
    if region.contains('[') || region.contains('|') || region.trim().is_empty() {
        return false;
    }
    region
        .split('\n')
        .all(|line| match line.as_bytes().first() {
            None | Some(b' ' | b'\t') => false,
            Some(c) => !matches!(
                c,
                b'#' | b'>' | b'-' | b'*' | b'+' | b'=' | b'`' | b'~' | b'<' | b'_' | b'0'..=b'9'
            ),
        })
}

/// The spans of `new_text` after one edit, without parsing the whole note: the paragraph around the
/// edit is parsed again, and the spans of the other paragraphs are kept (moved by the change).
/// `old` are the spans of the text before the edit. Returns `None` when the edit touches anything
/// that is not a plain paragraph (see `plain_paragraph`) or the frontmatter; the caller then parses
/// the whole note. The result equals `compute_spans(new_text)` (tested).
pub fn compute_spans_after_edit(
    old: &[StyleSpan],
    new_text: &str,
    edit: &crate::ui::text_input::invalidate::EditSpan,
) -> Option<Vec<StyleSpan>> {
    let (rs, le) = paragraph_around(new_text, edit.prefix, edit.new_end);
    let region = new_text.get(rs..le)?;
    if !plain_paragraph(region) {
        return None;
    }
    if frontmatter_end(new_text).is_some_and(|end| rs < end) {
        return None;
    }
    let delta = edit.delta();
    // Old offsets of the region's end: the text after the region is the same, moved by the change.
    let le_old = crate::ui::text_input::invalidate::shift(le, -delta);
    let mut out: Vec<StyleSpan> = Vec::with_capacity(old.len());
    for s in old {
        if s.range.end <= rs {
            out.push(s.clone());
        } else if s.range.start >= le_old {
            out.push(StyleSpan {
                range: crate::ui::text_input::invalidate::shift_range(&s.range, delta),
                style: s.style,
            });
        }
    }
    for s in compute_spans(region) {
        out.push(StyleSpan {
            range: s.range.start + rs..s.range.end + rs,
            style: s.style,
        });
    }
    out.sort_unstable_by_key(|s| (s.range.start, s.range.end, s.style.bits(), s.style.heading));
    out.dedup();
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incremental_spans_match_a_full_parse() {
        let doc = "# Title\n\nSome **bold** text here.\nAnother line with `code` and more.\n\n- item\n\nPlain paragraph with *em* words.\n\n```\ncode\n```\n\nEnd para here.\n\nLast words _under_ it\n";
        let mut used = 0usize;
        let mut positions: Vec<usize> = doc.char_indices().map(|(i, _)| i).collect();
        positions.push(doc.len());
        for &p in &positions {
            for insert in ["x", "*", "\n", "\n\n", " **b**", "1. "] {
                let mut new = doc.to_string();
                new.insert_str(p, insert);
                let old_spans = compute_spans(doc);
                let edit = crate::ui::text_input::invalidate::edit_span(doc, &new);
                if let Some(got) = compute_spans_after_edit(&old_spans, &new, &edit) {
                    used += 1;
                    assert_eq!(got, compute_spans(&new), "insert {insert:?} at {p}");
                }
            }
            if let Some(c) = doc.get(p..).and_then(|t| t.chars().next()) {
                let mut new = doc.to_string();
                new.replace_range(p..p + c.len_utf8(), "");
                let old_spans = compute_spans(doc);
                let edit = crate::ui::text_input::invalidate::edit_span(doc, &new);
                if let Some(got) = compute_spans_after_edit(&old_spans, &new, &edit) {
                    used += 1;
                    assert_eq!(got, compute_spans(&new), "delete at {p}");
                }
            }
        }
        assert!(
            used > 100,
            "the incremental path should cover plain paragraph edits ({used})"
        );
    }

    #[test]
    fn incremental_spans_fall_back_for_blocks() {
        let doc = "para one\n\n- list item\n";
        let new = "para one\n\n- list itemz\n";
        let edit = crate::ui::text_input::invalidate::edit_span(doc, new);
        assert!(compute_spans_after_edit(&compute_spans(doc), new, &edit).is_none());
        let doc2 = "a\n\nb c\n";
        let new2 = "a\n\nb [c\n";
        let edit2 = crate::ui::text_input::invalidate::edit_span(doc2, new2);
        assert!(compute_spans_after_edit(&compute_spans(doc2), new2, &edit2).is_none());
    }

    fn has(text: &str, slice: &str, flag: u32) -> bool {
        compute_spans(text)
            .iter()
            .any(|s| text.get(s.range.clone()) == Some(slice) && s.style.has(flag))
    }

    /// Some span with `flag` covers the bytes of `word` (which must occur in `text`).
    fn covers(text: &str, word: &str, flag: u32) -> bool {
        let Some(at) = text.find(word) else {
            return false;
        };
        let r = at..at + word.len();
        compute_spans(text)
            .iter()
            .any(|s| s.style.has(flag) && s.range.start <= r.start && s.range.end >= r.end)
    }

    fn count(text: &str, flag: u32) -> usize {
        compute_spans(text)
            .iter()
            .filter(|s| s.style.has(flag))
            .count()
    }

    fn heading_span(text: &str, slice: &str, level: u8) -> bool {
        compute_spans(text)
            .iter()
            .any(|s| s.style.heading == level && text.get(s.range.clone()) == Some(slice))
    }

    /// Checks the output invariants on `text` and that the line segments tile every line.
    fn check_invariants(text: &str) {
        let spans = compute_spans(text);
        let mut last_start = 0;
        for s in &spans {
            assert!(s.range.start < s.range.end, "empty span in {text:?}");
            assert!(s.range.end <= text.len(), "span past end in {text:?}");
            assert!(
                text.is_char_boundary(s.range.start),
                "bad start in {text:?}"
            );
            assert!(text.is_char_boundary(s.range.end), "bad end in {text:?}");
            assert!(s.range.start >= last_start, "unsorted in {text:?}");
            last_start = s.range.start;
            assert!(s.style.bits() != 0 || s.style.heading != 0, "plain span");
        }
        for w in spans.windows(2) {
            assert!(w[0] != w[1], "duplicate span in {text:?}");
        }
        let lines = split_lines(text);
        let segs = line_segments(&spans, &lines);
        assert_eq!(segs.len(), lines.len());
        for (line, line_segs) in lines.iter().zip(&segs) {
            if line.start >= line.end {
                assert!(line_segs.is_empty());
                continue;
            }
            let mut pos = line.start;
            for seg in line_segs {
                assert_eq!(seg.range.start, pos, "gap in segments of {text:?}");
                assert!(seg.range.end > seg.range.start);
                assert!(text.is_char_boundary(seg.range.start));
                assert!(text.is_char_boundary(seg.range.end));
                pos = seg.range.end;
            }
            assert_eq!(pos, line.end, "segments do not reach line end in {text:?}");
        }
    }

    // --- headings ---

    #[test]
    fn heading_levels_one_to_six() {
        for level in 1..=6u8 {
            let t = format!("{} Title", "#".repeat(level as usize));
            assert!(heading_span(&t, &t, level), "level {level} in {t:?}");
            assert!(
                has(&t, "#".repeat(level as usize).as_str(), SpanStyle::MARKER)
                    || has(
                        &t,
                        &format!("{} ", "#".repeat(level as usize)),
                        SpanStyle::MARKER
                    )
            );
        }
    }

    #[test]
    fn heading_seven_hashes_is_not_a_heading() {
        let t = "####### no";
        assert!(!compute_spans(t).iter().any(|s| s.style.heading > 0));
    }

    #[test]
    fn heading_without_space_is_not_a_heading() {
        let t = "#tag";
        assert!(!compute_spans(t).iter().any(|s| s.style.heading > 0));
        assert!(has(t, "#tag", SpanStyle::TAG));
    }

    #[test]
    fn heading_closing_hashes_are_markers() {
        let t = "## Title ##";
        assert!(heading_span(t, t, 2));
        assert!(has(t, " ##", SpanStyle::MARKER));
    }

    #[test]
    fn heading_empty_marker_only() {
        // CommonMark: a lone `#` is an empty level-1 heading.
        let t = "#";
        assert!(heading_span(t, "#", 1));
        assert!(has(t, "#", SpanStyle::MARKER));
    }

    #[test]
    fn heading_followed_by_newline_keeps_its_marker() {
        let t = "# 沉积\n\nbody";
        assert!(heading_span(t, "# 沉积", 1));
        assert!(has(t, "# ", SpanStyle::MARKER));
    }

    #[test]
    fn setext_heading_underline_is_marker() {
        let t = "Title\n===";
        assert!(compute_spans(t).iter().any(|s| s.style.heading == 1));
        assert!(has(t, "===", SpanStyle::MARKER));
    }

    #[test]
    fn setext_h2_with_dashes() {
        let t = "Sub\n---";
        assert!(compute_spans(t).iter().any(|s| s.style.heading == 2));
    }

    #[test]
    fn heading_hash_inside_is_not_tag() {
        let t = "# Title #tag";
        assert_eq!(count(t, SpanStyle::TAG), 0);
    }

    #[test]
    fn heading_with_link_and_bold() {
        let t = "# [a](b) **c**";
        assert!(heading_span(t, t, 1));
        assert!(has(t, "a", SpanStyle::LINK_TEXT));
        assert!(has(t, "c", SpanStyle::BOLD));
    }

    #[test]
    fn heading_with_cjk_and_emoji() {
        let t = "# 标题 😀";
        assert!(heading_span(t, t, 1));
        check_invariants(t);
    }

    // --- emphasis ---

    #[test]
    fn bold_with_stars() {
        let t = "**bold**";
        assert!(has(t, "bold", SpanStyle::BOLD));
        assert!(count(t, SpanStyle::MARKER) >= 2);
    }

    #[test]
    fn bold_with_underscores() {
        assert!(has("__b__", "b", SpanStyle::BOLD));
    }

    #[test]
    fn italic_with_star_and_underscore() {
        assert!(has("*it*", "it", SpanStyle::ITALIC));
        assert!(has("_it_", "it", SpanStyle::ITALIC));
    }

    #[test]
    fn bold_italic_triple_star() {
        let t = "***bi***";
        assert!(covers(t, "bi", SpanStyle::BOLD));
        assert!(covers(t, "bi", SpanStyle::ITALIC));
    }

    #[test]
    fn nested_emphasis() {
        let t = "**a *b* c**";
        assert!(covers(t, "a", SpanStyle::BOLD));
        assert!(has(t, "b", SpanStyle::ITALIC));
        assert!(covers(t, "b", SpanStyle::BOLD));
    }

    #[test]
    fn strikethrough_double_tilde() {
        assert!(has("~~x~~", "x", SpanStyle::STRIKE));
    }

    #[test]
    fn strikethrough_single_tilde_is_not_crash() {
        check_invariants("~x~ and ~~y~~");
    }

    #[test]
    fn unclosed_bold_is_plain() {
        assert_eq!(count("**bold", SpanStyle::BOLD), 0);
    }

    #[test]
    fn lone_star_is_plain() {
        assert!(compute_spans("a*b").is_empty());
    }

    #[test]
    fn lone_star_is_an_empty_list_item() {
        // CommonMark: a bullet with nothing after it is an empty list item.
        assert!(has("*", "*", SpanStyle::LIST_MARKER));
    }

    #[test]
    fn spaced_star_is_not_emphasis() {
        assert_eq!(count("a * b * c", SpanStyle::ITALIC), 0);
    }

    #[test]
    fn intraword_underscores_are_not_emphasis() {
        assert_eq!(count("snake_case_name", SpanStyle::ITALIC), 0);
    }

    #[test]
    fn bold_cjk_and_emoji() {
        let t = "**中文😀**";
        assert!(has(t, "中文😀", SpanStyle::BOLD));
        check_invariants(t);
    }

    #[test]
    fn emphasis_across_cjk_text() {
        let t = "这是*斜体*和**粗体**";
        assert!(has(t, "斜体", SpanStyle::ITALIC));
        assert!(has(t, "粗体", SpanStyle::BOLD));
    }

    // --- code ---

    #[test]
    fn inline_code_single_backtick() {
        let t = "`x`";
        assert!(has(t, "x", SpanStyle::CODE));
        assert!(count(t, SpanStyle::MARKER) == 2);
    }

    #[test]
    fn inline_code_double_backtick_with_inner_backtick() {
        let t = "``a`b``";
        assert!(has(t, "a`b", SpanStyle::CODE));
    }

    #[test]
    fn lone_backtick_is_plain() {
        assert_eq!(count("a ` b", SpanStyle::CODE), 0);
    }

    #[test]
    fn bold_inside_code_is_not_bold() {
        assert_eq!(count("`**x**`", SpanStyle::BOLD), 0);
    }

    #[test]
    fn fenced_code_with_language() {
        let t = "```rust\nlet x = 1;\n```";
        assert!(has(t, "```rust", SpanStyle::CODE_FENCE));
        assert!(has(t, "let x = 1;", SpanStyle::CODE_BLOCK));
        assert!(has(t, "```", SpanStyle::CODE_FENCE));
    }

    #[test]
    fn fenced_code_unclosed_runs_to_end() {
        let t = "```\ncode line";
        assert!(has(t, "```", SpanStyle::CODE_FENCE));
        assert!(has(t, "code line", SpanStyle::CODE_BLOCK));
    }

    #[test]
    fn fenced_code_tilde() {
        let t = "~~~\na\n~~~";
        assert!(has(t, "a", SpanStyle::CODE_BLOCK));
        assert!(has(t, "~~~", SpanStyle::CODE_FENCE));
    }

    #[test]
    fn indented_code_block() {
        let t = "para\n\n    code here\n";
        assert!(covers(t, "code here", SpanStyle::CODE_BLOCK));
    }

    #[test]
    fn code_block_hides_wikilink_and_tag() {
        let t = "```\n[[x]] #tag\n```";
        assert_eq!(count(t, SpanStyle::WIKILINK), 0);
        assert_eq!(count(t, SpanStyle::TAG), 0);
    }

    #[test]
    fn code_block_hides_bold() {
        let t = "```\n**not bold**\n```";
        assert_eq!(count(t, SpanStyle::BOLD), 0);
    }

    #[test]
    fn code_span_hides_wikilink() {
        assert_eq!(count("`[[x]]`", SpanStyle::WIKILINK), 0);
    }

    // --- links ---

    #[test]
    fn inline_link_parts() {
        let t = "[t](https://x.y)";
        assert!(has(t, "t", SpanStyle::LINK_TEXT));
        assert!(has(t, "https://x.y", SpanStyle::LINK_URL));
        assert!(has(t, "(", SpanStyle::MARKER));
        assert!(has(t, ")", SpanStyle::MARKER));
    }

    #[test]
    fn reference_link() {
        let t = "[t][r]\n\n[r]: http://x";
        assert!(covers(t, "t", SpanStyle::LINK_TEXT));
    }

    #[test]
    fn autolink_angle_brackets() {
        let t = "<https://a.b>";
        assert!(has(t, "https://a.b", SpanStyle::LINK_URL));
    }

    #[test]
    fn image_with_alt_and_source() {
        let t = "![alt](a.png)";
        assert!(has(t, "alt", SpanStyle::LINK_TEXT));
        assert!(has(t, "a.png", SpanStyle::LINK_URL));
        assert!(has(t, "!", SpanStyle::MARKER));
    }

    #[test]
    fn link_text_with_nested_brackets() {
        assert!(has("[a [b] c](u)", "a [b] c", SpanStyle::LINK_TEXT));
    }

    #[test]
    fn unclosed_link_is_plain() {
        assert_eq!(count("[x](", SpanStyle::LINK_TEXT), 0);
    }

    #[test]
    fn link_with_bold_text() {
        let t = "[**b**](u)";
        assert!(covers(t, "b", SpanStyle::BOLD));
        assert!(covers(t, "b", SpanStyle::LINK_TEXT));
    }

    #[test]
    fn footnote_reference() {
        let t = "text[^1]\n\n[^1]: note";
        assert!(count(t, SpanStyle::LINK_TEXT) >= 1);
    }

    // --- wikilinks and embeds ---

    #[test]
    fn wikilink_plain() {
        let t = "[[a]]";
        assert!(has(t, "a", SpanStyle::WIKILINK));
        assert!(has(t, "[[", SpanStyle::MARKER));
        assert!(has(t, "]]", SpanStyle::MARKER));
    }

    #[test]
    fn wikilink_embed_marks_bang() {
        let t = "![[img.png]]";
        assert!(has(t, "img.png", SpanStyle::WIKILINK));
        assert!(has(t, "!", SpanStyle::MARKER));
    }

    #[test]
    fn wikilink_alias() {
        assert!(has("[[a|b]]", "a|b", SpanStyle::WIKILINK));
    }

    #[test]
    fn wikilink_heading_fragment_is_not_tag() {
        let t = "[[a#h]]";
        assert!(has(t, "a#h", SpanStyle::WIKILINK));
        assert_eq!(count(t, SpanStyle::TAG), 0);
    }

    #[test]
    fn wikilink_unclosed_is_plain() {
        assert_eq!(count("[[a", SpanStyle::WIKILINK), 0);
    }

    #[test]
    fn wikilink_empty_is_plain() {
        assert_eq!(count("[[]]", SpanStyle::WIKILINK), 0);
    }

    #[test]
    fn wikilink_does_not_span_lines() {
        assert_eq!(count("[[a\nb]]", SpanStyle::WIKILINK), 0);
    }

    #[test]
    fn wikilink_cjk_target() {
        assert!(has("[[沉积的想法]]", "沉积的想法", SpanStyle::WIKILINK));
    }

    #[test]
    fn two_wikilinks_on_a_line() {
        assert_eq!(count("[[a]] and [[b]]", SpanStyle::WIKILINK), 2);
    }

    #[test]
    fn wikilink_opener_chain_is_linear_and_valid() {
        let t = "[[".repeat(2000);
        check_invariants(&t);
    }

    // --- tags ---

    #[test]
    fn tag_plain() {
        assert!(has("see #tag here", "#tag", SpanStyle::TAG));
    }

    #[test]
    fn tag_cjk() {
        assert!(has("#中文标签", "#中文标签", SpanStyle::TAG));
    }

    #[test]
    fn tag_with_slash_dash_underscore() {
        assert!(has("#a/b-c_d", "#a/b-c_d", SpanStyle::TAG));
    }

    #[test]
    fn tag_all_digits_is_not_tag() {
        assert_eq!(count("#123", SpanStyle::TAG), 0);
    }

    #[test]
    fn tag_digits_with_cjk_is_tag() {
        assert!(has("#2024年", "#2024年", SpanStyle::TAG));
    }

    #[test]
    fn hash_inside_word_is_not_tag() {
        assert_eq!(count("a#b", SpanStyle::TAG), 0);
    }

    #[test]
    fn csharp_is_not_tag() {
        assert_eq!(count("C# language", SpanStyle::TAG), 0);
    }

    #[test]
    fn url_fragment_is_not_tag() {
        assert_eq!(count("https://x.com/#frag", SpanStyle::TAG), 0);
    }

    #[test]
    fn tag_in_parentheses_is_not_tag() {
        assert_eq!(count("(#x)", SpanStyle::TAG), 0);
    }

    #[test]
    fn tag_stops_at_comma() {
        assert!(has("#tag, next", "#tag", SpanStyle::TAG));
    }

    #[test]
    fn tag_after_fullwidth_paren() {
        assert!(has("（#中文）", "#中文", SpanStyle::TAG));
    }

    #[test]
    fn two_tags() {
        assert_eq!(count("#tag1 #tag2", SpanStyle::TAG), 2);
    }

    #[test]
    fn tag_inline_code_is_not_tag() {
        assert_eq!(count("`#code`", SpanStyle::TAG), 0);
    }

    #[test]
    fn tag_after_cjk_letter_is_not_tag() {
        assert_eq!(count("中文#tag", SpanStyle::TAG), 0);
    }

    #[test]
    fn double_hash_is_not_tag() {
        assert_eq!(count("##tag", SpanStyle::TAG), 0);
    }

    #[test]
    fn tag_at_line_start_on_second_line() {
        assert!(has("a\n#tag", "#tag", SpanStyle::TAG));
    }

    // --- callouts and quotes ---

    #[test]
    fn callout_title() {
        let t = "> [!note] Title";
        assert!(has(t, "[!note] Title", SpanStyle::CALLOUT_TITLE));
        assert!(has(t, "> ", SpanStyle::MARKER));
    }

    #[test]
    fn callout_with_fold_sign() {
        assert!(has(
            "> [!warning]+ Hi",
            "[!warning]+ Hi",
            SpanStyle::CALLOUT_TITLE
        ));
    }

    #[test]
    fn plain_quote_is_not_callout() {
        assert_eq!(count("> plain", SpanStyle::CALLOUT_TITLE), 0);
        assert!(covers("> plain", "plain", SpanStyle::QUOTE));
    }

    #[test]
    fn unclosed_callout_is_not_callout() {
        assert_eq!(count("> [!note", SpanStyle::CALLOUT_TITLE), 0);
    }

    #[test]
    fn quote_nested_markers() {
        let t = "> > nested";
        assert!(count(t, SpanStyle::MARKER) >= 2);
        assert!(covers(t, "nested", SpanStyle::QUOTE));
    }

    #[test]
    fn quote_two_lines() {
        let t = "> a\n> b";
        assert!(covers(t, "b", SpanStyle::QUOTE));
        assert!(count(t, SpanStyle::MARKER) >= 2);
    }

    #[test]
    fn quote_hides_marker_inside_code() {
        let t = "> ```\n> >x\n> ```";
        check_invariants(t);
    }

    // --- lists ---

    #[test]
    fn bullet_dash() {
        assert!(has("- item", "-", SpanStyle::LIST_MARKER));
    }

    #[test]
    fn bullet_star() {
        assert!(has("* item", "*", SpanStyle::LIST_MARKER));
    }

    #[test]
    fn ordered_dot() {
        assert!(has("1. one", "1.", SpanStyle::LIST_MARKER));
    }

    #[test]
    fn ordered_paren() {
        assert!(has("2) two", "2)", SpanStyle::LIST_MARKER));
    }

    #[test]
    fn task_unchecked() {
        let t = "- [ ] task";
        assert!(has(t, "[ ]", SpanStyle::TASK_MARKER));
        assert!(has(t, "-", SpanStyle::LIST_MARKER));
    }

    #[test]
    fn task_checked() {
        assert!(has("- [x] done", "[x]", SpanStyle::TASK_MARKER));
        assert!(has("- [X] done", "[X]", SpanStyle::TASK_MARKER));
    }

    #[test]
    fn nested_list_markers() {
        let t = "- a\n  - nested";
        assert_eq!(count(t, SpanStyle::LIST_MARKER), 2);
    }

    #[test]
    fn list_item_with_code_block() {
        let t = "- item\n  ```\n  code\n  ```";
        assert!(covers(t, "code", SpanStyle::CODE_BLOCK));
        check_invariants(t);
    }

    #[test]
    fn dash_without_space_is_not_list() {
        assert_eq!(count("-item", SpanStyle::LIST_MARKER), 0);
    }

    // --- block ids, highlights, rules, frontmatter ---

    #[test]
    fn block_id_suffix() {
        assert!(has("para ^abc-1", "^abc-1", SpanStyle::BLOCK_ID));
    }

    #[test]
    fn block_id_needs_space_before() {
        assert_eq!(count("word^abc", SpanStyle::BLOCK_ID), 0);
    }

    #[test]
    fn block_id_bare_caret_is_not_id() {
        assert_eq!(count("a ^", SpanStyle::BLOCK_ID), 0);
    }

    #[test]
    fn block_id_alone_on_line_is_not_id() {
        assert_eq!(count("^abc", SpanStyle::BLOCK_ID), 0);
    }

    #[test]
    fn highlight_basic() {
        let t = "==hi==";
        assert!(has(t, "hi", SpanStyle::HIGHLIGHT));
        assert!(has(t, "==", SpanStyle::MARKER));
    }

    #[test]
    fn highlight_triple_equals_is_not() {
        assert_eq!(count("===", SpanStyle::HIGHLIGHT), 0);
    }

    #[test]
    fn highlight_unclosed_is_plain() {
        assert_eq!(count("==unclosed", SpanStyle::HIGHLIGHT), 0);
    }

    #[test]
    fn horizontal_rule_dashes() {
        assert!(has("---", "---", SpanStyle::HR));
    }

    #[test]
    fn horizontal_rule_stars() {
        assert_eq!(count("***", SpanStyle::HR), 1);
    }

    #[test]
    fn frontmatter_block_and_heading_after_it() {
        let t = "---\ntitle: x\n---\n# H";
        assert!(has(t, "---\ntitle: x\n---", SpanStyle::FRONTMATTER));
        assert!(heading_span(t, "# H", 1));
        assert_eq!(count(t, SpanStyle::HR), 0);
    }

    #[test]
    fn frontmatter_hides_tags() {
        let t = "---\ntags: #x\n---\nbody";
        assert_eq!(count(t, SpanStyle::TAG), 0);
    }

    #[test]
    fn frontmatter_unclosed_is_not_frontmatter() {
        let t = "---\nx: 1";
        assert_eq!(count(t, SpanStyle::FRONTMATTER), 0);
        check_invariants(t);
    }

    // --- tables ---

    #[test]
    fn table_pipes_are_markers() {
        let t = "| a | b |\n| --- | --- |\n| 1 | 2 |";
        assert!(count(t, SpanStyle::MARKER) >= 6);
        assert!(has(t, "| --- | --- |", SpanStyle::MARKER));
    }

    #[test]
    fn partial_table_is_plain() {
        assert_eq!(count("| a |", SpanStyle::MARKER), 0);
    }

    #[test]
    fn table_cell_inline_code() {
        let t = "| `x` | y |\n|---|---|";
        assert!(has(t, "x", SpanStyle::CODE));
        check_invariants(t);
    }

    // --- mixed, robustness, line segments ---

    #[test]
    fn empty_text_has_no_spans() {
        assert!(compute_spans("").is_empty());
        assert!(compute_spans("\n\n").is_empty());
    }

    #[test]
    fn crlf_document() {
        let t = "# 标题\r\n**粗**\r\n[[链接]]\r\n";
        assert!(has(t, "粗", SpanStyle::BOLD));
        assert!(has(t, "链接", SpanStyle::WIKILINK));
        check_invariants(t);
    }

    #[test]
    fn mixed_note_has_expected_kinds() {
        let t = "# 沉积\n\n> [!note] 注意\n> 内容 #标签\n\n- [ ] 整理 [[架构]]\n- [x] 完成 ==重点==\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| `c` | d |\n";
        assert!(heading_span(t, "# 沉积", 1));
        assert!(has(t, "[!note] 注意", SpanStyle::CALLOUT_TITLE));
        assert!(has(t, "#标签", SpanStyle::TAG));
        assert!(has(t, "[ ]", SpanStyle::TASK_MARKER));
        assert!(has(t, "架构", SpanStyle::WIKILINK));
        assert!(has(t, "重点", SpanStyle::HIGHLIGHT));
        assert!(has(t, "fn main() {}", SpanStyle::CODE_BLOCK));
        assert!(has(t, "c", SpanStyle::CODE));
        check_invariants(t);
    }

    #[test]
    fn unicode_offsets_are_char_boundaries() {
        let t = "中😀**a**文[[字]]#标签";
        check_invariants(t);
    }

    #[test]
    fn huge_single_line_stays_valid() {
        let t = "*a".repeat(20_000);
        check_invariants(&t);
    }

    #[test]
    fn huge_line_of_brackets_stays_valid() {
        let t = "[[ ]] ==a== #b ".repeat(3000);
        check_invariants(&t);
    }

    #[test]
    fn segments_tile_a_bold_link_line() {
        let t = "**a [b](c)** d";
        let spans = compute_spans(t);
        let lines = split_lines(t);
        let segs = line_segments(&spans, &lines);
        assert_eq!(segs.len(), 1);
        let line = &segs[0];
        assert_eq!(line.first().map(|s| s.range.start), Some(0));
        assert_eq!(line.last().map(|s| s.range.end), Some(t.len()));
        let bold_at_b = line
            .iter()
            .find(|s| &t[s.range.clone()] == "b")
            .map(|s| s.style);
        let Some(style) = bold_at_b else {
            panic!("no segment for b");
        };
        assert!(style.has(SpanStyle::BOLD));
        assert!(style.has(SpanStyle::LINK_TEXT));
    }

    #[test]
    fn segments_of_empty_and_plain_lines() {
        let t = "\nplain";
        let spans = compute_spans(t);
        let segs = line_segments(&spans, &split_lines(t));
        assert!(segs[0].is_empty());
        assert_eq!(segs[1].len(), 1);
        assert!(segs[1][0].style.is_plain());
    }

    #[test]
    fn split_lines_matches_str_split() {
        for t in ["", "a", "a\nb", "\n", "a\n", "\n\nx\n"] {
            let expect: Vec<&str> = t.split('\n').collect();
            let got: Vec<&str> = split_lines(t).into_iter().map(|r| &t[r]).collect();
            assert_eq!(expect, got, "for {t:?}");
        }
    }

    #[test]
    fn line_segments_clip_multi_line_quote() {
        let t = "> a\n> b\n\nc";
        let spans = compute_spans(t);
        let segs = line_segments(&spans, &split_lines(t));
        assert_eq!(segs.len(), 4);
        assert!(segs[1].iter().any(|s| s.style.has(SpanStyle::QUOTE)));
        assert!(segs[2].is_empty());
        assert!(!segs[3].iter().any(|s| s.style.has(SpanStyle::QUOTE)));
    }

    #[test]
    fn lazy_quote_continuation_belongs_to_the_quote() {
        // CommonMark: a line without `>` continues the quoted paragraph.
        let t = "> a\n> b\nc";
        let segs = line_segments(&compute_spans(t), &split_lines(t));
        assert!(segs[2].iter().any(|s| s.style.has(SpanStyle::QUOTE)));
    }

    /// Deterministic pseudo-random generator (LCG), so the fuzz test is repeatable.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }
        fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
            items[(self.next() as usize) % items.len()]
        }
    }

    #[test]
    fn fuzz_token_soup_keeps_invariants() {
        const TOKENS: &[&str] = &[
            "*",
            "**",
            "***",
            "_",
            "__",
            "`",
            "``",
            "```",
            "~~",
            "~",
            "#",
            "# ",
            "##",
            "[",
            "]",
            "[[",
            "]]",
            "(",
            ")",
            "![[",
            "![",
            "!",
            ">",
            "> ",
            "> [!note] ",
            "|",
            "| ",
            "---",
            "- ",
            "1. ",
            "- [ ] ",
            "- [x] ",
            "==",
            "^",
            "^id ",
            " ",
            "\n",
            "\r\n",
            "\r",
            "\t",
            "中",
            "😀",
            "a",
            "#tag",
            "    ",
            "<",
            ">",
            "http://a",
            "\\",
            "=",
            "---\n",
        ];
        let mut rng = Lcg(0x9E37_79B9_7F4A_7C15);
        for _ in 0..3000 {
            let n = (rng.next() % 80) as usize;
            let mut s = String::new();
            for _ in 0..n {
                s.push_str(rng.pick(TOKENS));
            }
            check_invariants(&s);
        }
    }

    #[test]
    fn fuzz_random_chars_keep_invariants() {
        let alphabet: Vec<char> = "ab *_`#[]()!>|-~=^\n\r\t中文😀é\u{3000}".chars().collect();
        let mut rng = Lcg(42);
        for _ in 0..1500 {
            let n = (rng.next() % 120) as usize;
            let s: String = (0..n)
                .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                .collect();
            check_invariants(&s);
        }
    }

    #[test]
    fn fuzz_prefixes_of_a_real_note_keep_invariants() {
        let note = "---\ntitle: 沉积\n---\n# 沉积的想法\n\n> [!note] 提示\n> 内容 #标签 [[链接|别名]]\n\n- [x] 完成 **粗** `代码`\n  - 子项 ==高亮== ^id-1\n\n```rust\nfn f() {}\n```\n\n| a | b |\n|---|:-:|\n| 1 | ~~2~~ |\n\n[link](https://x.y/#z) ![[图.png|300]]\n";
        let mut end = 0;
        while end <= note.len() {
            if note.is_char_boundary(end) {
                check_invariants(&note[..end]);
            }
            end += 1;
        }
    }

    #[test]
    fn long_document_stays_valid() {
        let block = "## 小节\n\n正文 **粗** *斜* `码` [[链接]] #标签 ==高亮==\n\n- [ ] 任务\n\n";
        let t = block.repeat(400);
        check_invariants(&t);
    }

    /// Measured, not run by default: `cargo test -p jiyue --release -- --ignored bench`.
    /// Prints the time for a 200 KB note and asserts the 15 ms budget in release builds.
    #[test]
    #[ignore]
    fn bench_200kb_document() {
        let block = "# 章节标题\n\n正文段落包含 **粗体**、*斜体*、`行内代码` 和 [链接](https://example.com)。\n\n> [!note] 提示\n> 引用 #标签 [[双链笔记|别名]] ==高亮==\n\n- [ ] 待办事项 ^blk-1\n- [x] 已完成\n\n```rust\nfn main() { println!(\"hi\"); }\n```\n\n| 列 | 值 |\n|---|---|\n| a | 1 |\n\n";
        let mut t = String::new();
        while t.len() < 200 * 1024 {
            t.push_str(block);
        }
        let mut best_spans = std::time::Duration::MAX;
        let mut best_segs = std::time::Duration::MAX;
        let mut spans_n = 0;
        for _ in 0..15 {
            let start = std::time::Instant::now();
            let spans = compute_spans(&t);
            let mid = std::time::Instant::now();
            let lines = split_lines(&t);
            let _segs = line_segments(&spans, &lines);
            best_spans = best_spans.min(mid - start);
            best_segs = best_segs.min(mid.elapsed());
            spans_n = spans.len();
        }
        let total = best_spans + best_segs;
        println!(
            "200 KB: compute_spans {:.2} ms + line_segments {:.2} ms = {:.2} ms best of 15 ({} bytes, {} spans)",
            best_spans.as_secs_f64() * 1000.0,
            best_segs.as_secs_f64() * 1000.0,
            total.as_secs_f64() * 1000.0,
            t.len(),
            spans_n
        );
        if !cfg!(debug_assertions) {
            assert!(total.as_millis() < 15, "over budget: {total:?}");
        }
    }
}
