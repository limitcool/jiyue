//! Command palette and quick switcher (Ctrl+K: notes first, Ctrl+P: commands first).
//! Layout follows the original palette mock-up.

use crate::ui::{ext, folder_key, keymap};
use gpui::{
    AnyElement, Context, ElementId, Entity, FontWeight, IntoElement, KeyDownEvent, MouseButton,
    ScrollHandle, SharedString, Subscription, div, prelude::*, px,
};

use crate::theme::{ActiveTheme, ThemeKind};
use crate::ui::icons::icon;
use crate::ui::widgets::{LineInput, LineInputEvent};
use crate::{AppCommand, RustdianApp};

/// Notes shown under 最近笔记 when the query is empty.
const RECENT_LIMIT: usize = 3;
/// Upper bound on note matches for a typed query (keeps rendering cheap in big vaults).
const SEARCH_LIMIT: usize = 50;
const LIST_MAX_HEIGHT: f32 = 440.0;
const DANGER_RGB: u32 = 0xE5484D;
const RECENT_LABEL: &str = "最近笔记";
const COMMAND_LABEL: &str = "命令";
/// Command rows for an empty query in quick-switch mode (Ctrl+K), in display order.
const EMPTY_SWITCH_COMMANDS: [&str; 4] = ["新建笔记", "切换主题", "打开知识图谱", "向右拆分"];
/// Command rows for an empty query in command mode (Ctrl+P), in display order.
const EMPTY_COMMAND_MODE_COMMANDS: [&str; 8] = [
    "新建笔记",
    "今日日记",
    "打开知识图谱",
    "向右拆分",
    "切换主题",
    "打开设置",
    "切换侧栏",
    "保存笔记",
];
const PLACEHOLDER: &str = "输入命令或笔记名…";

/// One selectable row: an index into `vault.notes` or into `commands()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Note(usize),
    Command(usize),
}

