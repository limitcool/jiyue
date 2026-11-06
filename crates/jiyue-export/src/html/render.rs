//! Markdown to HTML with the Jiyue syntax layer.
//!
//! The pipeline: pulldown-cmark events, adjacent text merged, then one transform pass that
//! rewrites what Jiyue adds (wikilinks, embeds, callouts, tags, highlights, block ids,
//! mermaid, math, unsafe links and raw HTML). Text inside code is never touched. The result
//! is handed to `pulldown_cmark::html::push_html`, which escapes every `Text` event.

use std::collections::HashMap;

use jiyue_md::embeds::{EmbedKind, embed_kind, slugify};
use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd, html::push_html};

use super::highlight;
use super::{AssetRef, ExportCtx, Heading, Unresolved, esc};

/// Maximum depth of `![[note]]` embeds inside embeds.
pub(super) const MAX_EMBED_DEPTH: u32 = 3;

const SAFE_SCHEMES: [&str; 4] = ["http", "https", "mailto", "tel"];

/// Context for one rendering pass.
pub(super) struct Cx<'c, 'a> {
    pub ctx: &'c ExportCtx<'a>,
    pub depth: u32,
}

/// Mutable results collected while rendering.
#[derive(Default)]
pub(super) struct State {
    pub headings: Vec<Heading>,
    pub slugs: HashMap<String, usize>,
    pub assets: Vec<AssetRef>,
    pub unresolved: Vec<Unresolved>,
    pub plain: String,
    /// Footnote numbers of the note being rendered, by lower-cased label (first reference order),
    /// and the numbers whose reference has been written (only the first one gets the anchor id).
    pub footnotes: HashMap<String, u32>,
    pub footnote_seen: std::collections::HashSet<u32>,
}

/// A footnote definition taken out of the body: its lower-cased label and its events.
struct Note<'e> {
    key: String,
    events: Vec<Event<'e>>,
}

fn html_event<'e>(s: String) -> Event<'e> {
    Event::Html(CowStr::from(s))
}

fn text_event<'e>(s: &str) -> Event<'e> {
    Event::Text(CowStr::from(s.to_string()))
}

/// Renders a Markdown body (frontmatter already removed) to an HTML fragment.
pub(super) fn render_markdown(md: &str, cx: &Cx<'_, '_>, st: &mut State) -> String {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH;
    let raw: Vec<Event<'_>> = Parser::new_ext(md, opts).collect();
    let events = merge_text(raw);
    collect_plain(&events, st);
    // Footnotes: the definitions leave the body and are listed at its end, in number order. A nested
    // render (an embed) has its own numbers, so the outer ones are put back afterwards.
    let (body, notes) = split_footnotes(&events);
    let saved_numbers = std::mem::replace(&mut st.footnotes, number_footnotes(&events));
    let saved_seen = std::mem::take(&mut st.footnote_seen);
    let mut out = transform(&body, cx, st);
    out.extend(footnote_list(notes, cx, st));
    st.footnotes = saved_numbers;
    st.footnote_seen = saved_seen;
    let mut html = String::with_capacity(md.len() + md.len() / 2);
    push_html(&mut html, out.into_iter());
    html
}

/// Takes the footnote definitions out of `events`. Returns the rest of the events and the
/// definitions in source order.
fn split_footnotes<'e>(events: &[Event<'e>]) -> (Vec<Event<'e>>, Vec<Note<'e>>) {
    let mut body = Vec::with_capacity(events.len());
    let mut notes = Vec::new();
    let mut i = 0usize;
    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::FootnoteDefinition(label)) => {
                let end = find_end(events, i + 1, |e| {
                    matches!(e, Event::End(TagEnd::FootnoteDefinition))
                });
                notes.push(Note {
                    key: label.to_lowercase(),
                    events: events[i + 1..end].to_vec(),
                });
                i = end + 1;
            }
            other => {
                body.push(other.clone());
                i += 1;
            }
        }
    }
    (body, notes)
}

