//! Remote version history of the active note: the versions the bucket kept, a read-only view of one
//! of them with a line diff against the editor text, and 恢复此版本 as one undoable edit.

use gpui::{
    AnyElement, Context, EntityInputHandler, FontWeight, IntoElement, SharedString, Window, div,
    prelude::*, px,
};
use jiyue_sync::RemoteStore;
use similar::{ChangeTag, TextDiff};

use super::config::SyncConfig;
use super::service::open_store;
use super::{Modal, close_modal};
use crate::RustdianApp;
use crate::jobs::spawn_blocking;
use crate::theme::ActiveTheme;
use crate::ui::UI_FONT;
use crate::ui::ext::ToastKind;
use crate::ui::widgets::{BadgeKind, ButtonKind, badge, button, modal};

/// Most version rows shown.
const ROW_LIMIT: usize = 200;
/// Most diff lines shown.
const DIFF_LIMIT: usize = 400;

/// One version of the note, as listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub version_id: String,
    pub modified_ms: Option<i64>,
    pub size: u64,
    pub is_delete_marker: bool,
    pub is_latest: bool,
}

/// How a line of the diff changed, going from the current text to the chosen version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffTag {
    Same,
    /// Present in the version only: restoring adds it.
    Added,
    /// Present in the current text only: restoring removes it.
    Removed,
}

/// One diff line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub tag: DiffTag,
    pub text: String,
}

/// Line diff from `current` to `version`.
pub fn diff_lines(current: &str, version: &str) -> Vec<DiffLine> {
    TextDiff::from_lines(current, version)
        .iter_all_changes()
        .map(|change| DiffLine {
            tag: match change.tag() {
                ChangeTag::Equal => DiffTag::Same,
                ChangeTag::Insert => DiffTag::Added,
                ChangeTag::Delete => DiffTag::Removed,
            },
            text: change.value().trim_end_matches(['\r', '\n']).to_owned(),
        })
        .collect()
}

/// Loading state of the dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    Loading,
    Ready,
    /// The bucket has no versioning, so there is no history to show.
    Unsupported,
    Failed(String),
}

/// State of the version dialog.
#[derive(Default)]
pub struct History {
    pub status: Status,
    /// Vault-relative path of the note the dialog is about.
    pub path: String,
    pub rows: Vec<Row>,
    pub selected: Option<usize>,
    pub loading_version: bool,
    /// Text of the selected version (None for delete markers and while loading).
    pub text: Option<String>,
    pub diff: Vec<DiffLine>,
    /// Message under the view, e.g. why there is no text.
    pub note: String,
}

/// Opens the dialog for the active note and lists its versions in the background.
pub fn open(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(path) = app.active_path().map(str::to_owned) else {
        app.toast(ToastKind::Info, "没有打开的笔记");
        return;
    };
    if !app.ext.sync.saved.enabled {
        app.toast(ToastKind::Info, "同步未启用，请先在设置中开启");
        return;
    }
    app.ext.sync.history = History {
        status: Status::Loading,
        path: path.clone(),
        ..History::default()
    };
    app.ext.sync.modal = Modal::Versions;
    let cfg = app.ext.sync.saved.clone();
    let asked = path.clone();
    spawn_blocking(
        cx,
        move || list_versions(&cfg, &asked),
        move |app, result, cx| {
            if app.ext.sync.history.path != path {
                return;
            }
            let h = &mut app.ext.sync.history;
            match result {
                Ok(None) => h.status = Status::Unsupported,
                Ok(Some(rows)) => {
                    h.rows = rows;
                    h.status = Status::Ready;
                }
                Err(e) => h.status = Status::Failed(e),
            }
            cx.notify();
        },
    );
    cx.notify();
}

/// `Ok(None)` when the store keeps no versions.
fn list_versions(cfg: &SyncConfig, rel: &str) -> Result<Option<Vec<Row>>, String> {
    let store = open_store(cfg, "")?;
    if !store.supports_versions() {
        return Ok(None);
    }
    let versions = store.list_version_details(rel).map_err(|e| e.to_string())?;
    let rows = versions
        .into_iter()
        .filter_map(|v| {
            let version_id = v.entry.version_id.clone()?;
            Some(Row {
                version_id,
                modified_ms: v.entry.modified_ms,
                size: v.entry.size,
                is_delete_marker: v.is_delete_marker,
                is_latest: v.is_latest,
            })
        })
        .take(ROW_LIMIT)
        .collect();
    Ok(Some(rows))
}

