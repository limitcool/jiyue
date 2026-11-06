//! Where a `/` opens the menu, and the query typed after it. Pure logic.

/// Byte offset of the start of the line that holds byte `at`.
pub fn line_start(text: &str, at: usize) -> usize {
    let at = at.min(text.len());
    text.get(..at)
        .and_then(|s| s.rfind('\n'))
        .map_or(0, |i| i + 1)
}

/// True when a `/` at byte `slash_at` opens the menu.
///
/// It must be the first character of its line or follow whitespace, so `a/b`, `https://` and
/// `中文/` never open it. It must not sit in a fenced code block, a `$$` block, the frontmatter,
/// an inline code span, inline math or an unclosed `[[`.
pub fn trigger_ok(text: &str, slash_at: usize) -> bool {
    if text.get(slash_at..slash_at + 1) != Some("/") {
        return false;
    }
    let before = text.get(..slash_at).unwrap_or("");
    if let Some(c) = before.chars().next_back()
        && !c.is_whitespace()
    {
        return false;
    }
    let ls = line_start(text, slash_at);
    let prefix = text.get(ls..slash_at).unwrap_or("");
    if prefix.matches('`').count() % 2 == 1 || prefix.matches('$').count() % 2 == 1 {
        return false;
    }
    if open_wikilink(prefix) {
        return false;
    }
    // A line that opens a fence or a math block takes no menu.
    let trimmed = prefix.trim_start();
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") || trimmed.starts_with("$$") {
        return false;
    }
    !in_block(text, ls)
}

/// True when `prefix` (the text of a line before the `/`) holds a `[[` that is not closed yet.
fn open_wikilink(prefix: &str) -> bool {
    match prefix.rfind("[[") {
        None => false,
        Some(open) => prefix.rfind("]]").is_none_or(|close| close < open),
    }
}

/// True when the lines before byte `upto` leave a block open: the frontmatter, a fenced code
/// block or a `$$` block. `upto` is the start of a line.
fn in_block(text: &str, upto: usize) -> bool {
    let body = text.get(..upto).unwrap_or("");
    let mut frontmatter = false;
    let mut fence: Option<char> = None;
    let mut math = false;
    for (i, line) in body.lines().enumerate() {
        let t = line.trim_end();
        if i == 0 && t == "---" {
            frontmatter = true;
            continue;
        }
        if frontmatter {
            if t == "---" || t == "..." {
                frontmatter = false;
            }
            continue;
        }
        let ts = line.trim_start();
        if let Some(fc) = fence {
            if is_fence_close(ts, fc) {
                fence = None;
            }
            continue;
        }
        if let Some(fc) = fence_open(ts) {
            fence = Some(fc);
            continue;
        }
        if t.trim() == "$$" {
            math = !math;
        }
    }
    frontmatter || fence.is_some() || math
}

/// The fence character when `ts` opens a fenced code block.
fn fence_open(ts: &str) -> Option<char> {
    if ts.starts_with("```") {
        Some('`')
    } else if ts.starts_with("~~~") {
        Some('~')
    } else {
        None
    }
}

/// True when `ts` closes a fence opened with character `fc`: three or more of it, nothing else.
fn is_fence_close(ts: &str, fc: char) -> bool {
    let t = ts.trim_end();
    t.chars().count() >= 3 && t.chars().all(|c| c == fc)
}

/// The query typed after the `/` at `slash_at`, when the menu is still valid for `caret`: the
/// `/` is still there, the caret is after it, and the query stays on one line.
pub fn query_of(text: &str, slash_at: usize, caret: usize) -> Option<&str> {
    if text.get(slash_at..slash_at + 1) != Some("/") || caret <= slash_at {
        return None;
    }
    let query = text.get(slash_at + 1..caret)?;
    if query.contains('\n') {
        return None;
    }
    Some(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_slash(text: &str) -> bool {
        // The first `/` of the text is the one typed (the tests put it where they mean it).
        let i = text.find('/').expect("test text has a slash");
        trigger_ok(text, i)
    }

    #[test]
    fn slash_at_line_start_or_after_whitespace_opens() {
        assert!(at_slash("/"));
        assert!(at_slash("hello /"));
        assert!(at_slash("hello\t/"));
        assert!(at_slash("第一行\n/"));
        assert!(at_slash("中文 /"));
    }

    #[test]
    fn slash_after_text_or_in_a_path_does_not_open() {
        assert!(!at_slash("hello/"));
        assert!(!at_slash("中文/"));
        assert!(!at_slash("a/b"));
        assert!(!at_slash("see https://example.com"));
        assert!(!at_slash("path\\/x"));
    }

    #[test]
    fn slash_inside_a_fenced_code_block_does_not_open() {
        assert!(!at_slash("```\n/\n```"));
        assert!(!at_slash("~~~\n/\n~~~"));
        assert!(!at_slash("```rust\nlet a = 1;\n/"));
        // After the fence closes, the menu works again.
        assert!(at_slash("```\ncode\n```\n/"));
    }

    #[test]
    fn slash_inside_math_or_frontmatter_does_not_open() {
        assert!(!at_slash("$$\n/\n$$"));
        assert!(!at_slash("---\ntitle: x\n/"));
        // Frontmatter ends at the second `---`.
        assert!(at_slash("---\ntitle: x\n---\n/"));
        // A `---` that is not on the first line is an ordinary rule.
        assert!(at_slash("text\n---\n/"));
    }

    #[test]
    fn slash_inside_inline_code_math_or_open_wikilink_does_not_open() {
        assert!(!at_slash("`code /"));
        assert!(at_slash("`a` /"));
        assert!(!at_slash("$x /"));
        assert!(at_slash("$x$ /"));
        assert!(!at_slash("[[note /"));
        assert!(at_slash("[[note]] /"));
    }

    #[test]
    fn query_follows_the_slash_on_one_line() {
        assert_eq!(query_of("/", 0, 1), Some(""));
        assert_eq!(query_of("hi /biao", 3, 8), Some("biao"));
        assert_eq!(query_of("/a\nb", 0, 4), None, "a query cannot cross a line");
        assert_eq!(query_of("hi /x", 3, 3), None, "caret before the slash");
        assert_eq!(query_of("hi x", 3, 4), None, "the slash is gone");
    }

    #[test]
    fn cjk_query_is_sliced_on_char_boundaries() {
        let text = "/标题";
        let caret = text.len();
        assert_eq!(query_of(text, 0, caret), Some("标题"));
        assert_eq!(query_of(text, 0, 2), None, "caret inside a character");
    }

    #[test]
    fn line_start_finds_the_start_of_the_slash_line() {
        assert_eq!(line_start("abc", 2), 0);
        assert_eq!(line_start("abc\ndef", 6), 4);
        assert_eq!(line_start("abc\n", 4), 4);
    }
}
