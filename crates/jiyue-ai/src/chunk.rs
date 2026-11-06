//! Splits a Markdown note into chunks for embedding.
//!
//! Rules:
//! - YAML frontmatter (`---` ... `---` at the very top) is skipped.
//! - Headings start a new section; each chunk carries its heading path (`A > B`).
//! - Inside a section, paragraphs (blank-line separated) are packed into chunks of up to about
//!   800 characters. Long paragraphs are split at sentence or space boundaries. CJK text works
//!   without spaces because break points also include `。！？；` and newlines.
//! - Fenced code blocks are atomic: a block is never split across chunks (only a block larger
//!   than 2000 characters is cut, at line boundaries).
//! - Consecutive prose chunks in a section overlap by 80 characters.
//! - Chunks with fewer than 20 non-whitespace characters are dropped.
//!
//! Chunk ranges are byte offsets into the original text and always fall on char boundaries.

use std::ops::Range;

/// Soft maximum for one prose chunk (characters).
pub const MAX_CHARS: usize = 800;
/// Characters shared by consecutive prose chunks.
pub const OVERLAP_CHARS: usize = 80;
/// Code blocks up to this size are kept whole.
pub const CODE_MAX_CHARS: usize = 2000;
/// Chunks with fewer non-whitespace characters are dropped.
pub const MIN_CHARS: usize = 20;

/// One chunk of a note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Vault-relative note path, as given to `chunk_note`.
    pub path: String,
    /// Headings above this chunk, e.g. `"Plan > Wave 1"`. Empty before the first heading.
    pub heading_path: String,
    pub text: String,
    /// Byte range of `text` in the original note.
    pub range: Range<usize>,
}

impl Chunk {
    /// Text to embed: the heading path (when present) followed by the chunk text. Including the
    /// heading gives the embedding model context about where the chunk sits in the note.
    pub fn embedding_text(&self) -> String {
        if self.heading_path.is_empty() {
            self.text.clone()
        } else {
            format!("{}\n{}", self.heading_path, self.text)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Unit {
    start: usize,
    end: usize,
    code: bool,
}

struct Section {
    heading: String,
    units: Vec<Unit>,
}

/// Splits `text` into chunks. `path` is stored in every chunk.
pub fn chunk_note(path: &str, text: &str) -> Vec<Chunk> {
    let body_start = frontmatter_len(text);
    let sections = parse_sections(text, body_start);
    let mut out = Vec::new();
    for section in sections {
        let units = split_oversized(text, &section.units);
        let mut ranges = pack(text, &units);
        apply_overlap(text, &mut ranges);
        for (start, end, _code) in ranges {
            let slice = &text[start..end];
            if slice.chars().filter(|c| !c.is_whitespace()).count() < MIN_CHARS {
                continue;
            }
            out.push(Chunk {
                path: path.to_string(),
                heading_path: section.heading.clone(),
                text: slice.to_string(),
                range: start..end,
            });
        }
    }
    out
}

/// Byte length of a leading `---` frontmatter block (including its closing line), or 0.
fn frontmatter_len(text: &str) -> usize {
    let bom = if text.starts_with('\u{feff}') { 3 } else { 0 };
    let rest = &text[bom..];
    let Some(first_nl) = rest.find('\n') else {
        return 0;
    };
    if rest[..first_nl].trim_end_matches('\r') != "---" {
        return 0;
    }
    let mut offset = bom + first_nl + 1;
    for line in rest[first_nl + 1..].split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        if content == "---" || content == "..." {
            return offset + line.len();
        }
        offset += line.len();
    }
    // Unclosed frontmatter: treat the whole file as body.
    0
}

/// (start, end) of each line, without its line terminator. Starts at `from`.
fn line_spans(text: &str, from: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut pos = from;
    for piece in text[from..].split_inclusive('\n') {
        let end = pos + piece.len();
        let mut content_end = if piece.ends_with('\n') { end - 1 } else { end };
        if content_end > pos && text.as_bytes()[content_end - 1] == b'\r' {
            content_end -= 1;
        }
        out.push((pos, content_end));
        pos = end;
    }
    out
}

fn fence_open(trimmed: &str) -> Option<(char, usize)> {
    let ch = trimmed.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let run = trimmed.chars().take_while(|&c| c == ch).count();
    if run >= 3 { Some((ch, run)) } else { None }
}

fn is_fence_close(trimmed: &str, fence: (char, usize)) -> bool {
    let (ch, len) = fence;
    let run = trimmed.chars().take_while(|&c| c == ch).count();
    run >= len && trimmed.chars().skip(run).all(char::is_whitespace)
}

/// Returns (level, title) for an ATX heading line.
fn heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|&c| c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.is_empty() && !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let title = rest.trim().trim_end_matches('#').trim().to_string();
    Some((level, title))
}

fn parse_sections(text: &str, body_start: usize) -> Vec<Section> {
    let lines = line_spans(text, body_start);
    let mut sections = vec![Section {
        heading: String::new(),
        units: Vec::new(),
    }];
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut para: Option<(usize, usize)> = None;

    fn flush(para: &mut Option<(usize, usize)>, sections: &mut [Section]) {
        if let Some((s, e)) = para.take()
            && let Some(sec) = sections.last_mut()
        {
            sec.units.push(Unit {
                start: s,
                end: e,
                code: false,
            });
        }
    }

    let mut i = 0;
    while i < lines.len() {
        let (ls, le) = lines[i];
        let line = &text[ls..le];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut para, &mut sections);
            i += 1;
            continue;
        }
        if let Some(fence) = fence_open(line.trim_start()) {
            flush(&mut para, &mut sections);
            let mut last = i;
            let mut closed = false;
            for (j, &(js, je)) in lines.iter().enumerate().skip(i + 1) {
                last = j;
                if is_fence_close(text[js..je].trim(), fence) {
                    closed = true;
                    break;
                }
            }
            if !closed {
                last = lines.len() - 1;
            }
            let block_end = lines[last].1;
            if let Some(sec) = sections.last_mut() {
                sec.units.push(Unit {
                    start: ls,
                    end: block_end,
                    code: true,
                });
            }
            i = last + 1;
            continue;
        }
        if let Some((level, title)) = heading(line) {
            flush(&mut para, &mut sections);
            while stack.last().is_some_and(|(l, _)| *l >= level) {
                stack.pop();
            }
            stack.push((level, title));
            let heading_path = stack
                .iter()
                .map(|(_, t)| t.as_str())
                .collect::<Vec<_>>()
                .join(" > ");
            sections.push(Section {
                heading: heading_path,
                units: vec![Unit {
                    start: ls,
                    end: le,
                    code: false,
                }],
            });
            i += 1;
            continue;
        }
        para = Some(match para {
            None => (ls, le),
            Some((s, _)) => (s, le),
        });
        i += 1;
    }
    flush(&mut para, &mut sections);
    sections.retain(|s| !s.units.is_empty());
    sections
}

