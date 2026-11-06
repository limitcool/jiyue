//! Settings dialog (820x540, centred over a scrim). Only controls that change something
//! today are offered; the rest of the design is left out on purpose.

use gpui::{
    AnyElement, Context, Div, FontWeight, IntoElement, KeyDownEvent, MouseButton, Rgba,
    ScrollHandle, SharedString, div, img, prelude::*, px,
};

use crate::daily::{self, DailySettings, Date};
use crate::theme::{ActiveTheme, RADIUS_L, ThemeKind};
use crate::ui::icons::icon;
use crate::ui::{display_family, fonts};
use crate::{AppCommand, RustdianApp};

/// Index of the 外观 category (the default when the dialog opens).
pub const CAT_APPEARANCE: usize = 1;
/// Index of the 日记 category.
pub const CAT_DAILY: usize = 2;
/// Index of the 同步 category (sync settings, rendered by `sync_ui`).
pub const CAT_SYNC: usize = 3;
/// Index of the AI category (rendered by `ai_ui`).
pub const CAT_AI: usize = 4;
/// Index of the 笔记库 category (rendered by `vaults_ui`).
pub const CAT_LIBRARY: usize = 6;

/// Category rows: (label, icon name).
const CATEGORIES: [(&str, &str); 8] = [
    ("编辑器", "pencil"),
    ("外观", "palette"),
    ("日记", "calendar-event"),
    ("同步", "link"),
    ("AI", "bulb"),
    ("快捷键", "keyboard"),
    ("笔记库", "folder"),
    ("关于", "info-circle"),
];

/// Daily folder presets, shown as a segmented control.
const FOLDER_PRESETS: [&str; 3] = ["日记", "Daily", "笔记/日记"];
/// Daily date format presets, cycled by clicking the format button.
const DATE_FORMAT_PRESETS: [&str; 3] = ["YYYY-MM-DD", "YYYY年M月D日", "YYYYMMDD"];

/// State of the settings dialog. Add fields freely.
pub struct SettingsState {
    pub open: bool,
    /// Selected category index (0 editor, 1 appearance, ...).
    pub category: usize,
    /// Working copy of the daily-note settings, written through on every change.
    /// `None` until the first change; the vault's file is read until then.
    pub daily: Option<DailySettings>,
    /// Scroll position of the right pane. Reset to the top when the category changes.
    pub scroll: ScrollHandle,
    /// Open font picker and number field of the 外观 page (see `ui/fonts_ui.rs`).
    pub fonts: crate::ui::fonts_ui::FontUi,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            open: false,
            category: CAT_APPEARANCE,
            daily: None,
            scroll: ScrollHandle::default(),
            fonts: Default::default(),
        }
    }
}

/// Keyboard handling while the dialog is open. Return true when the key was consumed.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent) -> bool {
    if app.settings.open && ev.keystroke.key == "escape" {
        if crate::ui::fonts_ui::handle_escape(app) {
            return true;
        }
        app.settings.open = false;
        return true;
    }
    false
}

/// Next entry of `options` after `current` (wraps around). Unknown values give the first entry.
pub fn next_preset<'a>(options: &[&'a str], current: &str) -> &'a str {
    match options.iter().position(|o| *o == current) {
        Some(i) => options[(i + 1) % options.len()],
        None => options[0],
    }
}

/// The example shown under the date format: `date` rendered with `fmt`.
pub fn date_format_example(date: Date, fmt: &str) -> String {
    daily::format_date(date, fmt)
}

impl RustdianApp {
    /// Writes a change to the daily settings: updates the working copy and saves it to the vault.
    fn update_daily(&mut self, change: impl FnOnce(&mut DailySettings)) {
        let mut settings = self
            .settings
            .daily
            .clone()
            .unwrap_or_else(|| self.vault.daily_settings());
        change(&mut settings);
        let _ = settings.save(&self.vault.root_dir);
        self.settings.daily = Some(settings);
    }

    /// Full-window overlay with the dialog. Called only while `settings.open` is true.
    pub(crate) fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let category = self.settings.category.min(CATEGORIES.len() - 1);
        let scrim = if t.kind.is_dark() {
            gpui::rgba(0x0000008c)
        } else {
            gpui::rgba(0x00000059)
        };

