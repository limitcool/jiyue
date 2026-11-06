//! Pure rules of the 属性 panel: what a note's frontmatter shows, and the text edits behind the
//! panel's buttons. Each edit returns the new note text; the caller writes it through the editor.

use std::collections::HashSet;

use jiyue_md::frontmatter::{self, Property, Value};

/// What the note's frontmatter block holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// No `---` block at the top of the note.
    None,
    /// A block that is not usable YAML (or not a mapping). Nothing is edited.
    Broken,
    /// The top-level properties, in document order.
    Props(Vec<Property>),
}

/// Reads the frontmatter block of a note.
pub fn block_state(text: &str) -> Block {
    let (fm, _) = frontmatter::split(text);
    let Some(fm) = fm else {
        // `split` reports a block it cannot use as no block at all. A `---` block that is still
        // delimited is broken, not absent.
        return match raw_block(text) {
            Some(raw) if !only_comments(&raw) => Block::Broken,
            _ => Block::None,
        };
    };
    let props = frontmatter::properties(text);
    if props.is_empty() && !only_comments(&fm.raw) {
        Block::Broken
    } else {
        Block::Props(props)
    }
}

/// Number of lines of the `---` block at the top of the note, both delimiter lines included.
/// Zero when the note has no delimited block. The preview blanks these lines (the 属性 panel
/// shows them), and the blank lines keep the source line numbers.
pub fn leading_block_lines(text: &str) -> usize {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return 0;
    }
    for (i, line) in lines.enumerate() {
        if matches!(line.trim_end(), "---" | "...") {
            return i + 2;
        }
    }
    0
}

/// The text between the `---` lines at the top of the note, when both delimiters are there.
fn raw_block(text: &str) -> Option<String> {
    let mut lines = text.split_inclusive('\n');
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let mut raw = String::new();
    for line in lines {
        let t = line.trim_end();
        if t == "---" || t == "..." {
            return Some(raw);
        }
        raw.push_str(line);
    }
    None
}

/// True when the block holds nothing but blank lines and comments.
fn only_comments(raw: &str) -> bool {
    raw.lines()
        .all(|l| l.trim().is_empty() || l.trim_start().starts_with('#'))
}

/// Text shown for a value (lists joined with `、`).
pub fn value_text(value: &Value) -> String {
    match value {
        Value::Text(s) | Value::Date(s) | Value::DateTime(s) => s.clone(),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                format!("{n}")
            }
        }
        Value::Bool(b) => if *b { "是" } else { "否" }.to_string(),
        Value::List(items) => items.join("、"),
        Value::Null => String::new(),
    }
}

/// The note text with property `key` set to the opposite of `current`.
pub fn toggle_bool(text: &str, key: &str, current: bool) -> String {
    frontmatter::set_property(text, key, &Value::Bool(!current))
}

/// The note text without property `key`.
pub fn delete_property(text: &str, key: &str) -> String {
    frontmatter::remove_property(text, key)
}

/// The note text without `item` in the list property `key`.
pub fn remove_list_item(text: &str, key: &str, item: &str) -> String {
    frontmatter::remove_from_list(text, key, item)
}

/// How an inline-edited property value is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Number,
    Date,
}

/// The inline-edit kind of a value, or `None` for values edited elsewhere (checkboxes, lists).
pub fn kind_of(value: &Value) -> Option<Kind> {
    match value {
        Value::Text(_) => Some(Kind::Text),
        Value::Number(_) => Some(Kind::Number),
        Value::Date(_) => Some(Kind::Date),
        _ => None,
    }
}

/// Turns the text typed into an inline field into a typed value. The error is a hint for the
/// user (Simplified Chinese), shown under the field.
pub fn parse_value(kind: Kind, raw: &str) -> Result<Value, &'static str> {
    let raw = raw.trim();
    match kind {
        Kind::Text => Ok(Value::Text(raw.to_string())),
        Kind::Number => match raw.parse::<f64>() {
            Ok(n) if n.is_finite() => Ok(Value::Number(n)),
            _ => Err("请输入数字，例如 3 或 2.5"),
        },
        Kind::Date => {
            if is_valid_date(raw) {
                Ok(Value::Date(raw.to_string()))
            } else {
                Err("日期格式应为 2026-11-05")
            }
        }
    }
}