/// The numbers of the footnote labels that are referenced, in order of their first reference.
/// pulldown-cmark only emits a reference for a label that has a definition.
fn number_footnotes(events: &[Event<'_>]) -> HashMap<String, u32> {
    let mut numbers: HashMap<String, u32> = HashMap::new();
    for ev in events {
        if let Event::FootnoteReference(label) = ev {
            let next = numbers.len() as u32 + 1;
            numbers.entry(label.to_lowercase()).or_insert(next);
        }
    }
    numbers
}

/// The footnote list at the end of a note: the referenced definitions in number order, each with
/// a back arrow to its first reference, then the unreferenced ones without numbers.
fn footnote_list<'e>(notes: Vec<Note<'e>>, cx: &Cx<'_, '_>, st: &mut State) -> Vec<Event<'e>> {
    if notes.is_empty() {
        return Vec::new();
    }
    let mut keyed: Vec<(Option<u32>, Note<'e>)> = notes
        .into_iter()
        .map(|n| (st.footnotes.get(&n.key).copied(), n))
        .collect();
    // Referenced first (by number); the sort is stable, so unreferenced keep their order.
    keyed.sort_by_key(|(num, _)| num.unwrap_or(u32::MAX));
    let mut out = Vec::new();
    out.push(html_event(
        "<section class=\"footnotes\">\n<ol>\n".to_string(),
    ));
    for (num, note) in keyed {
        match num {
            Some(n) => out.push(html_event(format!(
                "<li class=\"footnote-definition\" id=\"fn-{n}\">"
            ))),
            None => out.push(html_event("<li class=\"footnote-definition\">".to_string())),
        }
        out.extend(transform(&note.events, cx, st));
        if let Some(n) = num {
            out.push(html_event(format!(
                " <a href=\"#fnref-{n}\" class=\"footnote-backref\">&#8617;</a>"
            )));
        }
        out.push(html_event("</li>\n".to_string()));
    }
    out.push(html_event("</ol>\n</section>\n".to_string()));
    out
}