        let mut nav = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .w(px(200.0))
            .flex_none()
            .h_full()
            .bg(t.chrome)
            // The dialog clips only its rectangle: the nav paints its own left corners.
            .rounded_l(px(RADIUS_L))
            .pt(px(20.0))
            .pb(px(14.0))
            .pl(px(12.0))
            .pr(px(10.0))
            .child(
                div()
                    .px(px(8.0))
                    .pb(px(12.0))
                    .font_family(display_family(&t))
                    .text_size(px(17.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.tx)
                    .child("设置"),
            );
        for (idx, (name, ic)) in CATEGORIES.iter().enumerate() {
            nav = nav.child(nav_row(&t, idx, name, ic, idx == category, cx));
        }

        let body: AnyElement = match category {
            0 => editor_section(self, cx).into_any_element(),
            1 => appearance_section(self, cx).into_any_element(),
            2 => daily_section(self, cx).into_any_element(),
            3 => crate::ui::sync_ui::render_settings_section(self, cx),
            4 => crate::ui::ai_ui::render_settings_section(self, cx),
            5 => crate::ui::keymap_ui::render_section(self, cx),
            6 => crate::ui::vaults_ui::render_library_section(self, cx),
            _ => about_section(&t).into_any_element(),
        };
        let title = CATEGORIES[category].0;

        let close = div()
            .absolute()
            .right(px(14.0))
            .top(px(14.0))
            .size(px(30.0))
            .rounded(px(8.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .child(icon("x", 16.0, t.tx2))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.run_command(AppCommand::CloseSettings);
                    cx.notify();
                }),
            );

        // The heading stays put; the section scrolls when it is taller than the pane.
        let scroll_body = div()
            .id("settings_body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.settings.scroll)
            .pt(px(8.0))
            .pb(px(24.0))
            .pr(px(6.0))
            .child(body);
        let main = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .px(px(28.0))
            .pt(px(22.0))
            .flex()
            .flex_col()
            .overflow_hidden()
            .children(crate::ui::config_ui::banner(self, cx))
            .child(heading(&t, title))
            .child(scroll_body)
            .child(close);

        let dialog = div()
            .flex()
            .w(px(820.0))
            .h(px(540.0))
            .rounded(px(RADIUS_L))
            .overflow_hidden()
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .shadow_lg()
            // Clicks inside the dialog must not reach the scrim, which closes it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(nav)
            .child(main);

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(scrim)
            // Blocks the mouse for the sidebar and the editor below the scrim.
            .occlude()
            .font_family(fonts::ui_name())
            .text_size(px(13.0))
            .text_color(t.tx)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.run_command(AppCommand::CloseSettings);
                    cx.notify();
                }),
            )
            .child(dialog)
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// Building blocks
// ---------------------------------------------------------------------------

