//! The `/` insert menu (UI). Pure logic lives in `slash/`.
//!
//! Flow: the editor reports the note change (`ExtEvent::NoteChanged`). When exactly one `/` was
//! typed at the caret, not during an IME composition, and `trigger::trigger_ok` passes, the menu
//! opens under the caret. The text typed after the `/` filters the rows. Keys (up, down, enter,
//! tab, escape) are taken by a keystroke interceptor before the editor's own bindings, and only
//! while the menu is open, the editor has focus and no composition is open. Choosing a row
//! replaces `/query` with the snippet in one edit (one undo step).
//!
//! 图片, 嵌入笔记 and 双链 leave their `![[]]` or `[[]]` placeholder and open a picker card (a
//! filter field and a list). The picker owns the keys while it is open: up, down, enter and escape
//! are taken by the interceptor, and the filter field gets the typed text. Choosing a row replaces
//! the placeholder with the link in one edit. 从电脑选择图片… imports the files through the
//! attachment code and puts the links in the placeholder.

use std::cell::Cell;
use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    AnyElement, Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement, MouseButton,
    MouseDownEvent, PathPromptOptions, SharedString, Subscription, WeakEntity, Window, div,
    prelude::*, px,
};

use crate::RustdianApp;
use crate::ui::attach_ui::import::{self, Outcome, Source};
use crate::ui::attach_ui::{self, resolve};
use crate::ui::ext::ExtEvent;
use crate::ui::ext::slash::picker::{self, Candidate, Target};
use crate::ui::ext::slash::{self, Kind, rank, snippet, trigger};
use crate::ui::icons::icon;
use crate::ui::text_input::{EditorBlur, TextEditor};
use crate::ui::widgets::{ButtonKind, LineInput, LineInputEvent, button, modal, place_popover};

/// Rows shown at once; the list scrolls with the highlight past it.
const MAX_ROWS: usize = 8;
const ROW_H: f32 = 44.0;
const CARD_W: f32 = 300.0;
const CARD_PAD: f32 = 6.0;
/// Gap kept between the menu and the window edge.
const MARGIN: f32 = 8.0;
/// Window size assumed before the first keystroke has reported the real one.
const FALLBACK_WIN: (f32, f32) = (1280.0, 800.0);
/// The row that opens the file prompt in the image picker.
const BROWSE_LABEL: &str = "从电脑选择图片…";

/// What the keystroke interceptor and the render share without touching the app: whether the menu
/// or the picker is open, and the window size (reported on each keystroke).
#[derive(Default)]
pub struct Shared {
    open: Cell<bool>,
    picker: Cell<bool>,
    win: Cell<(f32, f32)>,
}

/// One open menu.
pub struct Session {
    /// Vault-relative path of the note.
    path: String,
    /// Byte offset of the `/`.
    slash_at: usize,
    /// The query as last filtered.
    query: String,
    /// Indexes into `slash::ENTRIES` of the rows shown, best first.
    rows: Vec<usize>,
    /// Index into `rows` of the highlighted row.
    highlight: usize,
}

/// One open picker (图片, 嵌入笔记, 双链).
pub struct Picker {
    /// Vault-relative path of the note the link goes into.
    path: String,
    /// The placeholder the command left: `![[]]` or `[[]]`, at this range of the note.
    anchor: Range<usize>,
    placeholder: &'static str,
    target: Target,
    /// Every row the picker can show (without the browse row).
    cands: Vec<Candidate>,
    /// The labels of `cands`, matched by the filter.
    labels: Vec<String>,
    /// The filter text as last applied.
    query: String,
    /// The rows shown: `None` is the browse row, `Some(i)` a candidate.
    rows: Vec<Option<usize>>,
    /// Index into `rows` of the highlighted row.
    highlight: usize,
    /// The filter field (the picker holds its keyboard focus).
    filter: Entity<LineInput>,
    subs: Vec<Subscription>,
}

impl Picker {
    /// The image picker starts with the browse row.
    fn browse(&self) -> bool {
        self.target == Target::Image
    }
}

