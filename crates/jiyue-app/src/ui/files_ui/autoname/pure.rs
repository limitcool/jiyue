//! Pure rules of the automatic file name of a new note: the default name (`未命名 N`), the
//! heading text to file name conversion (sanitising, reserved names, length, dedup) and the
//! eligibility decision. No app or file access here; unit tested.

/// Prefix of the default name of a new note (`未命名 N`).
const DEFAULT_PREFIX: &str = "未命名 ";
/// Longest file name stem made from a heading, in characters.
pub const MAX_STEM_CHARS: usize = 80;
/// How many `base N` names are tried before giving up on a free name.
const MAX_DEDUP: usize = 1000;

/// Whether a file name stem is the default name of a new note: `未命名 ` followed by digits.
pub fn is_default_name(stem: &str) -> bool {
    stem.strip_prefix(DEFAULT_PREFIX)
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Splits a vault-relative note path (`a/b/x.md`) into its folder prefix (`a/b/`, empty at the
/// root) and its file name stem (`x`).
pub fn split_note_path(path: &str) -> (&str, &str) {
    let (dir, file) = match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    };
    (dir, file.strip_suffix(".md").unwrap_or(file))
}

/// The text of the first non-empty line when that line is a level 1 heading (`# Title`), with
/// inline markup removed. `None` when the first non-empty line is not such a heading or the
/// heading is empty.
pub fn heading_text(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let rest = line.strip_prefix('#')?;
    if rest.starts_with('#') {
        return None;
    }
    let rest = rest.strip_prefix(' ')?;
    let plain = plain_text(rest.trim());
    let plain = plain.trim();
    if plain.is_empty() {
        None
    } else {
        Some(plain.to_string())
    }
}

/// Inline Markdown reduced to its visible text: `**bold**`, `*em*`, `` `code` ``, `~~strike~~`,
/// `[[target|alias]]` (the alias, else the target), `![alt](src)` and `[text](url)` (the text).
pub fn plain_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("![[").or_else(|| rest.strip_prefix("[["))
            && let Some(end) = after.find("]]")
        {
            let inner = &after[..end];
            out.push_str(inner.rsplit_once('|').map_or(inner, |(_, alias)| alias));
            rest = &after[end + 2..];
            continue;
        }
        let is_link = c == '[' || (c == '!' && rest[1..].starts_with('['));
        if is_link {
            let body = &rest[if c == '!' { 2 } else { 1 }..];
            if let Some(close) = body.find("](")
                && let Some(end) = body[close + 2..].find(')')
            {
                out.push_str(&body[..close]);
                rest = &body[close + 2 + end + 1..];
                continue;
            }
        }
        if !matches!(c, '*' | '`' | '~') {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// A heading made into a file name stem: characters Windows forbids become spaces, runs of
/// spaces collapse, leading and trailing dots and spaces go, the result is at most
/// `MAX_STEM_CHARS` characters, and a reserved device name (`CON`, `COM1`, ...) gets a `_`.
/// Empty when nothing is left.
pub fn sanitize_stem(text: &str) -> String {
    let replaced: String = text
        .chars()
        .map(|c| if is_forbidden(c) { ' ' } else { c })
        .collect();
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut stem = trim_edges(&collapsed);
    if stem.chars().count() > MAX_STEM_CHARS {
        stem = trim_edges(&stem.chars().take(MAX_STEM_CHARS).collect::<String>());
    }
    if is_reserved(&stem) {
        stem.push('_');
    }
    stem
}

fn is_forbidden(c: char) -> bool {
    c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

fn trim_edges(s: &str) -> String {
    s.trim_matches(|c: char| c.is_whitespace() || c == '.')
        .to_string()
}

/// Windows device names: `CON`, `PRN`, `AUX`, `NUL` and `COM0`-`COM9`, `LPT0`-`LPT9`, with or
/// without an extension (`CON.txt` is reserved too).
fn is_reserved(stem: &str) -> bool {
    let base = stem.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let bytes = base.as_bytes();
    bytes.len() == 4
        && (base.starts_with("COM") || base.starts_with("LPT"))
        && bytes[3].is_ascii_digit()
}

/// The file name stem for a heading, or `None` when there is nothing to rename to: the sanitised
/// heading is empty or equals the current stem. A taken name gets ` 2`, ` 3`, ... after it. `taken`
/// tells whether a stem is already used in the folder.
pub fn target_stem(
    heading: &str,
    current_stem: &str,
    taken: impl Fn(&str) -> bool,
) -> Option<String> {
    let base = sanitize_stem(heading);
    if base.is_empty() || base == current_stem {
        return None;
    }
    if !taken(&base) {
        return Some(base);
    }
    (2..MAX_DEDUP)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| !taken(candidate))
}

/// What the automatic rename needs to know about a note when its typing pauses.
#[derive(Clone, Copy)]
pub struct Check<'a> {
    /// The setting `新建笔记时用首个标题作为文件名` is on.
    pub enabled: bool,
    /// The note was created in this session (not persisted, so restarts disable it).
    pub created_here: bool,
    /// File name stem of the note now.
    pub stem: &'a str,
    /// File name stem that the automatic rename produced last, when it renamed the note before.
    pub auto_stem: Option<&'a str>,
    /// Plain text of the first heading, when the first line is one.
    pub heading: Option<&'a str>,
    /// Heading of the last try (a failed try, or the heading behind the last automatic rename).
    pub last_tried: Option<&'a str>,
    /// The editor is in the middle of an IME composition.
    pub composing: bool,
}

/// What to do with a note when its typing pauses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing now; the note stays in the session.
    Keep,
    /// The note is left out of the automatic rename from now on.
    Drop,
    /// Rename the note to this file name stem.
    Rename(String),
}