fn heading(t: &ActiveTheme, text: &str) -> Div {
    div()
        .mb(px(8.0))
        .font_family(display_family(t))
        .text_size(px(20.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.tx)
        .child(SharedString::from(text.to_string()))
}

fn nav_row(
    t: &ActiveTheme,
    idx: usize,
    name: &str,
    ic: &str,
    on: bool,
    cx: &mut Context<RustdianApp>,
) -> Div {
    let color = if on { t.ac } else { t.tx3 };
    let mut row = div()
        .flex()
        .items_center()
        .gap(px(9.0))
        .w_full()
        .h(px(34.0))
        .px(px(8.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .text_size(px(13.0))
        .text_color(if on { t.tx } else { t.tx2 })
        .font_weight(if on {
            FontWeight::MEDIUM
        } else {
            FontWeight::NORMAL
        })
        .child(icon(ic, 16.0, color))
        .child(SharedString::from(name.to_string()));
    if on {
        row = row.bg(t.active);
    } else {
        row = row.hover(|s| s.bg(t.hover));
    }
    row.on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _, _, cx| {
            this.settings.category = idx;
            this.settings.scroll.scroll_to_item(0);
            cx.notify();
        }),
    )
}

/// One settings row: title and description on the left, a control on the right.
pub(crate) fn setting_row(
    t: &ActiveTheme,
    title: &str,
    desc: &str,
    control: impl IntoElement,
    last: bool,
) -> Div {
    let left = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .min_w_0()
        .child(
            div()
                .text_size(px(13.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx)
                .child(SharedString::from(title.to_string())),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(t.tx3)
                .child(SharedString::from(desc.to_string())),
        );
    let mut row = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(20.0))
        .py(px(8.0))
        .child(left)
        .child(control);
    if !last {
        row = row.border_b_1().border_color(t.line_soft);
    }
    row
}

/// Muted explanatory line.
fn note(t: &ActiveTheme, text: &str) -> Div {
    div()
        .pt(px(12.0))
        .text_size(px(12.0))
        .text_color(t.tx3)
        .child(SharedString::from(text.to_string()))
}

/// Pill-shaped switch (36x20).
fn toggle(t: &ActiveTheme, on: bool) -> Div {
    let knob_left = if on { 18.0 } else { 2.0 };
    div()
        .relative()
        .flex_none()
        .w(px(36.0))
        .h(px(20.0))
        .rounded(px(10.0))
        .cursor_pointer()
        .bg(if on { t.ac } else { t.active })
        .border_1()
        .border_color(if on { t.ac } else { t.line })
        .child(
            div()
                .absolute()
                .top(px(1.0))
                .left(px(knob_left))
                .size(px(16.0))
                .rounded_full()
                .bg(t.raised),
        )
}

fn kbd(t: &ActiveTheme, text: &str) -> Div {
    div()
        .flex_none()
        .px(px(6.0))
        .py(px(1.0))
        .rounded(px(5.0))
        .border_1()
        .border_color(t.line)
        .bg(t.raised)
        .text_size(px(11.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.tx3)
        .child(SharedString::from(text.to_string()))
}

/// One theme preview card: a mini layout drawn with the theme's own colours.
fn theme_card(
    t: &ActiveTheme,
    kind: ThemeKind,
    current: ThemeKind,
    cx: &mut Context<RustdianApp>,
) -> Div {
    let p = ActiveTheme::from_kind(kind);
    let selected = kind == current;
    let bar = |x: f32, y: f32, w: f32, h: f32, color: Rgba| {
        div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w))
            .h(px(h))
            .rounded(px(2.0))
            .bg(color)
    };
    let line_col = Rgba { a: 0.3, ..p.ac };

    let mut preview = div()
        .relative()
        .w(px(96.0))
        .h(px(64.0))
        .rounded(px(8.0))
        .overflow_hidden()
        .bg(p.sheet)
        .border_1()
        .border_color(p.line)
        .child(
            div()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .bottom(px(0.0))
                .w(px(26.0))
                .bg(p.chrome),
        )
        .child(bar(34.0, 10.0, 30.0, 5.0, p.ac))
        .child(bar(34.0, 22.0, 44.0, 3.0, line_col))
        .child(bar(34.0, 30.0, 34.0, 3.0, line_col))
        .child(bar(34.0, 38.0, 40.0, 3.0, line_col));
    if selected {
        preview = preview.child(
            div()
                .absolute()
                .right(px(5.0))
                .top(px(5.0))
                .size(px(16.0))
                .rounded_full()
                .bg(t.ac)
                .flex()
                .items_center()
                .justify_center()
                .child(icon("check", 11.0, t.ac_on)),
        );
    }

    let ring = div()
        .w(px(104.0))
        .h(px(72.0))
        .rounded(px(10.0))
        .border_2()
        .border_color(if selected {
            t.ac
        } else {
            Rgba { a: 0.0, ..t.ac }
        })
        .flex()
        .items_center()
        .justify_center()
        .child(preview);

    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.0))
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.run_command(AppCommand::SetTheme(kind));
                cx.notify();
            }),
        )
        .child(ring)
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(if selected {
                    FontWeight::MEDIUM
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if selected { t.ac } else { t.tx2 })
                .child(kind.name_zh()),
        )
}

