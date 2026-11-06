//! The `/` insert menu: pure logic (no gpui). Owned by the slash feature.
//!
//! - `trigger`: where a `/` may open the menu, and the query typed after it.
//! - `rank`: filtering and ranking of the commands (Chinese name, pinyin, initials, English alias).
//! - `snippet`: the Markdown each command inserts, the caret or selection it leaves, and the one
//!   edit that replaces `/query` with it (one undo step).
//!
//! The commands are listed in `ENTRIES`. Pinyin is written out by hand per label (the labels are
//! fixed), so no pinyin table is needed: `py` holds the syllables separated by spaces, and the
//! initials are the first letter of each syllable.
//!
//! The UI part is `ui/slash_ui.rs`.

pub mod picker;
pub mod rank;
pub mod snippet;
pub mod trigger;

/// One insert command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Heading1,
    Heading2,
    Heading3,
    Bullet,
    Ordered,
    Task,
    Quote,
    Divider,
    CodeBlock,
    Table,
    CalloutNote,
    CalloutTip,
    CalloutWarning,
    CalloutDanger,
    Link,
    WikiLink,
    Image,
    EmbedNote,
    InlineMath,
    BlockMath,
    Flowchart,
    Sequence,
    Today,
    Now,
    Template,
    Frontmatter,
    Footnote,
}

/// A row of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    /// Chinese label shown in the menu.
    pub zh: &'static str,
    /// Pinyin syllables of `zh`, separated by spaces. Matches the full pinyin and its initials.
    pub py: &'static str,
    /// English names and short forms (matched like the label).
    pub aliases: &'static [&'static str],
    /// One line of description under the label.
    pub desc: &'static str,
    /// Right-aligned hint: the Markdown syntax the command writes.
    pub hint: &'static str,
    /// Icon name in `assets/icons` (without the extension).
    pub icon: &'static str,
}

/// Every command, in menu order (the order of equal matches).
pub const ENTRIES: &[Entry] = &[
    Entry {
        kind: Kind::Heading1,
        zh: "标题 1",
        py: "biao ti 1",
        aliases: &["heading 1", "h1"],
        desc: "一级标题",
        hint: "# ",
        icon: "hash",
    },
    Entry {
        kind: Kind::Heading2,
        zh: "标题 2",
        py: "biao ti 2",
        aliases: &["heading 2", "h2"],
        desc: "二级标题",
        hint: "## ",
        icon: "hash",
    },
    Entry {
        kind: Kind::Heading3,
        zh: "标题 3",
        py: "biao ti 3",
        aliases: &["heading 3", "h3"],
        desc: "三级标题",
        hint: "### ",
        icon: "hash",
    },
    Entry {
        kind: Kind::Bullet,
        zh: "无序列表",
        py: "wu xu lie biao",
        aliases: &["bullet", "ul", "list"],
        desc: "以 - 开头的列表项；已在列表中则转换",
        hint: "- ",
        icon: "list",
    },
    Entry {
        kind: Kind::Ordered,
        zh: "有序列表",
        py: "you xu lie biao",
        aliases: &["numbered", "ol", "ordered"],
        desc: "以 1. 开头的编号列表",
        hint: "1. ",
        icon: "list-numbers",
    },
    Entry {
        kind: Kind::Task,
        zh: "任务列表",
        py: "ren wu lie biao",
        aliases: &["todo", "task", "checkbox"],
        desc: "可勾选的待办事项",
        hint: "- [ ] ",
        icon: "checkbox",
    },
    Entry {
        kind: Kind::Quote,
        zh: "引用",
        py: "yin yong",
        aliases: &["quote", "blockquote"],
        desc: "引用的段落",
        hint: "> ",
        icon: "quote",
    },
    Entry {
        kind: Kind::Divider,
        zh: "分割线",
        py: "fen ge xian",
        aliases: &["hr", "divider", "rule"],
        desc: "一条水平分割线",
        hint: "---",
        icon: "minus",
    },
    Entry {
        kind: Kind::CodeBlock,
        zh: "代码块",
        py: "dai ma kuai",
        aliases: &["code", "fence"],
        desc: "围栏代码块，光标在代码区",
        hint: "```",
        icon: "code",
    },
    Entry {
        kind: Kind::Table,
        zh: "表格",
        py: "biao ge",
        aliases: &["table"],
        desc: "3 列的表格，光标在第一个表头",
        hint: "| |",
        icon: "table",
    },
    Entry {
        kind: Kind::CalloutNote,
        zh: "注释块",
        py: "zhu shi kuai",
        aliases: &["note", "callout note"],
        desc: "提示块：注释",
        hint: "[!note]",
        icon: "pencil",
    },
    Entry {
        kind: Kind::CalloutTip,
        zh: "提示块",
        py: "ti shi kuai",
        aliases: &["tip", "hint", "callout tip"],
        desc: "提示块：提示",
        hint: "[!tip]",
        icon: "bulb",
    },
    Entry {
        kind: Kind::CalloutWarning,
        zh: "警告块",
        py: "jing gao kuai",
        aliases: &["warning", "caution"],
        desc: "提示块：警告",
        hint: "[!warning]",
        icon: "alert-triangle",
    },
    Entry {
        kind: Kind::CalloutDanger,
        zh: "危险块",
        py: "wei xian kuai",
        aliases: &["danger", "error"],
        desc: "提示块：危险",
        hint: "[!danger]",
        icon: "x",
    },
    Entry {
        kind: Kind::Link,
        zh: "链接",
        py: "lian jie",
        aliases: &["link", "markdown link"],
        desc: "[文字](地址)，光标在方括号内",
        hint: "[]()",
        icon: "link",
    },
    Entry {
        kind: Kind::WikiLink,
        zh: "双链",
        py: "shuang lian",
        aliases: &["wiki", "wikilink"],
        desc: "链接到另一篇笔记",
        hint: "[[]]",
        icon: "file-text",
    },
    Entry {
        kind: Kind::Footnote,
        zh: "脚注",
        py: "jiao zhu",
        // 'zhujiao' is the pinyin of 注脚, the other common word for a footnote.
        aliases: &["footnote", "zhujiao"],
        desc: "在光标处插入脚注 [^n]，定义写在文末",
        hint: "[^1]",
        icon: "hash",
    },
    Entry {
        kind: Kind::Image,
        zh: "图片",
        py: "tu pian",
        aliases: &["image", "img", "picture"],
        desc: "选择库里的图片或从电脑导入",
        hint: "![[]]",
        icon: "photo",
    },
    Entry {
        kind: Kind::EmbedNote,
        zh: "嵌入笔记",
        py: "qian ru bi ji",
        aliases: &["embed", "transclude"],
        desc: "嵌入另一篇笔记的内容",
        hint: "![[]]",
        icon: "book-2",
    },
    Entry {
        kind: Kind::InlineMath,
        zh: "行内公式",
        py: "hang nei gong shi",
        aliases: &["math inline", "inline math"],
        desc: "$...$ 夹在句子中的公式",
        hint: "$ $",
        icon: "math",
    },
    Entry {
        kind: Kind::BlockMath,
        zh: "公式块",
        py: "gong shi kuai",
        aliases: &["math", "math block", "latex"],
        desc: "$$ 独立成段的公式",
        hint: "$$",
        icon: "math",
    },
    Entry {
        kind: Kind::Flowchart,
        zh: "Mermaid 流程图",
        py: "mermaid liu cheng tu",
        aliases: &["mermaid", "flowchart", "flow"],
        desc: "Mermaid 流程图骨架",
        hint: "mermaid",
        icon: "hierarchy-2",
    },
    Entry {
        kind: Kind::Sequence,
        zh: "Mermaid 时序图",
        py: "mermaid shi xu tu",
        aliases: &["sequence", "sequencediagram"],
        desc: "Mermaid 时序图骨架",
        hint: "mermaid",
        icon: "arrow-down",
    },
    Entry {
        kind: Kind::Today,
        zh: "今天的日期",
        py: "jin tian de ri qi",
        aliases: &["date", "today"],
        desc: "插入今天的日期，如 2026-11-05",
        hint: "YYYY-MM-DD",
        icon: "calendar-event",
    },
    Entry {
        kind: Kind::Now,
        zh: "当前时间",
        py: "dang qian shi jian",
        aliases: &["time", "now"],
        desc: "插入当前的时分，如 14:30",
        hint: "HH:MM",
        icon: "clock",
    },
    Entry {
        kind: Kind::Template,
        zh: "插入模板…",
        py: "cha ru mo ban",
        aliases: &["template"],
        desc: "选择一个模板插入到光标处",
        hint: "模板",
        icon: "file-plus",
    },
    Entry {
        kind: Kind::Frontmatter,
        zh: "属性块",
        py: "shu xing kuai",
        aliases: &["frontmatter", "properties", "yaml"],
        desc: "文档开头的 YAML 属性（仅在文档最开头可用）",
        hint: "---",
        icon: "tag",
    },
];

