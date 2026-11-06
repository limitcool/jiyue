//! The global search panel (Ctrl+Shift+F): a top-anchored overlay with a search field, scope
//! toggles, live results (score order, matches highlighted), recent searches and syntax hints.
//! Queries run in the background; the newest query wins.

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Context, Entity, Focusable, FontWeight, HighlightStyle, KeyDownEvent,
    MouseButton, ScrollHandle, StyledText, Subscription, Task, Window, div, prelude::*, px,
};
use jiyue_search::Hit;

use super::logic::{self, MatchOpts, Scope};
use super::{focus_soon, index};
use crate::jobs::spawn_blocking;
use crate::session::Debounce;
use crate::theme::ActiveTheme;
use crate::ui::UI_FONT;
use crate::ui::ext::ToastKind;
use crate::ui::icons::icon;
use crate::ui::widgets::{LineInput, LineInputEvent, controls};
use crate::{RustdianApp, ViewportMode};

const PLACEHOLDER: &str = "搜索全部笔记… 支持 tag: path: file: \"短语\" -排除";
const LIST_MAX_HEIGHT: f32 = 440.0;
/// Syntax hints under an empty query: (label, text appended to the query).
const SYNTAX_CHIPS: [(&str, &str); 5] = [
    ("tag:", "tag:"),
    ("path:", "path:"),
    ("file:", "file:"),
    ("\"短语\"", "\""),
    ("-排除", "-"),
];

/// State of the panel (owned by `search_ui::State`).
#[derive(Default)]
pub struct PanelState {
    pub open: bool,
    input: Option<Entity<LineInput>>,
    _input_sub: Option<Subscription>,
    pub scope: Scope,
    results: Vec<Hit>,
    /// Highlight words of the last search (positive terms of the query).
    terms: Vec<String>,
    selected: usize,
    /// A search for the current query has finished (results are current).
    searched: bool,
    scroll: ScrollHandle,
    history: Vec<String>,
    history_loaded: bool,
    debounce: Debounce,
    search_seq: u64,
    task: Option<Task<()>>,
}

/// Highlight style of matches: accent text in bold, plus the accent wash on unselected rows (the
/// selected row already has the accent wash, so the text colour carries the match there).
fn match_style(t: &ActiveTheme, selected: bool) -> HighlightStyle {
    HighlightStyle {
        color: Some(t.ac.into()),
        font_weight: Some(FontWeight::BOLD),
        background_color: (!selected).then(|| t.ac_soft.into()),
        ..Default::default()
    }
}

/// Highlights for `ranges` of `text`, in the match style.
fn highlights(
    text: &str,
    ranges: &[Range<usize>],
    t: &ActiveTheme,
    selected: bool,
) -> Vec<(Range<usize>, HighlightStyle)> {
    logic::normalize_ranges(text, ranges)
        .into_iter()
        .map(|r| (r, match_style(t, selected)))
        .collect()
}

/// Matches of the query words in a title (case insensitive).
fn title_ranges(title: &str, terms: &[String]) -> Vec<Range<usize>> {
    let opts = MatchOpts::default();
    terms
        .iter()
        .flat_map(|term| logic::find_matches(title, term, opts))
        .collect()
}

/// Current text of the search field.
fn input_text(app: &RustdianApp, cx: &App) -> String {
    app.ext
        .search
        .panel
        .input
        .as_ref()
        .map(|input| input.read(cx).text())
        .unwrap_or_default()
}

/// Loads the recent searches of the vault once.
fn ensure_history(app: &mut RustdianApp) {
    if app.ext.search.panel.history_loaded {
        return;
    }
    let root = app.vault.root_dir.clone();
    let st = &mut app.ext.search.panel;
    st.history = logic::load_history(&root);
    st.history_loaded = true;
}

