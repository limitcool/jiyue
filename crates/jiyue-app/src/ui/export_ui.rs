//! Export of notes. Owned by W3-E.
//!
//! - 导出当前笔记为 HTML…: a dialog for the theme (纸张 or 墨色), then the native save prompt with
//!   the note's title as the file name. The page is self-contained: images are embedded and
//!   formulas and Mermaid diagrams are drawn. There is no PDF engine: the hint says to print the HTML
//!   to PDF from a browser.
//! - 复制为 HTML and 复制为 Markdown (the text without its frontmatter) go to the clipboard.
//! - 导出整个笔记库为网站…: `jiyue-export` writes a static site of the whole vault to a folder.
//!
//! Testing: when `JIYUE_EXPORT_DIR` is set (tests and scripts only), the save prompt is skipped and
//! the files go to that folder (the site to `<dir>/site`). The variable is never set by the app.
//!
//! Layout: `pure` (themes, base64, file names; unit tested), `render` (the page of one note, built
//! with `jiyue-export`).

mod pure;
mod render;

pub use pure::Theme;

use std::path::PathBuf;

use gpui::{
    AnyElement, ClipboardItem, Context, FontWeight, KeyDownEvent, MouseButton, PathPromptOptions,
    SharedString, div, prelude::*, px,
};
use jiyue_export::{SiteOptions, export_vault_site};

use crate::AppCommand;
use crate::RustdianApp;
use crate::jobs::spawn_blocking;
use crate::ui::ext::{ExtCommand, PaletteCommand, ToastAction, ToastKind};
use crate::ui::widgets::{ButtonKind, button, modal, overlay};

/// Environment variable that replaces the save prompt (tests and scripts only).
pub const EXPORT_DIR_ENV: &str = "JIYUE_EXPORT_DIR";

/// Commands this module handles (sent as `ExtCommand::Export`).
#[derive(Clone, Debug)]
pub enum Command {
    /// 导出当前笔记为 HTML…: opens the theme dialog.
    OpenHtmlDialog,
    /// 复制为 HTML.
    CopyHtml,
    /// 复制为 Markdown.
    CopyMarkdown,
    /// 导出整个笔记库为网站….
    ExportSite,
}

/// Module state.
#[derive(Default)]
pub struct State {
    /// The HTML dialog is open.
    pub modal: bool,
    /// Theme of the next HTML export and copy.
    pub theme: Theme,
    /// An export is being written.
    pub busy: bool,
}

/// The note an export is about: its title, its text (the editor text when open) and its path.
struct Target {
    title: String,
    text: String,
}

fn export_dir() -> Option<PathBuf> {
    std::env::var_os(EXPORT_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The active note with its current text.
fn active_target(app: &RustdianApp, cx: &gpui::App) -> Option<Target> {
    let path = app.active_path()?.to_owned();
    let note = app.vault.notes.iter().find(|n| n.path == path)?;
    let text = app
        .editor_for(&path)
        .map(|e| e.read(cx).text())
        .unwrap_or_else(|| note.content().to_string());
    Some(Target {
        title: note.title.clone(),
        text,
    })
}

/// Runs a command of this module.
pub fn run(app: &mut RustdianApp, cmd: Command, cx: &mut Context<RustdianApp>) {
    match cmd {
        Command::OpenHtmlDialog => {
            if active_target(app, cx).is_none() {
                app.toast(ToastKind::Info, "没有打开的笔记");
            } else {
                app.ext.export.modal = true;
            }
        }
        Command::CopyHtml => copy_html(app, cx),
        Command::CopyMarkdown => copy_markdown(app, cx),
        Command::ExportSite => pick_site_folder(app, cx),
    }
    cx.notify();
}

/// Palette rows: the four export commands.
pub fn palette_commands() -> Vec<PaletteCommand> {
    vec![
        PaletteCommand {
            label: "导出当前笔记为 HTML…",
            icon: "file-text",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Export(Command::OpenHtmlDialog)),
        },
        PaletteCommand {
            label: "复制为 HTML",
            icon: "copy",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Export(Command::CopyHtml)),
        },
        PaletteCommand {
            label: "复制为 Markdown",
            icon: "copy",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Export(Command::CopyMarkdown)),
        },
        PaletteCommand {
            label: "导出整个笔记库为网站…",
            icon: "folder-open",
            shortcut: "",
            command: AppCommand::Ext(ExtCommand::Export(Command::ExportSite)),
        },
    ]
}