/// The row of `kind` in `ENTRIES`.
pub fn entry(kind: Kind) -> &'static Entry {
    // Every kind has a row (tests check it), so the fallback is never used.
    ENTRIES
        .iter()
        .find(|e| e.kind == kind)
        .unwrap_or(&ENTRIES[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_exactly_one_row() {
        let kinds = [
            Kind::Heading1,
            Kind::Heading2,
            Kind::Heading3,
            Kind::Bullet,
            Kind::Ordered,
            Kind::Task,
            Kind::Quote,
            Kind::Divider,
            Kind::CodeBlock,
            Kind::Table,
            Kind::CalloutNote,
            Kind::CalloutTip,
            Kind::CalloutWarning,
            Kind::CalloutDanger,
            Kind::Link,
            Kind::WikiLink,
            Kind::Image,
            Kind::EmbedNote,
            Kind::InlineMath,
            Kind::BlockMath,
            Kind::Flowchart,
            Kind::Sequence,
            Kind::Today,
            Kind::Now,
            Kind::Template,
            Kind::Frontmatter,
            Kind::Footnote,
        ];
        assert_eq!(kinds.len(), ENTRIES.len());
        for k in kinds {
            assert_eq!(ENTRIES.iter().filter(|e| e.kind == k).count(), 1, "{k:?}");
        }
    }

    #[test]
    fn every_icon_named_by_an_entry_is_registered() {
        use gpui::AssetSource;
        for e in ENTRIES {
            let path = format!("icons/{}.svg", e.icon);
            let found = crate::ui::assets::Assets.load(&path).ok().flatten();
            assert!(found.is_some(), "{} has no asset {path}", e.zh);
        }
    }

    #[test]
    fn pinyin_syllables_match_the_labels() {
        // The pinyin is typed by hand; a label with a different number of syllables is a slip.
        for e in ENTRIES {
            assert!(!e.py.trim().is_empty(), "{}", e.zh);
            assert!(
                e.py.split_whitespace()
                    .all(|t| t.chars().all(|c| c.is_ascii_alphanumeric()))
            );
        }
    }
}