/// Joins runs of adjacent `Text` events so syntax split across them is found.
fn merge_text(events: Vec<Event<'_>>) -> Vec<Event<'_>> {
    let mut out: Vec<Event<'_>> = Vec::with_capacity(events.len());
    let mut pending: Option<String> = None;
    for ev in events {
        match ev {
            Event::Text(t) => match pending.as_mut() {
                Some(buf) => buf.push_str(&t),
                None => pending = Some(t.to_string()),
            },
            other => {
                if let Some(buf) = pending.take() {
                    out.push(Event::Text(CowStr::from(buf)));
                }
                out.push(other);
            }
        }
    }
    if let Some(buf) = pending {
        out.push(Event::Text(CowStr::from(buf)));
    }
    out
}

fn collect_plain(events: &[Event<'_>], st: &mut State) {
    for ev in events {
        match ev {
            Event::Text(t) | Event::Code(t) => st.plain.push_str(t),
            Event::End(_) | Event::SoftBreak | Event::HardBreak => st.plain.push(' '),
            _ => {}
        }
    }
}

/// Index of the first event at or after `from` matching `pred`, or `events.len()`.
fn find_end(events: &[Event<'_>], from: usize, pred: impl Fn(&Event<'_>) -> bool) -> usize {
    (from..events.len())
        .find(|&j| pred(&events[j]))
        .unwrap_or(events.len())
}

/// Index of the `End(BlockQuote)` matching the `Start(BlockQuote)` at `start`.
fn blockquote_end(events: &[Event<'_>], start: usize) -> usize {
    let mut depth = 0usize;
    for (j, ev) in events.iter().enumerate().skip(start) {
        match ev {
            Event::Start(Tag::BlockQuote(_)) => depth += 1,
            Event::End(TagEnd::BlockQuote) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return j;
                }
            }
            _ => {}
        }
    }
    events.len()
}

fn plain_text_of(events: &[Event<'_>]) -> String {
    let mut s = String::new();
    for ev in events {
        if let Event::Text(t) | Event::Code(t) = ev {
            s.push_str(t);
        }
    }
    s.trim().to_string()
}

fn unique_id(st: &mut State, text: &str) -> String {
    let base = match slugify(text) {
        s if s.is_empty() => "section".to_string(),
        s => s,
    };
    let count = st.slugs.entry(base.clone()).or_insert(0);
    *count += 1;
    if *count == 1 {
        base
    } else {
        format!("{base}-{count}")
    }
}

fn transform<'e>(events: &[Event<'e>], cx: &Cx<'_, '_>, st: &mut State) -> Vec<Event<'e>> {
    let mut out: Vec<Event<'e>> = Vec::with_capacity(events.len());
    let mut i = 0usize;
    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::Heading { level, .. }) => {
                let level = *level;
                let end = find_end(events, i + 1, |e| {
                    matches!(e, Event::End(TagEnd::Heading(_)))
                });
                let inner = &events[i + 1..end];
                let text = plain_text_of(inner);
                let id = unique_id(st, &text);
                st.headings.push(Heading {
                    level: level as u8,
                    text,
                    id: id.clone(),
                });
                out.push(Event::Start(Tag::Heading {
                    level,
                    id: Some(CowStr::from(id)),
                    classes: Vec::new(),
                    attrs: Vec::new(),
                }));
                out.extend(transform(inner, cx, st));
                out.push(Event::End(TagEnd::Heading(level)));
                i = end + 1;
            }
            Event::Start(Tag::BlockQuote(_)) => {
                let end = blockquote_end(events, i);
                if let Some((kind, title, inner)) = callout_parts(events, i, end) {
                    let class = callout_class(&kind);
                    let title = match title {
                        Some(t) => t,
                        None => label_for(class).to_string(),
                    };
                    out.push(html_event(format!(
                        "<div class=\"callout callout-{class}\" data-callout=\"{class}\">\
<div class=\"callout-title\">{}</div><div class=\"callout-body\">",
                        esc(&title)
                    )));
                    out.extend(transform(&inner, cx, st));
                    out.push(html_event("</div></div>".to_string()));
                    i = end + 1;
                } else {
                    out.push(events[i].clone());
                    i += 1;
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase(),
                    CodeBlockKind::Indented => String::new(),
                };
                let end = find_end(events, i + 1, |e| {
                    matches!(e, Event::End(TagEnd::CodeBlock))
                });
                let mut code = String::new();
                for ev in &events[i + 1..end] {
                    if let Event::Text(t) = ev {
                        code.push_str(t);
                    }
                }
                out.push(html_event(code_block_html(&lang, &code, cx)));
                i = end + 1;
            }
            Event::FootnoteReference(label) => {
                match st.footnotes.get(&label.to_lowercase()).copied() {
                    Some(n) => {
                        // The first reference of a number carries the anchor the back arrow returns to.
                        let id = if st.footnote_seen.insert(n) {
                            format!(" id=\"fnref-{n}\"")
                        } else {
                            String::new()
                        };
                        out.push(html_event(format!(
                            "<sup class=\"footnote-reference\"{id}><a href=\"#fn-{n}\">{n}</a></sup>"
                        )));
                    }
                    None => out.push(text_event(&format!("[^{label}]"))),
                }
                i += 1;
            }
            Event::InlineMath(src) => {
                out.push(html_event(math_html(src, false, cx)));
                i += 1;
            }
            Event::DisplayMath(src) => {
                out.push(html_event(math_html(src, true, cx)));
                i += 1;
            }
            Event::Html(s) | Event::InlineHtml(s) => {
                if cx.ctx.options.allow_raw_html {
                    out.push(events[i].clone());
                } else {
                    out.push(Event::Text(s.clone()));
                }
                i += 1;
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let dest = rewrite_dest(dest_url, false, cx, st);
                out.push(Event::Start(Tag::Link {
                    link_type: *link_type,
                    dest_url: CowStr::from(dest),
                    title: title.clone(),
                    id: id.clone(),
                }));
                i += 1;
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let dest = rewrite_dest(dest_url, true, cx, st);
                out.push(Event::Start(Tag::Image {
                    link_type: *link_type,
                    dest_url: CowStr::from(dest),
                    title: title.clone(),
                    id: id.clone(),
                }));
                i += 1;
            }
            Event::Text(t) => {
                let last = matches!(events.get(i + 1), Some(Event::End(_)));
                scan_text(t, last, cx, st, &mut out);
                i += 1;
            }
            _ => {
                out.push(events[i].clone());
                i += 1;
            }
        }
    }
    out
}