/// Module state.
#[derive(Default)]
pub struct State {
    shared: Rc<Shared>,
    session: Option<Session>,
    /// Observer and blur subscriptions of the open menu. Dropped on close.
    subs: Vec<Subscription>,
    /// Keystroke interceptor for the whole app.
    keys: Option<Subscription>,
    /// The note last seen changing, and its text length then (to recognise a typed character).
    last: Option<(String, usize)>,
    /// The open picker, if any.
    picker: Option<Picker>,
}

/// Registers the keystroke interceptor. Called once at start-up.
pub fn init(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let weak: WeakEntity<RustdianApp> = cx.entity().downgrade();
    let shared = app.ext.slash.shared.clone();
    let keys = cx.intercept_keystrokes(move |ev, window, cx| {
        shared.win.set(window_size(window));
        let picker_open = shared.picker.get();
        if !shared.open.get() && !picker_open {
            return;
        }
        let m = ev.keystroke.modifiers;
        if m.control || m.alt || m.platform || m.shift {
            return;
        }
        let key = ev.keystroke.key.clone();
        if !matches!(key.as_str(), "up" | "down" | "enter" | "tab" | "escape") {
            return;
        }
        let focused = window.focused(cx);
        let _ = weak.update(cx, |app, cx| {
            let used = if picker_open {
                handle_picker_key(app, &key, window, cx)
            } else {
                handle_key(app, &key, focused.as_ref(), window, cx)
            };
            if used {
                cx.stop_propagation();
            }
        });
    });
    app.ext.slash.keys = Some(keys);
}

fn window_size(window: &Window) -> (f32, f32) {
    let s = window.viewport_size();
    (f32::from(s.width), f32::from(s.height))
}

/// Module events: a typed `/` opens the menu, edits and caret moves update it.
pub fn on_event(app: &mut RustdianApp, ev: &ExtEvent, cx: &mut Context<RustdianApp>) {
    match ev {
        ExtEvent::NoteOpened(path) => {
            if app
                .ext
                .slash
                .session
                .as_ref()
                .is_some_and(|s| &s.path != path)
            {
                close(app);
                cx.notify();
            }
            if app
                .ext
                .slash
                .picker
                .as_ref()
                .is_some_and(|p| &p.path != path)
            {
                drop_picker(app);
                cx.notify();
            }
        }
        ExtEvent::NoteChanged(path) => note_changed(app, path, cx),
        _ => {}
    }
}

/// The note changed by typing, paste or an edit of this app. Opens the menu after one typed `/`,
/// or refreshes the open one.
fn note_changed(app: &mut RustdianApp, path: &str, cx: &mut Context<RustdianApp>) {
    let Some(editor) = app.editor_for(path) else {
        return;
    };
    let (text, anchor, caret, composing) = {
        let ed = editor.read(cx);
        let (a, h) = ed.selection_offsets();
        (ed.text(), a, h, ed.is_composing())
    };
    let prev = app.ext.slash.last.replace((path.to_string(), text.len()));
    if app.ext.slash.session.is_some() {
        refresh(app, cx);
        return;
    }
    if composing || anchor != caret || caret == 0 {
        return;
    }
    let Some((prev_path, prev_len)) = prev else {
        return;
    };
    if prev_path != path || text.len() != prev_len + 1 {
        return;
    }
    let slash_at = caret - 1;
    if text.get(slash_at..caret) != Some("/") || !trigger::trigger_ok(&text, slash_at) {
        return;
    }
    open(app, path, slash_at, &editor, &text, cx);
}

/// Opens the menu for the `/` at `slash_at` and watches the editor.
fn open(
    app: &mut RustdianApp,
    path: &str,
    slash_at: usize,
    editor: &Entity<TextEditor>,
    text: &str,
    cx: &mut Context<RustdianApp>,
) {
    let rows = rank::matches("", frontmatter_ok(text, slash_at));
    app.ext.slash.session = Some(Session {
        path: path.to_string(),
        slash_at,
        query: String::new(),
        rows,
        highlight: 0,
    });
    app.ext.slash.shared.open.set(true);
    let observe = cx.observe(editor, |this: &mut RustdianApp, _, cx| refresh(this, cx));
    let blur = cx.subscribe(editor, |this: &mut RustdianApp, _, _: &EditorBlur, cx| {
        close(this);
        cx.notify();
    });
    app.ext.slash.subs = vec![observe, blur];
    cx.notify();
}

