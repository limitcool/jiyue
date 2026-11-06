//! Per-note facts kept by the catalog: title, aliases, tags, outgoing links and the body text
//! that goes into the search index. Pure functions, no I/O.

use jiyue_core::links::{self, LinkOccurrence};
use jiyue_md::frontmatter;

/// What the server knows about the text of one note.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteFacts {
    /// Frontmatter `title`, or the file stem.
    pub title: String,
    /// Frontmatter `aliases`.
    pub aliases: Vec<String>,
    /// Lower-case tags from frontmatter and from `#tags` in the body, without duplicates.
    pub tags: Vec<String>,
    /// Wiki links, embeds and Markdown links, skipping code and comments.
    pub links: Vec<LinkOccurrence>,
    /// Text after the frontmatter block.
    pub body: String,
}

/// Analyses the text of the note at vault-relative `rel`.
pub fn analyze_note(rel: &str, text: &str) -> NoteFacts {
    let body_start = frontmatter::split(text).1;
    let body = text.get(body_start..).unwrap_or(text);

    let title = frontmatter::title_from(text).unwrap_or_else(|| file_stem(rel).to_string());
    let aliases = frontmatter::aliases(text);

    let mut tags: Vec<String> = Vec::new();
    for raw in frontmatter::tags(text)
        .iter()
        .chain(inline_tags(body).iter())
    {
        push_tag(&mut tags, raw);
    }

    NoteFacts {
        title,
        aliases,
        tags,
        links: links::extract_links(text),
        body: body.to_string(),
    }
}

/// File name without its folder and without the extension.
pub fn file_stem(rel: &str) -> &str {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// Canonical tag form: trimmed, without leading `#` or surrounding slashes, lower case.
pub fn normalize_tag(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('#')
        .trim_matches('/')
        .to_lowercase()
}

fn push_tag(tags: &mut Vec<String>, raw: &str) {
    let tag = normalize_tag(raw);
    if !tag.is_empty() && !tags.contains(&tag) {
        tags.push(tag);
    }
}

/// `#tags` in the body, outside fenced code blocks and inline code spans. A tag starts at the
/// beginning of a line or after whitespace or opening punctuation, so URL fragments and heading
/// markers (`# Title`, with a space) are not tags. Purely numeric tags are ignored.
pub fn inline_tags(body: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let visible = blank_inline_code(line);
        scan_line_tags(&visible, &mut found);
    }
    found
}

/// Replaces the text of inline code spans with spaces, so tags inside them are not found.
fn blank_inline_code(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    for c in line.chars() {
        if c == '`' {
            in_code = !in_code;
            out.push(' ');
        } else if in_code {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

fn scan_line_tags(line: &str, found: &mut Vec<String>) {
    let chars: Vec<char> = line.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c != '#' {
            continue;
        }
        if i > 0 {
            let prev = chars[i - 1];
            if !(prev.is_whitespace() || "([{,;:!?\"'".contains(prev)) {
                continue;
            }
        }
        let mut tag = String::new();
        for &next in chars.iter().skip(i + 1) {
            if !is_tag_char(next) {
                break;
            }
            tag.push(next);
        }
        if tag.chars().all(|ch| ch.is_ascii_digit() || ch == '/') {
            continue;
        }
        push_tag(found, &tag);
    }
}

fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stem_drops_folder_and_extension() {
        assert_eq!(file_stem("a/b/note.md"), "note");
        assert_eq!(file_stem("读书笔记.md"), "读书笔记");
        assert_eq!(file_stem("README"), "README");
        assert_eq!(file_stem(".hidden"), ".hidden");
        assert_eq!(file_stem("a/b.c.md"), "b.c");
    }

    #[test]
    fn normalize_tag_lowercases_and_strips_hash_and_slashes() {
        assert_eq!(normalize_tag("#Project/Alpha/"), "project/alpha");
        assert_eq!(normalize_tag("  #Todo "), "todo");
        assert_eq!(normalize_tag("#"), "");
    }

    #[test]
    fn inline_tags_are_found_after_whitespace_or_at_line_start() {
        let tags = inline_tags("#alpha and text #beta, (#gamma)\n#delta");
        assert_eq!(tags, vec!["alpha", "beta", "gamma", "delta"]);
    }

    #[test]
    fn headings_urls_and_numbers_are_not_tags() {
        let tags = inline_tags("# Title\n## Sub\nsee https://x.org/#frag and #2024 and C#");
        assert!(tags.is_empty(), "got {tags:?}");
    }

    #[test]
    fn tags_in_code_are_ignored() {
        let body = "`#inline`\n```\n#fenced\n```\n#real";
        assert_eq!(inline_tags(body), vec!["real"]);
    }

    #[test]
    fn unicode_tags_are_kept() {
        assert_eq!(
            inline_tags("标签 #读书 #Rust_lang"),
            vec!["读书", "rust_lang"]
        );
    }

    #[test]
    fn analysis_merges_frontmatter_and_body_tags_once() {
        let text =
            "---\ntitle: Hello World\ntags: [Work, rust]\naliases: [Hi]\n---\nBody #rust #new\n";
        let facts = analyze_note("dir/hello.md", text);
        assert_eq!(facts.title, "Hello World");
        assert_eq!(facts.aliases, vec!["Hi"]);
        assert_eq!(facts.tags, vec!["work", "rust", "new"]);
        assert_eq!(facts.body, "Body #rust #new\n");
    }

    #[test]
    fn title_falls_back_to_file_stem() {
        let facts = analyze_note("x/plain.md", "no frontmatter here");
        assert_eq!(facts.title, "plain");
        assert_eq!(facts.body, "no frontmatter here");
        assert!(facts.links.is_empty());
    }

    #[test]
    fn links_are_extracted_and_code_is_skipped() {
        let text = "See [[Alpha|a]] and ![[pic.png]]\n`[[not-a-link]]`\n";
        let facts = analyze_note("n.md", text);
        let targets: Vec<&str> = facts.links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(targets, vec!["Alpha", "pic.png"]);
    }
}