/// True while the dialog is open.
pub fn overlay_open(app: &RustdianApp) -> bool {
    app.ext.export.modal
}

/// Escape closes the dialog.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    if !app.ext.export.modal || !overlay::is_escape(ev) {
        return false;
    }
    app.ext.export.modal = false;
    cx.notify();
    true
}

/// Builds the page of the active note in the background, then asks where to save it.
fn save_html(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if app.ext.export.busy {
        return;
    }
    let Some(target) = active_target(app, cx) else {
        app.toast(ToastKind::Info, "没有打开的笔记");
        return;
    };
    let theme = app.ext.export.theme;
    let root = app.vault.root_dir.clone();
    let file_name = pure::html_file_name(&target.title);
    app.ext.export.modal = false;
    app.ext.export.busy = true;
    let title = target.title.clone();
    spawn_blocking(
        cx,
        move || render::note_page(&root, &title, &target.text, theme),
        move |app, html, cx| {
            if let Some(dir) = export_dir() {
                write_file(app, dir.join(&file_name), html, cx);
                return;
            }
            let root = app.vault.root_dir.clone();
            let picked = cx.prompt_for_new_path(&root, Some(file_name.as_str()));
            cx.spawn(async move |this, cx| {
                let chosen = picked.await;
                let _ = this.update(cx, |app, cx| match chosen {
                    Ok(Ok(Some(path))) => write_file(app, path, html, cx),
                    _ => {
                        app.ext.export.busy = false;
                        cx.notify();
                    }
                });
            })
            .detach();
        },
    );
    cx.notify();
}

/// Writes `html` to `path` in the background. Success offers 在资源管理器中显示.
fn write_file(_app: &mut RustdianApp, path: PathBuf, html: String, cx: &mut Context<RustdianApp>) {
    let target = path.clone();
    spawn_blocking(
        cx,
        move || std::fs::write(&target, html.as_bytes()).map_err(|e| e.to_string()),
        move |app, result, cx| {
            app.ext.export.busy = false;
            match result {
                Ok(()) => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    app.toast_action(
                        ToastKind::Success,
                        format!("已导出 {name}。要打印为 PDF，请用浏览器打开后选择「打印」"),
                        ToastAction::Reveal(path),
                    );
                }
                Err(e) => app.toast(ToastKind::Error, format!("导出失败：{e}")),
            }
            cx.notify();
        },
    );
}

/// 复制为 HTML: the page of the active note (in the dialog's theme) goes to the clipboard.
fn copy_html(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(target) = active_target(app, cx) else {
        app.toast(ToastKind::Info, "没有打开的笔记");
        return;
    };
    let theme = app.ext.export.theme;
    let root = app.vault.root_dir.clone();
    spawn_blocking(
        cx,
        move || render::note_page(&root, &target.title, &target.text, theme),
        |app, html, cx| {
            let kb = html.len().div_ceil(1024);
            cx.write_to_clipboard(ClipboardItem::new_string(html));
            app.toast(ToastKind::Success, format!("已复制 HTML（约 {kb} KB）"));
            cx.notify();
        },
    );
}

/// 复制为 Markdown: the note's text without the frontmatter.
fn copy_markdown(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(target) = active_target(app, cx) else {
        app.toast(ToastKind::Info, "没有打开的笔记");
        return;
    };
    let body = pure::markdown_body(&target.text).to_owned();
    cx.write_to_clipboard(ClipboardItem::new_string(body));
    app.toast(ToastKind::Success, "已复制 Markdown（不含属性）");
    cx.notify();
}