/// True when the note starts with a frontmatter block (its first line is `---`).
fn has_frontmatter(text: &str) -> bool {
    text.lines().next().is_some_and(|l| l.trim_end() == "---")
}

/// The frontmatter command is offered only at the very start of a note without one.
fn frontmatter_ok(text: &str, slash_at: usize) -> bool {
    slash_at == 0 && !has_frontmatter(text)
}

/// Closes the menu and drops its subscriptions. Returns the session that was open.
fn close(app: &mut RustdianApp) -> Option<Session> {
    app.ext.slash.shared.open.set(false);
    app.ext.slash.subs.clear();
    app.ext.slash.session.take()
}

/// Re-reads the query from the editor and filters the rows. Closes the menu when the `/query` is
/// no longer valid, or when a space is typed after no match.
fn refresh(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let Some(session) = app.ext.slash.session.as_ref() else {
        return;
    };
    let path = session.path.clone();
    let slash_at = session.slash_at;
    let Some(editor) = app.editor_for(&path) else {
        close(app);
        cx.notify();
        return;
    };
    let (text, anchor, caret, composing) = {
        let ed = editor.read(cx);
        let (a, h) = ed.selection_offsets();
        (ed.text(), a, h, ed.is_composing())
    };
    let query = match trigger::query_of(&text, slash_at, caret) {
        Some(q) if anchor == caret => q.to_string(),
        _ => {
            close(app);
            cx.notify();
            return;
        }
    };
    // While the IME composes, the marked text is not a query yet: keep the list as it is.
    if composing {
        return;
    }
    let rows = rank::matches(&query, frontmatter_ok(&text, slash_at));
    let spaced = query.chars().any(char::is_whitespace);
    if spaced && (rank::normalize(&query).is_empty() || rows.is_empty()) {
        close(app);
        cx.notify();
        return;
    }
    if let Some(s) = app.ext.slash.session.as_mut() {
        if query != s.query {
            s.highlight = 0;
        }
        s.query = query;
        s.rows = rows;
        s.highlight = s.highlight.min(s.rows.len().saturating_sub(1));
    }
    cx.notify();
}

/// Keys while the menu is open. Returns true when the key was used (the editor must not see it).
fn handle_key(
    app: &mut RustdianApp,
    key: &str,
    focused: Option<&FocusHandle>,
    window: &mut Window,
    cx: &mut Context<RustdianApp>,
) -> bool {
    let Some(session) = app.ext.slash.session.as_ref() else {
        return false;
    };
    let path = session.path.clone();
    let rows_len = session.rows.len();
    let highlight = session.highlight;
    let Some(editor) = app.editor_for(&path) else {
        close(app);
        return false;
    };
    let (own_focus, composing) = {
        let ed = editor.read(cx);
        (ed.focus_handle(cx), ed.is_composing())
    };
    if focused != Some(&own_focus) || composing {
        return false;
    }
    match key {
        "escape" => {
            close(app);
            cx.notify();
            true
        }
        "up" | "down" if rows_len > 0 => {
            let next = if key == "down" {
                (highlight + 1) % rows_len
            } else {
                (highlight + rows_len - 1) % rows_len
            };
            if let Some(s) = app.ext.slash.session.as_mut() {
                s.highlight = next;
            }
            cx.notify();
            true
        }
        "enter" | "tab" => {
            if rows_len == 0 {
                // Nothing to pick: the key goes on to the editor (a newline).
                close(app);
                cx.notify();
                return false;
            }
            pick_row(app, highlight, window, cx);
            true
        }
        _ => false,
    }
}

/// Keys while a picker is open: escape closes it, the arrows move the highlight, enter chooses.
/// The filter field gets every other key. Returns true when the key was used.
fn handle_picker_key(
    app: &mut RustdianApp,
    key: &str,
    window: &mut Window,
    cx: &mut Context<RustdianApp>,
) -> bool {
    let Some(p) = app.ext.slash.picker.as_mut() else {
        return false;
    };
    match key {
        "escape" => {
            close_picker(app, window, cx);
            cx.notify();
            true
        }
        "up" | "down" => {
            let len = p.rows.len();
            if len > 0 {
                p.highlight = if key == "down" {
                    (p.highlight + 1) % len
                } else {
                    (p.highlight + len - 1) % len
                };
            }
            cx.notify();
            true
        }
        "enter" => {
            if p.rows.is_empty() {
                return true;
            }
            let index = p.highlight;
            confirm_row(app, index, window, cx);
            cx.notify();
            true
        }
        _ => false,
    }
}