/// Opens the panel with an empty query. Opening it again only focuses the field.
pub fn open(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if app.ext.search.panel.open {
        if let Some(input) = app.ext.search.panel.input.clone() {
            let handle = input.read(cx).focus_handle(cx);
            focus_soon(cx, handle);
        }
        return;
    }
    ensure_history(app);
    let theme = app.theme;
    let input = match app.ext.search.panel.input.clone() {
        Some(input) => input,
        None => {
            let input = cx.new(|cx| LineInput::new(cx, PLACEHOLDER));
            let sub = cx.subscribe(
                &input,
                |app: &mut RustdianApp, _input, ev: &LineInputEvent, cx| match ev {
                    LineInputEvent::Changed(_) => on_query_changed(app, cx),
                    // Enter and Escape are bound by the field itself, so they arrive here, not in
                    // `handle_key`. Ctrl+Enter is not bound and still comes through `handle_key`.
                    LineInputEvent::Submit(_) => {
                        let ix = app.ext.search.panel.selected;
                        activate(app, ix, false, cx);
                    }
                    LineInputEvent::Cancel => close(app, cx),
                },
            );
            app.ext.search.panel._input_sub = Some(sub);
            app.ext.search.panel.input = Some(input.clone());
            input
        }
    };
    input.update(cx, |input, cx| {
        input.set_theme(theme, cx);
        input.set_text("", cx);
    });
    let st = &mut app.ext.search.panel;
    st.open = true;
    st.results.clear();
    st.terms.clear();
    st.selected = 0;
    st.searched = false;
    st.search_seq += 1;
    st.debounce.restart();
    let handle = input.read(cx).focus_handle(cx);
    focus_soon(cx, handle);
    cx.notify();
}

/// Opens the panel with `query` in the field and runs it (opening a saved-search bookmark).
pub fn open_with_query(app: &mut RustdianApp, query: &str, cx: &mut Context<RustdianApp>) {
    open(app, cx);
    if let Some(input) = app.ext.search.panel.input.clone() {
        input.update(cx, |i, cx| i.set_text(query, cx));
    }
    on_query_changed(app, cx);
}

/// Closes the panel. Keyboard focus goes back to the editor (the overlay state drives it).
pub fn close(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    app.ext.search.panel.open = false;
    app.ext.search.panel.search_seq += 1;
    cx.notify();
}

/// The field changed: search after a short pause (an empty field clears at once).
fn on_query_changed(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if input_text(app, cx).trim().is_empty() {
        app.ext.search.panel.debounce.restart();
        run_query(app, cx);
        return;
    }
    let st = &mut app.ext.search.panel;
    let token = st.debounce.restart();
    st.task = Some(cx.spawn(async move |this, cx| {
        cx.background_executor()
            .timer(Duration::from_millis(logic::SEARCH_DEBOUNCE_MS))
            .await;
        let _ = this.update(cx, |app, cx| {
            if app.ext.search.panel.debounce.is_current(token) {
                run_query(app, cx);
            }
        });
    }));
}

/// Runs the search for the field's text and the scope, in the background.
fn run_query(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let text = input_text(app, cx);
    let folder = app.active_path().map(logic::folder_of).unwrap_or_default();
    let scope = app.ext.search.panel.scope;
    let query = logic::build_query(scope, &text, &folder);
    let st = &mut app.ext.search.panel;
    st.terms = logic::query_terms(&text);
    st.search_seq += 1;
    st.selected = 0;
    st.scroll.scroll_to_item(0);
    let seq = st.search_seq;
    let index = index::index(app);
    let st = &mut app.ext.search.panel;
    let Some(index) = index.filter(|_| !query.is_empty()) else {
        st.results.clear();
        st.searched = false;
        cx.notify();
        return;
    };
    spawn_blocking(
        cx,
        move || {
            index
                .try_search(&query, logic::RESULT_LIMIT)
                .map_err(|e| e.to_string())
        },
        move |app: &mut RustdianApp,
              result: Result<Vec<Hit>, String>,
              cx: &mut Context<RustdianApp>| {
            let st = &mut app.ext.search.panel;
            if st.search_seq != seq {
                return;
            }
            match result {
                Ok(hits) => st.results = hits,
                Err(err) => {
                    st.results.clear();
                    app.toast(ToastKind::Warn, format!("搜索失败：{err}"));
                }
            }
            let st = &mut app.ext.search.panel;
            st.searched = true;
            st.selected = 0;
            st.scroll.scroll_to_item(0);
            cx.notify();
        },
    );
}

