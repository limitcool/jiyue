//! What each command inserts, the caret or selection it leaves, and the one edit that puts it in
//! place of the `/query`. Pure logic.
//!
//! Block commands start a new line when the `/` follows text (`hello /h1` gives a heading on the
//! next line), and they take a blank line before the block when the line above is text, where
//! a table or a rule would otherwise join it. A list command converts the line's marker instead of
//! nesting: `- 项目 /` becomes `1. 项目 ` for 有序列表.

use std::ops::Range;

use super::Kind;
use super::trigger::line_start;
use crate::ui::text_input::footnote;

/// The date and time text that the two date commands insert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    /// `2026-11-05`
    pub date: String,
    /// `14:30`
    pub time: String,
}

/// The stamp of a local date and time.
pub fn stamp(y: i32, mo: u8, d: u8, h: u8, mi: u8) -> Stamp {
    Stamp {
        date: format!("{y:04}-{mo:02}-{d:02}"),
        time: format!("{h:02}:{mi:02}"),
    }
}

/// One edit that replaces `range` of the old text with `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    /// Byte range of the old text to replace: the `/query`, or the line part up to the caret.
    pub range: Range<usize>,
    pub text: String,
    /// Byte range of the new document to select after the edit. Empty means only a caret.
    pub select: Range<usize>,
    /// Open the template picker after the edit (the `/query` is removed by this edit first).
    pub open_template: bool,
}

/// Markdown with an optional placeholder: `before` + `ph` + `after`. The placeholder is selected
/// after the insert, so typing replaces it.
struct Snip {
    before: String,
    ph: String,
    after: String,
    /// A blank line goes before the block when the line above is text (tables, rules).
    gap: bool,
}

impl Snip {
    fn new(before: &str, ph: &str, after: &str) -> Self {
        Self {
            before: before.to_string(),
            ph: ph.to_string(),
            after: after.to_string(),
            gap: false,
        }
    }

    fn gap(mut self) -> Self {
        self.gap = true;
        self
    }

    fn text(&self) -> String {
        format!("{}{}{}", self.before, self.ph, self.after)
    }
}