/// Cuts units that are too big: prose at sentence/space boundaries, code at line boundaries.
fn split_oversized(text: &str, units: &[Unit]) -> Vec<Unit> {
    let mut out = Vec::new();
    for u in units {
        let len = text[u.start..u.end].chars().count();
        if u.code {
            if len <= CODE_MAX_CHARS {
                out.push(*u);
            } else {
                out.extend(split_code_lines(text, u));
            }
        } else if len <= MAX_CHARS {
            out.push(*u);
        } else {
            for (s, e) in split_prose(&text[u.start..u.end], MAX_CHARS) {
                out.push(Unit {
                    start: u.start + s,
                    end: u.start + e,
                    code: false,
                });
            }
        }
    }
    out
}

fn split_code_lines(text: &str, u: &Unit) -> Vec<Unit> {
    let mut out = Vec::new();
    let mut group_start = u.start;
    let mut count = 0usize;
    let mut pos = u.start;
    for piece in text[u.start..u.end].split_inclusive('\n') {
        let line_chars = piece.chars().count();
        if count > 0 && count + line_chars > CODE_MAX_CHARS {
            out.push(Unit {
                start: group_start,
                end: pos,
                code: true,
            });
            group_start = pos;
            count = 0;
        }
        count += line_chars;
        pos += piece.len();
    }
    if group_start < u.end {
        out.push(Unit {
            start: group_start,
            end: u.end,
            code: true,
        });
    }
    out
}

/// Splits prose into pieces of at most `max` characters, returning byte ranges relative to `s`.
/// Breaks prefer the last sentence end or space inside the window, then a hard cut.
fn split_prose(s: &str, max: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut piece_start = 0usize;
    let mut count = 0usize;
    let mut last_break: Option<usize> = None; // byte index just after a break character
    for (idx, ch) in s.char_indices() {
        if count == max {
            let cut = match last_break {
                Some(b) if b > piece_start => b,
                _ => idx,
            };
            out.push((piece_start, cut));
            piece_start = cut;
            count = s[cut..idx].chars().count();
            last_break = None;
        }
        count += 1;
        if is_break(ch) {
            last_break = Some(idx + ch.len_utf8());
        }
    }
    if piece_start < s.len() {
        out.push((piece_start, s.len()));
    }
    out
}