/// Sets the scope from the segmented control and searches again.
fn set_scope(app: &mut RustdianApp, ix: usize, cx: &mut Context<RustdianApp>) {
    app.ext.search.panel.scope = Scope::from_index(ix);
    run_query(app, cx);
    cx.notify();
}

/// Replaces the field text (a recent search) and searches.
fn set_query(app: &mut RustdianApp, text: &str, cx: &mut Context<RustdianApp>) {
    if let Some(input) = app.ext.search.panel.input.clone() {
        input.update(cx, |input, cx| input.set_text(text, cx));
        let handle = input.read(cx).focus_handle(cx);
        focus_soon(cx, handle);
    }
    run_query(app, cx);
}

/// Appends a syntax hint to the field text and searches.
fn append_query(app: &mut RustdianApp, extra: &str, cx: &mut Context<RustdianApp>) {
    let text = format!("{}{}", input_text(app, cx), extra);
    set_query(app, &text, cx);
}

fn move_selection(app: &mut RustdianApp, down: bool) {
    let st = &mut app.ext.search.panel;
    let len = st.results.len();
    if len == 0 {
        return;
    }
    st.selected = if down {
        (st.selected + 1).min(len - 1)
    } else {
        st.selected.saturating_sub(1)
    };
    st.scroll.scroll_to_item(st.selected);
}

/// Opens the note of result `ix`. `new_tab` opens a tab; otherwise the note replaces the current
/// tab. The caret goes to the first match line. The panel closes and the query is remembered.
fn activate(app: &mut RustdianApp, ix: usize, new_tab: bool, cx: &mut Context<RustdianApp>) {
    let Some(hit) = app.ext.search.panel.results.get(ix).cloned() else {
        return;
    };
    let key = index::norm(&hit.path);
    let Some(idx) = app
        .vault
        .notes
        .iter()
        .position(|n| index::norm(&n.path) == key)
    else {
        app.toast(ToastKind::Warn, "这篇笔记已经不存在");
        return;
    };
    let terms = app.ext.search.panel.terms.clone();
    let caret = logic::first_hit(app.vault.notes[idx].content(), &terms);
    let query = input_text(app, cx);
    let root = app.vault.root_dir.clone();
    let st = &mut app.ext.search.panel;
    logic::push_history(&mut st.history, &query);
    let _ = logic::save_history(&root, &st.history);

    if new_tab {
        app.open_note_by_index(idx);
    } else {
        open_in_current_tab(app, idx);
    }
    close(app, cx);
    if let Some((row, col)) = caret {
        app.request_source_caret(row, col);
    }
    cx.notify();
}

/// Shows note `idx` in the active tab (the tab takes the new note). A note that already has a tab,
/// or the first note of an empty tab set, is opened the normal way.
pub fn open_in_current_tab(app: &mut RustdianApp, idx: usize) {
    let vault = &mut app.vault;
    if vault.open_tabs.is_empty() || vault.open_tabs.contains(&idx) {
        app.open_note_by_index(idx);
        return;
    }
    let pos = vault.active_tab_index.min(vault.open_tabs.len() - 1);
    vault.open_tabs[pos] = idx;
    vault.active_tab_index = pos;
    vault.active_note_index = idx;
    app.show_palette = false;
    app.viewport_mode = ViewportMode::Editor;
}

/// Keys while the panel is open. Typing goes to the field (it is focused); this handles the list
/// and the close key. Returns true when the key was used.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent, cx: &mut Context<RustdianApp>) -> bool {
    let m = ev.keystroke.modifiers;
    let plain = !m.control && !m.alt && !m.platform;
    match ev.keystroke.key.as_str() {
        "escape" => {
            close(app, cx);
            true
        }
        "up" | "down" if plain => {
            move_selection(app, ev.keystroke.key == "down");
            true
        }
        "enter" if m.control && !m.alt && !m.platform => {
            let ix = app.ext.search.panel.selected;
            activate(app, ix, m.control, cx);
            true
        }
        _ => false,
    }
}

// ----- rendering --------------------------------------------------------------------------

fn section_label(text: &str, t: &ActiveTheme) -> AnyElement {
    div()
        .px(px(10.0))
        .pt(px(10.0))
        .pb(px(4.0))
        .text_size(px(11.0))
        .text_color(t.tx3)
        .child(text.to_string())
        .into_any_element()
}