/// Non-empty sections in visual order, each with its heading.
pub type Sections = Vec<(&'static str, Vec<Entry>)>;

/// A row of the 命令 section.
#[derive(Clone)]
pub struct PaletteCommand {
    pub label: &'static str,
    pub icon: &'static str,
    /// Shortcut text shown as a chip, read from the registry (`ui/keymap.rs`); empty when unbound.
    pub shortcut: String,
    pub danger: bool,
    pub command: AppCommand,
}

/// Palette state. The selection is an index into the flattened rows (both sections).
#[derive(Default)]
pub struct PaletteState {
    pub selected: usize,
    /// Scroll handle of the result list; used to keep the selected row visible.
    pub scroll: ScrollHandle,
    /// Set once the palette has been seen open; the query and selection reset on each open.
    pub was_open: bool,
    /// The query field (IME-capable). `RustdianApp::palette_query` mirrors its text.
    pub input: Option<Entity<LineInput>>,
    /// Subscriptions of `input`, kept alive here.
    pub subs: Vec<Subscription>,
}

/// The command list in display order.
pub fn commands() -> Vec<PaletteCommand> {
    let mut list = vec![
        cmd(
            "新建笔记",
            "square-rounded-plus",
            "Ctrl N",
            AppCommand::NewNote,
        ),
        cmd("保存笔记", "check", "", AppCommand::SaveNote),
        cmd(
            "今日日记",
            "calendar-event",
            "Ctrl Shift D",
            AppCommand::OpenToday,
        ),
        cmd("上一篇日记", "chevron-left", "", AppCommand::DailyPrev),
        cmd("下一篇日记", "chevron-right", "", AppCommand::DailyNext),
        cmd("打开知识图谱", "affiliate", "Ctrl G", AppCommand::OpenGraph),
        cmd(
            "向右拆分",
            "layout-columns",
            "Ctrl \\",
            AppCommand::SplitRight,
        ),
        cmd("向下拆分", "arrow-down", "", AppCommand::SplitDown),
        cmd("关闭分屏", "x", "", AppCommand::CloseSplit),
        cmd("后退", "chevron-left", "", AppCommand::NavBack),
        cmd("前进", "chevron-right", "", AppCommand::NavForward),
        cmd("切换侧栏", "folder", "Ctrl B", AppCommand::ToggleSidebar),
        cmd(
            "切换右栏",
            "layout-sidebar-right",
            "",
            AppCommand::ToggleRightPanel,
        ),
        cmd(
            "插入脚注",
            "hash",
            "",
            AppCommand::Ext(crate::ui::ext::ExtCommand::InsertFootnote),
        ),
        cmd("打开设置", "settings", "Ctrl ,", AppCommand::OpenSettings),
        cmd("切换主题", "palette", "Ctrl T", AppCommand::CycleTheme),
        cmd(
            "主题：纸",
            "palette",
            "",
            AppCommand::SetTheme(ThemeKind::Paper),
        ),
        cmd(
            "主题：墨",
            "palette",
            "",
            AppCommand::SetTheme(ThemeKind::Ink),
        ),
        cmd(
            "主题：苔",
            "palette",
            "",
            AppCommand::SetTheme(ThemeKind::Moss),
        ),
        cmd(
            "主题：石墨",
            "palette",
            "",
            AppCommand::SetTheme(ThemeKind::Graphite),
        ),
        cmd(
            "主题：雾",
            "palette",
            "",
            AppCommand::SetTheme(ThemeKind::Fog),
        ),
    ];
    // Rows contributed by feature modules come after the built-in ones. `删除当前笔记` lives only
    // in the files module (confirm dialog + trash); it is the single danger row.
    list.extend(ext::palette_commands().into_iter().map(|c| PaletteCommand {
        label: c.label,
        icon: c.icon,
        shortcut: keymap::shortcut_for(&c.command),
        danger: matches!(
            c.command,
            AppCommand::Ext(crate::ui::ext::ExtCommand::Files(
                crate::ui::files_ui::Command::DeleteActive
            ))
        ),
        command: c.command,
    }));
    list
}

fn cmd(
    label: &'static str,
    icon: &'static str,
    _shortcut: &'static str,
    command: AppCommand,
) -> PaletteCommand {
    PaletteCommand {
        label,
        icon,
        shortcut: keymap::shortcut_for(&command),
        danger: false,
        command,
    }
}

/// Case-insensitive substring match. `query` must already be trimmed and lowercased.
fn label_matches(label: &str, query: &str) -> bool {
    query.is_empty() || label.to_lowercase().contains(query)
}

fn normalize(query: &str) -> String {
    query.trim().to_lowercase()
}

/// Note indices for 最近笔记. Empty query: open tabs first, then vault order, capped at
/// `RECENT_LIMIT`. Typed query: every note whose title matches, capped at `SEARCH_LIMIT`.
pub fn note_rows(titles: &[&str], open_tabs: &[usize], query: &str) -> Vec<usize> {
    let query = normalize(query);
    if query.is_empty() {
        let mut out: Vec<usize> = Vec::new();
        for idx in open_tabs.iter().copied().chain(0..titles.len()) {
            if idx < titles.len() && !out.contains(&idx) {
                out.push(idx);
                if out.len() == RECENT_LIMIT {
                    break;
                }
            }
        }
        out
    } else {
        titles
            .iter()
            .enumerate()
            .filter(|(_, title)| label_matches(title, &query))
            .map(|(idx, _)| idx)
            .take(SEARCH_LIMIT)
            .collect()
    }
}

/// Command indices whose label matches the query.
pub fn command_rows(labels: &[&str], query: &str) -> Vec<usize> {
    let query = normalize(query);
    labels
        .iter()
        .enumerate()
        .filter(|(_, label)| label_matches(label, &query))
        .map(|(idx, _)| idx)
        .collect()
}

/// Command indices for the 命令 section. An empty query shows only the preset rows of the mode,
/// so the danger row never appears there; a typed query filters every command.
pub fn command_rows_for(labels: &[&str], query: &str, command_first: bool) -> Vec<usize> {
    if !normalize(query).is_empty() {
        return command_rows(labels, query);
    }
    let preset: &[&str] = if command_first {
        &EMPTY_COMMAND_MODE_COMMANDS[..]
    } else {
        &EMPTY_SWITCH_COMMANDS[..]
    };
    preset
        .iter()
        .filter_map(|want| labels.iter().position(|label| label == want))
        .collect()
}

/// Muted hint row shown under the list while the query is empty.
pub fn empty_hint(total_commands: usize) -> String {
    crate::t!("palette-empty-hint", count = total_commands)
}

/// Builds the two sections; `command_first` puts 命令 above 最近笔记. Empty sections are dropped.
pub fn build_sections(notes: Vec<usize>, commands: Vec<usize>, command_first: bool) -> Sections {
    let note_section = (
        RECENT_LABEL,
        notes.into_iter().map(Entry::Note).collect::<Vec<_>>(),
    );
    let command_section = (
        COMMAND_LABEL,
        commands.into_iter().map(Entry::Command).collect::<Vec<_>>(),
    );
    let mut out = if command_first {
        vec![command_section, note_section]
    } else {
        vec![note_section, command_section]
    };
    out.retain(|(_, rows)| !rows.is_empty());
    out
}

/// All selectable rows in visual order.
pub fn flatten(sections: &Sections) -> Vec<Entry> {
    sections
        .iter()
        .flat_map(|(_, rows)| rows.iter().copied())
        .collect()
}

/// Child index (headings count as children) of the flattened row `flat`, for scrolling.
pub fn child_index(sections: &Sections, flat: usize) -> usize {
    let mut pos = 0;
    let mut headings = 0;
    for (_, rows) in sections {
        headings += 1;
        if flat < pos + rows.len() {
            return flat + headings;
        }
        pos += rows.len();
    }
    flat + headings
}

/// Moves the selection by one row, wrapping at both ends.
pub fn step(selected: usize, len: usize, down: bool) -> usize {
    if len == 0 {
        return 0;
    }
    let cur = selected.min(len - 1);
    if down {
        (cur + 1) % len
    } else if cur == 0 {
        len - 1
    } else {
        cur - 1
    }
}

fn current_sections(app: &RustdianApp) -> Sections {
    let query = app.palette_query.as_str();
    let titles: Vec<&str> = app.vault.notes.iter().map(|n| n.title.as_str()).collect();
    let notes = note_rows(&titles, &app.vault.open_tabs, query);
    let labels: Vec<&str> = commands().iter().map(|c| c.label).collect();
    let command_rows = command_rows_for(&labels, query, app.is_command_mode);
    build_sections(notes, command_rows, app.is_command_mode)
}

fn scroll_to_flat(app: &RustdianApp, sections: &Sections, flat: usize) {
    app.palette
        .scroll
        .scroll_to_item(child_index(sections, flat));
}

/// Runs a row (from the keyboard or a click) and closes the palette.
fn run_entry(app: &mut RustdianApp, entry: Entry) {
    match entry {
        Entry::Note(idx) => app.run_command(AppCommand::OpenNote(idx)),
        Entry::Command(ix) => {
            if let Some(item) = commands().into_iter().nth(ix) {
                app.run_command(item.command);
            }
        }
    }
    app.run_command(AppCommand::ClosePalette);
}

/// Creates the palette's query field and subscribes to it. Called once from `RustdianApp::new`.
/// Typing, Enter (Submit) and Escape (Cancel) reach the app through `LineInputEvent`.
pub fn install(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let input = cx.new(|cx| LineInput::new(cx, PLACEHOLDER));
    let sub = cx.subscribe(
        &input,
        |this: &mut RustdianApp, _, ev: &LineInputEvent, cx| {
            match ev {
                LineInputEvent::Changed(text) => {
                    this.palette_query = text.clone();
                    this.palette.selected = 0;
                    this.palette.scroll.scroll_to_item(0);
                }
                LineInputEvent::Submit(_) => {
                    let entries = flatten(&current_sections(this));
                    if let Some(entry) = entries.get(this.palette.selected).copied() {
                        run_entry(this, entry);
                    }
                }
                LineInputEvent::Cancel => this.run_command(AppCommand::ClosePalette),
            }
            cx.notify();
        },
    );
    app.palette.input = Some(input);
    app.palette.subs.push(sub);
}

/// Once per opening: clears the query and its field, and asks for focus for the field.
/// Called from `RustdianApp::render` before the focus step.
pub fn sync_open(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    if !app.show_palette {
        app.palette.was_open = false;
        return;
    }
    if app.palette.was_open {
        return;
    }
    app.palette.was_open = true;
    app.palette.selected = 0;
    app.palette_query.clear();
    app.palette.scroll.scroll_to_item(0);
    if let Some(input) = app.palette.input.clone() {
        input.update(cx, |i, cx| i.set_text("", cx));
    }
    app.want_focus = true;
}

/// Keyboard handling, called first for every key press. Returns true when consumed.
/// Typed text, Backspace, Enter and Escape belong to the query field (`LineInput`).
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent) -> bool {
    // Closed palette: global shortcuts are the registry's (main.rs).
    if !app.show_palette {
        return false;
    }

    let m = ev.keystroke.modifiers;
    let plain = !m.control && !m.alt && !m.platform;
    let key = ev.keystroke.key.as_str();
    match key {
        "up" | "down" if plain => {
            let sections = current_sections(app);
            let len = flatten(&sections).len();
            if len > 0 {
                let next = step(app.palette.selected, len, key == "down");
                app.palette.selected = next;
                scroll_to_flat(app, &sections, next);
            }
            true
        }
        _ => false,
    }
}