/// Segmented control: one segment per preset, the matching one highlighted.
fn segmented(
    t: &ActiveTheme,
    options: &[&'static str],
    current: &str,
    cx: &mut Context<RustdianApp>,
    apply: fn(&mut DailySettings, &str),
) -> Div {
    let mut control = div()
        .flex()
        .h(px(30.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(t.line)
        .bg(t.sheet)
        .overflow_hidden();
    for (i, name) in options.iter().copied().enumerate() {
        let on = name == current;
        let mut seg = div()
            .flex()
            .items_center()
            .px(px(12.0))
            .h_full()
            .text_size(px(12.5))
            .cursor_pointer()
            .text_color(if on { t.tx } else { t.tx2 })
            .font_weight(if on {
                FontWeight::MEDIUM
            } else {
                FontWeight::NORMAL
            })
            .child(SharedString::from(name.to_string()));
        if on {
            seg = seg.bg(t.active);
        }
        if i > 0 {
            seg = seg.border_l_1().border_color(t.line_soft);
        }
        seg = seg.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.update_daily(|d| apply(d, name));
                cx.notify();
            }),
        );
        control = control.child(seg);
    }
    control
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

fn editor_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    div()
        .flex()
        .flex_col()
        .mt(px(10.0))
        .child(setting_row(
            &t,
            "实时预览",
            "在编辑区直接显示 Markdown 效果",
            toggle(&t, app.live_preview).on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.live_preview = !this.live_preview;
                    cx.notify();
                }),
            ),
            false,
        ))
        .child(setting_row(
            &t,
            "新建笔记时用首个标题作为文件名",
            "新笔记用 # 标题 的文字命名文件（仅限本次运行中新建的笔记）",
            toggle(
                &t,
                crate::ui::files_ui::autoname::load(&app.vault.root_dir).auto_name_from_heading,
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    crate::ui::files_ui::autoname::toggle(this);
                    cx.notify();
                }),
            ),
            true,
        ))
        .child(note(&t, "更多编辑选项将在支持文本编辑后提供"))
}

fn appearance_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let mut cards = div().flex().gap(px(8.0)).mt(px(10.0));
    for kind in ThemeKind::all() {
        cards = cards.child(theme_card(&t, kind, t.kind, cx));
    }
    let theme_block = div()
        .flex()
        .flex_col()
        .py(px(8.0))
        .border_b_1()
        .border_color(t.line_soft)
        .child(
            div()
                .text_size(px(13.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx)
                .child("主题"),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(t.tx3)
                .child("选择界面的配色方案"),
        )
        .child(cards);

    let left_toggle = toggle(&t, app.left_sidebar_open).on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, _, _, cx| {
            this.run_command(AppCommand::ToggleSidebar);
            cx.notify();
        }),
    );
    let right_toggle = toggle(&t, app.right_sidebar_open).on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, _, _, cx| {
            this.run_command(AppCommand::ToggleRightPanel);
            cx.notify();
        }),
    );

    div()
        .flex()
        .flex_col()
        .child(theme_block)
        .child(setting_row(
            &t,
            "显示左侧栏",
            "文件树、搜索与标签所在的侧栏",
            left_toggle,
            false,
        ))
        .child(setting_row(
            &t,
            "显示右侧栏",
            "大纲与反向链接所在的面板",
            right_toggle,
            true,
        ))
        .child(crate::ui::fonts_ui::render_section(app, cx))
}

fn daily_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let daily = app
        .settings
        .daily
        .clone()
        .unwrap_or_else(|| app.vault.daily_settings());

    let folder = segmented(&t, &FOLDER_PRESETS, &daily.folder, cx, |d, name| {
        d.folder = name.to_string()
    });

    let format_button = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .w(px(200.0))
        .h(px(32.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(t.line)
        .bg(t.sheet)
        .cursor_pointer()
        .text_size(px(13.0))
        .text_color(t.tx)
        .child(SharedString::from(daily.date_format.clone()))
        .child(icon("selector", 15.0, t.tx3))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.update_daily(|d| {
                    d.date_format = next_preset(&DATE_FORMAT_PRESETS, &d.date_format).to_string();
                });
                cx.notify();
            }),
        );
    let example = date_format_example(Date::today(), &daily.date_format);
    let format_control = div()
        .flex()
        .flex_col()
        .items_end()
        .gap(px(4.0))
        .child(format_button)
        .child(
            div()
                .text_size(px(11.5))
                .text_color(t.tx3)
                .child(SharedString::from(format!("示例：{example}"))),
        );

    div()
        .flex()
        .flex_col()
        .mt(px(10.0))
        .child(setting_row(
            &t,
            "日记文件夹",
            "新建日记的位置，相对于笔记库根目录",
            folder,
            false,
        ))
        .child(setting_row(
            &t,
            "日期格式",
            "日记标题与文件名使用的日期格式",
            format_control,
            false,
        ))
        .child(crate::ui::template_ui::daily_template_row(app, cx))
        .child(crate::ui::template_ui::folder_row(app, cx))
        .child(note(
            &t,
            "已有的日记不会被重命名，新的设置只影响之后创建的日记。",
        ))
}