/// If the blockquote `[start, end)` begins with a `[!type] Title` line, returns the type,
/// the optional title, and the body events with that marker line removed.
fn callout_parts<'e>(
    events: &[Event<'e>],
    start: usize,
    end: usize,
) -> Option<(String, Option<String>, Vec<Event<'e>>)> {
    if end <= start + 2 || !matches!(events.get(start + 1), Some(Event::Start(Tag::Paragraph))) {
        return None;
    }
    let Some(Event::Text(first)) = events.get(start + 2) else {
        return None;
    };
    let line: &str = first;
    let rest = line.strip_prefix("[!")?;
    let close = rest.find(']')?;
    let kind = &rest[..close];
    if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
        return None;
    }
    let title = rest[close + 1..].trim();
    let title = (!title.is_empty()).then(|| title.to_string());

    let mut inner: Vec<Event<'e>> = events[start + 1..end].to_vec();
    // inner[0] = Start(Paragraph), inner[1] = the marker text.
    inner.remove(1);
    if matches!(inner.get(1), Some(Event::SoftBreak | Event::HardBreak)) {
        inner.remove(1);
    }
    if matches!(inner.first(), Some(Event::Start(Tag::Paragraph)))
        && matches!(inner.get(1), Some(Event::End(TagEnd::Paragraph)))
    {
        inner.drain(0..2);
    }
    Some((kind.to_string(), title, inner))
}

fn callout_class(kind: &str) -> &'static str {
    match kind.to_ascii_lowercase().as_str() {
        "info" | "abstract" | "summary" | "tldr" => "info",
        "tip" | "hint" | "important" => "tip",
        "warning" | "caution" | "attention" => "warning",
        "danger" | "error" | "failure" | "fail" | "missing" => "danger",
        "quote" | "cite" => "quote",
        "example" => "example",
        "bug" => "bug",
        "success" | "check" | "done" => "success",
        "question" | "help" | "faq" => "question",
        _ => "note",
    }
}

fn label_for(class: &str) -> &'static str {
    match class {
        "info" => "Info",
        "tip" => "Tip",
        "warning" => "Warning",
        "danger" => "Danger",
        "quote" => "Quote",
        "example" => "Example",
        "bug" => "Bug",
        "success" => "Success",
        "question" => "Question",
        _ => "Note",
    }
}

fn code_block_html(lang: &str, code: &str, cx: &Cx<'_, '_>) -> String {
    if lang == "mermaid" {
        if let Some(render) = cx.ctx.render_mermaid
            && let Some(svg) = render(code)
        {
            return format!("<div class=\"jiyue-mermaid\">{svg}</div>");
        }
        return format!(
            "<div class=\"jiyue-mermaid\" data-src=\"{0}\"><pre class=\"jiyue-code\"><code>{0}</code></pre></div>\n",
            esc(code)
        );
    }
    let body = highlight::highlight(lang, code).unwrap_or_else(|| esc(code));
    if lang.is_empty() {
        format!("<pre class=\"jiyue-code\"><code>{body}</code></pre>\n")
    } else {
        format!(
            "<pre class=\"jiyue-code\"><code class=\"language-{}\">{body}</code></pre>\n",
            esc(lang)
        )
    }
}

fn math_html(src: &str, display: bool, cx: &Cx<'_, '_>) -> String {
    if let Some(render) = cx.ctx.render_math
        && let Some(svg) = render(src, display)
    {
        return if display {
            format!("<div class=\"jiyue-math jiyue-math-block\">{svg}</div>")
        } else {
            format!("<span class=\"jiyue-math\">{svg}</span>")
        };
    }
    if display {
        format!(
            "<div class=\"jiyue-math jiyue-math-block\" data-latex=\"{0}\">{0}</div>\n",
            esc(src)
        )
    } else {
        format!(
            "<span class=\"jiyue-math\" data-latex=\"{0}\">{0}</span>",
            esc(src)
        )
    }
}

/// Safe href for a link or image destination, recording assets and unresolved targets.
fn rewrite_dest(dest: &str, image: bool, cx: &Cx<'_, '_>, st: &mut State) -> String {
    let d = dest.trim();
    if d.is_empty() || d.starts_with('#') {
        return d.to_string();
    }
    if let Some(scheme) = url_scheme(d) {
        return if SAFE_SCHEMES.contains(&scheme.as_str()) {
            d.to_string()
        } else {
            "#".to_string()
        };
    }
    if d.starts_with("//") {
        return d.to_string();
    }
    let decoded = percent_decode(d);
    let (path, frag) = match decoded.split_once('#') {
        Some((p, f)) => (p.trim().to_string(), Some(f.to_string())),
        None => (decoded.trim().to_string(), None),
    };
    if path.is_empty() {
        return d.to_string();
    }
    match (cx.ctx.resolve_link)(&path) {
        Some(href) => {
            let kind = embed_kind(&path);
            if image || kind != EmbedKind::Note {
                st.assets.push(AssetRef {
                    target: path,
                    href: href.clone(),
                    kind,
                });
            }
            join_fragment(href, frag.as_deref())
        }
        None => {
            st.unresolved.push(Unresolved {
                target: path,
                image,
            });
            d.to_string()
        }
    }
}

