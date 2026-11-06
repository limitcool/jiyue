//! Attachment settings: `.jiyue/attach.json` in the vault, and the 附件设置 modal that edits them.

use std::io;
use std::path::Path;
use std::rc::Rc;

use gpui::{AnyElement, Context, Entity, IntoElement, MouseDownEvent, div, prelude::*, px};

use crate::RustdianApp;
use crate::ui::widgets::{ButtonKind, LineInput, button, form_row, modal, segmented};
use jiyue_md::attach::{AttachFolder, AttachSettings, LinkStyle};

/// The vault configuration file (the `[attachments]` section), relative to the vault root.
pub const SETTINGS_FILE: &str = ".jiyue/config.toml";
/// Section of the vault configuration file that holds these settings.
pub const SECTION: &str = "attachments";
/// Folder name used when the field is left empty.
pub const DEFAULT_FOLDER: &str = "附件";
/// Labels of the four folder modes, in the order of `mode_index`.
pub const MODE_LABELS: [&str; 4] = ["保险库根目录", "固定文件夹", "与笔记同目录", "笔记子文件夹"];
/// Labels of the two link styles.
pub const LINK_LABELS: [&str; 2] = ["Wiki 嵌入", "Markdown"];

/// Reads the settings of the vault at `root`. A missing or broken file gives the defaults.
pub fn load(root: &Path) -> AttachSettings {
    jiyue_core::config::load_section::<AttachSettings>(&root.join(SETTINGS_FILE), SECTION).value
}

/// Writes the settings to the `[attachments]` section, keeping the rest of the file.
pub fn save(root: &Path, settings: &AttachSettings) -> io::Result<()> {
    jiyue_core::config::save_section(
        &root.join(SETTINGS_FILE),
        SECTION,
        settings,
        jiyue_core::config::templates::VAULT_CONFIG,
        &[],
    )
    .map_err(io::Error::other)
}

/// Index into `MODE_LABELS` for a folder setting.
pub fn mode_index(folder: &AttachFolder) -> usize {
    match folder {
        AttachFolder::VaultRoot => 0,
        AttachFolder::Fixed(_) => 1,
        AttachFolder::SameAsNote => 2,
        AttachFolder::SubfolderOfNote(_) => 3,
    }
}

/// The folder setting for a mode index and the name field. An empty name becomes `附件`.
pub fn folder_for(mode: usize, name: &str) -> AttachFolder {
    let name = name.trim();
    let name = if name.is_empty() {
        DEFAULT_FOLDER
    } else {
        name
    }
    .to_string();
    match mode {
        0 => AttachFolder::VaultRoot,
        1 => AttachFolder::Fixed(name),
        2 => AttachFolder::SameAsNote,
        _ => AttachFolder::SubfolderOfNote(name),
    }
}

/// The name a folder setting shows in the name field (empty when it has none).
pub fn folder_name(folder: &AttachFolder) -> String {
    match folder {
        AttachFolder::Fixed(name) | AttachFolder::SubfolderOfNote(name) => name.clone(),
        _ => String::new(),
    }
}

/// Index into `LINK_LABELS` for a link style.
pub fn link_index(style: LinkStyle) -> usize {
    match style {
        LinkStyle::WikiEmbed => 0,
        LinkStyle::Markdown => 1,
    }
}