/// Inserts the row at `index` (of the shown rows) and closes the menu.
fn pick_row(
    app: &mut RustdianApp,
    index: usize,
    window: &mut Window,
    cx: &mut Context<RustdianApp>,
) {
    let Some(kind) = app
        .ext
        .slash
        .session
        .as_ref()
        .and_then(|s| s.rows.get(index))
        .map(|&i| slash::ENTRIES[i].kind)
    else {
        return;
    };
    choose(app, kind, window, cx);
    cx.notify();
}

/// The current date and time as the snippets write them.
fn now_stamp() -> snippet::Stamp {
    let ms = jiyue_history::system_now_ms();
    match crate::ui::history_ui::pure::local_parts(ms) {
        Some((y, mo, d, h, mi)) => snippet::stamp(y, mo, d, h, mi),
        None => snippet::stamp(1970, 1, 1, 0, 0),
    }
}

/// The picker a command opens after its placeholder: the target list and the placeholder text.
fn picker_for(kind: Kind) -> Option<(Target, &'static str)> {
    match kind {
        Kind::Image => Some((Target::Image, "![[]]")),
        Kind::EmbedNote => Some((Target::Embed, "![[]]")),
        Kind::WikiLink => Some((Target::Link, "[[]]")),
        _ => None,
    }
}

/// Replaces the `/query` with the snippet of `kind`: one edit, then the selection the snippet
/// leaves. Templates remove the query and open the template picker. The link commands open their
/// picker after the placeholder.
fn choose(app: &mut RustdianApp, kind: Kind, window: &mut Window, cx: &mut Context<RustdianApp>) {
    let Some(session) = close(app) else {
        return;
    };
    let path = session.path.clone();
    let Some(editor) = app.editor_for(&path) else {
        return;
    };
    let stamp = now_stamp();
    let slash_at = session.slash_at;
    let result = editor.update(cx, |ed, cx| {
        let text = ed.text();
        let (_, caret) = ed.selection_offsets();
        trigger::query_of(&text, slash_at, caret)?;
        let edit = snippet::plan(&text, slash_at, caret, kind, &stamp);
        if !ed.replace_text_range(edit.range.clone(), &edit.text, cx) {
            return Some((false, edit));
        }
        ed.restore_selection(edit.select.start, edit.select.end, cx);
        Some((true, edit))
    });
    match result {
        Some((true, edit)) => {
            if edit.open_template {
                app.dispatch_ext(
                    crate::ui::ext::ExtCommand::Template(crate::ui::template_ui::Command::Insert),
                    cx,
                );
            } else if let Some((target, placeholder)) = picker_for(kind) {
                let anchor = slash_at..slash_at + placeholder.len();
                open_picker(app, &path, anchor, placeholder, target, window, cx);
            }
        }
        Some((false, _)) => {
            app.toast(
                crate::ui::ext::ToastKind::Info,
                "请先结束输入法的输入，再插入",
            );
        }
        None => {}
    }
}