/// Lowercased scheme of a URL, when the text starts with one (`javascript`, `https` ...).
fn url_scheme(d: &str) -> Option<String> {
    let cleaned: String = d
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace())
        .collect();
    let idx = cleaned.find(':')?;
    let before = &cleaned[..idx];
    let starts_alpha = before.starts_with(|c: char| c.is_ascii_alphabetic());
    let valid = before
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if starts_alpha && valid {
        Some(before.to_ascii_lowercase())
    } else {
        None
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn join_fragment(href: String, frag: Option<&str>) -> String {
    match frag {
        Some(f) if !f.is_empty() => format!("{href}#{f}"),
        _ => href,
    }
}

/// Anchor id for `#Heading` or `#^block` in a wikilink.
fn anchor_frag(a: &str) -> String {
    match a.strip_prefix('^') {
        Some(id) => format!("^{id}"),
        None => slugify(a),
    }
}

fn missing_html(target: &str, title: &str) -> String {
    format!(
        "<span class=\"jiyue-missing\" title=\"{}\">{}</span>",
        esc(title),
        esc(target)
    )
}

/// Scans one text event for Jiyue inline syntax and pushes the resulting events.
fn scan_text<'e>(s: &str, last: bool, cx: &Cx<'_, '_>, st: &mut State, out: &mut Vec<Event<'e>>) {
    let b = s.as_bytes();
    let n = b.len();
    let mut plain = 0usize;
    let mut i = 0usize;
    while i < n {
        let found = match b[i] {
            b'!' if s[i..].starts_with("![[") => try_wiki(s, i, true, cx, st),
            b'[' if s[i..].starts_with("[[") => try_wiki(s, i, false, cx, st),
            b'=' if s[i..].starts_with("==") => try_highlight(s, i),
            b'#' => try_tag(s, i),
            b'^' if last => try_block_id(s, i),
            _ => None,
        };
        match found {
            Some((end, events)) => {
                if plain < i {
                    out.push(text_event(&s[plain..i]));
                }
                out.extend(events);
                i = end;
                plain = end;
            }
            None => i += 1,
        }
    }
    if plain < n {
        out.push(text_event(&s[plain..]));
    }
}

/// True when the byte at `i` starts a word: at the beginning or after whitespace.
fn at_word_start(s: &str, i: usize) -> bool {
    i == 0 || s[..i].chars().next_back().is_some_and(char::is_whitespace)
}

fn try_wiki<'e>(
    s: &str,
    i: usize,
    embed: bool,
    cx: &Cx<'_, '_>,
    st: &mut State,
) -> Option<(usize, Vec<Event<'e>>)> {
    let start = i + if embed { 3 } else { 2 };
    let rel = s[start..].find("]]")?;
    let inner = &s[start..start + rel];
    if inner.trim().is_empty() || inner.contains(['[', ']', '\n']) {
        return None;
    }
    let end = start + rel + 2;
    let (body, alias) = match inner.split_once('|') {
        Some((b, a)) => (b, Some(a.trim()).filter(|a| !a.is_empty())),
        None => (inner, None),
    };
    let (target, anchor) = match body.split_once('#') {
        Some((t, a)) => (t.trim(), Some(a.trim()).filter(|a| !a.is_empty())),
        None => (body.trim(), None),
    };
    let events = if embed {
        embed_events(target, anchor, alias, cx, st)
    } else {
        link_events(target, anchor, alias, cx, st)
    };
    Some((end, events))
}

