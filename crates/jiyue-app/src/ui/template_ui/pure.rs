//! Pure logic of the templates: which folder holds them, filtering the picker, the caret after an
//! insertion and the cycle of the daily template. No I/O (the caller reads the folder).

use jiyue_export::{Moment, TemplateInfo};

use crate::ui::history_ui::pure::local_parts;

/// Folder used when the settings do not name one.
pub const DEFAULT_FOLDER: &str = "模板";
/// Folder used when the vault has this one and not the default.
pub const ENGLISH_FOLDER: &str = "Templates";
/// Title of a note created from a template before it is renamed.
pub const NEW_NOTE_TITLE: &str = "无标题";

/// The folder the templates are read from: the configured one when it exists; otherwise the
/// default, then `Templates` when that one exists. `exists` tells whether a vault-relative folder
/// is there.
pub fn choose_folder(configured: &str, exists: impl Fn(&str) -> bool) -> String {
    let configured = configured.trim();
    let configured = if configured.is_empty() {
        DEFAULT_FOLDER
    } else {
        configured
    };
    if exists(configured) {
        return configured.to_owned();
    }
    if configured == DEFAULT_FOLDER && exists(ENGLISH_FOLDER) {
        return ENGLISH_FOLDER.to_owned();
    }
    configured.to_owned()
}

/// Indexes of the templates that match `query` (case-insensitive, in name or path), in order. An
/// empty query matches everything.
pub fn filter(items: &[TemplateInfo], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            q.is_empty() || t.name.to_lowercase().contains(&q) || t.path.to_lowercase().contains(&q)
        })
        .map(|(i, _)| i)
        .collect()
}

/// Local wall-clock moment of a Unix time in milliseconds.
pub fn moment(ms: i64) -> Option<Moment> {
    let (y, mo, d, h, mi) = local_parts(ms)?;
    Some(Moment {
        y,
        m: mo as u32,
        d: d as u32,
        h: h as u32,
        mi: mi as u32,
        s: 0,
    })
}

/// Zero-based (row, column) of a byte offset in `text`. The column counts characters, as the
/// editor's caret does.
pub fn row_col(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = text.get(..offset).unwrap_or(text);
    let row = before.matches('\n').count();
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = before[line_start..].chars().count();
    (row, col)
}

/// Where the caret goes after a template is inserted at `start`: at `{{cursor}}` when the template
/// has one, otherwise after the inserted text.
pub fn caret_after_insert(
    start: usize,
    inserted_len: usize,
    cursor_offset: Option<usize>,
) -> usize {
    match cursor_offset {
        Some(off) => start + off.min(inserted_len),
        None => start + inserted_len,
    }
}

/// The template the daily-note setting shows after one click: `None` (built-in) → the first
/// template, each template → the next, the last one → built-in again. `current` is the template
/// path now set (`None` for the built-in one).
pub fn next_daily_template(current: Option<&str>, templates: &[TemplateInfo]) -> Option<String> {
    if templates.is_empty() {
        return None;
    }
    let next = match current.and_then(|c| templates.iter().position(|t| t.path == c)) {
        None => Some(0),
        Some(i) if i + 1 < templates.len() => Some(i + 1),
        Some(_) => None,
    };
    next.map(|i| templates[i].path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiyue_export::{Locale, TemplateCtx, render_template};

    fn info(name: &str, folder: &str) -> TemplateInfo {
        TemplateInfo {
            name: name.to_owned(),
            path: format!("{folder}/{name}.md"),
        }
    }

    #[test]
    fn folder_prefers_the_configured_one_then_the_english_fallback() {
        let only_english = |f: &str| f == "Templates";
        assert_eq!(choose_folder("", only_english), "Templates");
        assert_eq!(choose_folder("模板", only_english), "Templates");
        assert_eq!(
            choose_folder("Tpl", only_english),
            "Tpl",
            "explicit names are kept"
        );
        let none = |_: &str| false;
        assert_eq!(choose_folder("", none), "模板");
        let both = |f: &str| f == "模板" || f == "Templates";
        assert_eq!(choose_folder("", both), "模板");
    }

    #[test]
    fn filter_matches_name_and_path_ignoring_case() {
        let items = vec![
            info("会议记录", "模板"),
            info("Daily", "Templates"),
            info("读书", "模板"),
        ];
        assert_eq!(filter(&items, ""), vec![0, 1, 2]);
        assert_eq!(filter(&items, "会议"), vec![0]);
        assert_eq!(filter(&items, "daily"), vec![1]);
        assert_eq!(filter(&items, "templates"), vec![1], "path matches too");
        assert!(filter(&items, "zzz").is_empty());
    }

    #[test]
    fn row_and_column_are_zero_based_in_characters() {
        let text = "ab\n中文\nxyz";
        assert_eq!(row_col(text, 0), (0, 0));
        assert_eq!(row_col(text, 2), (0, 2));
        assert_eq!(row_col(text, 3), (1, 0));
        // Two Chinese characters are six bytes: the caret is after 2 characters on row 1.
        assert_eq!(row_col(text, 3 + 6), (1, 2));
        assert_eq!(
            row_col(text, 999),
            (2, 3),
            "offsets past the end are clamped"
        );
    }

    #[test]
    fn caret_goes_to_the_cursor_marker_or_the_end() {
        assert_eq!(caret_after_insert(10, 20, Some(7)), 17);
        assert_eq!(caret_after_insert(10, 20, None), 30);
        assert_eq!(
            caret_after_insert(10, 5, Some(99)),
            15,
            "clamped to the inserted text"
        );
    }

    #[test]
    fn daily_template_cycles_through_the_list_and_back_to_built_in() {
        let list = vec![info("a", "模板"), info("b", "模板")];
        assert_eq!(
            next_daily_template(None, &list).as_deref(),
            Some("模板/a.md")
        );
        assert_eq!(
            next_daily_template(Some("模板/a.md"), &list).as_deref(),
            Some("模板/b.md")
        );
        assert_eq!(next_daily_template(Some("模板/b.md"), &list), None);
        assert_eq!(
            next_daily_template(Some("模板/gone.md"), &list).as_deref(),
            Some("模板/a.md")
        );
        assert_eq!(next_daily_template(None, &[]), None);
    }

    #[test]
    fn moment_gives_local_fields() {
        let m = moment(0).expect("epoch");
        assert!(m.y == 1969 || m.y == 1970);
        assert_eq!(m.s, 0);
    }

    #[test]
    fn template_variables_expand_with_the_note_title_and_time() {
        let now = Moment {
            y: 2026,
            m: 11,
            d: 5,
            h: 14,
            mi: 30,
            s: 0,
        };
        let ctx = TemplateCtx::new(now, "沉积的想法", "想法/沉积的想法.md", "vault", Locale::Zh);
        let out = render_template(
            "# {{title}}\n{{date}} {{time}}\n{{datetime}}\n{{cursor}}done",
            &ctx,
        );
        assert_eq!(
            out.text,
            "# 沉积的想法\n2026-11-05 14:30\n2026-11-05 14:30\ndone"
        );
        assert_eq!(
            out.cursor_offset,
            Some("# 沉积的想法\n2026-11-05 14:30\n2026-11-05 14:30\n".len())
        );
    }
}