/// Opens the picker for `target` after the placeholder at `anchor`, and gives its filter field
/// the keyboard focus.
fn open_picker(
    app: &mut RustdianApp,
    path: &str,
    anchor: Range<usize>,
    placeholder: &'static str,
    target: Target,
    window: &mut Window,
    cx: &mut Context<RustdianApp>,
) {
    let cands = match target {
        Target::Image => picker::image_candidates(&app.ext.attach.files.files),
        Target::Embed | Target::Link => {
            let notes: Vec<(String, String)> = app
                .vault
                .notes
                .iter()
                .map(|n| (n.title.clone(), n.path.clone()))
                .collect();
            picker::note_candidates(&notes, target)
        }
    };
    let labels: Vec<String> = cands.iter().map(|c| c.label.clone()).collect();
    let browse = target == Target::Image;
    let rows = picker::visible(&labels, browse, "");
    let theme = app.theme;
    let hint = match target {
        Target::Image => "搜索图片…",
        Target::Embed => "搜索要嵌入的笔记…",
        Target::Link => "搜索要链接的笔记…",
    };
    let filter = cx.new(|cx| LineInput::new(cx, hint));
    filter.update(cx, |i, cx| i.set_theme(theme, cx));
    let changed = cx.subscribe(
        &filter,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
            if let LineInputEvent::Changed(query) = ev {
                if let Some(p) = this.ext.slash.picker.as_mut() {
                    p.rows = picker::visible(&p.labels, p.browse(), query);
                    p.query = query.clone();
                    p.highlight = 0;
                }
                cx.notify();
            }
        },
    );
    filter.read(cx).focus(window);
    app.ext.slash.picker = Some(Picker {
        path: path.to_string(),
        anchor,
        placeholder,
        target,
        cands,
        labels,
        query: String::new(),
        rows,
        highlight: 0,
        filter,
        subs: vec![changed],
    });
    app.ext.slash.shared.picker.set(true);
    cx.notify();
}

/// Removes the picker without touching the note.
fn drop_picker(app: &mut RustdianApp) {
    app.ext.slash.picker = None;
    app.ext.slash.shared.picker.set(false);
}

/// Removes the picker and gives the keyboard focus back to the note it was opened for.
fn take_picker(
    app: &mut RustdianApp,
    window: &mut Window,
    cx: &mut Context<RustdianApp>,
) -> Option<Picker> {
    let p = app.ext.slash.picker.take()?;
    app.ext.slash.shared.picker.set(false);
    if let Some(editor) = app.editor_for(&p.path) {
        let handle = editor.read(cx).focus_handle(cx);
        window.focus(&handle);
    }
    Some(p)
}

/// Closes the picker and returns the focus to the note.
fn close_picker(app: &mut RustdianApp, window: &mut Window, cx: &mut Context<RustdianApp>) {
    let _ = take_picker(app, window, cx);
}

/// Chooses the row at `index` of the picker: a link is inserted, the browse row opens the prompt.
fn confirm_row(
    app: &mut RustdianApp,
    index: usize,
    window: &mut Window,
    cx: &mut Context<RustdianApp>,
) {
    let Some(p) = app.ext.slash.picker.as_ref() else {
        return;
    };
    let Some(row) = p.rows.get(index).copied() else {
        return;
    };
    match row {
        None => browse(app, window, cx),
        Some(i) => {
            let Some(link) = p.cands.get(i).map(|c| c.link.clone()) else {
                return;
            };
            if let Some(done) = take_picker(app, window, cx) {
                apply_link(
                    app,
                    &done.path,
                    done.anchor.clone(),
                    done.placeholder,
                    &link,
                    cx,
                );
            }
        }
    }
}

/// Replaces the placeholder at `anchor` of the note at `path` with `link`: one undoable edit, the
/// caret after the link, then the note is saved. Nothing changes when the placeholder has moved.
fn apply_link(
    app: &mut RustdianApp,
    path: &str,
    anchor: Range<usize>,
    placeholder: &str,
    link: &str,
    cx: &mut Context<RustdianApp>,
) {
    let Some(editor) = app.editor_for(path) else {
        app.toast(
            crate::ui::ext::ToastKind::Error,
            "笔记没有打开，无法插入链接",
        );
        return;
    };
    let placed = editor.update(cx, |ed, cx| {
        let text = ed.text();
        if !picker::anchor_matches(&text, &anchor, placeholder) {
            return false;
        }
        if !ed.replace_text_range(anchor.clone(), link, cx) {
            return false;
        }
        let caret = picker::caret_after(&anchor, link);
        ed.restore_selection(caret.start, caret.end, cx);
        true
    });
    if placed {
        app.save_path(path, cx);
    } else {
        app.toast(
            crate::ui::ext::ToastKind::Warn,
            "占位符已被改动，链接没有插入",
        );
    }
}

/// The image picker's browse row: the native file prompt, then the files are imported.
fn browse(app: &mut RustdianApp, window: &mut Window, cx: &mut Context<RustdianApp>) {
    let Some(done) = take_picker(app, window, cx) else {
        return;
    };
    let note = done.path;
    let anchor = done.anchor;
    let placeholder = done.placeholder;
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: Some(SharedString::from("导入图片")),
    });
    cx.spawn(async move |this, cx| {
        if let Ok(Ok(Some(paths))) = picked.await {
            let _ = this.update(cx, |app, cx| {
                import_picked(app, paths, &note, anchor, placeholder, cx);
            });
        }
    })
    .detach();
}

