//! The Mermaid block widget of the live view (see `live/diagram.rs`). It is part of `text_input`
//! (declared there with `#[path]`) so it can use the unit types and shaping helpers of the editor.
//!
//! A diagram is drawn as its picture. While it renders, a box of [`diagram::LOADING_H`] is drawn
//! (the picture replaces it once). A diagram kind the renderer draws poorly, and a source it
//! refuses, get a card: the reason, and the first source lines for an error. A click anywhere on
//! the widget puts the caret in the block, which shows the source (the generic block click).

use gpui::{Rgba, Window};
use jiyue_diagram::DiagramTheme;

use super::context::DiagramState;
use super::diagram::{self, DiagramKey};
use super::embed;
use super::{IMG_GAP, IMG_TOP, LineCtx, Widget, WidgetText, label_runs, plain_text, shape_one};
use crate::theme::{ActiveTheme, ThemeMode};
use crate::ui::{MONO_FONT, UI_FONT, UI_FONT_FALLBACK};

/// The renderer's theme from the note's colours. The page is transparent: the diagram sits on the
/// note sheet, and the node fill is the raised tone of the theme.
fn diagram_theme(t: &ActiveTheme) -> DiagramTheme {
    let hex = |c: Rgba| embed::hex_colour(c.r, c.g, c.b);
    DiagramTheme {
        background: "transparent".into(),
        foreground: hex(t.tx),
        muted: hex(t.tx3),
        accent: hex(t.ac),
        node_fill: hex(t.raised),
        node_stroke: hex(t.tx3),
        line: hex(t.tx2),
        font_family: format!("{UI_FONT}, {UI_FONT_FALLBACK}, Arial"),
        font_size: 14.0,
        dark: matches!(t.mode, ThemeMode::Dark),
    }
}

/// A card that says why the diagram is not drawn: a title and up to a few lines of source.
fn notice(window: &Window, ctx: &LineCtx, title: &str, lines: &[String]) -> (f32, Widget) {
    let t = ctx.theme;
    let mut texts: Vec<WidgetText> = Vec::new();
    if let Some(wl) = shape_one(
        window,
        title,
        &label_runs(title, t.tx2, UI_FONT),
        13.5,
        None,
    ) {
        texts.push(plain_text(36.0, 10.0, wl, 18.0));
    }
    let mut y = 32.0;
    for line in lines {
        if let Some(wl) = shape_one(
            window,
            line,
            &label_runs(line, t.tx3, MONO_FONT),
            12.0,
            None,
        ) {
            texts.push(plain_text(36.0, y, wl, 16.0));
        }
        y += 16.0;
    }
    let panel_h = (y + 10.0).max(44.0);
    (
        IMG_TOP + panel_h + IMG_GAP,
        Widget {
            panel: Some((0.0, IMG_TOP, ctx.wrap_w, panel_h)),
            icons: vec![((12.0, IMG_TOP + 12.0, 16.0, 16.0), Some("file-text"), t.tx3)],
            texts,
            ..Widget::default()
        },
    )
}

/// The reserved box of a diagram that is still rendering. Its height is fixed, so the picture
/// that replaces it moves the text below once.
fn loading(window: &Window, ctx: &LineCtx, built_gen: u64) -> (f32, Widget) {
    let t = ctx.theme;
    let label = "正在渲染图表";
    let mut texts: Vec<WidgetText> = Vec::new();
    if let Some(wl) = shape_one(
        window,
        label,
        &label_runs(label, t.tx3, UI_FONT),
        12.5,
        None,
    ) {
        texts.push(plain_text(12.0, IMG_TOP + 12.0, wl, 18.0));
    }
    (
        IMG_TOP + diagram::LOADING_H + IMG_GAP,
        Widget {
            panel: Some((0.0, IMG_TOP, ctx.wrap_w, diagram::LOADING_H)),
            texts,
            loading: true,
            vault_gen: Some(built_gen),
            ..Widget::default()
        },
    )
}

/// The unit of a Mermaid block (the whole fenced block, `block` is its source lines).
pub(super) fn mermaid_widget(window: &Window, ctx: &LineCtx, block: &str) -> (f32, Widget) {
    let t = ctx.theme;
    let dark = matches!(t.mode, ThemeMode::Dark);
    let source = diagram::source_of(block);
    if source.trim().is_empty() {
        return notice(window, ctx, "图表为空", &[]);
    }
    if let diagram::Support::Unsupported(label) = diagram::support(&source, dark) {
        let title = format!("{label}：此图表类型暂不支持预览，点击编辑源码");
        return notice(window, ctx, &title, &[]);
    }
    let Some(vault) = ctx.vault else {
        return notice(
            window,
            ctx,
            "无法显示图表",
            &diagram::first_lines(&source, 3, 80),
        );
    };
    let key = DiagramKey {
        source: source.clone(),
        theme: ctx.theme_key,
        width: diagram::width_bucket(ctx.wrap_w),
    };
    let (built_gen, state) = {
        let mut v = vault.borrow_mut();
        v.queue_diagram(key.clone(), diagram_theme(t));
        (v.data_gen, v.diagram_state(&key))
    };
    match state {
        Some(DiagramState::Ready { image, css }) => {
            let (w, h) = css;
            let x = ((ctx.wrap_w - w) / 2.0).max(0.0);
            (
                IMG_TOP + h + IMG_GAP,
                Widget {
                    image: Some(((x, IMG_TOP, w, h), image)),
                    vault_gen: Some(built_gen),
                    ..Widget::default()
                },
            )
        }
        Some(DiagramState::Failed(err)) => {
            let title = format!("图表无法显示：{}", diagram::error_message(&err));
            let mut w = notice(window, ctx, &title, &diagram::first_lines(&source, 3, 80));
            w.1.vault_gen = Some(built_gen);
            w
        }
        Some(DiagramState::Loading) | None => loading(window, ctx, built_gen),
    }
}