/// A key cap (`kbd`) holding `child`.
fn key_chip(t: &ActiveTheme) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .px(px(6.0))
        .h(px(18.0))
        .rounded(px(5.0))
        .border_1()
        .border_color(t.line)
        .bg(t.hover)
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.tx2)
        .whitespace_nowrap()
}

fn key_text(text: &str, t: &ActiveTheme) -> AnyElement {
    key_chip(t).child(text.to_string()).into_any_element()
}

fn key_icon(name: &str, t: &ActiveTheme) -> AnyElement {
    key_chip(t)
        .child(icon(name, 12.0, t.tx2))
        .into_any_element()
}

fn footer_hint(keys: Vec<AnyElement>, label: &str) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .children(keys)
        .child(label.to_string())
        .into_any_element()
}

fn hint_row(text: &str, t: &ActiveTheme) -> AnyElement {
    div()
        .px(px(16.0))
        .py(px(8.0))
        .text_size(px(12.0))
        .text_color(t.tx3)
        .child(text.to_string())
        .into_any_element()
}

fn section_label(label: &str, t: &ActiveTheme) -> AnyElement {
    div()
        .px(px(16.0))
        .pt(px(10.0))
        .pb(px(4.0))
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.tx3)
        .child(label.to_string())
        .into_any_element()
}