/// Imports the chosen files on a background thread, then puts their links in the placeholder.
fn import_picked(
    app: &mut RustdianApp,
    paths: Vec<PathBuf>,
    note: &str,
    anchor: Range<usize>,
    placeholder: &'static str,
    cx: &mut Context<RustdianApp>,
) {
    let sources: Vec<Source> = paths
        .into_iter()
        .filter(|p| p.is_file())
        .map(Source::File)
        .collect();
    if sources.is_empty() {
        app.toast(crate::ui::ext::ToastKind::Warn, "没有选择图片文件");
        return;
    }
    let name_taken: HashSet<String> = app
        .ext
        .attach
        .files
        .files
        .iter()
        .map(|f| resolve::basename(f).to_string())
        .collect();
    let job = import::Job {
        root: app.vault.root_dir.clone(),
        settings: app.ext.attach.settings.clone(),
        note_path: note.to_string(),
        sources,
        now: import::now_stamp(),
        name_taken,
    };
    let note = note.to_string();
    let task = cx
        .background_executor()
        .spawn(async move { import::run_job(&job) });
    cx.spawn(async move |this, cx| {
        let outcomes = task.await;
        let _ = this.update(cx, |app, cx| {
            finish_import(app, &note, anchor, placeholder, outcomes, cx);
        });
    })
    .detach();
}

/// The links of the finished imports go into the placeholder; the errors are reported.
fn finish_import(
    app: &mut RustdianApp,
    note: &str,
    anchor: Range<usize>,
    placeholder: &str,
    outcomes: Vec<Outcome>,
    cx: &mut Context<RustdianApp>,
) {
    let mut links = Vec::new();
    let mut errors = Vec::new();
    for outcome in outcomes {
        match outcome {
            Outcome::Saved { link, .. } => links.push(link),
            Outcome::Failed(error) => errors.push(error),
        }
    }
    attach_ui::refresh_index(app, cx);
    if links.is_empty() {
        let text = errors
            .first()
            .cloned()
            .unwrap_or_else(|| "导入失败".to_string());
        app.toast(crate::ui::ext::ToastKind::Error, text);
        cx.notify();
        return;
    }
    let count = links.len();
    apply_link(app, note, anchor, placeholder, &links.join(" "), cx);
    app.toast(
        crate::ui::ext::ToastKind::Success,
        format!("已导入 {count} 个图片"),
    );
    if let Some(error) = errors.first() {
        app.toast(crate::ui::ext::ToastKind::Error, error.clone());
    }
    cx.notify();
}