/// Loads the text of one version (blocking, background thread).
fn get_version(cfg: &SyncConfig, rel: &str, version_id: &str) -> Result<String, String> {
    let store = open_store(cfg, "")?;
    let bytes = store
        .get(rel, Some(version_id))
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Selects a version: loads its text and builds the diff against the editor.
pub fn select(app: &mut RustdianApp, index: usize, cx: &mut Context<RustdianApp>) {
    let h = &mut app.ext.sync.history;
    let Some(row) = h.rows.get(index).cloned() else {
        return;
    };
    h.selected = Some(index);
    h.text = None;
    h.diff.clear();
    h.note.clear();
    if row.is_delete_marker {
        h.note = "这是删除标记，没有内容可以恢复。".into();
        cx.notify();
        return;
    }
    h.loading_version = true;
    let cfg = app.ext.sync.saved.clone();
    let path = app.ext.sync.history.path.clone();
    let asked = path.clone();
    spawn_blocking(
        cx,
        move || get_version(&cfg, &asked, &row.version_id),
        move |app, result, cx| {
            if app.ext.sync.history.path != path || app.ext.sync.history.selected != Some(index) {
                return;
            }
            let current = app
                .editor_for(&path)
                .map(|e| e.read(cx).text())
                .unwrap_or_default();
            let h = &mut app.ext.sync.history;
            h.loading_version = false;
            match result {
                Ok(text) => {
                    h.diff = diff_lines(&current, &text);
                    h.text = Some(text);
                }
                Err(e) => h.note = format!("无法读取此版本：{e}"),
            }
            cx.notify();
        },
    );
    cx.notify();
}

/// 恢复此版本: replaces the editor text with the version as ONE undoable edit. The normal autosave
/// and sync then upload it.
pub fn restore(app: &mut RustdianApp, window: &mut Window, cx: &mut Context<RustdianApp>) {
    let Some(text) = app.ext.sync.history.text.clone() else {
        return;
    };
    let path = app.ext.sync.history.path.clone();
    let Some(editor) = app.editor_for(&path) else {
        app.toast(ToastKind::Info, "笔记未打开，无法恢复");
        return;
    };
    editor.update(cx, |ed, cx| {
        let len = ed.text().len();
        ed.restore_selection(0, len, cx);
        ed.replace_text_in_range(None, &text, window, cx);
    });
    close_modal(app);
    app.toast(ToastKind::Success, "已恢复此版本，保存后会同步到远端");
    cx.notify();
}

/// Absolute time in UTC, `2026-11-04 15:30`.
fn absolute_utc(ms: i64) -> String {
    let raw = jiyue_sync::state::format_timestamp(ms);
    if raw.len() < 13 {
        return raw;
    }
    format!(
        "{}-{}-{} {}:{}",
        &raw[0..4],
        &raw[4..6],
        &raw[6..8],
        &raw[9..11],
        &raw[11..13]
    )
}

/// The version dialog.
pub fn render(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let h = &app.ext.sync.history;
    let title = format!("远端历史版本 · {}", h.path);

    let body = match &h.status {
        Status::Idle | Status::Loading => text(&t, t.tx3, "正在读取远端版本…").into_any_element(),
        Status::Unsupported => text(
            &t,
            t.tx2,
            "当前桶未开启版本控制。在服务器上为桶开启版本控制后，同步会保留历史版本；应用不会修改桶的设置。",
        )
        .into_any_element(),
        Status::Failed(e) => text(&t, t.st, &format!("无法读取远端版本：{e}")).into_any_element(),
        Status::Ready if h.rows.is_empty() => {
            text(&t, t.tx3, "远端还没有这篇笔记的历史版本。").into_any_element()
        }
        Status::Ready => ready_body(&t, h, cx),
    };

    let mut footer: Vec<AnyElement> = vec![
        button(
            &t,
            "关闭",
            ButtonKind::Ghost,
            None,
            cx.listener(|this, _, _, cx| {
                close_modal(this);
                cx.notify();
            }),
        )
        .into_any_element(),
    ];
    if h.text.is_some() {
        footer.push(
            button(
                &t,
                "恢复此版本",
                ButtonKind::Primary,
                None,
                cx.listener(|this, _, window, cx| {
                    restore(this, window, cx);
                }),
            )
            .into_any_element(),
        );
    }

    modal(
        &t,
        &title,
        div().w_full().font_family(UI_FONT).child(body),
        footer,
        780.0,
        cx.listener(|this, _, _, cx| {
            close_modal(this);
            cx.notify();
        }),
    )
}

/// Version list on the left, the selected version's diff on the right.
fn ready_body(t: &ActiveTheme, h: &History, cx: &mut Context<RustdianApp>) -> AnyElement {
    let mut list = div()
        .flex()
        .flex_col()
        .w(px(300.0))
        .flex_none()
        .max_h(px(420.0))
        .overflow_hidden()
        .gap(px(2.0));
    for (i, row) in h.rows.iter().enumerate() {
        let selected = h.selected == Some(i);
        let when = row
            .modified_ms
            .map(|ms| {
                format!(
                    "{} · {}",
                    super::schedule::relative_zh(jiyue_sync::state::now_ms(), ms),
                    absolute_utc(ms)
                )
            })
            .unwrap_or_else(|| "时间未知".into());
        let mut flags = div().flex().items_center().gap(px(4.0));
        if row.is_latest {
            flags = flags.child(badge(t, "最新", BadgeKind::Accent));
        }
        if row.is_delete_marker {
            flags = flags.child(badge(t, "删除标记", BadgeKind::Warn));
        }
        list = list.child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(8.0))
                .cursor_pointer()
                .bg(if selected { t.active } else { t.sheet })
                .hover(|s| s.bg(t.hover))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(t.tx)
                        .child(SharedString::from(when)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(t.tx3)
                                .child(SharedString::from(super::schedule::bytes_text(row.size))),
                        )
                        .child(flags),
                )
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        select(this, i, cx);
                    }),
                ),
        );
    }

    let mut pane = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(6.0))
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx3)
                .child(if h.loading_version {
                    "正在读取…"
                } else if h.text.is_some() {
                    "与当前笔记相比（+ 恢复后会加入，- 恢复后会去掉）"
                } else {
                    "选择左侧的版本查看差异"
                }),
        );
    if !h.note.is_empty() {
        pane = pane.child(text(t, t.st, &h.note));
    }
    if h.text.is_some() {
        let mut lines = div()
            .flex()
            .flex_col()
            .id("history_diff")
            .max_h(px(380.0))
            .overflow_y_scroll()
            .p(px(8.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(t.line_soft)
            .font_family(crate::ui::MONO_FONT)
            .text_size(px(12.0));
        if h.diff.iter().all(|l| l.tag == DiffTag::Same) {
            lines = lines.child(div().text_color(t.tx3).child("此版本与当前笔记相同。"));
        }
        for line in h.diff.iter().take(DIFF_LIMIT) {
            let (mark, color) = match line.tag {
                DiffTag::Same => (" ", t.tx3),
                DiffTag::Added => ("+", t.ok),
                DiffTag::Removed => ("-", t.st),
            };
            lines =
                lines.child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .text_color(color)
                        .child(SharedString::from(mark.to_owned()))
                        .child(div().min_w_0().child(SharedString::from(
                            if line.text.is_empty() {
                                " ".to_owned()
                            } else {
                                line.text.clone()
                            },
                        ))),
                );
        }
        if h.diff.len() > DIFF_LIMIT {
            lines = lines.child(div().text_color(t.tx3).child(SharedString::from(format!(
                "…… 还有 {} 行未显示",
                h.diff.len() - DIFF_LIMIT
            ))));
        }
        pane = pane.child(lines);
    }

    div()
        .flex()
        .items_start()
        .gap(px(14.0))
        .child(list)
        .child(pane)
        .into_any_element()
}

fn text(_t: &ActiveTheme, color: gpui::Rgba, value: &str) -> gpui::Div {
    div()
        .text_size(px(13.0))
        .text_color(color)
        .child(SharedString::from(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_marks_added_and_removed_lines() {
        let lines = diff_lines("a\nb\nc\n", "a\nB\nc\nd\n");
        let tags: Vec<DiffTag> = lines.iter().map(|l| l.tag).collect();
        assert_eq!(
            tags,
            vec![
                DiffTag::Same,
                DiffTag::Removed,
                DiffTag::Added,
                DiffTag::Same,
                DiffTag::Added
            ]
        );
        assert_eq!(lines[2].text, "B");
    }

    #[test]
    fn identical_texts_have_only_same_lines() {
        let lines = diff_lines("x\n", "x\n");
        assert!(lines.iter().all(|l| l.tag == DiffTag::Same));
    }

    #[test]
    fn absolute_time_is_readable() {
        assert_eq!(absolute_utc(0), "1970-01-01 00:00");
    }
}