/// The modal of the attachment settings. Empty when it is closed.
pub fn render_modal(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let state = &app.ext.attach;
    if !state.settings_open {
        return Vec::new();
    }
    let t = app.theme;
    let mode = mode_index(&state.draft.folder);
    let uses_name = mode == 1 || mode == 3;

    let pick_mode: crate::ui::widgets::controls::PickIndex =
        Rc::new(cx.listener(|this: &mut RustdianApp, idx: &usize, _, cx| {
            let name = this.ext.attach.name_text(cx);
            this.ext.attach.draft.folder = folder_for(*idx, &name);
            cx.notify();
        }));
    let pick_link: crate::ui::widgets::controls::PickIndex =
        Rc::new(cx.listener(|this: &mut RustdianApp, idx: &usize, _, cx| {
            this.ext.attach.draft.link_style = if *idx == 0 {
                LinkStyle::WikiEmbed
            } else {
                LinkStyle::Markdown
            };
            cx.notify();
        }));

    let link_mode = if state.draft.link_style == LinkStyle::WikiEmbed {
        0
    } else {
        1
    };
    let name_control: gpui::AnyElement = match &state.folder_input {
        Some(input) if uses_name => div().w(px(220.0)).child(input.clone()).into_any_element(),
        _ => div()
            .text_size(px(12.5))
            .text_color(t.tx3)
            .child(if mode == 0 || mode == 2 {
                "此方式不需要文件夹名"
            } else {
                "未填写时使用 附件"
            })
            .into_any_element(),
    };

    let body = div()
        .flex()
        .flex_col()
        .font_family(crate::ui::fonts::ui_name())
        .child(form_row(
            &t,
            "保存位置",
            "粘贴或拖入的文件保存到这里",
            segmented(&t, &MODE_LABELS, mode, pick_mode),
            false,
        ))
        .child(form_row(
            &t,
            "文件夹名",
            "固定文件夹用保险库内的路径；子文件夹是笔记所在目录下的名字",
            name_control,
            false,
        ))
        .child(form_row(
            &t,
            "链接格式",
            "插入到笔记里的链接写法",
            segmented(&t, &LINK_LABELS, link_mode, pick_link),
            true,
        ));

    let footer = vec![
        button(
            &t,
            "取消",
            ButtonKind::Ghost,
            None,
            cx.listener(|this: &mut RustdianApp, _: &MouseDownEvent, _, cx| {
                this.ext.attach.settings_open = false;
                cx.notify();
            }),
        )
        .into_any_element(),
        button(
            &t,
            "保存",
            ButtonKind::Primary,
            None,
            cx.listener(|this: &mut RustdianApp, _: &MouseDownEvent, _, cx| {
                crate::ui::attach_ui::save_settings(this, cx);
                cx.notify();
            }),
        )
        .into_any_element(),
    ];

    vec![modal(
        &t,
        "附件设置",
        body,
        footer,
        480.0,
        cx.listener(|this: &mut RustdianApp, _: &MouseDownEvent, _, cx| {
            this.ext.attach.settings_open = false;
            cx.notify();
        }),
    )]
}

/// Creates the entity behind the folder-name field of the modal.
pub fn new_name_input(cx: &mut Context<RustdianApp>) -> Entity<LineInput> {
    cx.new(|cx| LineInput::new(cx, DEFAULT_FOLDER))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn missing_or_broken_file_gives_defaults() {
        let dir = std::env::temp_dir().join("jiyue-attach-settings-missing-xyz");
        assert_eq!(load(&dir), AttachSettings::default());
        assert!(toml::from_str::<AttachSettings>("not toml [").is_err());
        assert_eq!(
            toml::from_str::<AttachSettings>("").expect("empty"),
            AttachSettings::default()
        );
    }

    #[test]
    fn partial_json_keeps_defaults_for_missing_fields() {
        let s: AttachSettings = toml::from_str("link_style = \"Markdown\"").expect("partial");
        assert_eq!(s.link_style, LinkStyle::Markdown);
        assert_eq!(s.folder, AttachSettings::default().folder);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir().join(format!(
            "jiyue-attach-settings-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&dir).unwrap();
        let s = AttachSettings {
            folder: AttachFolder::SubfolderOfNote("img".to_string()),
            link_style: LinkStyle::Markdown,
            default_pasted_prefix: "截图".to_string(),
        };
        save(&dir, &s).expect("save");
        assert_eq!(load(&dir), s);
        assert!(dir.join(".jiyue").join("config.toml").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mode_and_name_mapping() {
        assert_eq!(folder_for(0, "x"), AttachFolder::VaultRoot);
        assert_eq!(folder_for(1, " 图 "), AttachFolder::Fixed("图".to_string()));
        assert_eq!(folder_for(1, "  "), AttachFolder::Fixed("附件".to_string()));
        assert_eq!(folder_for(2, "x"), AttachFolder::SameAsNote);
        assert_eq!(
            folder_for(3, "img"),
            AttachFolder::SubfolderOfNote("img".to_string())
        );
        for folder in [
            AttachFolder::VaultRoot,
            AttachFolder::Fixed("a".to_string()),
            AttachFolder::SameAsNote,
            AttachFolder::SubfolderOfNote("b".to_string()),
        ] {
            assert_eq!(
                folder_for(mode_index(&folder), &folder_name(&folder)),
                folder
            );
        }
        assert_eq!(link_index(LinkStyle::WikiEmbed), 0);
        assert_eq!(link_index(LinkStyle::Markdown), 1);
    }
}