impl RustdianApp {
    /// Quick switcher (Ctrl+K) and command palette (Ctrl+P): a scrim plus the 580px modal.
    pub(crate) fn render_palette_modal(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        let query = self.palette_query.as_str();
        let sections = current_sections(self);
        let entries = flatten(&sections);
        let selected = self.palette.selected.min(entries.len().saturating_sub(1));
        let cmds = commands();

        let scrim = if t.kind.is_dark() {
            gpui::rgba(0x0000008C)
        } else {
            gpui::rgba(0x00000059)
        };

        let mut rows: Vec<AnyElement> = Vec::new();
        let mut flat = 0usize;
        for (label, items) in &sections {
            rows.push(section_label(label, &t));
            for entry in items {
                rows.push(self.palette_row(cx, &cmds, flat, *entry, flat == selected));
                flat += 1;
            }
        }
        if rows.is_empty() {
            rows.push(
                div()
                    .px(px(16.0))
                    .py(px(24.0))
                    .text_size(px(13.0))
                    .text_color(t.tx3)
                    .child(SharedString::from(crate::t!("palette-no-matches")))
                    .into_any_element(),
            );
        }

        if query.trim().is_empty() {
            rows.push(hint_row(&empty_hint(cmds.len()), &t));
        }

        let field = match &self.palette.input {
            Some(input) => {
                input.update(cx, |i, cx| i.set_theme(t, cx));
                input.clone().into_any_element()
            }
            None => div().into_any_element(),
        };
        let search = div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .h(px(54.0))
            .px(px(16.0))
            .border_b_1()
            .border_color(t.line_soft)
            .child(icon("search", 18.0, t.tx3))
            .child(div().flex_1().min_w_0().child(field))
            .child(key_text("Esc", &t));

        let list = div()
            .id("palette_list")
            .flex()
            .flex_col()
            .max_h(px(LIST_MAX_HEIGHT))
            .overflow_y_scroll()
            .track_scroll(&self.palette.scroll)
            .pb(px(6.0))
            .children(rows);

        let footer = div()
            .flex()
            .items_center()
            .gap(px(18.0))
            .h(px(38.0))
            .px(px(16.0))
            .border_t_1()
            .border_color(t.line_soft)
            .bg(t.hover)
            .text_size(px(11.5))
            .text_color(t.tx3)
            .child(footer_hint(
                vec![key_icon("arrow-up", &t), key_icon("arrow-down", &t)],
                &crate::t!("palette-footer-select"),
            ))
            .child(footer_hint(
                vec![key_icon("corner-down-left", &t)],
                &crate::t!("palette-footer-open"),
            ))
            .child(footer_hint(
                vec![key_text("Esc", &t)],
                &crate::t!("palette-footer-close"),
            ));

        let card = div()
            .flex()
            .flex_col()
            .w(px(580.0))
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .rounded(px(14.0))
            .shadow_lg()
            .overflow_hidden()
            .font_family(crate::ui::fonts::ui_name())
            .text_size(px(13.0))
            .text_color(t.tx)
            .child(search)
            .child(list)
            .child(footer);

        // The scrim and the card are siblings so a click on the card does not reach the scrim.
        // `occlude` stops clicks on the scrim from reaching the sidebar and the editor below it.
        div()
            .absolute()
            .inset_0()
            .occlude()
            .child(div().absolute().inset_0().bg(scrim).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.run_command(AppCommand::ClosePalette);
                    cx.notify();
                }),
            ))
            .child(
                div()
                    .absolute()
                    .top(px(110.0))
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(card),
            )
    }

    /// One selectable row: icon, label, folder name or shortcut chip.
    fn palette_row(
        &self,
        cx: &mut Context<Self>,
        cmds: &[PaletteCommand],
        flat: usize,
        entry: Entry,
        selected: bool,
    ) -> AnyElement {
        let t = self.theme;
        let danger = gpui::rgb(DANGER_RGB);
        let (icon_name, label, meta, shortcut, is_danger) = match entry {
            Entry::Note(idx) => {
                let Some(note) = self.vault.notes.get(idx) else {
                    return div().into_any_element();
                };
                let folder = folder_key(&note.path)
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .to_string();
                (
                    "file-text",
                    note.title.clone(),
                    folder,
                    String::new(),
                    false,
                )
            }
            Entry::Command(ix) => {
                let Some(item) = cmds.get(ix) else {
                    return div().into_any_element();
                };
                (
                    item.icon,
                    item.label.to_string(),
                    String::new(),
                    item.shortcut.to_string(),
                    item.danger,
                )
            }
        };

        let text_color = if is_danger {
            danger
        } else if selected {
            t.ac
        } else {
            t.tx
        };
        let icon_color = if is_danger {
            danger
        } else if selected {
            t.ac
        } else {
            t.tx3
        };
        let meta_color = if selected { t.ac } else { t.tx3 };

        let mut row = div()
            .id(ElementId::NamedInteger(
                SharedString::from("palette_row"),
                flat as u64,
            ))
            .flex()
            .items_center()
            .gap(px(10.0))
            .h(px(36.0))
            .ml(px(6.0))
            .mr(px(6.0))
            .px(px(10.0))
            .rounded(px(8.0))
            .text_size(px(13.0))
            .text_color(text_color)
            .cursor_pointer()
            .when(selected, |d| d.bg(t.ac_soft))
            .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
            .child(icon(icon_name, 16.0, icon_color))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(label),
            );
        if !meta.is_empty() {
            row = row.child(
                div()
                    .flex_none()
                    .text_size(px(12.0))
                    .text_color(meta_color)
                    .child(meta),
            );
        }
        if !shortcut.is_empty() {
            row = row.child(key_text(&shortcut, &t));
        }

        row.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                run_entry(this, entry);
                cx.notify();
            }),
        )
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_match_is_case_insensitive() {
        assert!(label_matches("GPUI 笔记", "gpui"));
        assert!(!label_matches("读书笔记", "gpui"));
        assert!(label_matches("anything", ""));
    }

    #[test]
    fn empty_query_lists_open_tabs_first() {
        let titles = ["a", "b", "c", "d"];
        assert_eq!(note_rows(&titles, &[2, 0], ""), vec![2, 0, 1]);
    }

    #[test]
    fn empty_query_is_capped_to_recent_limit() {
        let titles = ["a", "b", "c", "d", "e"];
        assert_eq!(note_rows(&titles, &[], "  ").len(), RECENT_LIMIT);
    }

    #[test]
    fn query_filters_note_titles() {
        let titles = ["Rust notes", "Daily", "rust book"];
        assert_eq!(note_rows(&titles, &[], "RUST"), vec![0, 2]);
    }

    #[test]
    fn command_filter_matches_labels() {
        let labels = ["新建笔记", "切换主题", "主题：墨"];
        assert_eq!(command_rows(&labels, "主题"), vec![1, 2]);
        assert_eq!(command_rows(&labels, "  "), vec![0, 1, 2]);
        assert!(command_rows(&labels, "zzz").is_empty());
    }

    #[test]
    fn step_wraps_at_both_ends() {
        assert_eq!(step(2, 3, true), 0);
        assert_eq!(step(0, 3, false), 2);
        assert_eq!(step(0, 3, true), 1);
        assert_eq!(step(1, 3, false), 0);
        assert_eq!(step(0, 0, true), 0);
    }

    #[test]
    fn section_order_follows_mode() {
        let notes_first = build_sections(vec![0], vec![1], false);
        assert_eq!(notes_first[0].0, RECENT_LABEL);
        assert_eq!(notes_first[1].0, COMMAND_LABEL);
        let commands_first = build_sections(vec![0], vec![1], true);
        assert_eq!(commands_first[0].0, COMMAND_LABEL);
        assert_eq!(commands_first[1].0, RECENT_LABEL);
    }

    #[test]
    fn empty_sections_are_dropped() {
        let sections = build_sections(vec![], vec![3], false);
        assert_eq!(sections.len(), 1);
        assert_eq!(flatten(&sections), vec![Entry::Command(3)]);
        assert!(build_sections(vec![], vec![], true).is_empty());
    }

    #[test]
    fn child_index_counts_headings() {
        let sections = build_sections(vec![5, 6], vec![9], false);
        // Children: heading, note 5, note 6, heading, command 9.
        assert_eq!(child_index(&sections, 0), 1);
        assert_eq!(child_index(&sections, 1), 2);
        assert_eq!(child_index(&sections, 2), 4);
    }

    fn labels_of(list: &[PaletteCommand]) -> Vec<&'static str> {
        list.iter().map(|c| c.label).collect()
    }

    #[test]
    fn empty_switch_mode_lists_four_core_commands() {
        let cmds = commands();
        let labels = labels_of(&cmds);
        let got: Vec<&str> = command_rows_for(&labels, "", false)
            .into_iter()
            .map(|i| labels[i])
            .collect();
        assert_eq!(got, EMPTY_SWITCH_COMMANDS.to_vec());
    }

    #[test]
    fn empty_command_mode_lists_eight_useful_commands() {
        let cmds = commands();
        let labels = labels_of(&cmds);
        let got: Vec<&str> = command_rows_for(&labels, "  ", true)
            .into_iter()
            .map(|i| labels[i])
            .collect();
        assert_eq!(got, EMPTY_COMMAND_MODE_COMMANDS.to_vec());
        assert_eq!(got.len(), 8);
    }

    #[test]
    fn empty_state_never_shows_danger_row() {
        let cmds = commands();
        let labels = labels_of(&cmds);
        for command_first in [false, true] {
            let rows = command_rows_for(&labels, "", command_first);
            assert!(rows.iter().all(|&i| !cmds[i].danger));
        }
    }

    #[test]
    fn typed_query_searches_all_commands_including_danger() {
        let cmds = commands();
        let labels = labels_of(&cmds);
        // Other modules add rows (files, sync), so keep only the danger row of this label.
        let danger: Vec<usize> = command_rows_for(&labels, "删除当前笔记", false)
            .into_iter()
            .filter(|&i| cmds[i].danger)
            .collect();
        assert_eq!(danger.len(), 1);
        assert!(cmds[danger[0]].danger);
        assert_eq!(command_rows_for(&labels, "主题", false).len(), 6);
        assert_eq!(command_rows_for(&labels, "主题", true).len(), 6);
    }

    #[test]
    fn preset_rows_exist_in_command_table() {
        let cmds = commands();
        let labels = labels_of(&cmds);
        for want in EMPTY_SWITCH_COMMANDS
            .iter()
            .chain(EMPTY_COMMAND_MODE_COMMANDS.iter())
        {
            assert!(labels.contains(want), "missing preset row {want}");
        }
    }

    #[test]
    fn split_commands_are_listed_by_direction() {
        let cmds = commands();
        let labels = labels_of(&cmds);
        assert!(labels.contains(&"向右拆分"));
        assert!(labels.contains(&"向下拆分"));
        assert!(labels.contains(&"关闭分屏"));
        assert!(!labels.contains(&"切换分屏"));
    }

    #[test]
    fn hint_reports_real_command_count() {
        // The table grows as feature modules add rows, so compare with the live count.
        let n = commands().len();
        assert!(n >= 17);
        assert_eq!(
            empty_hint(n),
            format!("输入以搜索全部 {n} 条命令和所有笔记")
        );
    }

    #[test]
    fn command_table_has_one_danger_row() {
        let list = commands();
        assert_eq!(list.iter().filter(|c| c.danger).count(), 1);
        assert!(list.iter().all(|c| !c.label.is_empty()));
    }

    #[test]
    fn delete_note_appears_once_as_the_danger_row() {
        let list = commands();
        let rows: Vec<&PaletteCommand> =
            list.iter().filter(|c| c.label == "删除当前笔记").collect();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].danger);
    }
}
