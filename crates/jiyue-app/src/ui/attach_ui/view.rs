//! Preview blocks for images, files and note embeds, and the lightbox. Called from the editor's
//! block loop (`editor::body_blocks`) for lines that hold one embed and nothing else.

use gpui::{
    AnyElement, ClickEvent, Context, ElementId, FontWeight, IntoElement, MouseButton,
    MouseDownEvent, ObjectFit, SharedString, div, img, prelude::*, px, relative,
};
use jiyue_md::embeds::{
    Anchor, EmbedKind, Link, LinkKind, embed_kind, extract_block_text, extract_section, parse_links,
};
use jiyue_md::frontmatter;

use super::resolve::{COLUMN_WIDTH, EmbedGuard, basename, caption, display_size, embed_guard};
use crate::RustdianApp;
use crate::theme::ActiveTheme;
use crate::ui::UI_FONT;
use crate::ui::display_family;
use crate::ui::editor::{Clicks, body_blocks};
use crate::ui::icons::icon;
use crate::ui::widgets::overlay::scrim_colour;

/// The block for a line that holds exactly one image or embed. `None` for every other line.
pub fn embed_line(
    app: &RustdianApp,
    clicks: Clicks,
    line: &str,
    lineno: usize,
    cx: &mut Context<RustdianApp>,
) -> Option<AnyElement> {
    let trimmed = line.trim();
    if !trimmed.starts_with("![") {
        return None;
    }
    let links = parse_links(trimmed);
    let [link] = links.as_slice() else {
        return None;
    };
    if link.range != (0..trimmed.len()) {
        return None;
    }
    match link.kind {
        LinkKind::Embed if embed_kind(&link.target) == EmbedKind::Note => {
            Some(note_embed(app, clicks, link, lineno, cx))
        }
        LinkKind::Embed | LinkKind::MdImage => Some(file_block(app, clicks, link, lineno, cx)),
        _ => None,
    }
}

/// An image or a file. Images are drawn; other files get a card with their name.
fn file_block(
    app: &RustdianApp,
    clicks: Clicks,
    link: &Link,
    lineno: usize,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    if link.external {
        return placeholder(&t, "link", "外部图片不加载", &link.target);
    }
    let name = basename(&link.target).to_string();
    let hint = app.ext.attach.fixed_folder();
    let Some(rel) = app
        .ext
        .attach
        .files
        .resolve(clicks.path, &link.target, hint.as_deref())
    else {
        return placeholder(
            &t,
            "file-text",
            &format!("找不到图片：{name}"),
            &link.target,
        );
    };
    match embed_kind(&rel) {
        EmbedKind::Image => image_block(app, &rel, link, lineno, cx),
        _ => {
            let card = div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .mt(px(8.0))
                .mb(px(16.0))
                .h(px(44.0))
                .px(px(14.0))
                .rounded(px(10.0))
                .bg(t.raised)
                .border_1()
                .border_color(t.line)
                .font_family(UI_FONT)
                .text_size(px(13.5))
                .text_color(t.tx2)
                .child(icon("file-text", 16.0, t.tx3))
                .child(SharedString::from(rel));
            card.into_any_element()
        }
    }
}

/// An image drawn at its display size, with a border, rounded corners and a caption. A click
/// opens the lightbox.
fn image_block(
    app: &RustdianApp,
    rel: &str,
    link: &Link,
    lineno: usize,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    let Some(natural) = app.ext.attach.files.image_size(rel) else {
        return placeholder(
            &t,
            "file-text",
            &format!("无法显示图片：{}", basename(rel)),
            rel,
        );
    };
    let Some((w, h)) = display_size(natural, link.size, COLUMN_WIDTH) else {
        return placeholder(
            &t,
            "file-text",
            &format!("无法显示图片：{}", basename(rel)),
            rel,
        );
    };
    let abs = app.vault.root_dir.join(rel);
    let open_rel = rel.to_string();
    // Click listeners need an element id (one per line, the image's source position).
    let picture = img(abs)
        .object_fit(ObjectFit::Contain)
        .id(ElementId::NamedInteger(
            SharedString::from("img"),
            lineno as u64,
        ))
        .w(px(w))
        .h(px(h))
        .rounded(px(8.0))
        .border_1()
        .border_color(t.line)
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
        })
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            this.ext.attach.lightbox = Some(open_rel.clone());
            cx.notify();
        }));
    let mut block = div()
        .flex()
        .flex_col()
        .items_start()
        .mt(px(8.0))
        .mb(px(16.0))
        .child(picture);
    if let Some(text) = caption(link.alias.as_deref()) {
        block = block.child(
            div()
                .mt(px(6.0))
                .font_family(UI_FONT)
                .text_size(px(12.5))
                .text_color(t.tx3)
                .child(SharedString::from(text)),
        );
    }
    block.into_any_element()
}