/// The menu: a card under the caret, flipped above it near the bottom of the window.
pub fn overlays(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Vec<AnyElement> {
    let mut out = Vec::new();
    if let Some(card) = menu_overlay(app, cx) {
        out.push(card);
    }
    if let Some(card) = picker_overlay(app, cx) {
        out.push(card);
    }
    out
}

fn menu_overlay(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Option<AnyElement> {
    let s = app.ext.slash.session.as_ref()?;
    let editor = app.editor_for(&s.path)?;
    let (caret, own_focus) = {
        let ed = editor.read(cx);
        (ed.caret_origin(), ed.focus_handle(cx))
    };
    let (x, y, row_h) = caret?;
    let t = app.theme;
    let win = app.ext.slash.shared.win.get();
    let win = if win.0 > 0.0 && win.1 > 0.0 {
        win
    } else {
        FALLBACK_WIN
    };

    // Same as `min(MAX_ROWS).max(1)`; the `max(1)` keeps the clamp bounds ordered.
    let shown = s.rows.len().clamp(1, MAX_ROWS.max(1));
    let card_h = shown as f32 * ROW_H + 2.0 * CARD_PAD;
    let (left, top) = place_popover((x, y, 1.0, row_h), (CARD_W, card_h), win, MARGIN);

    let start = if s.highlight >= MAX_ROWS {
        s.highlight + 1 - MAX_ROWS
    } else {
        0
    };
    let mut list = div().flex().flex_col();
    if s.rows.is_empty() {
        list = list.child(
            div()
                .h(px(ROW_H))
                .px(px(14.0))
                .flex()
                .items_center()
                .text_size(px(13.0))
                .text_color(t.tx3)
                .child("没有匹配的命令"),
        );
    }
    for (row, &idx) in s.rows.iter().enumerate().skip(start).take(MAX_ROWS) {
        let entry = &slash::ENTRIES[idx];
        let on = row == s.highlight;
        let focus = own_focus.clone();
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .h(px(ROW_H))
                .mx(px(4.0))
                .px(px(10.0))
                .rounded(px(8.0))
                .cursor_pointer()
                .bg(if on { t.active } else { t.raised })
                .hover(|st| st.bg(t.hover))
                .child(icon(entry.icon, 16.0, if on { t.ac } else { t.tx2 }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(1.0))
                        .child(
                            div()
                                .text_size(px(13.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(t.tx)
                                .child(SharedString::from(entry.zh)),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(t.tx3)
                                .child(SharedString::from(entry.desc)),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.5))
                        .text_color(t.tx3)
                        .child(SharedString::from(entry.hint)),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        // Keep the editor focused: the click must not reach the root's focus.
                        cx.stop_propagation();
                        window.focus(&focus);
                        pick_row(this, row, window, cx);
                    }),
                ),
        );
    }

    let card = div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(CARD_W))
        .py(px(CARD_PAD))
        .rounded(px(10.0))
        .border_1()
        .border_color(t.line)
        .bg(t.raised)
        .shadow_lg()
        .overflow_hidden()
        .font_family(crate::ui::fonts::ui_name())
        .text_size(px(13.0))
        .text_color(t.tx)
        .child(list)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    Some(card.into_any_element())
}

/// The picker card: the filter field, the rows (从电脑选择图片… first for images), 取消.
fn picker_overlay(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Option<AnyElement> {
    let p = app.ext.slash.picker.as_ref()?;
    let t = app.theme;
    let title = match p.target {
        Target::Image => "插入图片",
        Target::Embed => "嵌入笔记",
        Target::Link => "链接到笔记",
    };
    let start = if p.highlight >= MAX_ROWS {
        p.highlight + 1 - MAX_ROWS
    } else {
        0
    };
    let mut list = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .max_h(px(360.0))
        .overflow_hidden();
    if p.rows.is_empty() {
        list = list.child(
            div()
                .py(px(12.0))
                .text_size(px(13.0))
                .text_color(t.tx3)
                .child("没有匹配的项目"),
        );
    }
    for (row, item) in p
        .rows
        .iter()
        .copied()
        .enumerate()
        .skip(start)
        .take(MAX_ROWS)
    {
        let (name, detail) = match item {
            None => (
                BROWSE_LABEL.to_string(),
                "选择电脑上的图片，导入到附件文件夹".to_string(),
            ),
            Some(i) => match p.cands.get(i) {
                Some(c) => (c.label.clone(), c.detail.clone()),
                None => continue,
            },
        };
        let on = row == p.highlight;
        list = list.child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .px(px(10.0))
                .py(px(7.0))
                .rounded(px(8.0))
                .cursor_pointer()
                .bg(if on { t.active } else { t.sheet })
                .hover(|st| st.bg(t.hover))
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.tx)
                        .child(SharedString::from(name)),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(t.tx3)
                        .child(SharedString::from(detail)),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        confirm_row(this, row, window, cx);
                        cx.notify();
                    }),
                ),
        );
    }

    let body = div()
        .flex()
        .flex_col()
        .font_family(crate::ui::fonts::ui_name())
        .child(div().mb(px(10.0)).child(p.filter.clone()))
        .child(list);
    let footer = vec![
        button(
            &t,
            "取消",
            ButtonKind::Ghost,
            None,
            cx.listener(|this, _, window, cx| {
                close_picker(this, window, cx);
                cx.notify();
            }),
        )
        .into_any_element(),
    ];
    let dismiss = cx.listener(|this: &mut RustdianApp, _, window, cx| {
        close_picker(this, window, cx);
        cx.notify();
    });
    Some(modal(&t, title, body, footer, 520.0, dismiss))
}