fn link_events<'e>(
    target: &str,
    anchor: Option<&str>,
    alias: Option<&str>,
    cx: &Cx<'_, '_>,
    st: &mut State,
) -> Vec<Event<'e>> {
    let frag = anchor.map(anchor_frag);
    if target.is_empty() {
        let text = match (alias, anchor) {
            (Some(a), _) => a.to_string(),
            (None, Some(a)) => format!("#{a}"),
            (None, None) => String::new(),
        };
        let href = format!("#{}", frag.unwrap_or_default());
        return vec![html_event(format!(
            "<a class=\"jiyue-link\" href=\"{}\">{}</a>",
            esc(&href),
            esc(&text)
        ))];
    }
    let text = match (alias, anchor) {
        (Some(a), _) => a.to_string(),
        (None, Some(a)) => format!("{target}#{a}"),
        (None, None) => target.to_string(),
    };
    match (cx.ctx.resolve_link)(target) {
        Some(href) => vec![html_event(format!(
            "<a class=\"jiyue-link\" href=\"{}\">{}</a>",
            esc(&join_fragment(href, frag.as_deref())),
            esc(&text)
        ))],
        None => {
            st.unresolved.push(Unresolved {
                target: target.to_string(),
                image: false,
            });
            vec![html_event(missing_html(&text, "未找到笔记"))]
        }
    }
}

/// `300` or `300x200` from an embed's display option.
fn parse_size(a: &str) -> Option<(u32, Option<u32>)> {
    let a = a.trim();
    match a.split_once('x') {
        Some((w, h)) => Some((w.trim().parse().ok()?, Some(h.trim().parse().ok()?))),
        None => Some((a.parse().ok()?, None)),
    }
}

fn embed_events<'e>(
    target: &str,
    anchor: Option<&str>,
    alias: Option<&str>,
    cx: &Cx<'_, '_>,
    st: &mut State,
) -> Vec<Event<'e>> {
    let kind = embed_kind(target);
    let size = alias.and_then(parse_size);
    let caption = match (alias, size) {
        (Some(a), None) => a,
        _ => target,
    };
    match kind {
        EmbedKind::Note => {
            if cx.depth < MAX_EMBED_DEPTH
                && let Some(resolve) = cx.ctx.resolve_embed
            {
                let full = match anchor {
                    Some(a) => format!("{target}#{a}"),
                    None => target.to_string(),
                };
                if let Some(md) = resolve(&full) {
                    let inner = render_nested(&md, cx, st);
                    return vec![html_event(format!(
                        "<div class=\"jiyue-embed\" data-embed=\"{}\">{inner}</div>",
                        esc(target)
                    ))];
                }
            }
            st.unresolved.push(Unresolved {
                target: target.to_string(),
                image: false,
            });
            vec![html_event(missing_html(target, "未找到嵌入的笔记"))]
        }
        EmbedKind::Image => match (cx.ctx.resolve_link)(target) {
            Some(href) => {
                st.assets.push(AssetRef {
                    target: target.to_string(),
                    href: href.clone(),
                    kind,
                });
                let mut attrs = String::new();
                if let Some((w, h)) = size {
                    attrs.push_str(&format!(" width=\"{w}\""));
                    if let Some(h) = h {
                        attrs.push_str(&format!(" height=\"{h}\""));
                    }
                }
                vec![html_event(format!(
                    "<img class=\"jiyue-img\" src=\"{}\" alt=\"{}\" loading=\"lazy\"{attrs}>",
                    esc(&href),
                    esc(caption)
                ))]
            }
            None => {
                st.unresolved.push(Unresolved {
                    target: target.to_string(),
                    image: true,
                });
                vec![html_event(missing_html(target, "图片不存在"))]
            }
        },
        _ => match (cx.ctx.resolve_link)(target) {
            Some(href) => {
                st.assets.push(AssetRef {
                    target: target.to_string(),
                    href: href.clone(),
                    kind,
                });
                vec![html_event(format!(
                    "<a class=\"jiyue-file\" href=\"{}\">{}</a>",
                    esc(&href),
                    esc(caption)
                ))]
            }
            None => {
                st.unresolved.push(Unresolved {
                    target: target.to_string(),
                    image: false,
                });
                vec![html_event(missing_html(target, "文件不存在"))]
            }
        },
    }
}

/// Renders embedded Markdown one level deeper. Its assets and unresolved targets count for
/// the page, its headings do not appear in the page's table of contents.
fn render_nested(md: &str, cx: &Cx<'_, '_>, st: &mut State) -> String {
    let child = Cx {
        ctx: cx.ctx,
        depth: cx.depth + 1,
    };
    let mut child_state = State::default();
    let html = render_markdown(md, &child, &mut child_state);
    st.assets.append(&mut child_state.assets);
    st.unresolved.append(&mut child_state.unresolved);
    html
}