fn about_section(t: &ActiveTheme) -> Div {
    div()
        .flex()
        .flex_col()
        .mt(px(10.0))
        .gap(px(6.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(14.0))
                .child(img("logo/icon-64.png").size(px(64.0)).flex_none())
                .child(
                    div()
                        .font_family(display_family(t))
                        .text_size(px(22.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.tx)
                        .child("霁月 Jiyue"),
                ),
        )
        .child(
            div()
                .text_size(px(13.0))
                .text_color(t.tx2)
                .child("基于 GPUI 的本地 Markdown 笔记应用"),
        )
        .child(
            div()
                .text_size(px(13.0))
                .text_color(t.tx2)
                .child(SharedString::from(format!(
                    "版本 {}",
                    env!("CARGO_PKG_VERSION")
                ))),
        )
        .child(
            div()
                .pt(px(8.0))
                .text_size(px(12.5))
                .text_color(t.tx3)
                .child("笔记保存在本地的 .md 文件中"),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_cycling() {
        let options = ["a", "b", "c"];
        assert_eq!(next_preset(&options, "a"), "b");
        assert_eq!(next_preset(&options, "b"), "c");
        assert_eq!(next_preset(&options, "c"), "a");
        assert_eq!(next_preset(&options, "unknown"), "a");
    }

    #[test]
    fn date_format_presets_cycle_through_all() {
        let mut current = DATE_FORMAT_PRESETS[0].to_string();
        let mut seen = Vec::new();
        for _ in 0..DATE_FORMAT_PRESETS.len() {
            current = next_preset(&DATE_FORMAT_PRESETS, &current).to_string();
            seen.push(current.clone());
        }
        assert_eq!(seen, vec!["YYYY年M月D日", "YYYYMMDD", "YYYY-MM-DD"]);
    }

    #[test]
    fn date_format_examples() {
        let date = Date::new(2026, 11, 4).expect("valid date");
        assert_eq!(date_format_example(date, "YYYY-MM-DD"), "2026-11-04");
        assert_eq!(date_format_example(date, "YYYY年M月D日"), "2026年11月4日");
        assert_eq!(date_format_example(date, "YYYYMMDD"), "20261104");
    }

    #[test]
    fn presets_are_parseable_formats() {
        let date = Date::new(2026, 11, 4).expect("valid date");
        for fmt in DATE_FORMAT_PRESETS {
            let text = date_format_example(date, fmt);
            assert_eq!(daily::parse_date(&text, fmt), Some(date), "format {fmt}");
        }
    }

    #[test]
    fn default_state_opens_appearance() {
        let state = SettingsState::default();
        assert!(!state.open);
        assert_eq!(state.category, CAT_APPEARANCE);
        assert!(state.daily.is_none());
        assert_eq!(CATEGORIES[CAT_DAILY].0, "日记");
    }

    #[test]
    fn sync_and_ai_categories_follow_daily() {
        assert_eq!(CATEGORIES[CAT_SYNC].0, "同步");
        assert_eq!(CATEGORIES[CAT_AI].0, "AI");
        assert_eq!(CAT_SYNC, CAT_DAILY + 1);
        assert_eq!(CAT_AI, CAT_SYNC + 1);
        assert_eq!(CATEGORIES[CAT_AI + 1].0, "快捷键");
        assert_eq!(CATEGORIES.len(), 8);
    }
}