/// A card that says what is missing or why nothing is drawn.
pub fn placeholder(t: &ActiveTheme, icon_name: &str, text: &str, detail: &str) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .mt(px(8.0))
        .mb(px(16.0))
        .h(px(60.0))
        .px(px(14.0))
        .rounded(px(10.0))
        .bg(t.raised)
        .border_1()
        .border_color(t.line)
        .font_family(UI_FONT)
        .child(icon(icon_name, 18.0, t.tx3))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(13.5))
                        .text_color(t.tx2)
                        .child(SharedString::from(text.to_string())),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(t.tx3)
                        .child(SharedString::from(detail.to_string())),
                ),
        )
        .into_any_element()
}

/// `![[note]]`, `![[note#Heading]]` or `![[note#^block]]`: the target text in a card.
fn note_embed(
    app: &RustdianApp,
    clicks: Clicks,
    link: &Link,
    lineno: usize,
    cx: &mut Context<RustdianApp>,
) -> AnyElement {
    let t = app.theme;
    let title = link.target.trim().to_string();
    let Some(idx) = app.vault.note_index_by_title(&title) else {
        return placeholder(&t, "file-text", &format!("找不到笔记：{title}"), "");
    };
    let Some(note) = app.vault.notes.get(idx) else {
        return placeholder(&t, "file-text", &format!("找不到笔记：{title}"), "");
    };
    let label = match &link.anchor {
        Some(Anchor::Heading(h)) => format!("{title}#{h}"),
        Some(Anchor::Block(id)) => format!("{title}#^{id}"),
        None => title.clone(),
    };
    match embed_guard(clicks.depth, clicks.stack, &note.path) {
        EmbedGuard::Cycle => return placeholder(&t, "file-text", "循环嵌入", &label),
        EmbedGuard::TooDeep => return placeholder(&t, "file-text", "嵌入层级过深", &label),
        EmbedGuard::Ok => {}
    }
    let full = note.content();
    let body = match &link.anchor {
        None => {
            let (_, start) = frontmatter::split(full);
            full.get(start..).unwrap_or(full).to_string()
        }
        Some(Anchor::Heading(h)) => match extract_section(full, h) {
            Some(text) => text,
            None => return placeholder(&t, "file-text", "找不到引用的标题", &label),
        },
        Some(Anchor::Block(id)) => match extract_block_text(full, id) {
            Some(text) => text,
            None => return placeholder(&t, "file-text", "找不到引用的块", &label),
        },
    };

    let mut stack: Vec<String> = clicks.stack.to_vec();
    stack.push(note.path.clone());
    let inner = Clicks::embedded(&note.path, clicks.depth + 1, &stack);
    let lines: Vec<&str> = body.lines().collect();
    let family = display_family(&t);
    let blocks = body_blocks(app, &t, &lines, &title, family, inner, cx);

    let open_title = title.clone();
    let open_anchor = link.anchor.clone();
    let header = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .h(px(34.0))
        .px(px(12.0))
        .border_b_1()
        .border_color(t.line_soft)
        .font_family(UI_FONT)
        .child(icon("file-text", 14.0, t.tx3))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx2)
                .child(SharedString::from(label)),
        )
        .child(
            div()
                .id(ElementId::NamedInteger(
                    SharedString::from("embed-open"),
                    lineno as u64,
                ))
                .px(px(7.0))
                .py(px(2.0))
                .rounded(px(6.0))
                .text_size(px(12.0))
                .text_color(t.ac)
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child("打开")
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    super::open_target(this, &open_title, open_anchor.clone(), cx);
                })),
        );

    div()
        .mt(px(8.0))
        .mb(px(16.0))
        .rounded(px(10.0))
        .bg(t.raised)
        .border_1()
        .border_color(t.line)
        .overflow_hidden()
        .child(header)
        .child(
            div()
                .px(px(16.0))
                .py(px(10.0))
                .text_size(px(15.0))
                .line_height(relative(1.9))
                .text_color(t.tx)
                .children(blocks),
        )
        .into_any_element()
}

/// The full-window image viewer. A click outside the picture or Escape closes it.
pub fn lightbox(app: &RustdianApp, rel: &str, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let abs = app.vault.root_dir.join(rel);
    // The picture fills the window inside a margin and is contained, so it fits the window at any
    // size. A press on the picture does nothing; a press on the margin closes the viewer.
    let picture = img(abs)
        .object_fit(ObjectFit::Contain)
        .id("attach-lightbox-img")
        .size_full()
        .rounded(px(8.0))
        .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
        });
    let frame = div()
        .id("attach-lightbox-frame")
        .size_full()
        .p(px(28.0))
        .child(picture);
    div()
        .id("attach-lightbox")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(scrim_colour(&t))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.ext.attach.lightbox = None;
                cx.notify();
            }),
        )
        .child(frame)
        .into_any_element()
}