/// True for `YYYY-MM-DD` with a real calendar day (leap years included).
pub fn is_valid_date(s: &str) -> bool {
    if !s.is_ascii() || s.len() != 10 || s.as_bytes()[4] != b'-' || s.as_bytes()[7] != b'-' {
        return false;
    }
    let part = |range: std::ops::Range<usize>| s[range].parse::<u32>().ok();
    let (Some(year), Some(month), Some(day)) = (part(0..4), part(5..7), part(8..10)) else {
        return false;
    };
    if !(1..=12).contains(&month) || day == 0 {
        return false;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    day <= days
}

/// The tags typed into the 添加标签 field. Spaces, commas (`,` `，` `、` `;` `；`) separate tags, so
/// one entry can add several. A leading `#` is dropped from each. Empty and repeated tags are
/// dropped; the order is kept.
pub fn tags_from_input(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in
        raw.split(|c: char| c.is_whitespace() || matches!(c, ',' | '，' | '、' | ';' | '；'))
    {
        let tag = part.trim().trim_start_matches('#').trim();
        if !tag.is_empty() && !out.iter().any(|t| t == tag) {
            out.push(tag.to_string());
        }
    }
    out
}

/// The last tag of the note's `tags` property: the end of the list, or of a comma/space string.
/// `None` when there are no tags.
pub fn last_tag(text: &str) -> Option<String> {
    let prop = frontmatter::properties(text)
        .into_iter()
        .find(|p| p.key == "tags")?;
    match prop.value {
        Value::List(items) => items.last().cloned(),
        Value::Text(s) => frontmatter::split_tokens(&s).last().cloned(),
        _ => None,
    }
}

/// Tags shown as chips for a `tags` property written as one string (`tags: a, b`). Empty for any
/// other value; lists are shown from their items.
pub fn tag_chips(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Text(s) => Some(frontmatter::split_tokens(s)),
        _ => None,
    }
}

/// True for the properties that hold tags.
pub fn is_tag_key(key: &str) -> bool {
    key == "tags" || key == "tag"
}

/// Checks a new property name typed in the rename field. `old` is the current name (it may keep
/// its own name), `others` are the other names of the note (compared case-sensitively). Returns the
/// trimmed name, or a hint for the user.
pub fn check_key(raw: &str, old: &str, others: &[String]) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("名称不能为空");
    }
    if name == "---" || name == "..." {
        return Err("名称不能是 ---");
    }
    if name.chars().count() > 80 {
        return Err("名称最多 80 个字");
    }
    if name.chars().any(char::is_control) {
        return Err("名称不能包含换行或控制字符");
    }
    if name != old && others.iter().any(|o| o == name) {
        return Err("已有同名属性");
    }
    Ok(name.to_string())
}

/// The property names of a note, in document order.
pub fn keys(text: &str) -> Vec<String> {
    frontmatter::properties(text)
        .into_iter()
        .map(|p| p.key)
        .collect()
}