/// How a command is inserted.
enum Action {
    /// Replaces the `/query` in place.
    Inline(Snip),
    /// Replaces the `/query`, on a line of its own.
    Block(Snip),
    /// Converts the marker of the line to this one (`- `, `1. `, `- [ ] `).
    List(&'static str),
    /// Removes the `/query` and opens the template picker.
    Template,
    /// Removes the `/query`, adds the reference `[^n]` at its place and the definition at the end.
    Footnote,
}

fn action(kind: Kind, stamp: &Stamp) -> Action {
    use Kind::*;
    match kind {
        Heading1 => Action::Block(Snip::new("# ", "标题", "")),
        Heading2 => Action::Block(Snip::new("## ", "标题", "")),
        Heading3 => Action::Block(Snip::new("### ", "标题", "")),
        Bullet => Action::List("- "),
        Ordered => Action::List("1. "),
        Task => Action::List("- [ ] "),
        Quote => Action::Block(Snip::new("> ", "", "")),
        Divider => Action::Block(Snip::new("---", "", "").gap()),
        CodeBlock => Action::Block(Snip::new("```\n", "", "\n```")),
        Table => Action::Block(
            Snip::new(
                "| ",
                "列 1",
                " | 列 2 | 列 3 |\n| --- | --- | --- |\n|  |  |  |\n|  |  |  |",
            )
            .gap(),
        ),
        CalloutNote => Action::Block(Snip::new("> [!note] ", "标题", "\n> ")),
        CalloutTip => Action::Block(Snip::new("> [!tip] ", "标题", "\n> ")),
        CalloutWarning => Action::Block(Snip::new("> [!warning] ", "标题", "\n> ")),
        CalloutDanger => Action::Block(Snip::new("> [!danger] ", "标题", "\n> ")),
        Link => Action::Inline(Snip::new("[", "", "]()")),
        WikiLink => Action::Inline(Snip::new("[[", "", "]]")),
        Image | EmbedNote => Action::Inline(Snip::new("![[", "", "]]")),
        InlineMath => Action::Inline(Snip::new("$", "", "$")),
        BlockMath => Action::Block(Snip::new("$$\n", "", "\n$$")),
        Flowchart => Action::Block(Snip::new(
            "```mermaid\nflowchart TD\n    A[",
            "开始",
            "] --> B[结束]\n```",
        )),
        Sequence => Action::Block(Snip::new(
            "```mermaid\nsequenceDiagram\n    A->>B: ",
            "消息",
            "\n```",
        )),
        Today => Action::Inline(Snip::new(&stamp.date, "", "")),
        Now => Action::Inline(Snip::new(&stamp.time, "", "")),
        Template => Action::Template,
        Footnote => Action::Footnote,
        Frontmatter => Action::Block(Snip::new(
            "---\ntitle: ",
            "标题",
            &format!("\ndate: {}\ntags: []\n---", stamp.date),
        )),
    }
}

/// The edit that runs `kind` for the `/query` at `slash_at`..`caret`. The caller has checked the
/// trigger and the query (see `trigger`).
pub fn plan(text: &str, slash_at: usize, caret: usize, kind: Kind, stamp: &Stamp) -> Edit {
    let caret = caret.min(text.len()).max(slash_at);
    match action(kind, stamp) {
        Action::Inline(s) => {
            let start = slash_at + s.before.len();
            Edit {
                range: slash_at..caret,
                text: s.text(),
                select: start..start + s.ph.len(),
                open_template: false,
            }
        }
        Action::Block(s) => block_edit(text, slash_at, caret, &s),
        Action::List(marker) => list_edit(text, slash_at, caret, marker),
        Action::Template => Edit {
            range: slash_at..caret,
            text: String::new(),
            select: slash_at..slash_at,
            open_template: true,
        },
        Action::Footnote => footnote_edit(text, slash_at, caret),
    }
}

/// The footnote command as one edit of the whole note: the `/query` is taken out, the reference
/// goes at its place and the definition at the end (`footnote::plan_insert`, the same text that
/// `TextEditor::insert_footnote` makes). One edit means one undo step. The caret goes after the
/// colon of the new definition.
fn footnote_edit(text: &str, slash_at: usize, caret: usize) -> Edit {
    let before = text.get(..slash_at).unwrap_or("");
    let after = text.get(caret..).unwrap_or("");
    let removed = format!("{before}{after}");
    let plan = footnote::plan_insert(&removed, slash_at);
    Edit {
        range: 0..text.len(),
        text: plan.text,
        select: plan.caret..plan.caret,
        open_template: false,
    }
}

fn block_edit(text: &str, slash_at: usize, caret: usize, s: &Snip) -> Edit {
    let ls = line_start(text, slash_at);
    let blank = is_blank_prefix(text.get(ls..slash_at).unwrap_or(""));
    let (start, lead) = if blank {
        let gap = s.gap && prev_line_nonblank(text, ls);
        (ls, if gap { "\n" } else { "" })
    } else {
        // The `/` follows text: the block starts a new line (two, when it needs a gap).
        (slash_at, if s.gap { "\n\n" } else { "\n" })
    };
    let tail = if has_more_text(text, caret) { "\n" } else { "" };
    let at = start + lead.len() + s.before.len();
    Edit {
        range: start..caret,
        text: format!("{lead}{}{tail}", s.text()),
        select: at..at + s.ph.len(),
        open_template: false,
    }
}

fn list_edit(text: &str, slash_at: usize, caret: usize, marker: &str) -> Edit {
    let ls = line_start(text, slash_at);
    let prefix = text.get(ls..slash_at).unwrap_or("");
    let (head, body) = split_prefix(prefix);
    let new_prefix = format!("{head}{marker}{body}");
    let at = ls + new_prefix.len();
    Edit {
        range: ls..caret,
        text: new_prefix,
        select: at..at,
        open_template: false,
    }
}

/// True when a line prefix (text before the `/`) holds nothing but whitespace and a list marker.
fn is_blank_prefix(prefix: &str) -> bool {
    strip_list_marker(prefix.trim_start()).trim().is_empty()
}

/// True when the line above the line that starts at `ls` holds text.
fn prev_line_nonblank(text: &str, ls: usize) -> bool {
    if ls == 0 {
        return false;
    }
    let end = ls - 1;
    let start = text
        .get(..end)
        .and_then(|s| s.rfind('\n'))
        .map_or(0, |i| i + 1);
    text.get(start..end).is_some_and(|l| !l.trim().is_empty())
}

/// True when the caret's line has text after the caret, or the next line has text. A block that
/// is followed by such text gets a newline of its own.
fn has_more_text(text: &str, caret: usize) -> bool {
    let rest_all = text.get(caret..).unwrap_or("");
    let line_len = rest_all.find('\n').unwrap_or(rest_all.len());
    if !rest_all[..line_len].trim().is_empty() {
        return true;
    }
    let after_newline = rest_all.get(line_len + 1..).unwrap_or("");
    if line_len >= rest_all.len() {
        return false;
    }
    let next = after_newline.split('\n').next().unwrap_or("");
    !next.trim().is_empty()
}

/// Markers that a list line starts with, longest first.
const LIST_MARKERS: &[&str] = &["- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "- ", "* ", "+ "];

/// `s` without a leading list marker (`- `, `* `, `+ `, `- [ ] `, `1. `, `1) `). `s` has no leading
/// whitespace.
pub fn strip_list_marker(s: &str) -> &str {
    for m in LIST_MARKERS {
        if let Some(rest) = s.strip_prefix(m) {
            return rest;
        }
    }
    let digits = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0 {
        let rest = &s[digits..];
        if let Some(r) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return r;
        }
    }
    s
}

/// Splits a line prefix into what stays in front (indentation and block quote markers) and the
/// text after its list marker, if it has one.
fn split_prefix(prefix: &str) -> (String, &str) {
    let indent_len = prefix.len() - prefix.trim_start().len();
    let mut head = prefix[..indent_len].to_string();
    let mut rest = &prefix[indent_len..];
    while let Some(r) = rest.strip_prefix("> ") {
        head.push_str("> ");
        rest = r;
    }
    (head, strip_list_marker(rest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiyue_core::editor_state::{EditorState, Selection};

    fn st() -> Stamp {
        stamp(2026, 11, 5, 14, 30)
    }

    /// The document after `e` is applied to `text`.
    fn apply(text: &str, e: &Edit) -> String {
        format!(
            "{}{}{}",
            &text[..e.range.start],
            e.text,
            &text[e.range.end..]
        )
    }

    /// The text that the selection of `e` covers in the document after the edit.
    fn selected(text: &str, e: &Edit) -> String {
        apply(text, e)[e.select.clone()].to_string()
    }

    fn run(text: &str, slash_at: usize, caret: usize, kind: Kind) -> (String, String) {
        let e = plan(text, slash_at, caret, kind, &st());
        (apply(text, &e), selected(text, &e))
    }

    #[test]
    fn stamp_formats_date_and_time() {
        assert_eq!(st().date, "2026-11-05");
        assert_eq!(st().time, "14:30");
        assert_eq!(stamp(2026, 1, 2, 3, 4).date, "2026-01-02");
    }

    #[test]
    fn heading_on_an_empty_line_replaces_the_query_and_selects_the_title() {
        let (doc, sel) = run("/biao", 0, 5, Kind::Heading1);
        assert_eq!(doc, "# 标题");
        assert_eq!(sel, "标题");
        let (doc, _) = run("/h2", 0, 3, Kind::Heading2);
        assert_eq!(doc, "## 标题");
        let (doc, _) = run("/h3", 0, 3, Kind::Heading3);
        assert_eq!(doc, "### 标题");
    }

    #[test]
    fn block_after_text_starts_a_new_line() {
        let (doc, sel) = run("hello /h1", 6, 9, Kind::Heading1);
        assert_eq!(doc, "hello \n# 标题");
        assert_eq!(sel, "标题");
    }

    #[test]
    fn text_after_the_caret_on_the_line_follows_the_block() {
        let (doc, _) = run("/h1 tail", 0, 3, Kind::Heading1);
        assert_eq!(doc, "# 标题\n tail");
    }

    #[test]
    fn next_line_with_text_gets_a_blank_line_after_a_block() {
        let (doc, _) = run("/h1\nNEXT", 0, 3, Kind::Heading1);
        assert_eq!(doc, "# 标题\n\nNEXT");
    }

    #[test]
    fn table_after_a_paragraph_gets_a_blank_line_and_selects_the_first_header() {
        let (doc, sel) = run("text\n/bg", 5, 8, Kind::Table);
        assert_eq!(
            doc,
            "text\n\n| 列 1 | 列 2 | 列 3 |\n| --- | --- | --- |\n|  |  |  |\n|  |  |  |"
        );
        assert_eq!(sel, "列 1");
    }

    #[test]
    fn divider_after_a_paragraph_is_not_a_setext_heading() {
        let (doc, _) = run("text\n/fg", 5, 8, Kind::Divider);
        assert_eq!(doc, "text\n\n---");
    }

    #[test]
    fn list_converts_the_marker_of_a_list_line_instead_of_nesting() {
        // "- 项目 /" becomes "1. 项目 " and the caret goes after it.
        let text = "- 项目 /bt";
        let slash = "- 项目 ".len();
        let e = plan(text, slash, text.len(), Kind::Ordered, &st());
        assert_eq!(apply(text, &e), "1. 项目 ");
        assert_eq!(e.select, 10..10);
    }

    #[test]
    fn list_on_a_plain_line_adds_the_marker_before_the_text() {
        let (doc, _) = run("项目 /x", 7, 9, Kind::Bullet);
        assert_eq!(doc, "- 项目 ");
    }

    #[test]
    fn task_list_on_an_empty_line_leaves_the_caret_after_the_box() {
        let e = plan("/rw", 0, 3, Kind::Task, &st());
        assert_eq!(apply("/rw", &e), "- [ ] ");
        assert_eq!(e.select, 6..6);
    }

    #[test]
    fn list_keeps_indentation_and_quote_markers() {
        let text = "  > - 旧 /x";
        let slash = "  > - 旧 ".len();
        let e = plan(text, slash, text.len(), Kind::Task, &st());
        assert_eq!(apply(text, &e), "  > - [ ] 旧 ");
    }

    #[test]
    fn quote_and_callouts_put_the_caret_in_the_title() {
        let (doc, _) = run("/yy", 0, 3, Kind::Quote);
        assert_eq!(doc, "> ");
        let (doc, sel) = run("/ti", 0, 3, Kind::CalloutTip);
        assert_eq!(doc, "> [!tip] 标题\n> ");
        assert_eq!(sel, "标题");
        let (doc, _) = run("/wx", 0, 3, Kind::CalloutWarning);
        assert_eq!(doc, "> [!warning] 标题\n> ");
    }

    #[test]
    fn code_block_puts_the_caret_inside_the_fence() {
        let e = plan("/dm", 0, 3, Kind::CodeBlock, &st());
        let doc = apply("/dm", &e);
        assert_eq!(doc, "```\n\n```");
        assert_eq!(
            e.select,
            4..4,
            "the caret is on the blank line of the fence"
        );
    }

    #[test]
    fn link_and_wikilink_put_the_caret_between_the_brackets() {
        let (doc, _) = run("a /lj", 2, 5, Kind::Link);
        assert_eq!(doc, "a []()");
        let e = plan("a /lj", 2, 5, Kind::Link, &st());
        assert_eq!(e.select, 3..3);
        let e = plan("/sl", 0, 3, Kind::WikiLink, &st());
        assert_eq!(apply("/sl", &e), "[[]]");
        assert_eq!(e.select, 2..2);
    }

    #[test]
    fn image_and_note_embed_insert_the_embed_syntax() {
        let e = plan("/tp", 0, 3, Kind::Image, &st());
        assert_eq!(apply("/tp", &e), "![[]]");
        assert_eq!(e.select, 3..3);
        let e = plan("/qr", 0, 3, Kind::EmbedNote, &st());
        assert_eq!(apply("/qr", &e), "![[]]");
    }

    #[test]
    fn math_inline_and_block() {
        let e = plan("/hn", 0, 3, Kind::InlineMath, &st());
        assert_eq!(apply("/hn", &e), "$$");
        assert_eq!(e.select, 1..1);
        let e = plan("/gs", 0, 3, Kind::BlockMath, &st());
        assert_eq!(apply("/gs", &e), "$$\n\n$$");
        assert_eq!(e.select, 3..3);
    }

    #[test]
    fn mermaid_skeletons_select_the_first_node_label() {
        let (doc, sel) = run("/lct", 0, 4, Kind::Flowchart);
        assert!(doc.starts_with("```mermaid\nflowchart TD\n    A["));
        assert!(doc.ends_with("] --> B[结束]\n```"));
        assert_eq!(sel, "开始");
        let (doc, sel) = run("/sxt", 0, 4, Kind::Sequence);
        assert!(doc.starts_with("```mermaid\nsequenceDiagram\n    A->>B: "));
        assert_eq!(sel, "消息");
    }

    #[test]
    fn dates_insert_the_stamp_at_the_caret() {
        let (doc, _) = run("/jt", 0, 3, Kind::Today);
        assert_eq!(doc, "2026-11-05");
        let e = plan("/jt", 0, 3, Kind::Today, &st());
        assert_eq!(e.select, 10..10);
        let (doc, _) = run("/dq", 0, 3, Kind::Now);
        assert_eq!(doc, "14:30");
    }

    #[test]
    fn template_removes_the_query_and_asks_for_the_picker() {
        let text = "x /cr";
        let e = plan(text, 2, 5, Kind::Template, &st());
        assert!(e.open_template);
        assert_eq!(apply(text, &e), "x ");
    }

    #[test]
    fn frontmatter_block_at_the_document_start_selects_the_title() {
        let (doc, sel) = run("/sx", 0, 3, Kind::Frontmatter);
        assert_eq!(doc, "---\ntitle: 标题\ndate: 2026-11-05\ntags: []\n---");
        assert_eq!(sel, "标题");
    }

    #[test]
    fn footnote_takes_the_query_out_and_adds_reference_and_definition() {
        // "正文 /zj" with the caret at the end: the reference replaces the query, the definition
        // goes to the end of the note, and the caret lands after the colon.
        let text = "正文 /zj";
        let e = plan(text, 7, text.len(), Kind::Footnote, &st());
        assert_eq!(e.range, 0..text.len(), "one edit covers the whole note");
        assert_eq!(apply(text, &e), "正文 [^1]\n\n[^1]: ");
        assert_eq!(e.select.start, e.select.end, "only a caret");
        assert_eq!(e.select.start, "正文 [^1]\n\n[^1]: ".len());
        assert!(!e.open_template);
    }

    #[test]
    fn footnote_numbers_skip_used_labels() {
        let text = "a[^1] b /zj\n\n[^1]: old";
        let slash = text.find('/').expect("query");
        let e = plan(text, slash, slash + 3, Kind::Footnote, &st());
        let doc = apply(text, &e);
        assert!(doc.contains("a[^1] b [^2]"), "{doc}");
        assert!(doc.ends_with("[^2]: "), "{doc}");
    }

    #[test]
    fn undo_after_the_snippet_restores_the_typed_query() {
        // The typed `/biao` and the snippet are two undo steps: one undo gives back `/biao`.
        let mut state = EditorState::new("");
        for ch in ["/", "b", "i", "a", "o"] {
            state.insert_text(ch);
        }
        assert_eq!(state.text(), "/biao");
        let text = state.text().to_string();
        let e = plan(&text, 0, 5, Kind::Heading1, &st());
        state.replace_range(
            e.range.start,
            e.range.end,
            &e.text,
            Selection {
                anchor: e.select.start,
                head: e.select.end,
            },
        );
        assert_eq!(state.text(), "# 标题");
        assert!(state.undo());
        assert_eq!(state.text(), "/biao");
    }

    #[test]
    fn strip_list_marker_handles_bullets_tasks_and_numbers() {
        assert_eq!(strip_list_marker("- 项目"), "项目");
        assert_eq!(strip_list_marker("- [ ] 项目"), "项目");
        assert_eq!(strip_list_marker("3. 项目"), "项目");
        assert_eq!(strip_list_marker("3) 项目"), "项目");
        assert_eq!(strip_list_marker("3项目"), "3项目");
        assert_eq!(strip_list_marker("项目"), "项目");
    }
}
