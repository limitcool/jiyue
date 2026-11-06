//! Wikilink resolution and rewriting when notes are renamed or moved. Owned by W1-A.
//!
//! Obsidian-like link semantics, implemented from scratch. Paths are vault-relative with forward
//! slashes. Links inside fenced code blocks, inline code and HTML comments are never reported and
//! never rewritten.

use std::ops::Range;

use crate::fsops::ends_with_ignore_case;

/// A note known to the link resolver. `aliases` are the frontmatter aliases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteRef {
    /// Vault-relative path with forward slashes, including `.md` for notes.
    pub path: String,
    pub aliases: Vec<String>,
}

impl NoteRef {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            aliases: Vec::new(),
        }
    }

    pub fn with_aliases(path: impl Into<String>, aliases: &[&str]) -> Self {
        Self {
            path: path.into(),
            aliases: aliases.iter().map(|a| a.to_string()).collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// `[[target#anchor|alias]]`
    Wiki,
    /// `![[target#anchor|alias]]`
    Embed,
    /// `[text](path)`, or the image form `![alt](path)`. The range then starts at the `!`.
    MdLink,
}

/// One link found in a note's text. All offsets are byte offsets into the scanned text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkOccurrence {
    /// The whole link syntax, for example `[[a|b]]` or `![alt](x.png)`.
    pub range: Range<usize>,
    /// The raw target text only (no `#anchor`, no `|alias`, no surrounding spaces). This is the
    /// span that a rename rewrites.
    pub target_range: Range<usize>,
    pub kind: LinkKind,
    /// Wiki and embed: the trimmed target. Markdown: the percent-decoded path.
    pub target: String,
    /// Text after `#`, for example `Heading` or `^block-id`.
    pub anchor: Option<String>,
    /// Wiki and embed: text after `|`. Markdown: the link text.
    pub alias: Option<String>,
}

/// A replacement for one note's text, produced by [`plan_rename_updates`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEdit {
    /// The note's path after the rename is applied. Write the edit here once the rename is done.
    pub path: String,
    pub new_text: String,
    /// Number of links rewritten.
    pub replaced: usize,
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Resolves a wikilink target (as written, may include `#anchor` and `|alias`) to an index into
/// `notes`. Resolution order:
///
/// 1. exact vault-relative path, with or without `.md` (case-insensitive fallback);
/// 2. for targets with a folder part, a path suffix match (`a/x` matches `p/a/x.md`);
/// 3. basename match, case-insensitive, when the target has no folder part;
/// 4. frontmatter alias match.
///
/// Several candidates are broken by: same folder as `from_path` first, then the shortest path,
/// then alphabetical order. An empty target (`[[#Heading]]`) resolves to `from_path` itself.
pub fn resolve_link(notes: &[NoteRef], target: &str, from_path: &str) -> Option<usize> {
    let from = norm_rel(from_path);
    let t = clean_target(target);
    if t.is_empty() {
        return notes.iter().position(|n| n.path == from);
    }
    resolve_name(notes, &t, &from)
}

/// Resolves any occurrence returned by [`extract_links`]. Markdown links are relative to the
/// folder of `from_path` (`../x.md`, percent-decoded already); wiki and embed links use
/// [`resolve_link`] rules.
pub fn resolve_occurrence(
    notes: &[NoteRef],
    occ: &LinkOccurrence,
    from_path: &str,
) -> Option<usize> {
    match occ.kind {
        LinkKind::MdLink => resolve_md(notes, &occ.target, &norm_rel(from_path)),
        LinkKind::Wiki | LinkKind::Embed => resolve_link(notes, &occ.target, from_path),
    }
}

/// Links in `text` that do not resolve to any note in `notes`. Anchor-only links are skipped.
pub fn unresolved_links(notes: &[NoteRef], text: &str, from_path: &str) -> Vec<LinkOccurrence> {
    extract_links(text)
        .into_iter()
        .filter(|occ| !occ.target.is_empty())
        .filter(|occ| resolve_occurrence(notes, occ, from_path).is_none())
        .collect()
}

fn clean_target(raw: &str) -> String {
    let before_alias = raw.split('|').next().unwrap_or("");
    let before_anchor = before_alias.split('#').next().unwrap_or("");
    let t = before_anchor.trim().replace('\\', "/");
    if let Some(rest) = t.strip_prefix("./") {
        rest.to_string()
    } else {
        t
    }
}

fn resolve_name(notes: &[NoteRef], t: &str, from: &str) -> Option<usize> {
    // Targets without a folder part go straight to basename matching, so that a closer note wins
    // over a root-level note of the same name.
    let mut found = if t.contains('/') {
        exact_candidates(notes, t)
    } else {
        Vec::new()
    };
    if found.is_empty() && t.contains('/') {
        let suffix = format!("/{}", t.to_lowercase());
        let suffix_md = format!("{suffix}.md");
        found = candidates(notes, |p| {
            let lp = p.to_lowercase();
            lp.ends_with(&suffix) || lp.ends_with(&suffix_md)
        });
    }
    if found.is_empty() && !t.contains('/') {
        let key = strip_md(t).to_lowercase();
        found = candidates(notes, |p| strip_md(base_of(p)).to_lowercase() == key);
    }
    if found.is_empty() && !t.contains('/') {
        let key = t.to_lowercase();
        found = notes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.aliases.iter().any(|a| a.to_lowercase() == key))
            .map(|(i, _)| i)
            .collect();
    }
    pick(notes, &found, from)
}

fn resolve_md(notes: &[NoteRef], target: &str, from: &str) -> Option<usize> {
    if target.is_empty() {
        return None;
    }
    let joined = if target.starts_with('/') {
        normalize(target)?
    } else {
        normalize(&format!("{}/{}", dir_of(from), target))?
    };
    if joined.is_empty() {
        return None;
    }
    let found = exact_candidates(notes, &joined);
    pick(notes, &found, from)
}

/// Exact path matches (`t` or `t.md`), case-sensitive first, then case-insensitive.
fn exact_candidates(notes: &[NoteRef], t: &str) -> Vec<usize> {
    let with_md = format!("{t}.md");
    let exact = candidates(notes, |p| p == t || p == with_md);
    if !exact.is_empty() {
        return exact;
    }
    let lower = t.to_lowercase();
    let lower_md = format!("{lower}.md");
    candidates(notes, |p| {
        let lp = p.to_lowercase();
        lp == lower || lp == lower_md
    })
}

fn candidates(notes: &[NoteRef], pred: impl Fn(&str) -> bool) -> Vec<usize> {
    notes
        .iter()
        .enumerate()
        .filter(|(_, n)| pred(&n.path))
        .map(|(i, _)| i)
        .collect()
}