fn is_break(ch: char) -> bool {
    matches!(
        ch,
        '\n' | ' ' | '\t' | '。' | '！' | '？' | '；' | '.' | '!' | '?' | ';' | '，' | ','
    )
}

/// Packs units into chunks of at most `MAX_CHARS` characters. Returns (start, end, is_code).
fn pack(text: &str, units: &[Unit]) -> Vec<(usize, usize, bool)> {
    let mut out: Vec<(usize, usize, bool)> = Vec::new();
    let mut cur: Option<(usize, usize, usize, bool)> = None; // start, end, chars, code
    for u in units {
        let ulen = text[u.start..u.end].chars().count();
        match cur {
            None => cur = Some((u.start, u.end, ulen, u.code)),
            Some((s, e, n, code)) => {
                let gap = text[e..u.start].chars().count();
                if n + gap + ulen <= MAX_CHARS {
                    cur = Some((s, u.end, n + gap + ulen, code && u.code));
                } else {
                    out.push((s, e, code));
                    cur = Some((u.start, u.end, ulen, u.code));
                }
            }
        }
    }
    if let Some((s, e, _, code)) = cur {
        out.push((s, e, code));
    }
    out
}

/// Starts each prose chunk up to `OVERLAP_CHARS` characters earlier, inside the previous chunk.
/// The overlap start is moved forward to a word boundary when one is close by (spaces).
fn apply_overlap(text: &str, ranges: &mut [(usize, usize, bool)]) {
    for i in 1..ranges.len() {
        let (ps, pe, pcode) = ranges[i - 1];
        let (cs, ce, ccode) = ranges[i];
        if pcode || ccode || ce <= cs {
            continue;
        }
        let mut p = back_chars(text, pe, OVERLAP_CHARS);
        if p < ps {
            p = ps;
        }
        // Move to just after a space when one falls within the next 20 characters.
        if let Some(off) = text[p..pe]
            .char_indices()
            .take(20)
            .find(|(_, c)| c.is_whitespace())
            .map(|(o, c)| o + c.len_utf8())
        {
            p += off;
        }
        if p < cs && p < pe {
            ranges[i].0 = p;
        }
    }
}