fn hint(keys: AnyElement, text: &str, t: &ActiveTheme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(keys)
        .child(text.to_string())
        .text_color(t.tx3)
        .into_any_element()
}

/// One result row: title with matches, folder, and a two-line snippet with matches.
fn result_row(
    app: &RustdianApp,
    cx: &mut Context<RustdianApp>,
    ix: usize,
    hit: &Hit,
    selected: bool,
) -> AnyElement {
    let t = app.theme;
    let terms = &app.ext.search.panel.terms;
    let ranges = title_ranges(&hit.title, terms);
    let folder = logic::folder_of(&hit.path);
    let folder_label = if folder.is_empty() {
        "笔记库根目录".to_string()
    } else {
        folder
    };
    let title = StyledText::new(hit.title.clone())
        .with_highlights(highlights(&hit.title, &ranges, &t, selected));
    let snippet = StyledText::new(hit.snippet.clone()).with_highlights(highlights(
        &hit.snippet,
        &hit.highlights,
        &t,
        selected,
    ));
    div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .when(selected, |d| d.bg(t.ac_soft))
        .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.ext.search.panel.selected = ix;
                activate(this, ix, false, cx);
                cx.notify();
            }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.tx)
                        .child(title),
                )
                .child(
                    div()
                        .flex_none()
                        .max_w(px(200.0))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(11.5))
                        .text_color(t.tx3)
                        .child(folder_label),
                ),
        )
        .child(
            div()
                .h(px(36.0))
                .overflow_hidden()
                .text_size(px(12.0))
                .line_height(px(18.0))
                .text_color(t.tx3)
                .child(snippet),
        )
        .into_any_element()
}

/// A recent search row (click fills the field).
fn history_row(t: &ActiveTheme, cx: &mut Context<RustdianApp>, query: &str) -> AnyElement {
    let text = query.to_string();
    div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .px(px(12.0))
        .h(px(30.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(t.hover))
        .text_color(t.tx)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| set_query(this, &text, cx)),
        )
        .child(icon("clock", 13.0, t.tx3))
        .child(query.to_string())
        .into_any_element()
}