/// The note text with a new empty text property named 属性, 属性 2, ... (not used yet).
pub fn add_property(text: &str) -> String {
    let used: HashSet<String> = frontmatter::properties(text)
        .into_iter()
        .map(|p| p.key)
        .collect();
    let key = (1..)
        .map(|n| {
            if n == 1 {
                "属性".to_string()
            } else {
                format!("属性 {n}")
            }
        })
        .find(|k| !used.contains(k))
        .unwrap_or_else(|| "属性".to_string());
    frontmatter::set_property(text, &key, &Value::Text(String::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str =
        "---\n# 个人备注\ntitle: 标题\ndone: false\ntags: [读书, 随笔]\n---\n# 标题\n正文\n";

    fn prop<'a>(props: &'a [Property], key: &str) -> Option<&'a Value> {
        props.iter().find(|p| p.key == key).map(|p| &p.value)
    }

    #[test]
    fn block_states() {
        assert_eq!(block_state("# 没有属性\n"), Block::None);
        assert!(matches!(block_state(NOTE), Block::Props(_)));
        // Unclosed list: not valid YAML.
        assert_eq!(
            block_state("---\nkey: [unclosed\n---\n正文\n"),
            Block::Broken
        );
        // A block with only comments is not an error.
        assert_ne!(block_state("---\n# 只有注释\n---\n"), Block::Broken);
        // A horizontal rule at the top with no closing `---` is not a block.
        assert_eq!(block_state("---\n正文\n"), Block::None);
    }

    #[test]
    fn toggle_keeps_comments_and_other_lines() {
        let next = toggle_bool(NOTE, "done", false);
        assert!(next.contains("# 个人备注\n"));
        assert!(next.contains("title: 标题\n"));
        assert!(next.contains("tags: [读书, 随笔]\n"));
        assert!(next.ends_with("---\n# 标题\n正文\n"));
        match block_state(&next) {
            Block::Props(props) => assert_eq!(prop(&props, "done"), Some(&Value::Bool(true))),
            other => panic!("unexpected {other:?}"),
        }
        // And back again.
        let back = toggle_bool(&next, "done", true);
        assert_eq!(back, NOTE);
    }

    #[test]
    fn delete_removes_only_that_property() {
        let next = delete_property(NOTE, "title");
        assert!(!next.contains("title:"));
        assert!(next.contains("done: false\n"));
        assert!(next.contains("# 个人备注\n"));
        assert!(next.ends_with("正文\n"));
    }

    #[test]
    fn list_items_are_removed_one_at_a_time() {
        let next = remove_list_item(NOTE, "tags", "读书");
        match block_state(&next) {
            Block::Props(props) => {
                assert_eq!(
                    prop(&props, "tags"),
                    Some(&Value::List(vec!["随笔".to_string()]))
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(next.contains("# 个人备注\n"));
    }

    #[test]
    fn add_property_uses_a_free_name() {
        let once = add_property(NOTE);
        assert!(once.contains("属性:"));
        match block_state(&once) {
            Block::Props(props) => assert!(prop(&props, "属性").is_some()),
            other => panic!("unexpected {other:?}"),
        }
        let twice = add_property(&once);
        match block_state(&twice) {
            Block::Props(props) => {
                assert!(prop(&props, "属性").is_some());
                assert!(prop(&props, "属性 2").is_some());
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(twice.contains("# 个人备注\n"));
    }

    #[test]
    fn add_property_to_a_note_without_frontmatter_creates_the_block() {
        let next = add_property("# 无属性\n正文\n");
        assert!(next.starts_with("---\n"));
        assert!(next.ends_with("# 无属性\n正文\n"));
        assert!(matches!(block_state(&next), Block::Props(p) if p.len() == 1));
    }

    #[test]
    fn leading_block_line_count() {
        assert_eq!(leading_block_lines(NOTE), 6);
        assert_eq!(leading_block_lines("---\na: 1\n---\n正文"), 3);
        assert_eq!(leading_block_lines("---\nkey: [unclosed\n---\n"), 3);
        assert_eq!(leading_block_lines("---\n正文没有结束"), 0);
        assert_eq!(leading_block_lines("# 无\n---\n"), 0);
    }

    #[test]
    fn values_read_as_text() {
        assert_eq!(value_text(&Value::Number(3.0)), "3");
        assert_eq!(value_text(&Value::Number(2.5)), "2.5");
        assert_eq!(value_text(&Value::Bool(true)), "是");
        assert_eq!(
            value_text(&Value::List(vec!["a".into(), "b".into()])),
            "a、b"
        );
        assert_eq!(value_text(&Value::Date("2026-11-05".into())), "2026-11-05");
        assert_eq!(value_text(&Value::Null), "");
    }

    #[test]
    fn inline_values_are_typed() {
        assert_eq!(parse_value(Kind::Number, " 2.5 "), Ok(Value::Number(2.5)));
        assert_eq!(parse_value(Kind::Number, "3"), Ok(Value::Number(3.0)));
        assert!(parse_value(Kind::Number, "三").is_err());
        assert!(parse_value(Kind::Number, "").is_err());
        assert!(parse_value(Kind::Number, "inf").is_err());
        assert_eq!(
            parse_value(Kind::Date, "2026-11-05"),
            Ok(Value::Date("2026-11-05".into()))
        );
        assert!(parse_value(Kind::Date, "2026-02-29").is_err());
        assert_eq!(
            parse_value(Kind::Date, "2028-02-29"),
            Ok(Value::Date("2028-02-29".into()))
        );
        assert!(parse_value(Kind::Date, "2026/11/05").is_err());
        assert_eq!(
            parse_value(Kind::Text, "  你好 "),
            Ok(Value::Text("你好".into()))
        );
        assert_eq!(kind_of(&Value::Bool(true)), None);
        assert_eq!(kind_of(&Value::Number(1.0)), Some(Kind::Number));
    }

    #[test]
    fn dates_need_a_real_day() {
        assert!(is_valid_date("2026-12-31"));
        assert!(!is_valid_date("2026-04-31"));
        assert!(!is_valid_date("2026-13-01"));
        assert!(!is_valid_date("2026-00-10"));
        assert!(!is_valid_date("1900-02-29"));
        assert!(is_valid_date("2000-02-29"));
        assert!(!is_valid_date("２０２６-11-05"));
    }

    #[test]
    fn tags_from_the_add_field() {
        assert_eq!(tags_from_input("  #读书  "), vec!["读书".to_string()]);
        // Spaces and commas (ASCII and CJK) separate several tags at once.
        assert_eq!(
            tags_from_input("读书, 随笔，项目/霁月 #读书 、 ；学习"),
            vec![
                "读书".to_string(),
                "随笔".to_string(),
                "项目/霁月".to_string(),
                "学习".to_string()
            ]
        );
        assert!(tags_from_input("  #  ").is_empty());
        assert!(tags_from_input("").is_empty());
    }

    #[test]
    fn add_tags_creates_or_extends_the_tags_list() {
        let next = frontmatter::add_tags("# 无属性\n", "tags", &["读书".into()]);
        assert!(matches!(
            block_state(&next),
            Block::Props(ref p) if p.iter().any(|x| x.key == "tags")
        ));
        let more = frontmatter::add_tags(NOTE, "tags", &["新标签".into(), "第三".into()]);
        assert!(more.contains("新标签"));
        assert!(more.contains("第三"));
        assert!(more.contains("读书"));
        assert!(more.contains("# 个人备注\n"));
    }

    #[test]
    fn last_tag_reads_the_end_of_either_form() {
        assert_eq!(last_tag(NOTE), Some("随笔".to_string()));
        assert_eq!(
            last_tag(&fm_block("tags: 甲, 乙\n")),
            Some("乙".to_string())
        );
        assert_eq!(last_tag(&fm_block("tags: []\n")), None);
        assert_eq!(last_tag("# 无属性\n"), None);
    }

    fn fm_block(inner: &str) -> String {
        format!("---\n{inner}---\n正文\n")
    }

    #[test]
    fn chips_only_for_comma_strings() {
        assert_eq!(
            tag_chips(&Value::Text("甲, 乙 丙".into())),
            Some(vec!["甲".to_string(), "乙".to_string(), "丙".to_string()])
        );
        assert_eq!(tag_chips(&Value::List(vec!["甲".into()])), None);
        assert!(is_tag_key("tags") && is_tag_key("tag") && !is_tag_key("title"));
    }

    #[test]
    fn key_names_are_checked() {
        let others = vec!["title".to_string(), "状态".to_string()];
        assert_eq!(
            check_key("  日期  ", "title", &others),
            Ok("日期".to_string())
        );
        // Keeping its own name is fine.
        assert_eq!(
            check_key("title", "title", &others),
            Ok("title".to_string())
        );
        assert_eq!(check_key("  ", "title", &others), Err("名称不能为空"));
        assert_eq!(check_key("---", "title", &others), Err("名称不能是 ---"));
        assert_eq!(check_key("状态", "title", &others), Err("已有同名属性"));
        // Case-sensitive: a different case is another name.
        assert_eq!(
            check_key("Title", "title", &others),
            Ok("Title".to_string())
        );
        assert!(check_key("a\nb", "title", &others).is_err());
        let long: String = "长".repeat(81);
        assert!(check_key(&long, "title", &others).is_err());
    }

    #[test]
    fn renamed_keys_keep_their_values_in_the_note() {
        let text = "---\ntitle: 标题\n# 注释\nstatus: 草稿\n---\n正文\n";
        let next = frontmatter::rename_property(text, "status", "状态 2");
        assert_eq!(keys(&next), vec!["title".to_string(), "状态 2".to_string()]);
        assert!(next.contains("# 注释\n"));
    }
}
