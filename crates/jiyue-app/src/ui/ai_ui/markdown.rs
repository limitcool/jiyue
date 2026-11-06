//! Lightweight Markdown for assistant replies: headings, paragraphs, bullet and numbered lists,
//! fenced code blocks, and `[[note]]` citations.
//!
//! Inline emphasis and code spans are shown as plain text (their markers are removed). Pure
//! functions, unit-tested.

/// One block of an assistant reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        text: String,
    },
    /// Consecutive text lines, kept in order.
    Paragraph {
        lines: Vec<String>,
    },
    Bullets(Vec<String>),
    Numbered(Vec<String>),
    /// Fenced code. An unclosed fence runs to the end of the text (streaming replies).
    Code {
        lang: String,
        text: String,
    },
}

/// Parses a reply into blocks.
pub fn parse(src: &str) -> Vec<Block> {
    let lines: Vec<&str> = src.lines().collect();
    let mut blocks = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.starts_with("```") {
            flush_paragraph(&mut para, &mut blocks);
            let lang = trimmed.trim_start_matches('`').trim().to_string();
            let mut body: Vec<&str> = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with("```") {
                body.push(lines[i]);
                i += 1;
            }
            // Skip the closing fence when it exists.
            i += 1;
            blocks.push(Block::Code {
                lang,
                text: body.join("\n"),
            });
            continue;
        }
        if trimmed.is_empty() {
            flush_paragraph(&mut para, &mut blocks);
            i += 1;
            continue;
        }
        if let Some((level, text)) = heading(trimmed) {
            flush_paragraph(&mut para, &mut blocks);
            blocks.push(Block::Heading { level, text });
            i += 1;
            continue;
        }
        if bullet(trimmed).is_some() {
            flush_paragraph(&mut para, &mut blocks);
            let mut items = Vec::new();
            while i < lines.len() {
                match bullet(lines[i].trim()) {
                    Some(item) => items.push(item),
                    None => break,
                }
                i += 1;
            }
            blocks.push(Block::Bullets(items));
            continue;
        }
        if numbered(trimmed).is_some() {
            flush_paragraph(&mut para, &mut blocks);
            let mut items = Vec::new();
            while i < lines.len() {
                match numbered(lines[i].trim()) {
                    Some(item) => items.push(item),
                    None => break,
                }
                i += 1;
            }
            blocks.push(Block::Numbered(items));
            continue;
        }
        para.push(trimmed.to_string());
        i += 1;
    }
    flush_paragraph(&mut para, &mut blocks);
    blocks
}

fn flush_paragraph(para: &mut Vec<String>, blocks: &mut Vec<Block>) {
    if !para.is_empty() {
        blocks.push(Block::Paragraph {
            lines: std::mem::take(para),
        });
    }
}

fn heading(trimmed: &str) -> Option<(u8, String)> {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim();
    if text.is_empty() {
        return None;
    }
    Some((hashes as u8, text.to_string()))
}

fn bullet(trimmed: &str) -> Option<String> {
    ["- ", "* ", "+ "]
        .iter()
        .find_map(|p| trimmed.strip_prefix(p))
        .map(|rest| rest.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn numbered(trimmed: &str) -> Option<String> {
    let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let rest = &trimmed[digits..];
    let rest = rest
        .strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))?;
    let text = rest.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// Note titles cited as `[[title]]`, `[[title|alias]]` or `[[title#heading]]`, in order, without
/// duplicates.
pub fn citations(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (target, _, _) in wikilinks(text) {
        if !out.contains(&target) {
            out.push(target);
        }
    }
    out
}

/// Inline text for display: citations become their bare title, `**` and backticks are removed.
pub fn plain_inline(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let Some(len) = rest[start..].find("]]") else {
            break;
        };
        out.push_str(&rest[..start]);
        let inner = &rest[start + 2..start + len];
        out.push_str(link_target(inner).as_str());
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "")
}

/// `(target, start, end)` for every complete `[[...]]` on the text, byte offsets.
fn wikilinks(text: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(rel) = text[pos..].find("[[") {
        let start = pos + rel;
        let Some(close) = text[start + 2..].find("]]") else {
            break;
        };
        let inner_start = start + 2;
        let end = inner_start + close + 2;
        let target = link_target(&text[inner_start..inner_start + close]);
        if !target.is_empty() && !target.contains('\n') {
            out.push((target, start, end));
        }
        pos = end;
    }
    out
}

fn link_target(inner: &str) -> String {
    inner
        .split('|')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_paragraphs_and_lists() {
        let src = "# 标题\n第一行\n第二行\n\n- 甲\n- 乙\n\n2. 丙\n3) 丁\n";
        let blocks = parse(src);
        assert_eq!(
            blocks,
            vec![
                Block::Heading {
                    level: 1,
                    text: "标题".into()
                },
                Block::Paragraph {
                    lines: vec!["第一行".into(), "第二行".into()]
                },
                Block::Bullets(vec!["甲".into(), "乙".into()]),
                Block::Numbered(vec!["丙".into(), "丁".into()]),
            ]
        );
    }

    #[test]
    fn code_fence_keeps_body_and_language() {
        let src = "前\n```rust\nfn main() {}\n\n- not a list\n```\n后";
        let blocks = parse(src);
        assert_eq!(blocks.len(), 3);
        assert_eq!(
            blocks[1],
            Block::Code {
                lang: "rust".into(),
                text: "fn main() {}\n\n- not a list".into()
            }
        );
    }

    #[test]
    fn unclosed_fence_runs_to_the_end() {
        let blocks = parse("```\npartial code");
        assert_eq!(
            blocks,
            vec![Block::Code {
                lang: String::new(),
                text: "partial code".into()
            }]
        );
    }

    #[test]
    fn hashes_without_space_are_text() {
        assert_eq!(
            parse("#tag 不是标题"),
            vec![Block::Paragraph {
                lines: vec!["#tag 不是标题".into()]
            }]
        );
        assert_eq!(parse("####### x").len(), 1);
        assert!(matches!(parse("####### x")[0], Block::Paragraph { .. }));
    }

    #[test]
    fn citations_are_unique_and_clean() {
        let text = "见 [[笔记 A]] 和 [[B|别名]]，还有 [[C#章节]] 与 [[笔记 A]]。";
        assert_eq!(citations(text), vec!["笔记 A", "B", "C"]);
        assert!(citations("没有链接 [[ 未闭合").is_empty());
        assert!(citations("[[]]").is_empty());
    }

    #[test]
    fn plain_inline_strips_markers() {
        assert_eq!(
            plain_inline("**重点** 见 [[笔记|别名]] 与 `code`"),
            "重点 见 笔记 与 code"
        );
        assert_eq!(plain_inline("[[未闭合"), "[[未闭合");
    }
}