fn try_highlight<'e>(s: &str, i: usize) -> Option<(usize, Vec<Event<'e>>)> {
    let rest = &s[i + 2..];
    let close = rest.find("==")?;
    let inner = &rest[..close];
    if inner.trim().is_empty() {
        return None;
    }
    Some((
        i + 2 + close + 2,
        vec![html_event(format!("<mark>{}</mark>", esc(inner)))],
    ))
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

fn try_tag<'e>(s: &str, i: usize) -> Option<(usize, Vec<Event<'e>>)> {
    if !at_word_start(s, i) {
        return None;
    }
    let rest = &s[i + 1..];
    let mut len = 0usize;
    for (idx, c) in rest.char_indices() {
        if !is_tag_char(c) {
            break;
        }
        len = idx + c.len_utf8();
    }
    let name = rest[..len].trim_end_matches('/');
    let first = name.chars().next()?;
    if !(first.is_alphanumeric() || first == '_') {
        return None;
    }
    if !name.chars().any(|c| c.is_alphabetic() || c == '_') {
        return None;
    }
    Some((
        i + 1 + name.len(),
        vec![html_event(format!(
            "<span class=\"jiyue-tag\">#{}</span>",
            esc(name)
        ))],
    ))
}

fn try_block_id<'e>(s: &str, i: usize) -> Option<(usize, Vec<Event<'e>>)> {
    if !at_word_start(s, i) {
        return None;
    }
    let id = s[i + 1..].trim_end();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    Some((
        s.len(),
        vec![html_event(format!(
            "<span class=\"jiyue-block-id\" id=\"{}\"></span>",
            esc(&format!("^{id}"))
        ))],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_scheme_detects_obfuscated_javascript() {
        assert_eq!(url_scheme("javascript:x").as_deref(), Some("javascript"));
        assert_eq!(url_scheme("java\tscript:x").as_deref(), Some("javascript"));
        assert_eq!(url_scheme("JAVA SCRIPT :x").as_deref(), Some("javascript"));
        assert_eq!(url_scheme("notes/a:b.md"), None);
        assert_eq!(url_scheme("./x:y"), None);
        assert_eq!(url_scheme("#a:b"), None);
        assert_eq!(url_scheme("1abc:x"), None);
    }

    #[test]
    fn percent_decode_handles_bad_sequences() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("%E4%B8%AD.png"), "中.png");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%2"), "%2");
    }

    #[test]
    fn parse_size_variants() {
        assert_eq!(parse_size("300"), Some((300, None)));
        assert_eq!(parse_size("300x200"), Some((300, Some(200))));
        assert_eq!(parse_size(" 40 x 20 "), Some((40, Some(20))));
        assert_eq!(parse_size("abc"), None);
        assert_eq!(parse_size("1x"), None);
    }

    #[test]
    fn anchor_fragments() {
        assert_eq!(anchor_frag("^Block-1"), "^Block-1");
        assert_eq!(anchor_frag("First Heading"), "first-heading");
        assert_eq!(anchor_frag("第一 章"), "第一-章");
    }

    #[test]
    fn callout_class_mapping() {
        assert_eq!(callout_class("CAUTION"), "warning");
        assert_eq!(callout_class("error"), "danger");
        assert_eq!(callout_class("hint"), "tip");
        assert_eq!(callout_class("done"), "success");
        assert_eq!(callout_class("whatever"), "note");
    }

    #[test]
    fn merge_text_joins_runs() {
        let evs = vec![
            Event::Text(CowStr::from("a")),
            Event::Text(CowStr::from("b")),
            Event::SoftBreak,
            Event::Text(CowStr::from("c")),
        ];
        let merged = merge_text(evs);
        assert_eq!(merged.len(), 3);
        assert!(matches!(&merged[0], Event::Text(t) if t.as_ref() == "ab"));
    }

    #[test]
    fn at_word_start_rules() {
        assert!(at_word_start("#a", 0));
        assert!(at_word_start("x #a", 2));
        assert!(!at_word_start("x#a", 1));
        assert!(at_word_start("中 #a", "中 ".len()));
    }

    #[test]
    fn tag_rules() {
        let (end, _) = try_tag("#日记/2026 x", 0).expect("tag");
        assert_eq!(&"#日记/2026 x"[..end], "#日记/2026");
        assert!(try_tag("#2026", 0).is_none());
        assert!(try_tag("# 标题", 0).is_none());
        let (end2, _) = try_tag("#tag/", 0).expect("tag");
        assert_eq!(end2, "#tag".len());
    }
}