/// Renders the panel: a scrim and the card. The scrim closes the panel on a click.
pub fn render(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> AnyElement {
    let t = app.theme;
    let st = &app.ext.search.panel;
    let Some(input) = st.input.clone() else {
        return div().into_any_element();
    };
    let query_now = input.read(cx).text();
    let has_query = !query_now.trim().is_empty();

    let scrim = if t.kind.is_dark() {
        gpui::rgba(0x0000008C)
    } else {
        gpui::rgba(0x00000059)
    };

    let view = cx.entity().downgrade();
    let on_scope: controls::PickIndex = Rc::new(move |ix: &usize, _: &mut Window, cx: &mut App| {
        let ix = *ix;
        let _ = view.update(cx, |app, cx| set_scope(app, ix, cx));
    });
    let labels: Vec<&str> = Scope::ALL.iter().map(|s| s.label()).collect();
    let scope_row = div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(16.0))
        .pb(px(10.0))
        .child(controls::segmented(&t, &labels, st.scope.index(), on_scope))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(div().text_size(px(11.5)).text_color(t.tx3).child(
                    if has_query && st.searched {
                        format!("共 {} 条", st.results.len())
                    } else {
                        String::new()
                    },
                ))
                // 书签: saves the query as a bookmark (see ui/bookmarks).
                .when(has_query, |d| {
                    let save_query = query_now.clone();
                    d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .px(px(8.0))
                            .py(px(3.0))
                            .rounded(px(6.0))
                            .text_size(px(11.5))
                            .text_color(t.ac)
                            .cursor_pointer()
                            .hover(|s| s.bg(t.hover))
                            .child(icon("bookmark", 13.0, t.ac))
                            .child("加入书签")
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    crate::ui::ext::bookmarks::add_search(this, &save_query);
                                    cx.notify();
                                }),
                            ),
                    )
                }),
        );

    let header = div()
        .flex()
        .items_center()
        .gap(px(10.0))
        .px(px(16.0))
        .pt(px(14.0))
        .pb(px(10.0))
        .child(div().flex_1().min_w_0().child(input))
        .child(controls::kbd(&t, "Esc"));

    let mut list = div()
        .id("global-search-list")
        .flex()
        .flex_col()
        .max_h(px(LIST_MAX_HEIGHT))
        .overflow_y_scroll()
        .track_scroll(&st.scroll)
        .px(px(6.0))
        .pb(px(6.0));

    if !has_query {
        if !st.history.is_empty() {
            list = list.child(section_label("最近搜索", &t));
            for query in &st.history {
                list = list.child(history_row(&t, cx, query));
            }
        }
        list = list.child(section_label("语法", &t));
        let mut chips = div()
            .flex()
            .flex_wrap()
            .gap(px(6.0))
            .px(px(12.0))
            .pb(px(8.0));
        for (label, insert) in SYNTAX_CHIPS {
            let insert = insert.to_string();
            chips = chips.child(
                div()
                    .px(px(8.0))
                    .py(px(3.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(t.line)
                    .bg(t.sheet)
                    .text_size(px(11.5))
                    .text_color(t.tx3)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| append_query(this, &insert, cx)),
                    )
                    .child(label),
            );
        }
        list = list.child(chips);
    } else if st.results.is_empty() {
        if st.searched {
            list = list.child(controls::empty_state(
                &t,
                "search",
                "没有匹配的笔记",
                "换个关键词，或去掉 -排除 / 标签限制再试",
            ));
        } else {
            let text = if app.ext.search.service.index.is_some() {
                "正在搜索…"
            } else {
                "索引准备中…"
            };
            list = list.child(
                div()
                    .px(px(16.0))
                    .py(px(24.0))
                    .text_size(px(13.0))
                    .text_color(t.tx3)
                    .child(text),
            );
        }
    } else {
        for (ix, hit) in st.results.iter().enumerate() {
            list = list.child(result_row(app, cx, ix, hit, ix == st.selected));
        }
    }

    let footer = div()
        .flex()
        .items_center()
        .gap(px(18.0))
        .h(px(36.0))
        .px(px(16.0))
        .border_t_1()
        .border_color(t.line_soft)
        .bg(t.hover)
        .text_size(px(11.5))
        .text_color(t.tx3)
        .child(hint(
            icon("arrow-up", 12.0, t.tx3).into_any_element(),
            "选择",
            &t,
        ))
        .child(hint(
            controls::kbd(&t, "Enter").into_any_element(),
            "打开",
            &t,
        ))
        .child(hint(
            controls::kbd(&t, "Ctrl Enter").into_any_element(),
            "新标签",
            &t,
        ))
        .child(hint(
            controls::kbd(&t, "Esc").into_any_element(),
            "关闭",
            &t,
        ));

    let card = div()
        .occlude()
        .flex()
        .flex_col()
        .w(px(680.0))
        .bg(t.raised)
        .border_1()
        .border_color(t.line)
        .rounded(px(14.0))
        .shadow_lg()
        .overflow_hidden()
        .font_family(UI_FONT)
        .text_size(px(13.0))
        .text_color(t.tx)
        .child(header)
        .child(scope_row)
        .child(list)
        .child(footer);

    // The scrim and the card are siblings, so a click on the card does not reach the scrim.
    div()
        .absolute()
        .inset_0()
        .child(div().absolute().inset_0().bg(scrim).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| close(this, cx)),
        ))
        .child(
            div()
                .absolute()
                .top(px(96.0))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(card),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_matches_are_case_insensitive_and_cover_every_term() {
        let title = "Rust 与 沉积 rust";
        let mut ranges = title_ranges(title, &["rust".to_string(), "沉积".to_string()]);
        ranges.sort_by_key(|r| r.start);
        assert_eq!(ranges.len(), 3);
        let words: Vec<&str> = ranges.iter().map(|r| &title[r.clone()]).collect();
        assert_eq!(words, vec!["Rust", "沉积", "rust"]);
    }

    #[test]
    fn syntax_chips_cover_the_documented_syntax() {
        let labels: Vec<&str> = SYNTAX_CHIPS.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, vec!["tag:", "path:", "file:", "\"短语\"", "-排除"]);
    }
}