fn pick(notes: &[NoteRef], found: &[usize], from: &str) -> Option<usize> {
    let from_dir = dir_of(from);
    found
        .iter()
        .filter_map(|&i| {
            let p = notes.get(i)?.path.as_str();
            let key = (dir_of(p) != from_dir, p.chars().count(), p.to_string());
            Some((key, i))
        })
        .min()
        .map(|(_, i)| i)
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn norm_rel(s: &str) -> String {
    s.replace('\\', "/").trim_matches('/').to_string()
}

fn dir_of(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

fn base_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn has_md(s: &str) -> bool {
    ends_with_ignore_case(s, ".md")
}

fn strip_md(s: &str) -> &str {
    if has_md(s) { &s[..s.len() - 3] } else { s }
}

/// Collapses `.` and `..`. Returns `None` if the path climbs above the vault root.
fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Relative path from the folder `from_dir` to the file `to`, using `..` where needed.
fn relative_path(from_dir: &str, to: &str) -> String {
    let from: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let to_parts: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let Some((_, dirs)) = to_parts.split_last() else {
        return String::new();
    };
    let common = from
        .iter()
        .zip(dirs.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts: Vec<&str> = vec![".."; from.len() - common];
    parts.extend_from_slice(&to_parts[common..]);
    parts.join("/")
}

/// Percent-decodes `%XX` sequences. Invalid sequences are kept as written.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2]))
        {
            out.push(hi * 16 + lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn hex_value(b: u8) -> Option<u8> {
    char::from(b).to_digit(16).map(|d| d as u8)
}

/// Encodes the characters that break a Markdown link destination. Other text (CJK included) is
/// kept as is.
fn encode_md_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            ' ' => out.push_str("%20"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '%' => out.push_str("%25"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            other => out.push(other),
        }
    }
    out
}

fn is_external(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("://")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
        || lower.starts_with("data:")
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// Finds every wiki link, embed and Markdown link in `text`, skipping code and comments.
pub fn extract_links(text: &str) -> Vec<LinkOccurrence> {
    let masked = masked_text(text);
    let m = masked.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < m.len() {
        let open = if m[i] == b'!' { i + 1 } else { i };
        if m.get(open) != Some(&b'[') {
            i += 1;
            continue;
        }
        if m.get(open + 1) == Some(&b'[') {
            match parse_wiki(text, &masked, i, open) {
                Some((occ, end)) => {
                    out.push(occ);
                    i = end;
                }
                None => i = open + 2,
            }
        } else {
            match parse_md(text, &masked, i, open) {
                Some((occ, end)) => {
                    out.push(occ);
                    i = end;
                }
                None => i = open + 1,
            }
        }
    }
    out
}

fn split_once_or(s: &str, sep: char) -> (&str, Option<&str>) {
    match s.split_once(sep) {
        Some((head, tail)) => (head, Some(tail)),
        None => (s, None),
    }
}

/// Parses `[[...]]` starting at `start` (the `!` or the first `[`), with `open` pointing at the
/// first `[`. Returns the occurrence and the byte index just after it.
fn parse_wiki(
    text: &str,
    masked: &str,
    start: usize,
    open: usize,
) -> Option<(LinkOccurrence, usize)> {
    let body_start = open + 2;
    let m = masked.as_bytes();
    let mut k = body_start;
    let mut close = None;
    while k < m.len() {
        let c = m[k];
        if c == b'\n' || c == b'[' {
            return None;
        }
        if c == b']' {
            if m.get(k + 1) == Some(&b']') {
                close = Some(k);
            }
            break;
        }
        k += 1;
    }
    let body_end = close?;
    let end = body_end + 2;
    let body = text.get(body_start..body_end)?;
    // Skip bodies that overlap masked code: the masked copy must equal the original text.
    if masked.get(body_start..body_end)? != body || body.trim().is_empty() {
        return None;
    }
    let (left, alias) = split_once_or(body, '|');
    let (target_part, anchor) = split_once_or(left, '#');
    let target = target_part.trim();
    let lead = target_part.len() - target_part.trim_start().len();
    let t_start = body_start + lead;
    let occ = LinkOccurrence {
        range: start..end,
        target_range: t_start..t_start + target.len(),
        kind: if open > start {
            LinkKind::Embed
        } else {
            LinkKind::Wiki
        },
        target: target.to_string(),
        anchor: anchor
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_string),
        alias: alias
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_string),
    };
    Some((occ, end))
}

/// Parses `[text](url)` with `open` pointing at `[`. Returns the occurrence and the end index.
fn parse_md(
    text: &str,
    masked: &str,
    start: usize,
    open: usize,
) -> Option<(LinkOccurrence, usize)> {
    let m = masked.as_bytes();
    let text_close = find_on_line(m, open + 1, b']')?;
    if m.get(text_close + 1) != Some(&b'(') {
        return None;
    }
    let url_open = text_close + 2;
    let (raw_start, raw_end, end) = if m.get(url_open) == Some(&b'<') {
        let gt = find_on_line(m, url_open + 1, b'>')?;
        let paren = find_on_line(m, gt + 1, b')')?;
        (url_open + 1, gt, paren + 1)
    } else {
        let paren = find_on_line(m, url_open, b')')?;
        (url_open, paren, paren + 1)
    };
    let raw = text.get(raw_start..raw_end)?;
    if masked.get(raw_start..raw_end)? != raw {
        return None;
    }
    // Optional title: `url "title"` or `url 'title'`.
    let cut = [raw.find(" \""), raw.find(" '")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(raw.len());
    let url_part = &raw[..cut];
    let lead = url_part.len() - url_part.trim_start().len();
    let url = url_part.trim();
    if url.is_empty() || url.starts_with('#') || is_external(url) {
        return None;
    }
    let t_start = raw_start + lead;
    let hash = url.find('#');
    let path_part = match hash {
        Some(h) => url[..h].trim_end(),
        None => url,
    };
    let anchor = hash
        .map(|h| url[h + 1..].trim())
        .filter(|a| !a.is_empty())
        .map(str::to_string);
    let link_text = text.get(open + 1..text_close)?.trim();
    let occ = LinkOccurrence {
        range: start..end,
        target_range: t_start..t_start + path_part.len(),
        kind: LinkKind::MdLink,
        target: percent_decode(path_part),
        anchor,
        alias: (!link_text.is_empty()).then(|| link_text.to_string()),
    };
    Some((occ, end))
}

fn find_on_line(m: &[u8], from: usize, byte: u8) -> Option<usize> {
    for (k, &c) in m.iter().enumerate().skip(from) {
        match c {
            b'\n' => return None,
            c if c == byte => return Some(k),
            _ => {}
        }
    }
    None
}

/// Copy of `text` with every code and comment byte (except newlines) replaced by a space. The
/// copy has the same length, so offsets carry over.
fn masked_text(text: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    mask_fences(text, &mut bytes);
    mask_inline_code(&mut bytes);
    mask_html_comments(&mut bytes);
    // Only ASCII spaces are written, and masked ranges start and end on char boundaries, so the
    // result stays valid UTF-8.
    String::from_utf8(bytes).unwrap_or_else(|_| text.to_string())
}

fn mask(bytes: &mut [u8], start: usize, end: usize) {
    for b in &mut bytes[start..end] {
        if *b != b'\n' {
            *b = b' ';
        }
    }
}

/// Returns `(fence char, run length, rest of line is blank)` for a line that starts a fence.
fn fence_marker(line: &str) -> Option<(u8, usize, bool)> {
    let ch = *line.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let run = line.bytes().take_while(|&b| b == ch).count();
    if run < 3 {
        return None;
    }
    let rest_blank = line[run..].trim().is_empty();
    Some((ch, run, rest_blank))
}

fn mask_fences(text: &str, bytes: &mut [u8]) {
    let mut open: Option<(u8, usize)> = None;
    let mut pos = 0;
    for line in text.split_inclusive('\n') {
        let start = pos;
        let end = pos + line.len();
        pos = end;
        let indent = line.len() - line.trim_start_matches(' ').len();
        let body = &line[indent..];
        match open {
            None => {
                if indent <= 3
                    && let Some((ch, run, _)) = fence_marker(body)
                {
                    open = Some((ch, run));
                    mask(bytes, start, end);
                }
            }
            Some((ch, run)) => {
                mask(bytes, start, end);
                if indent <= 3
                    && let Some((c2, n2, true)) = fence_marker(body)
                    && c2 == ch
                    && n2 >= run
                {
                    open = None;
                }
            }
        }
    }
}

fn run_len(bytes: &[u8], at: usize) -> usize {
    bytes[at..].iter().take_while(|&&b| b == b'`').count()
}

fn mask_inline_code(bytes: &mut [u8]) {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let run = run_len(bytes, i);
        let mut j = i + run;
        let mut close = None;
        while j < bytes.len() {
            if bytes[j] == b'`' {
                let r = run_len(bytes, j);
                if r == run {
                    close = Some(j + r);
                    break;
                }
                j += r;
            } else {
                j += 1;
            }
        }
        match close {
            Some(end) => {
                mask(bytes, i, end);
                i = end;
            }
            None => i += run,
        }
    }
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn mask_html_comments(bytes: &mut [u8]) {
    let mut i = 0;
    while let Some(off) = find_bytes(&bytes[i..], b"<!--") {
        let start = i + off;
        let after = start + 4;
        let end = match find_bytes(&bytes[after..], b"-->") {
            Some(o) => after + o + 3,
            None => bytes.len(),
        };
        mask(bytes, start, end);
        i = end;
    }
}

// ---------------------------------------------------------------------------
// Rename planning
// ---------------------------------------------------------------------------

/// Maps a path through a rename `old` -> `new` (a file, or a folder and everything under it).
fn map_path(path: &str, old: &str, new: &str) -> Option<String> {
    if path == old {
        return Some(new.to_string());
    }
    if !has_md(old) && path == format!("{old}.md") {
        return Some(if has_md(new) {
            new.to_string()
        } else {
            format!("{new}.md")
        });
    }
    let prefix = format!("{old}/");
    path.strip_prefix(prefix.as_str())
        .map(|rest| format!("{new}/{rest}"))
}

/// Plans the link updates needed when `old_rel` is renamed or moved to `new_rel`.
///
/// `all_notes` is the vault listing (paths and aliases) and must describe the state before the
/// rename, with the old paths. Entries may be any file the caller wants links to resolve to,
/// for example attachments. `notes_text` yields `(path, text)` for every note to scan; paths are
/// the pre-rename paths. The returned edits use the post-rename path in [`FileEdit::path`]. Apply
/// them after performing the rename.
///
/// Rewrites every link whose target moved (or whose resolution changed), and every relative
/// Markdown link in a note that moved. Wiki links keep their style: basename-only stays basename
/// when the new basename is unique in the vault, otherwise the path is written from the vault
/// root. Anchors, aliases, block references, embed syntax and `.md` suffixes are kept. Calling it
/// again on its own output finds nothing left to change.
pub fn plan_rename_updates(
    notes_text: impl Iterator<Item = (String, String)>,
    old_rel: &str,
    new_rel: &str,
    all_notes: &[NoteRef],
) -> Vec<FileEdit> {
    let old = norm_rel(old_rel);
    let new = norm_rel(new_rel);
    if old.is_empty() || new.is_empty() || old == new {
        return Vec::new();
    }
    let docs: Vec<(String, String)> = notes_text.collect();
    let mapped: Vec<String> = all_notes
        .iter()
        .map(|n| map_path(&n.path, &old, &new).unwrap_or_else(|| n.path.clone()))
        .collect();
    let any_doc_moves = docs
        .iter()
        .any(|(p, _)| map_path(&norm_rel(p), &old, &new).is_some());
    let any_target_moves = mapped.iter().zip(all_notes).any(|(m, n)| *m != n.path);
    if !any_doc_moves && !any_target_moves {
        return Vec::new();
    }
    let new_list: Vec<NoteRef> = all_notes
        .iter()
        .zip(&mapped)
        .map(|(n, p)| NoteRef {
            path: p.clone(),
            aliases: n.aliases.clone(),
        })
        .collect();

    let mut edits = Vec::new();
    for (raw_path, text) in docs {
        let path = norm_rel(&raw_path);
        let new_from = map_path(&path, &old, &new).unwrap_or_else(|| path.clone());
        let mut reps: Vec<(Range<usize>, String)> = Vec::new();
        for occ in extract_links(&text) {
            if let Some(rep) = rewrite_for(&occ, &text, &path, &new_from, all_notes, &new_list) {
                reps.push((occ.target_range.clone(), rep));
            }
        }
        if reps.is_empty() {
            continue;
        }
        let replaced = reps.len();
        let new_text = apply_reps(&text, reps);
        if new_text != text {
            edits.push(FileEdit {
                path: new_from,
                new_text,
                replaced,
            });
        }
    }
    edits
}

/// The replacement text for one link's target, or `None` when the link needs no change.
fn rewrite_for(
    occ: &LinkOccurrence,
    text: &str,
    from_old: &str,
    from_new: &str,
    all: &[NoteRef],
    new_list: &[NoteRef],
) -> Option<String> {
    let before = resolve_occurrence(all, occ, from_old)?;
    let old_target = all.get(before)?.path.as_str();
    let new_target = new_list.get(before)?.path.as_str();
    let target_moved = old_target != new_target;
    let link_moved = from_old != from_new;
    let raw = text.get(occ.target_range.clone())?;
    match occ.kind {
        LinkKind::MdLink => {
            if !target_moved && !link_moved {
                return None;
            }
            let rel = if occ.target.starts_with('/') {
                format!("/{new_target}")
            } else {
                let rel = relative_path(dir_of(from_new), new_target);
                if has_md(&occ.target) {
                    rel
                } else {
                    strip_md(&rel).to_string()
                }
            };
            let prefix =
                if raw.starts_with("./") && !rel.starts_with("../") && !rel.starts_with('/') {
                    "./"
                } else {
                    ""
                };
            let out = format!("{prefix}{}", encode_md_path(&rel));
            (out != raw).then_some(out)
        }
        LinkKind::Wiki | LinkKind::Embed => {
            let keep_md = has_md(&occ.target);
            let full = if keep_md {
                new_target.to_string()
            } else {
                strip_md(new_target).to_string()
            };
            if target_moved {
                let basename_style = !occ.target.contains('/');
                let base = if keep_md {
                    base_of(new_target).to_string()
                } else {
                    strip_md(base_of(new_target)).to_string()
                };
                let unique = basename_is_unique(new_list, base_of(new_target));
                let chosen = if basename_style && unique { base } else { full };
                return (chosen != raw).then_some(chosen);
            }
            // The target did not move. Only a change in resolution needs a rewrite.
            if resolve_occurrence(new_list, occ, from_new) == Some(before) {
                return None;
            }
            (full != raw).then_some(full)
        }
    }
}

fn basename_is_unique(list: &[NoteRef], file_name: &str) -> bool {
    let key = strip_md(file_name).to_lowercase();
    list.iter()
        .filter(|n| strip_md(base_of(&n.path)).to_lowercase() == key)
        .count()
        == 1
}

/// Applies non-overlapping replacements; `reps` may come in any order.
fn apply_reps(text: &str, mut reps: Vec<(Range<usize>, String)>) -> String {
    reps.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (range, rep) in reps {
        if range.start < last
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            continue;
        }
        out.push_str(&text[last..range.start]);
        out.push_str(&rep);
        last = range.end;
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes(paths: &[&str]) -> Vec<NoteRef> {
        paths.iter().map(|p| NoteRef::new(*p)).collect()
    }

    fn plan(docs: &[(&str, &str)], old: &str, new: &str, all: &[NoteRef]) -> Vec<FileEdit> {
        plan_rename_updates(
            docs.iter().map(|(p, t)| (p.to_string(), t.to_string())),
            old,
            new,
            all,
        )
    }

    fn edit_text(edits: &[FileEdit], path: &str) -> Option<String> {
        edits
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.new_text.clone())
    }

    fn occ_text<'a>(text: &'a str, occ: &LinkOccurrence) -> &'a str {
        &text[occ.target_range.clone()]
    }

    // ---- resolve_link ----------------------------------------------------

    #[test]
    fn resolve_exact_path_with_and_without_md() {
        let all = notes(&["a/x.md", "b/x.md"]);
        assert_eq!(resolve_link(&all, "a/x", ""), Some(0));
        assert_eq!(resolve_link(&all, "b/x.md", ""), Some(1));
    }

    #[test]
    fn resolve_strips_anchor_and_alias() {
        let all = notes(&["a/x.md"]);
        assert_eq!(resolve_link(&all, "x#Heading|shown", "z.md"), Some(0));
        assert_eq!(resolve_link(&all, "a/x#^block", "z.md"), Some(0));
        assert_eq!(resolve_link(&all, " x | alias ", "z.md"), Some(0));
    }

    #[test]
    fn resolve_basename_is_case_insensitive() {
        let all = notes(&["Notes/Idea.md"]);
        assert_eq!(resolve_link(&all, "idea", "z.md"), Some(0));
        assert_eq!(resolve_link(&all, "IDEA.MD", "z.md"), Some(0));
    }

    #[test]
    fn resolve_cjk_basename() {
        let all = notes(&["想法/沉积的想法.md"]);
        assert_eq!(resolve_link(&all, "沉积的想法", "日记/今天.md"), Some(0));
        assert_eq!(
            resolve_link(&all, "沉积的想法#标题", "日记/今天.md"),
            Some(0)
        );
    }

    #[test]
    fn resolve_prefers_same_folder() {
        let all = notes(&["a/x.md", "b/x.md"]);
        assert_eq!(resolve_link(&all, "x", "b/from.md"), Some(1));
        assert_eq!(resolve_link(&all, "x", "a/from.md"), Some(0));
    }

    #[test]
    fn resolve_prefers_shortest_path_then_alphabetical() {
        let all = notes(&["deep/er/x.md", "x.md"]);
        assert_eq!(resolve_link(&all, "x", "z/q.md"), Some(1));
        let all = notes(&["b/x.md", "a/x.md"]);
        assert_eq!(resolve_link(&all, "x", "z/q.md"), Some(1));
    }

    #[test]
    fn resolve_same_folder_beats_shorter_path() {
        let all = notes(&["x.md", "deep/folder/x.md"]);
        assert_eq!(resolve_link(&all, "x", "deep/folder/here.md"), Some(1));
    }

    #[test]
    fn resolve_title_beats_alias() {
        let all = vec![
            NoteRef::with_aliases("a.md", &["foo"]),
            NoteRef::new("foo.md"),
        ];
        assert_eq!(resolve_link(&all, "foo", "z.md"), Some(1));
    }

    #[test]
    fn resolve_alias_when_no_title_matches() {
        let all = vec![NoteRef::with_aliases("a.md", &["Bar Baz"])];
        assert_eq!(resolve_link(&all, "bar baz", "z.md"), Some(0));
    }

    #[test]
    fn resolve_path_suffix_match() {
        let all = notes(&["projects/a/readme.md", "x/readme.md"]);
        assert_eq!(resolve_link(&all, "a/readme", "z.md"), Some(0));
    }

    #[test]
    fn resolve_folder_target_does_not_fall_back_to_basename() {
        let all = notes(&["elsewhere/x.md"]);
        assert_eq!(resolve_link(&all, "missing/x", "z.md"), None);
    }

    #[test]
    fn resolve_missing_is_none() {
        let all = notes(&["a.md"]);
        assert_eq!(resolve_link(&all, "nope", "a.md"), None);
        assert_eq!(resolve_link(&[], "anything", "a.md"), None);
    }

    #[test]
    fn resolve_empty_target_means_same_note() {
        let all = notes(&["a.md", "b.md"]);
        assert_eq!(resolve_link(&all, "#Heading", "b.md"), Some(1));
        assert_eq!(resolve_link(&all, "", "a.md"), Some(0));
    }

    #[test]
    fn resolve_dot_slash_prefix_is_ignored() {
        let all = notes(&["sub/x.md"]);
        assert_eq!(resolve_link(&all, "./sub/x", "z.md"), Some(0));
    }

    #[test]
    fn resolve_non_note_attachment_by_full_name() {
        let all = notes(&["附件/图.png"]);
        assert_eq!(resolve_link(&all, "图.png", "z.md"), Some(0));
        assert_eq!(resolve_link(&all, "图", "z.md"), None);
    }

    #[test]
    fn resolve_markdown_link_relative_to_folder() {
        let all = notes(&["a/x.md", "d/doc.md"]);
        let occ = extract_links("[t](../a/x.md)").remove(0);
        assert_eq!(resolve_occurrence(&all, &occ, "d/doc.md"), Some(0));
        let occ = extract_links("[t](./x.md)").remove(0);
        assert_eq!(resolve_occurrence(&all, &occ, "a/doc.md"), Some(0));
    }

    #[test]
    fn resolve_markdown_link_climbing_above_root_is_none() {
        let all = notes(&["a.md"]);
        let occ = extract_links("[t](../../a.md)").remove(0);
        assert_eq!(resolve_occurrence(&all, &occ, "d/doc.md"), None);
    }

    #[test]
    fn resolve_markdown_link_percent_encoded_space() {
        let all = notes(&["old name.md"]);
        let occ = extract_links("[t](old%20name.md)").remove(0);
        assert_eq!(resolve_occurrence(&all, &occ, "x.md"), Some(0));
    }

    #[test]
    fn unresolved_links_lists_only_broken_ones() {
        let all = notes(&["a.md"]);
        let text = "[[a]] [[missing]] [x](nope.md) [[#self]] [y](https://e.com)";
        let broken = unresolved_links(&all, text, "a.md");
        let targets: Vec<&str> = broken.iter().map(|o| o.target.as_str()).collect();
        assert_eq!(targets, vec!["missing", "nope.md"]);
    }

    // ---- extract_links ---------------------------------------------------

    #[test]
    fn extract_simple_wiki_link() {
        let text = "see [[Note]] end";
        let occs = extract_links(text);
        assert_eq!(occs.len(), 1);
        let occ = &occs[0];
        assert_eq!(occ.kind, LinkKind::Wiki);
        assert_eq!(occ.target, "Note");
        assert_eq!(&text[occ.range.clone()], "[[Note]]");
        assert_eq!(occ_text(text, occ), "Note");
        assert_eq!(occ.anchor, None);
        assert_eq!(occ.alias, None);
    }

    #[test]
    fn extract_alias_and_anchor() {
        let text = "[[a#h|al]]";
        let occ = extract_links(text).remove(0);
        assert_eq!(occ.target, "a");
        assert_eq!(occ.anchor.as_deref(), Some("h"));
        assert_eq!(occ.alias.as_deref(), Some("al"));
        assert_eq!(occ_text(text, &occ), "a");
    }

    #[test]
    fn extract_block_reference() {
        let text = "[[a#^id|x]]";
        let occ = extract_links(text).remove(0);
        assert_eq!(occ.anchor.as_deref(), Some("^id"));
        assert_eq!(occ.alias.as_deref(), Some("x"));
    }

    #[test]
    fn extract_anchor_only_has_empty_target() {
        let text = "[[#Heading]]";
        let occ = extract_links(text).remove(0);
        assert_eq!(occ.target, "");
        assert_eq!(occ.anchor.as_deref(), Some("Heading"));
        assert_eq!(occ.target_range, 2..2);
    }

    #[test]
    fn extract_embed_with_size_alias() {
        let text = "![[图.png|300]]";
        let occ = extract_links(text).remove(0);
        assert_eq!(occ.kind, LinkKind::Embed);
        assert_eq!(occ.range, 0..text.len());
        assert_eq!(occ.target, "图.png");
        assert_eq!(occ.alias.as_deref(), Some("300"));
        assert_eq!(occ_text(text, &occ), "图.png");
    }

    #[test]
    fn extract_trims_spaces_and_keeps_target_range_exact() {
        let text = "[[  spaced name  |a]]";
        let occ = extract_links(text).remove(0);
        assert_eq!(occ.target, "spaced name");
        assert_eq!(occ_text(text, &occ), "spaced name");
    }

    #[test]
    fn extract_markdown_link_decodes_target() {
        let text = "[t](old%20name.md)";
        let occ = extract_links(text).remove(0);
        assert_eq!(occ.kind, LinkKind::MdLink);
        assert_eq!(occ.target, "old name.md");
        assert_eq!(occ_text(text, &occ), "old%20name.md");
        assert_eq!(occ.alias.as_deref(), Some("t"));
        assert_eq!(occ.range, 0..text.len());
    }

    #[test]
    fn extract_markdown_link_with_title_and_anchor() {
        let text = r#"[t](a.md "Title") [u](b.md#Head)"#;
        let occs = extract_links(text);
        assert_eq!(occs.len(), 2);
        assert_eq!(occs[0].target, "a.md");
        assert_eq!(occ_text(text, &occs[0]), "a.md");
        assert_eq!(occs[1].target, "b.md");
        assert_eq!(occs[1].anchor.as_deref(), Some("Head"));
        assert_eq!(occ_text(text, &occs[1]), "b.md");
    }

    #[test]
    fn extract_markdown_angle_bracket_destination() {
        let text = "[t](<a b.md>)";
        let occ = extract_links(text).remove(0);
        assert_eq!(occ.target, "a b.md");
        assert_eq!(occ_text(text, &occ), "a b.md");
    }

    #[test]
    fn extract_markdown_image_starts_range_at_bang() {
        let text = "x ![alt](img.png) y";
        let occ = extract_links(text).remove(0);
        assert_eq!(occ.kind, LinkKind::MdLink);
        assert_eq!(&text[occ.range.clone()], "![alt](img.png)");
        assert_eq!(occ.target, "img.png");
    }

    #[test]
    fn extract_skips_external_and_same_note_links() {
        let text = "[x](https://e.com/a.md) [m](mailto:a@b.c) [h](#top) [t](tel:123) [d](data:x)";
        assert!(extract_links(text).is_empty());
    }

    #[test]
    fn extract_ignores_fenced_backtick_block() {
        let text = "```\n[[a]]\n```\n[[b]]";
        let occs = extract_links(text);
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].target, "b");
    }

    #[test]
    fn extract_ignores_fenced_tilde_block() {
        let text = "~~~md\n[[a]] [x](y.md)\n~~~\n[[b]]";
        let occs = extract_links(text);
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].target, "b");
    }

    #[test]
    fn extract_fence_closes_only_with_same_char_and_length() {
        let text = "````\n[[a]]\n```\n[[b]]\n````\n[[c]]";
        let occs = extract_links(text);
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].target, "c");
    }

    #[test]
    fn extract_unclosed_fence_runs_to_end() {
        let text = "[[a]]\n```\n[[b]]\n[[c]]";
        let occs = extract_links(text);
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].target, "a");
    }

    #[test]
    fn extract_indented_four_spaces_is_not_a_fence() {
        let text = "    ```\n[[a]]";
        let occs = extract_links(text);
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].target, "a");
    }

    #[test]
    fn extract_ignores_inline_code() {
        let text = "`[[a]]` and [[b]]";
        let occs = extract_links(text);
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].target, "b");
    }

    #[test]
    fn extract_ignores_double_backtick_inline_code() {
        let text = "``[[a]] ` still code ``[[b]] [[c]]";
        let occs = extract_links(text);
        let targets: Vec<&str> = occs.iter().map(|o| o.target.as_str()).collect();
        assert_eq!(targets, vec!["b", "c"]);
    }

    #[test]
    fn extract_unmatched_backtick_is_literal() {
        let text = "a ` b [[x]]";
        let occs = extract_links(text);
        assert_eq!(occs.len(), 1);
    }

    #[test]
    fn extract_ignores_html_comments() {
        let text = "<!-- [[a]] --> [[b]]";
        let occs = extract_links(text);
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].target, "b");
    }

    #[test]
    fn extract_ignores_multiline_html_comment() {
        let text = "<!--\n[[a]]\n[x](y.md)\n-->\n[[b]]";
        let occs = extract_links(text);
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].target, "b");
    }

    #[test]
    fn extract_unclosed_html_comment_runs_to_end() {
        let text = "[[a]] <!-- [[b]]";
        let occs = extract_links(text);
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].target, "a");
    }

    #[test]
    fn extract_ignores_task_checkbox_brackets() {
        let text = "- [ ] task [[a]]\n- [x] done";
        let occs = extract_links(text);
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].target, "a");
    }

    #[test]
    fn extract_rejects_empty_unclosed_and_multiline_wiki() {
        for bad in ["[[]]", "[[   ]]", "[[a", "[[a\nb]]", "[[a]b]]", "[[x]"] {
            assert!(extract_links(bad).is_empty(), "{bad:?} should not match");
        }
    }

    #[test]
    fn extract_handles_cjk_offsets() {
        let text = "中文[[笔记]]文字 ![[图 片.png]] 结尾";
        let occs = extract_links(text);
        assert_eq!(occs.len(), 2);
        assert_eq!(&text[occs[0].range.clone()], "[[笔记]]");
        assert_eq!(&text[occs[1].range.clone()], "![[图 片.png]]");
        assert_eq!(occ_text(text, &occs[1]), "图 片.png");
    }

    #[test]
    fn extract_many_links_in_order() {
        let text = "[[a]] [[b|x]] [c](c.md) ![[d]]";
        let targets: Vec<String> = extract_links(text).into_iter().map(|o| o.target).collect();
        assert_eq!(targets, vec!["a", "b", "c.md", "d"]);
    }

    #[test]
    fn extract_empty_text_has_no_links() {
        assert!(extract_links("").is_empty());
        assert!(extract_links("no links [here] (really)").is_empty());
    }

    // ---- plan_rename_updates --------------------------------------------

    #[test]
    fn plan_basename_link_stays_basename_when_unique() {
        let all = notes(&["想法/沉积的想法.md", "日记/2026.md"]);
        let docs = [(
            "日记/2026.md",
            "见 [[沉积的想法]] 与 [[沉积的想法|别名]] 和 [[沉积的想法#标题]]",
        )];
        let edits = plan(&docs, "想法/沉积的想法.md", "想法/新名字.md", &all);
        assert_eq!(
            edit_text(&edits, "日记/2026.md").as_deref(),
            Some("见 [[新名字]] 与 [[新名字|别名]] 和 [[新名字#标题]]")
        );
        assert_eq!(edits[0].replaced, 3);
    }

    #[test]
    fn plan_keeps_block_reference_and_alias() {
        let all = notes(&["a.md", "z.md"]);
        let docs = [("z.md", "[[a#^blk|shown]]")];
        let edits = plan(&docs, "a.md", "b.md", &all);
        assert_eq!(
            edit_text(&edits, "z.md").as_deref(),
            Some("[[b#^blk|shown]]")
        );
    }

    #[test]
    fn plan_rewrites_embed_and_keeps_size() {
        let all = notes(&["沉积.md", "z.md"]);
        let docs = [("z.md", "![[沉积#^x]] ![[沉积|300]]")];
        let edits = plan(&docs, "沉积.md", "新.md", &all);
        assert_eq!(
            edit_text(&edits, "z.md").as_deref(),
            Some("![[新#^x]] ![[新|300]]")
        );
    }

    #[test]
    fn plan_keeps_md_suffix_style() {
        let all = notes(&["a.md", "z.md"]);
        let docs = [("z.md", "[[a.md]] [[a]]")];
        let edits = plan(&docs, "a.md", "b.md", &all);
        assert_eq!(edit_text(&edits, "z.md").as_deref(), Some("[[b.md]] [[b]]"));
    }

    #[test]
    fn plan_ambiguous_new_basename_becomes_path_qualified() {
        // Renaming a/x.md to b/x.md while c/x.md exists: the basename would be ambiguous.
        let all = notes(&["a/x.md", "c/x.md", "d/doc.md"]);
        let docs = [("d/doc.md", "[[x]] and [[a/x]]")];
        let edits = plan(&docs, "a/x.md", "b/x.md", &all);
        assert_eq!(
            edit_text(&edits, "d/doc.md").as_deref(),
            Some("[[b/x]] and [[b/x]]")
        );
    }

    #[test]
    fn plan_path_qualified_link_stays_path_qualified() {
        let all = notes(&["a/x.md", "d/doc.md"]);
        let docs = [("d/doc.md", "[[a/x]]")];
        let edits = plan(&docs, "a/x.md", "q/y.md", &all);
        assert_eq!(edit_text(&edits, "d/doc.md").as_deref(), Some("[[q/y]]"));
    }

    #[test]
    fn plan_rename_to_new_folder_keeps_unique_basename_when_possible() {
        let all = notes(&["a/x.md", "d/doc.md"]);
        let docs = [("d/doc.md", "[[x]]")];
        // Basename-only link to a uniquely named note: the basename still resolves, so the text
        // is kept.
        let edits = plan(&docs, "a/x.md", "b/x.md", &all);
        assert!(edits.is_empty());
    }

    #[test]
    fn plan_markdown_link_rewritten_with_relative_path_and_encoding() {
        let all = notes(&["a/x.md", "d/doc.md"]);
        let docs = [("d/doc.md", "[t](../a/x.md)")];
        let edits = plan(&docs, "a/x.md", "a/sub/New Name.md", &all);
        assert_eq!(
            edit_text(&edits, "d/doc.md").as_deref(),
            Some("[t](../a/sub/New%20Name.md)")
        );
    }

    #[test]
    fn plan_markdown_link_with_spaces_and_cjk() {
        let all = notes(&["old name.md", "x.md"]);
        let docs = [("x.md", "[t](old%20name.md#Head)")];
        let edits = plan(&docs, "old name.md", "新 名字.md", &all);
        assert_eq!(
            edit_text(&edits, "x.md").as_deref(),
            Some("[t](新%20名字.md#Head)")
        );
    }

    #[test]
    fn plan_markdown_link_keeps_missing_md_suffix_style() {
        let all = notes(&["a.md", "z.md"]);
        let docs = [("z.md", "[t](a)")];
        let edits = plan(&docs, "a.md", "folder/b.md", &all);
        assert_eq!(edit_text(&edits, "z.md").as_deref(), Some("[t](folder/b)"));
    }

    #[test]
    fn plan_markdown_link_in_moved_note_is_relative_to_new_folder() {
        let all = notes(&["a/x.md", "d/doc.md"]);
        let docs = [("d/doc.md", "[t](../a/x.md)")];
        // Moving the linking note from d/ to e/f/ must change the relative path.
        let edits = plan(&docs, "d", "e/f", &all);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].path, "e/f/doc.md");
        assert_eq!(edits[0].new_text, "[t](../../a/x.md)");
    }

    #[test]
    fn plan_folder_rename_updates_wiki_and_markdown_links() {
        let all = notes(&["a/x.md", "a/sub/y.md", "d/doc.md"]);
        let docs = [("d/doc.md", "[[a/x]] and [y](../a/sub/y.md)")];
        let edits = plan(&docs, "a", "b", &all);
        assert_eq!(
            edit_text(&edits, "d/doc.md").as_deref(),
            Some("[[b/x]] and [y](../b/sub/y.md)")
        );
    }

    #[test]
    fn plan_folder_rename_keeps_unique_basename_wiki_links() {
        let all = notes(&["a/x.md", "d/doc.md"]);
        let docs = [("d/doc.md", "[[x]]")];
        assert!(plan(&docs, "a", "b", &all).is_empty());
    }

    #[test]
    fn plan_folder_rename_moves_linking_note_too() {
        let all = notes(&["a/x.md", "a/doc.md"]);
        let docs = [("a/doc.md", "[t](x.md)")];
        let edits = plan(&docs, "a", "b", &all);
        // Both notes are inside the folder; the relative link stays valid, so no edit.
        assert!(edits.is_empty());
    }

    #[test]
    fn plan_renamed_note_own_relative_links_are_updated() {
        let all = notes(&["a/x.md", "a/doc.md"]);
        let docs = [("a/doc.md", "[t](x.md)")];
        // doc.md moves to top level; its link to x.md must become a/x.md.
        let edits = plan(&docs, "a/doc.md", "doc.md", &all);
        assert_eq!(edit_text(&edits, "doc.md").as_deref(), Some("[t](a/x.md)"));
    }

    #[test]
    fn plan_never_rewrites_inside_code() {
        let all = notes(&["a.md", "z.md"]);
        let text = "```\n[[a]]\n```\n`[[a]]` <!-- [[a]] -->\n[[a]]";
        let docs = [("z.md", text)];
        let edits = plan(&docs, "a.md", "b.md", &all);
        assert_eq!(edits.len(), 1);
        assert_eq!(
            edits[0].new_text,
            "```\n[[a]]\n```\n`[[a]]` <!-- [[a]] -->\n[[b]]"
        );
        assert_eq!(edits[0].replaced, 1);
    }

    #[test]
    fn plan_is_idempotent_on_its_own_output() {
        let all = notes(&["a/x.md", "c/x.md", "d/doc.md"]);
        let docs = [("d/doc.md", "[[a/x]] [t](../a/x.md) [[x]]")];
        let first = plan(&docs, "a/x.md", "b/x.md", &all);
        assert_eq!(first.len(), 1);
        // Pre-rename listing: nothing resolves to the old path any more.
        let second = plan(
            &[("b/doc.md", first[0].new_text.as_str())],
            "a/x.md",
            "b/x.md",
            &all,
        );
        assert!(second.is_empty());
        // Post-rename listing: the old path is gone, so nothing matches either.
        let post = notes(&["b/x.md", "c/x.md", "d/doc.md"]);
        let third = plan(
            &[("d/doc.md", first[0].new_text.as_str())],
            "a/x.md",
            "b/x.md",
            &post,
        );
        assert!(third.is_empty());
    }

    #[test]
    fn plan_rewrite_then_rescan_has_no_links_to_old_name() {
        let all = notes(&["old.md", "z.md"]);
        let docs = [("z.md", "[[old]] [[old|a]] [o](old.md)")];
        let first = plan(&docs, "old.md", "new.md", &all);
        let text = first[0].new_text.clone();
        let remaining: Vec<LinkOccurrence> = extract_links(&text)
            .into_iter()
            .filter(|o| o.target.contains("old"))
            .collect();
        assert!(remaining.is_empty(), "{text}");
    }

    #[test]
    fn plan_no_change_when_nothing_renamed() {
        let all = notes(&["a.md", "z.md"]);
        let docs = [("z.md", "[[a]]")];
        assert!(plan(&docs, "unrelated.md", "other.md", &all).is_empty());
        assert!(plan(&docs, "a.md", "a.md", &all).is_empty());
        assert!(plan(&docs, "", "x.md", &all).is_empty());
    }

    #[test]
    fn plan_skips_notes_without_relevant_links() {
        let all = notes(&["a.md", "z.md"]);
        let docs = [("z.md", "nothing here"), ("q.md", "[[a]]")];
        let edits = plan(&docs, "a.md", "b.md", &all);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].path, "q.md");
    }

    #[test]
    fn plan_case_only_rename_updates_links() {
        let all = notes(&["note.md", "z.md"]);
        let docs = [("z.md", "[[note]] [[note.md]]")];
        let edits = plan(&docs, "note.md", "Note.md", &all);
        assert_eq!(
            edit_text(&edits, "z.md").as_deref(),
            Some("[[Note]] [[Note.md]]")
        );
    }

    #[test]
    fn plan_cjk_folder_rename() {
        let all = notes(&["想法/沉积的想法.md", "日记/今天.md"]);
        let docs = [(
            "日记/今天.md",
            "[[想法/沉积的想法]] [x](../想法/沉积的想法.md)",
        )];
        let edits = plan(&docs, "想法", "灵感", &all);
        assert_eq!(
            edit_text(&edits, "日记/今天.md").as_deref(),
            Some("[[灵感/沉积的想法]] [x](../灵感/沉积的想法.md)")
        );
    }

    #[test]
    fn plan_attachment_rename_via_plain_file_entries() {
        let all = notes(&["附件/图.png", "z.md"]);
        let docs = [("z.md", "![[图.png|300]] ![t](附件/图.png)")];
        let edits = plan(&docs, "附件/图.png", "资源/新图.png", &all);
        // Basename embed stays basename (the new name is unique); the md link keeps its folder.
        assert_eq!(
            edit_text(&edits, "z.md").as_deref(),
            Some("![[新图.png|300]] ![t](资源/新图.png)")
        );
    }

    #[test]
    fn plan_reports_post_rename_path_for_moved_note() {
        let all = notes(&["a/x.md"]);
        let docs = [("a/x.md", "self [[x]]")];
        let edits = plan(&docs, "a", "b", &all);
        assert!(edits.is_empty());
        let edits = plan(&[("a/x.md", "[[a/x]]")], "a/x.md", "b/y.md", &all);
        assert_eq!(edits[0].path, "b/y.md");
        assert_eq!(edits[0].new_text, "[[b/y]]");
    }

    #[test]
    fn plan_counts_replacements_per_file() {
        let all = notes(&["a.md", "z.md"]);
        let docs = [("z.md", "[[a]] [[a]] [[a]] [[nope]]")];
        let edits = plan(&docs, "a.md", "bb.md", &all);
        assert_eq!(edits[0].replaced, 3);
        assert_eq!(edits[0].new_text, "[[bb]] [[bb]] [[bb]] [[nope]]");
    }

    #[test]
    fn plan_rejects_empty_and_unsafe_inputs_without_panicking() {
        let all = notes(&["a.md"]);
        assert!(plan(&[("a.md", "[[a]]")], "", "", &all).is_empty());
        assert!(plan(&[("a.md", "[t](../../../etc/x)")], "a.md", "b.md", &all).is_empty());
    }

    // ---- helpers ----------------------------------------------------------

    #[test]
    fn percent_decode_handles_valid_and_invalid_sequences() {
        assert_eq!(percent_decode("a%20b.md"), "a b.md");
        assert_eq!(percent_decode("%E6%83%B3%E6%B3%95.md"), "想法.md");
        assert_eq!(percent_decode("100%.md"), "100%.md");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%FF"), "%FF");
    }

    #[test]
    fn encode_md_path_escapes_only_breaking_characters() {
        assert_eq!(encode_md_path("a b(c).md"), "a%20b%28c%29.md");
        assert_eq!(encode_md_path("想法/中文.md"), "想法/中文.md");
        assert_eq!(encode_md_path("50%#?.md"), "50%25%23%3F.md");
    }

    #[test]
    fn relative_path_climbs_and_descends() {
        assert_eq!(relative_path("", "a/x.md"), "a/x.md");
        assert_eq!(relative_path("a", "a/x.md"), "x.md");
        assert_eq!(relative_path("a/b", "a/x.md"), "../x.md");
        assert_eq!(relative_path("d", "a/b/x.md"), "../a/b/x.md");
        assert_eq!(relative_path("a/b", "a/b/c/d.md"), "c/d.md");
    }

    #[test]
    fn normalize_collapses_dots() {
        assert_eq!(normalize("a/./b/../c.md").as_deref(), Some("a/c.md"));
        assert_eq!(normalize("../x").as_deref(), None);
        assert_eq!(normalize("a//b").as_deref(), Some("a/b"));
    }

    #[test]
    fn map_path_handles_files_folders_and_bare_names() {
        assert_eq!(map_path("a.md", "a.md", "b.md").as_deref(), Some("b.md"));
        assert_eq!(map_path("a.md", "a", "b").as_deref(), Some("b.md"));
        assert_eq!(map_path("a/x.md", "a", "q/r").as_deref(), Some("q/r/x.md"));
        assert_eq!(map_path("ab/x.md", "a", "q").as_deref(), None);
        assert_eq!(map_path("other.md", "a.md", "b.md").as_deref(), None);
    }

    #[test]
    fn apply_reps_is_order_independent_and_safe() {
        let text = "abcdef";
        let reps = vec![(4..5, "E".to_string()), (0..1, "A".to_string())];
        assert_eq!(apply_reps(text, reps), "AbcdEf");
        // Overlapping and out-of-range replacements are skipped.
        let reps = vec![
            (0..3, "X".to_string()),
            (1..2, "Y".to_string()),
            (9..12, "Z".to_string()),
        ];
        assert_eq!(apply_reps(text, reps), "Xdef");
    }

    #[test]
    fn apply_reps_skips_non_char_boundaries() {
        let text = "中文";
        let reps = vec![(1..2, "x".to_string())];
        assert_eq!(apply_reps(text, reps), "中文");
    }

    #[test]
    fn masked_text_keeps_length_and_newlines() {
        let text = "中 `代码` ok\n```\nx\n```\n<!-- c -->[[a]]";
        let masked = masked_text(text);
        assert_eq!(masked.len(), text.len());
        assert_eq!(masked.matches('\n').count(), text.matches('\n').count());
        assert!(!masked.contains("代码"));
        assert!(masked.contains("[[a]]"));
    }

    #[test]
    fn is_external_detects_schemes() {
        assert!(is_external("https://x"));
        assert!(is_external("MAILTO:a@b"));
        assert!(!is_external("notes/a.md"));
    }
}