/// 导出整个笔记库为网站…: asks for a folder, then writes the site there.
fn pick_site_folder(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let root = app.vault.root_dir.clone();
    if let Some(dir) = export_dir() {
        export_site(app, root, dir.join("site"), cx);
        return;
    }
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some(SharedString::from("导出到此文件夹")),
    });
    cx.spawn(async move |this, cx| {
        let chosen = picked.await;
        let _ = this.update(cx, |app, cx| {
            if let Ok(Ok(Some(paths))) = chosen
                && let Some(folder) = paths.into_iter().next()
            {
                export_site(app, root, folder, cx);
            }
        });
    })
    .detach();
}

/// Writes the whole vault as a static site into `out` in the background.
fn export_site(_app: &mut RustdianApp, root: PathBuf, out: PathBuf, cx: &mut Context<RustdianApp>) {
    let title = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "笔记".to_owned());
    let opts = SiteOptions {
        title,
        ..SiteOptions::default()
    };
    let target = out.clone();
    spawn_blocking(
        cx,
        move || export_vault_site(&root, &target, &opts).map_err(|e| e.to_string()),
        move |app, result, cx| {
            match result {
                Ok(report) => {
                    let index = out.join("index.html");
                    let mut text = format!("已导出 {} 篇笔记到 {}", report.pages, out.display());
                    if report.unresolved_links > 0 {
                        text.push_str(&format!("，{} 个链接未解析", report.unresolved_links));
                    }
                    app.toast_action(ToastKind::Success, text, ToastAction::Reveal(index));
                }
                Err(e) => app.toast(ToastKind::Error, format!("导出网站失败：{e}")),
            }
            cx.notify();
        },
    );
}

/// The dialog: the theme choice, the PDF hint, and 取消 / 导出….
pub fn overlays(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    if !app.ext.export.modal {
        return Vec::new();
    }
    let t = app.theme;
    let current = app.ext.export.theme;

    let mut choices = div().flex().items_center().gap(px(8.0));
    for (i, theme) in Theme::ALL.iter().copied().enumerate() {
        let on = theme == current;
        choices = choices.child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(96.0))
                .h(px(64.0))
                .rounded(px(10.0))
                .border_1()
                .border_color(if on { t.ac } else { t.line })
                .bg(if on { t.active } else { t.sheet })
                .cursor_pointer()
                .font_family(crate::ui::fonts::ui_name())
                .text_size(px(13.0))
                .font_weight(if on {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if on { t.tx } else { t.tx2 })
                .child(SharedString::from(theme.label()))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.ext.export.theme = Theme::ALL[i];
                        cx.notify();
                    }),
                ),
        );
    }

    let body = div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .font_family(crate::ui::fonts::ui_name())
        .child(div().text_size(px(12.5)).text_color(t.tx3).child("主题"))
        .child(choices)
        .child(
            div()
                .text_size(px(13.0))
                .text_color(t.tx2)
                .child("生成一个独立的 HTML 文件：图片嵌入其中，公式与 Mermaid 图表已绘制好。"),
        )
        .child(
            div()
                .text_size(px(12.5))
                .text_color(t.tx3)
                .child("需要 PDF：导出后用浏览器打开，选择「打印」→「另存为 PDF」。"),
        );

    let footer = vec![
        button(
            &t,
            "取消",
            ButtonKind::Ghost,
            None,
            cx.listener(|this, _, _, cx| {
                this.ext.export.modal = false;
                cx.notify();
            }),
        )
        .into_any_element(),
        button(
            &t,
            "导出…",
            ButtonKind::Primary,
            None,
            cx.listener(|this, _, _, cx| {
                save_html(this, cx);
            }),
        )
        .into_any_element(),
    ];
    let close = cx.listener(|this: &mut RustdianApp, _, _, cx| {
        this.ext.export.modal = false;
        cx.notify();
    });
    vec![modal(&t, "导出为 HTML", body, footer, 460.0, close)]
}