/// The decision for a note at a pause. `taken` tells whether a stem is used in the note's folder.
///
/// A note that was never renamed automatically is renamed only while its name is still the
/// default `未命名 N`. A note whose name was produced by an automatic rename is renamed again
/// when its heading changed, as long as its name still equals that produced name; any other name
/// means the user renamed it, and the note is dropped for good.
pub fn decide(c: &Check, taken: impl Fn(&str) -> bool) -> Action {
    if !c.enabled || !c.created_here {
        return Action::Keep;
    }
    match c.auto_stem {
        Some(auto) if c.stem != auto => return Action::Drop,
        None if !is_default_name(c.stem) => return Action::Drop,
        _ => {}
    }
    if c.composing {
        return Action::Keep;
    }
    let Some(heading) = c.heading.filter(|h| !h.trim().is_empty()) else {
        return Action::Keep;
    };
    if c.last_tried == Some(heading) {
        return Action::Keep;
    }
    match target_stem(heading, c.stem, taken) {
        Some(stem) => Action::Rename(stem),
        None => Action::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_name_detection() {
        assert!(is_default_name("未命名 1"));
        assert!(is_default_name("未命名 12"));
        assert!(!is_default_name("未命名"));
        assert!(!is_default_name("未命名 "));
        assert!(!is_default_name("未命名 6 2"));
        assert!(!is_default_name("未命名 a"));
        assert!(!is_default_name("我的未命名 3"));
        assert!(!is_default_name("Untitled 3"));
    }

    #[test]
    fn note_path_split() {
        assert_eq!(split_note_path("未命名 6.md"), ("", "未命名 6"));
        assert_eq!(split_note_path("日记/未命名 6.md"), ("日记/", "未命名 6"));
        assert_eq!(split_note_path("a/b/x.md"), ("a/b/", "x"));
    }

    #[test]
    fn heading_is_the_first_non_empty_line() {
        assert_eq!(
            heading_text("# 我的第一篇\n\n正文"),
            Some("我的第一篇".into())
        );
        assert_eq!(heading_text("\n\n  # 标题  \n"), Some("标题".into()));
        assert_eq!(heading_text("正文\n# 标题"), None);
        assert_eq!(heading_text(""), None);
        assert_eq!(heading_text("#"), None);
        assert_eq!(heading_text("# "), None);
        assert_eq!(heading_text("## 二级"), None);
        assert_eq!(heading_text("#标签"), None);
    }

    #[test]
    fn inline_markup_is_stripped() {
        assert_eq!(heading_text("# **加粗** 标题"), Some("加粗 标题".into()));
        assert_eq!(
            heading_text("# 看 [[笔记A|别名]] 与 [[笔记B]]"),
            Some("看 别名 与 笔记B".into())
        );
        assert_eq!(
            heading_text("# [链接](https://x.y) 与 ![图](a.png)"),
            Some("链接 与 图".into())
        );
        assert_eq!(heading_text("# `代码` ~~删除~~"), Some("代码 删除".into()));
        assert_eq!(heading_text("# **  **"), None);
    }

    #[test]
    fn forbidden_characters_become_spaces() {
        assert_eq!(
            sanitize_stem("a\\b/c:d*e?f\"g<h>i|j"),
            "a b c d e f g h i j"
        );
        assert_eq!(sanitize_stem("  多   个  空格  "), "多 个 空格");
        assert_eq!(sanitize_stem("tab\tand\ncontrol"), "tab and control");
    }

    #[test]
    fn trailing_dots_and_spaces_are_removed() {
        assert_eq!(sanitize_stem("标题..."), "标题");
        assert_eq!(sanitize_stem("标题. . "), "标题");
        assert_eq!(sanitize_stem(".隐藏"), "隐藏");
        assert_eq!(sanitize_stem("..."), "");
    }

    #[test]
    fn reserved_names_get_an_underscore() {
        assert_eq!(sanitize_stem("CON"), "CON_");
        assert_eq!(sanitize_stem("nul"), "nul_");
        assert_eq!(sanitize_stem("COM1"), "COM1_");
        assert_eq!(sanitize_stem("lpt9"), "lpt9_");
        assert_eq!(sanitize_stem("CON.txt"), "CON.txt_");
        assert_eq!(sanitize_stem("COM10"), "COM10");
        assert_eq!(sanitize_stem("CONSOLE"), "CONSOLE");
        assert_eq!(sanitize_stem("console notes"), "console notes");
    }

    #[test]
    fn long_headings_are_cut_to_80_characters() {
        let long: String = "字".repeat(100);
        let stem = sanitize_stem(&long);
        assert_eq!(stem.chars().count(), MAX_STEM_CHARS);
        let ascii = "a".repeat(120);
        assert_eq!(sanitize_stem(&ascii).chars().count(), MAX_STEM_CHARS);
        // Cutting right after a dot does not leave a trailing dot.
        let dotted = format!("{}.{}", "a".repeat(79), "b".repeat(10));
        assert_eq!(sanitize_stem(&dotted), "a".repeat(79));
    }

    #[test]
    fn empty_heading_gives_no_name() {
        assert_eq!(target_stem("  ", "未命名 1", |_| false), None);
        assert_eq!(target_stem("/:*", "未命名 1", |_| false), None);
    }

    #[test]
    fn same_name_gives_no_rename() {
        assert_eq!(target_stem("未命名 1", "未命名 1", |_| false), None);
    }

    #[test]
    fn taken_names_get_a_number() {
        let taken = |s: &str| s == "我的笔记" || s == "我的笔记 2";
        assert_eq!(
            target_stem("我的笔记", "未命名 1", taken),
            Some("我的笔记 3".into())
        );
        assert_eq!(
            target_stem("新标题", "未命名 1", taken),
            Some("新标题".into())
        );
    }

    #[test]
    fn cjk_heading_is_the_name() {
        assert_eq!(
            target_stem("我的第一篇", "未命名 6", |_| false),
            Some("我的第一篇".to_string())
        );
    }

    #[test]
    fn first_rename_of_a_default_name() {
        let base = Check {
            enabled: true,
            created_here: true,
            stem: "未命名 6",
            auto_stem: None,
            heading: Some("我的第一篇"),
            last_tried: None,
            composing: false,
        };
        assert_eq!(
            decide(&base, |_| false),
            Action::Rename("我的第一篇".into())
        );
        // Off, not from this session, or composing: nothing happens.
        let off = Check {
            enabled: false,
            ..base
        };
        assert_eq!(decide(&off, |_| false), Action::Keep);
        let old = Check {
            created_here: false,
            ..base
        };
        assert_eq!(decide(&old, |_| false), Action::Keep);
        let composing = Check {
            composing: true,
            ..base
        };
        assert_eq!(decide(&composing, |_| false), Action::Keep);
        // No heading, or a blank one.
        assert_eq!(
            decide(
                &Check {
                    heading: None,
                    ..base
                },
                |_| false
            ),
            Action::Keep
        );
        assert_eq!(
            decide(
                &Check {
                    heading: Some(" "),
                    ..base
                },
                |_| false
            ),
            Action::Keep
        );
        // A failed try with the same heading is not repeated; a changed heading is tried again.
        let tried = Check {
            last_tried: Some("我的第一篇"),
            ..base
        };
        assert_eq!(decide(&tried, |_| false), Action::Keep);
        let retry = Check {
            last_tried: Some("旧标题"),
            ..base
        };
        assert_eq!(
            decide(&retry, |_| false),
            Action::Rename("我的第一篇".into())
        );
    }

    #[test]
    fn heading_changed_after_an_automatic_rename_renames_again() {
        // Typing paused after `# 我`: the note became `我`; the heading then grows.
        let c = Check {
            enabled: true,
            created_here: true,
            stem: "我",
            auto_stem: Some("我"),
            heading: Some("我的第一篇"),
            last_tried: Some("我"),
            composing: false,
        };
        assert_eq!(decide(&c, |_| false), Action::Rename("我的第一篇".into()));
    }

    #[test]
    fn user_rename_in_between_drops_the_note() {
        let c = Check {
            enabled: true,
            created_here: true,
            stem: "书名",
            auto_stem: Some("我"),
            heading: Some("我的第一篇"),
            last_tried: Some("我"),
            composing: false,
        };
        assert_eq!(decide(&c, |_| false), Action::Drop);
        // Dropped for good: even a default-looking name does not bring it back.
        let back = Check {
            stem: "未命名 2",
            ..c
        };
        assert_eq!(decide(&back, |_| false), Action::Drop);
        // A user rename of a note that was never auto-renamed also drops it.
        let plain = Check {
            stem: "我的笔记",
            auto_stem: None,
            ..c
        };
        assert_eq!(decide(&plain, |_| false), Action::Drop);
    }

    #[test]
    fn unchanged_heading_keeps_the_note() {
        let c = Check {
            enabled: true,
            created_here: true,
            stem: "我的第一篇",
            auto_stem: Some("我的第一篇"),
            heading: Some("我的第一篇"),
            last_tried: Some("我的第一篇"),
            composing: false,
        };
        assert_eq!(decide(&c, |_| false), Action::Keep);
    }

    #[test]
    fn emptied_heading_keeps_the_produced_name() {
        let base = Check {
            enabled: true,
            created_here: true,
            stem: "我",
            auto_stem: Some("我"),
            heading: None,
            last_tried: Some("我"),
            composing: false,
        };
        assert_eq!(decide(&base, |_| false), Action::Keep);
        assert_eq!(
            decide(
                &Check {
                    heading: Some("  "),
                    ..base
                },
                |_| false
            ),
            Action::Keep
        );
        // The note is still in the session: a new heading renames it again.
        let back = Check {
            heading: Some("新标题"),
            ..base
        };
        assert_eq!(decide(&back, |_| false), Action::Rename("新标题".into()));
    }

    #[test]
    fn dedup_collision_gets_a_number() {
        let c = Check {
            enabled: true,
            created_here: true,
            stem: "未命名 6",
            auto_stem: None,
            heading: Some("我的第一篇"),
            last_tried: None,
            composing: false,
        };
        let taken = |s: &str| s == "我的第一篇";
        assert_eq!(decide(&c, taken), Action::Rename("我的第一篇 2".into()));
        // The heading equals the note's own stem: nothing to do.
        let same = Check {
            stem: "我的第一篇",
            auto_stem: Some("我的第一篇"),
            last_tried: Some("旧"),
            ..c
        };
        assert_eq!(decide(&same, |_| false), Action::Keep);
    }
}