/// Byte index that lies `n` characters before `end`, not before `floor`.
fn back_chars(text: &str, end: usize, n: usize) -> usize {
    let mut idx = 0;
    let mut count = 0;
    for (i, _) in text[..end].char_indices().rev() {
        idx = i;
        count += 1;
        if count == n {
            break;
        }
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_ranges_valid(text: &str, chunks: &[Chunk]) {
        for c in chunks {
            assert_eq!(&text[c.range.clone()], c.text, "range must match text");
            assert!(text.is_char_boundary(c.range.start));
            assert!(text.is_char_boundary(c.range.end));
        }
    }

    #[test]
    fn frontmatter_is_skipped() {
        let text = "---\ntitle: Secret Title\ntags: [a]\n---\n# Real\n\nThis paragraph has enough words to keep.\n";
        let chunks = chunk_note("n.md", text);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| !c.text.contains("Secret Title")));
        assert_ranges_valid(text, &chunks);
    }

    #[test]
    fn unclosed_frontmatter_is_body() {
        let text = "---\nsome body text that is long enough to be a chunk of its own\n";
        let chunks = chunk_note("n.md", text);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.starts_with("---"));
    }

    #[test]
    fn headings_give_heading_paths_and_split_sections() {
        let text = "# Plan\n\nIntro paragraph with more than twenty characters.\n\n## Wave 1\n\nDetails of wave one, long enough to keep.\n\n### Sub\n\nNested text that is also long enough.\n\n## Wave 2\n\nAnother section with enough characters here.\n";
        let chunks = chunk_note("plan.md", text);
        let paths: Vec<&str> = chunks.iter().map(|c| c.heading_path.as_str()).collect();
        assert!(paths.contains(&"Plan"));
        assert!(paths.contains(&"Plan > Wave 1"));
        assert!(paths.contains(&"Plan > Wave 1 > Sub"));
        assert!(paths.contains(&"Plan > Wave 2"));
        assert_ranges_valid(text, &chunks);
    }

    #[test]
    fn code_fence_stays_in_one_chunk_even_with_blank_lines() {
        let code = "```rust\nfn main() {\n\n    println!(\"hi\");\n\n}\n```";
        let text = format!(
            "Intro that is definitely long enough for a chunk.\n\n{code}\n\nOutro text after the block, also long enough."
        );
        let chunks = chunk_note("c.md", &text);
        let holding: Vec<&Chunk> = chunks
            .iter()
            .filter(|c| c.text.contains("fn main"))
            .collect();
        assert_eq!(holding.len(), 1, "code block must not be split");
        assert!(
            holding[0].text.contains(code),
            "whole block must be in one chunk"
        );
        assert_ranges_valid(&text, &chunks);
    }

    #[test]
    fn heading_like_lines_inside_code_are_not_headings() {
        let text = "Para long enough to keep as a first chunk here.\n\n```\n# not a heading\n```\n";
        let chunks = chunk_note("c.md", text);
        assert!(chunks.iter().all(|c| c.heading_path.is_empty()));
    }

    #[test]
    fn long_english_paragraph_is_split_with_overlap() {
        let sentence = "The quick brown fox jumps over the lazy dog near the riverbank. ";
        let text = sentence.repeat(40); // about 2800 characters
        let chunks = chunk_note("long.md", &text);
        assert!(chunks.len() >= 3, "got {} chunks", chunks.len());
        for c in &chunks {
            assert!(
                c.text.chars().count() <= MAX_CHARS + OVERLAP_CHARS + 20,
                "too big: {}",
                c.text.chars().count()
            );
        }
        for pair in chunks.windows(2) {
            assert!(
                pair[1].range.start < pair[0].range.end,
                "consecutive chunks must overlap"
            );
        }
        assert_ranges_valid(&text, &chunks);
    }

    #[test]
    fn cjk_text_splits_at_sentence_marks_without_spaces() {
        let sentence = "这是一段很长的中文笔记内容，用于测试按字符而不是字节切分的逻辑。";
        let text = sentence.repeat(60);
        let chunks = chunk_note("zh.md", &text);
        assert!(chunks.len() >= 3);
        for c in &chunks {
            assert!(c.text.chars().count() <= MAX_CHARS + OVERLAP_CHARS + 5);
            assert!(!c.text.is_empty());
        }
        assert_ranges_valid(&text, &chunks);
        // Chunks should end at a sentence mark (the overlap starts mid-sentence, the chunk end
        // is a break), so most chunks end with 。.
        let ends_ok = chunks
            .iter()
            .filter(|c| c.text.ends_with(['。', '，']))
            .count();
        assert!(
            ends_ok >= chunks.len() - 1,
            "chunk ends: {ends_ok}/{}",
            chunks.len()
        );
    }

    #[test]
    fn cjk_without_any_break_is_hard_cut_on_char_boundaries() {
        let text = "汉".repeat(2000);
        let chunks = chunk_note("hard.md", &text);
        assert!(chunks.len() >= 2);
        assert_ranges_valid(&text, &chunks);
    }

    #[test]
    fn tiny_chunks_are_skipped() {
        assert!(chunk_note("t.md", "hi there").is_empty());
        assert!(chunk_note("t.md", "   \n\n  ").is_empty());
        assert!(chunk_note("t.md", "# Title\n\nok").is_empty());
    }

    #[test]
    fn embedding_text_prefixes_heading_path() {
        let text = "# Top\n\nBody that is long enough to be kept as one chunk.\n";
        let chunks = chunk_note("e.md", text);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].embedding_text().starts_with("Top\n"));
    }

    #[test]
    fn oversized_code_is_cut_by_lines() {
        let line = "let value = compute_something(argument_one, argument_two);\n";
        let body = line.repeat(60); // about 3700 characters
        let text = format!("```\n{body}```\n");
        let chunks = chunk_note("code.md", &text);
        assert!(chunks.len() >= 2);
        assert_ranges_valid(&text, &chunks);
    }

    #[test]
    fn crlf_line_endings_are_handled() {
        let text = "---\r\ntitle: x\r\n---\r\n# H\r\n\r\nA paragraph written with CRLF endings, long enough.\r\n";
        let chunks = chunk_note("crlf.md", text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].heading_path, "H");
        assert!(!chunks[0].text.contains("title"));
        assert_ranges_valid(text, &chunks);
    }

    #[test]
    fn empty_input_gives_no_chunks() {
        assert!(chunk_note("e.md", "").is_empty());
    }
}
