//! Settings dialog (880x600, centred over a scrim).

use std::rc::Rc;

use futures_util::StreamExt;
use gpui::{
    AnyElement, Context, Div, Entity, FontWeight, IntoElement, KeyDownEvent, MouseButton, Rgba,
    ScrollHandle, SharedString, div, img, prelude::*, px,
};

use crate::daily::{self, DailySettings, Date};
use crate::theme::{ActiveTheme, RADIUS_L, RADIUS_M, ThemeKind, ThemeMode};
use crate::ui::icons::icon;
use crate::ui::widgets::controls::{
    ButtonKind, ButtonSize, button, button_sized, choice_chip, form_row, form_row_with_extra,
    icon_button, progress_bar, section_title, select, setting_card, spinner, toggle_view,
};
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
/// Index of the Memos category (rendered by `memos_ui`).
pub const CAT_MEMOS: usize = 5;
/// Index of the 笔记库 category (rendered by `vaults_ui`).
pub const CAT_LIBRARY: usize = 7;

/// Category rows: (label, icon name).
const CATEGORIES: [(&str, &str); 9] = [
    ("编辑器", "pencil"),
    ("外观", "palette"),
    ("日记", "calendar-event"),
    ("同步", "link"),
    ("AI", "bulb"),
    ("Memos", "checkbox"),
    ("快捷键", "keyboard"),
    ("笔记库", "folder"),
    ("关于", "info-circle"),
];

/// Returns the localized label for a category.
pub fn category_label(idx: usize) -> String {
    let key = match idx {
        0 => "settings-cat-editor",
        1 => "settings-cat-appearance",
        2 => "settings-cat-daily",
        3 => "settings-cat-sync",
        4 => "settings-cat-ai",
        5 => "settings-cat-memos",
        6 => "settings-cat-keymap",
        7 => "settings-cat-vaults",
        _ => "settings-cat-about",
    };
    crate::t!(key)
}

/// Daily folder presets, shown as a segmented control.
const FOLDER_PRESETS: [&str; 3] = ["Daily", "Journal", "Notes/Daily"];
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
    /// Auto update state.
    pub update: crate::update::UpdateState,
    /// Input entity for custom daily folder.
    pub daily_folder_input: Option<Entity<crate::ui::widgets::LineInput>>,
    /// Input entity for custom template folder.
    pub template_folder_input: Option<Entity<crate::ui::widgets::LineInput>>,
    /// Whether the daily date-format dropdown is open.
    pub daily_format_open: bool,
    /// Whether the daily note template dropdown is open.
    pub daily_template_open: bool,
    pub _daily_sub: Option<gpui::Subscription>,
    pub _tmpl_sub: Option<gpui::Subscription>,
    /// State of mobile config export QR modal.
    pub export_qr_open: bool,
    pub export_include_sync: bool,
    pub export_include_ai: bool,
    pub export_include_memos: bool,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            open: false,
            category: CAT_APPEARANCE,
            daily: None,
            scroll: ScrollHandle::default(),
            fonts: Default::default(),
            update: crate::update::UpdateState::Idle,
            daily_folder_input: None,
            template_folder_input: None,
            daily_format_open: false,
            daily_template_open: false,
            _daily_sub: None,
            _tmpl_sub: None,
            export_qr_open: false,
            export_include_sync: true,
            export_include_ai: true,
            export_include_memos: true,
        }
    }
}

/// Initializes the settings inputs for custom folders.
pub fn install(app: &mut RustdianApp, cx: &mut Context<RustdianApp>) {
    let t = app.theme;
    let daily = app
        .settings
        .daily
        .clone()
        .unwrap_or_else(|| app.vault.daily_settings());
    let initial_daily_folder = daily.folder.clone();
    let daily_input = cx.new(|cx| {
        let mut input = crate::ui::widgets::LineInput::new(cx, "自定义目录");
        input.set_theme(t, cx);
        input.set_text(&initial_daily_folder, cx);
        input
    });
    let sub_daily = cx.subscribe(
        &daily_input,
        |this: &mut RustdianApp, _, ev: &crate::ui::widgets::LineInputEvent, cx| match ev {
            crate::ui::widgets::LineInputEvent::Changed(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    this.update_daily(|d| d.folder = trimmed.to_string());
                    cx.notify();
                }
            }
            crate::ui::widgets::LineInputEvent::Submit(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    this.update_daily(|d| d.folder = trimmed.to_string());
                    this.toast(
                        crate::ui::ext::ToastKind::Info,
                        format!("日记目录已设为: {trimmed}"),
                    );
                    cx.notify();
                }
            }
            crate::ui::widgets::LineInputEvent::Cancel => {}
        },
    );

    let current_tmpl = jiyue_export::TemplateRules::load(&app.vault.root_dir).templates_folder;
    let template_input = cx.new(|cx| {
        let mut input = crate::ui::widgets::LineInput::new(cx, "自定义模板目录");
        input.set_theme(t, cx);
        input.set_text(&current_tmpl, cx);
        input
    });
    let sub_tmpl = cx.subscribe(
        &template_input,
        |this: &mut RustdianApp, _, ev: &crate::ui::widgets::LineInputEvent, cx| match ev {
            crate::ui::widgets::LineInputEvent::Changed(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    let root = this.vault.root_dir.clone();
                    let mut rules = jiyue_export::TemplateRules::load(&root);
                    rules.templates_folder = trimmed.to_string();
                    let _ = rules.save(&root);
                    cx.notify();
                }
            }
            crate::ui::widgets::LineInputEvent::Submit(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    let root = this.vault.root_dir.clone();
                    let mut rules = jiyue_export::TemplateRules::load(&root);
                    rules.templates_folder = trimmed.to_string();
                    let _ = rules.save(&root);
                    this.toast(
                        crate::ui::ext::ToastKind::Info,
                        format!("模板目录已设为: {trimmed}"),
                    );
                    cx.notify();
                }
            }
            crate::ui::widgets::LineInputEvent::Cancel => {}
        },
    );

    app.settings.daily_folder_input = Some(daily_input);
    app.settings.template_folder_input = Some(template_input);
    app.settings._daily_sub = Some(sub_daily);
    app.settings._tmpl_sub = Some(sub_tmpl);
}

/// Keyboard handling while the dialog is open. Return true when the key was consumed.
pub fn handle_key(app: &mut RustdianApp, ev: &KeyDownEvent) -> bool {
    if app.settings.open && ev.keystroke.key == "escape" {
        if crate::ui::fonts_ui::handle_escape(app) {
            return true;
        }
        if app.settings.daily_format_open || app.settings.daily_template_open {
            app.settings.daily_format_open = false;
            app.settings.daily_template_open = false;
            return true;
        }
        app.settings.open = false;
        return true;
    }
    false
}

/// Next entry of `options` after `current` (wraps around). Unknown values give the first entry.
#[cfg(test)]
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
    /// Checks for application updates from GitHub. If `manual` is true, displays toast notifications.
    pub fn trigger_check_update(&mut self, manual: bool, cx: &mut Context<Self>) {
        if matches!(
            self.settings.update,
            crate::update::UpdateState::Checking | crate::update::UpdateState::Downloading { .. }
        ) {
            return;
        }
        self.settings.update = crate::update::UpdateState::Checking;
        cx.notify();

        let current_version = env!("CARGO_PKG_VERSION");
        let owner = crate::update::DEFAULT_REPO_OWNER;
        let repo = crate::update::DEFAULT_REPO_NAME;

        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move { crate::update::check_for_update(owner, repo, current_version) })
                .await;
            let _ = this.update(cx, |app, cx| {
                match res {
                    Ok(Some(info)) => {
                        let ver = info.version.clone();
                        app.settings.update = crate::update::UpdateState::Available(info);
                        app.toast(
                            crate::ui::ext::ToastKind::Info,
                            format!("发现新版本 v{ver}，可前往设置查看"),
                        );
                    }
                    Ok(None) => {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        app.settings.update =
                            crate::update::UpdateState::UpToDate { checked_at: now };
                        if manual {
                            app.toast(crate::ui::ext::ToastKind::Success, "当前已是最新版本");
                        }
                    }
                    Err(err) => {
                        app.settings.update = crate::update::UpdateState::Failed(err.clone());
                        if manual {
                            app.toast(
                                crate::ui::ext::ToastKind::Error,
                                format!("检查更新失败: {err}"),
                            );
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Downloads the available update installer.
    pub fn trigger_download_update(
        &mut self,
        info: crate::update::UpdateInfo,
        cx: &mut Context<Self>,
    ) {
        let version = info.version.clone();
        let total = info.file_size;
        self.settings.update = crate::update::UpdateState::Downloading {
            version: version.clone(),
            progress: 0.0,
            downloaded: 0,
            total,
        };
        cx.notify();

        let (tx, mut rx) = futures_channel::mpsc::unbounded::<(f32, u64, u64)>();
        let info_clone = info.clone();

        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    crate::update::download_installer(&info_clone, move |p, d, t| {
                        let _ = tx.unbounded_send((p, d, t));
                    })
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                match res {
                    Ok(installer_path) => {
                        app.settings.update = crate::update::UpdateState::Ready {
                            version: version.clone(),
                            installer_path,
                        };
                        app.toast(
                            crate::ui::ext::ToastKind::Success,
                            format!("新版本 v{version} 已下载就绪，可立即重启更新"),
                        );
                    }
                    Err(err) => {
                        app.settings.update = crate::update::UpdateState::Failed(err.clone());
                        app.toast(
                            crate::ui::ext::ToastKind::Error,
                            format!("下载更新失败: {err}"),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx| {
            while let Some((p, d, t)) = rx.next().await {
                let _ = this.update(cx, |app, cx| {
                    if let crate::update::UpdateState::Downloading {
                        progress,
                        downloaded,
                        total,
                        ..
                    } = &mut app.settings.update
                    {
                        *progress = p;
                        *downloaded = d;
                        *total = t;
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    /// Triggers silent installation and app relaunch.
    pub fn trigger_apply_update(
        &mut self,
        installer_path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        if let Err(err) = crate::update::apply_update_and_restart(&installer_path) {
            self.toast(
                crate::ui::ext::ToastKind::Error,
                format!("启动更新失败: {err}"),
            );
            cx.notify();
        }
    }

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
        let scrim = t.scrim;

        let mut nav = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .w(px(204.0))
            .flex_none()
            .h_full()
            .bg(t.chrome)
            .border_r_1()
            .border_color(t.border_subtle)
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
                    .child(SharedString::from(crate::t!("settings-title"))),
            );
        for (idx, (_name, ic)) in CATEGORIES.iter().enumerate() {
            let label = category_label(idx);
            nav = nav.child(nav_row(&t, idx, &label, ic, idx == category, cx));
        }
        nav = nav
            .child(div().flex_1())
            .child(div().h(px(1.0)).w_full().my(px(6.0)).bg(t.border_subtle))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .w_full()
                    .h(px(34.0))
                    .px(px(8.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .text_size(px(13.0))
                    .text_color(t.tx2)
                    .hover(|s| s.bg(t.hover).text_color(t.tx))
                    .child(icon("qrcode", 16.0, t.ac))
                    .child("手机扫码同步")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.settings.export_qr_open = true;
                            cx.notify();
                        }),
                    ),
            );

        let body: AnyElement = match category {
            0 => editor_section(self, cx).into_any_element(),
            1 => appearance_section(self, cx).into_any_element(),
            2 => daily_section(self, cx).into_any_element(),
            3 => crate::ui::sync_ui::render_settings_section(self, cx),
            4 => crate::ui::ai_ui::render_settings_section(self, cx),
            5 => crate::ui::memos_ui::render_settings_section(self, cx),
            6 => crate::ui::keymap_ui::render_section(self, cx),
            7 => crate::ui::vaults_ui::render_library_section(self, cx),
            _ => about_section(self, cx).into_any_element(),
        };
        let title = category_label(category);

        let close_btn = icon_button(
            &t,
            "x",
            cx.listener(|this, _, _, cx| {
                this.run_command(AppCommand::CloseSettings);
                cx.notify();
            }),
        );

        let heading_row = div()
            .flex()
            .items_center()
            .justify_between()
            .pb(px(12.0))
            .child(heading(&t, &title))
            .child(close_btn);

        // The heading stays put; the section scrolls when it is taller than the pane.
        let scroll_body = div()
            .id("settings_body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.settings.scroll)
            .pt(px(2.0))
            .pb(px(24.0))
            .pr(px(8.0))
            .child(body);
        let main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(t.surface)
            .px(px(28.0))
            .pt(px(20.0))
            .flex()
            .flex_col()
            .overflow_hidden()
            .children(crate::ui::config_ui::banner(self, cx))
            .child(heading_row)
            .child(scroll_body);

        let dialog = div()
            .flex()
            .w(px(880.0))
            .h(px(600.0))
            .rounded(px(RADIUS_L))
            .overflow_hidden()
            .bg(t.surface)
            .border_1()
            .border_color(t.border_default)
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
            .when(self.settings.export_qr_open, |d| {
                d.child(crate::ui::export_config::render_modal(self, cx))
            })
            .into_any_element()
    }
}

// ---------------------------------------------------------------------------
// Building blocks
// ---------------------------------------------------------------------------

fn heading(t: &ActiveTheme, text: &str) -> Div {
    div()
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
            this.settings.daily_format_open = false;
            this.settings.daily_template_open = false;
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
    form_row(t, title, desc, control, last)
}

/// Muted explanatory line.
fn note(t: &ActiveTheme, text: &str) -> Div {
    div()
        .pt(px(10.0))
        .text_size(px(12.0))
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

    let card_border = if selected {
        t.ac
    } else {
        match (t.mode, p.mode) {
            (ThemeMode::Light, ThemeMode::Dark) => Rgba {
                a: 0.16,
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
            (ThemeMode::Dark, ThemeMode::Dark) => p.line_soft,
            _ => t.border_default,
        }
    };

    let mut preview = div()
        .relative()
        .w(px(100.0))
        .h(px(68.0))
        .rounded(px(9.0))
        .overflow_hidden()
        .bg(p.sheet)
        .border_2()
        .border_color(card_border)
        .shadow_sm()
        .hover(|h| {
            if !selected {
                h.border_color(t.line).shadow_md()
            } else {
                h
            }
        })
        .child(
            div()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .bottom(px(0.0))
                .w(px(27.0))
                .rounded_l(px(7.0))
                .bg(p.chrome),
        )
        .child(bar(35.0, 11.0, 32.0, 5.0, p.ac))
        .child(bar(35.0, 23.0, 46.0, 3.0, line_col))
        .child(bar(35.0, 32.0, 36.0, 3.0, line_col))
        .child(bar(35.0, 41.0, 42.0, 3.0, line_col));

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
                .child(icon("check", 10.0, t.ac_on)),
        );
    }

    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(5.0))
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.run_command(AppCommand::SetTheme(kind));
                cx.notify();
            }),
        )
        .child(preview)
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(if selected {
                    FontWeight::MEDIUM
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if selected { t.ac } else { t.tx2 })
                .child(kind.localized_name()),
        )
}

// ---------------------------------------------------------------------------
// Sections
// ---------------------------------------------------------------------------

fn editor_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    div().flex().flex_col().child(
        setting_card(&t)
            .child(setting_row(
                &t,
                &crate::t!("settings-live-preview"),
                &crate::t!("settings-live-preview-desc"),
                toggle_view(&t, app.live_preview).on_mouse_down(
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
                &crate::t!("settings-autoname"),
                &crate::t!("settings-autoname-desc"),
                toggle_view(
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
                false,
            ))
            .child(setting_row(
                &t,
                "附件与图片粘贴",
                "配置粘贴图片与拖入附件的保存位置与整理规则",
                button_sized(
                    &t,
                    "配置规则…",
                    ButtonKind::Secondary,
                    ButtonSize::Small,
                    Some("folder"),
                    cx.listener(|this, _, _, cx| {
                        crate::ui::attach_ui::open_settings(this, cx);
                        cx.notify();
                    }),
                ),
                true,
            )),
    )
}

fn appearance_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let mut cards = div().flex().gap(px(10.0)).mt(px(10.0));
    for kind in ThemeKind::all() {
        cards = cards.child(theme_card(&t, kind, t.kind, cx));
    }
    let theme_block = div()
        .flex()
        .flex_col()
        .py(px(12.0))
        .border_b_1()
        .border_color(t.line_soft)
        .child(
            div()
                .text_size(px(13.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.tx)
                .child(SharedString::from(crate::t!("settings-theme"))),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(t.tx3)
                .child(SharedString::from(crate::t!("settings-theme-desc"))),
        )
        .child(cards);

    let left_toggle = toggle_view(&t, app.left_sidebar_open).on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, _, _, cx| {
            this.run_command(AppCommand::ToggleSidebar);
            cx.notify();
        }),
    );
    let right_toggle = toggle_view(&t, app.right_sidebar_open).on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, _, _, cx| {
            this.run_command(AppCommand::ToggleRightPanel);
            cx.notify();
        }),
    );

    let cur_locale = crate::i18n::configured_locale();
    let mut lang_buttons = div().flex().flex_none().items_center().gap(px(6.0));
    for (code, label) in crate::i18n::SUPPORTED_LOCALES {
        let is_selected = cur_locale == code;
        let btn = choice_chip(
            &t,
            label,
            is_selected,
            cx.listener(move |this, _, _, cx| {
                crate::i18n::set_locale(code);
                crate::config_store::sync_ui(this);
                cx.notify();
            }),
        );
        lang_buttons = lang_buttons.child(btn);
    }

    let lang_row = setting_row(
        &t,
        &crate::t!("settings-language"),
        &crate::t!("settings-language-desc"),
        lang_buttons,
        false,
    );

    let theme_card_group = setting_card(&t)
        .child(theme_block)
        .child(lang_row)
        .child(setting_row(
            &t,
            &crate::t!("settings-show-left-sidebar"),
            &crate::t!("settings-show-left-sidebar-desc"),
            left_toggle,
            false,
        ))
        .child(setting_row(
            &t,
            &crate::t!("settings-show-right-panel"),
            &crate::t!("settings-show-right-panel-desc"),
            right_toggle,
            true,
        ));

    div()
        .flex()
        .flex_col()
        .child(section_title(&t, &crate::t!("settings-theme-section")).pt(px(4.0)))
        .child(theme_card_group)
        .child(crate::ui::fonts_ui::render_section(app, cx))
}

fn daily_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let daily = app
        .settings
        .daily
        .clone()
        .unwrap_or_else(|| app.vault.daily_settings());

    if let Some(inp) = &app.settings.daily_folder_input {
        inp.update(cx, |i, cx| i.set_theme(t, cx));
    }
    if let Some(inp) = &app.settings.template_folder_input {
        inp.update(cx, |i, cx| i.set_theme(t, cx));
    }

    let mut folder_presets = div().flex().items_center().justify_end().gap(px(6.0));
    for name in FOLDER_PRESETS {
        let on = name == daily.folder;
        folder_presets = folder_presets.child(choice_chip(
            &t,
            name,
            on,
            cx.listener(move |this, _, _, cx| {
                this.update_daily(|d| d.folder = name.to_string());
                if let Some(inp) = &this.settings.daily_folder_input {
                    inp.update(cx, |i, cx| i.set_text(name, cx));
                }
                cx.notify();
            }),
        ));
    }

    let folder_input = div()
        .w(px(200.0))
        .h(px(30.0))
        .when_some(app.settings.daily_folder_input.clone(), |d, inp| {
            d.child(inp)
        });

    let mut format_options: Vec<&str> = DATE_FORMAT_PRESETS.to_vec();
    if !format_options.contains(&daily.date_format.as_str()) {
        format_options.push(daily.date_format.as_str());
    }
    let fmt_selected = format_options
        .iter()
        .position(|f| *f == daily.date_format)
        .unwrap_or(0);
    let owned_formats: Vec<String> = format_options.iter().map(|s| (*s).to_string()).collect();
    let on_pick_format = Rc::new(cx.listener(move |this, idx: &usize, _, cx| {
        if let Some(fmt) = owned_formats.get(*idx).cloned() {
            this.update_daily(|d| d.date_format = fmt);
        }
        this.settings.daily_format_open = false;
        cx.notify();
    }));

    let format_select = select(
        &t,
        &format_options,
        fmt_selected,
        app.settings.daily_format_open,
        fmt_selected,
        220.0,
        cx.listener(|this, _, _, cx| {
            this.settings.daily_format_open = true;
            this.settings.daily_template_open = false;
            cx.notify();
        }),
        cx.listener(|this, _, _, cx| {
            this.settings.daily_format_open = false;
            cx.notify();
        }),
        on_pick_format,
    );
    let example = date_format_example(Date::today(), &daily.date_format);
    let format_desc = format!(
        "{} · {}",
        crate::t!("settings-daily-format-desc"),
        crate::t!("settings-daily-format-example", example = example.as_str())
    );

    div()
        .flex()
        .flex_col()
        .child(
            setting_card(&t)
                .child(form_row_with_extra(
                    &t,
                    &crate::t!("settings-daily-folder"),
                    &crate::t!("settings-daily-folder-desc"),
                    folder_input,
                    folder_presets,
                    false,
                ))
                .child(setting_row(
                    &t,
                    &crate::t!("settings-daily-format"),
                    &format_desc,
                    format_select,
                    false,
                ))
                .child(crate::ui::template_ui::daily_template_row(app, cx))
                .child(crate::ui::template_ui::folder_row(app, cx)),
        )
        .child(note(&t, &crate::t!("settings-daily-note-hint")))
}

fn about_section(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let brand_card = setting_card(&t).py(px(16.0)).gap(px(10.0)).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(14.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(14.0))
                    .child(img("logo/icon-64.png").size(px(56.0)).flex_none())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .child(
                                div()
                                    .font_family(display_family(&t))
                                    .text_size(px(20.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.tx)
                                    .child("霁月 Jiyue"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .text_color(t.tx3)
                                    .child("基于 GPUI 的本地 Markdown 笔记应用"),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(t.tx2)
                    .child(SharedString::from(format!(
                        "{} v{}",
                        crate::t!("update-current-version"),
                        env!("CARGO_PKG_VERSION")
                    ))),
            ),
    );

    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(brand_card)
        .child(update_block(app, cx))
        .child(note(
            &t,
            "笔记保存在本地的纯文本 Markdown 文件中，无需联网即可使用。",
        ))
}

fn update_block(app: &RustdianApp, cx: &mut Context<RustdianApp>) -> Div {
    let t = app.theme;
    let card = setting_card(&t).py(px(14.0)).gap(px(10.0));

    match &app.settings.update {
        crate::update::UpdateState::Idle => {
            let btn = button(
                &t,
                &crate::t!("update-check-now"),
                ButtonKind::Secondary,
                Some("refresh"),
                cx.listener(|this, _, _, cx| {
                    this.trigger_check_update(true, cx);
                }),
            );
            card.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(t.tx2)
                            .child(SharedString::from(crate::t!("update-channel"))),
                    )
                    .child(btn),
            )
        }
        crate::update::UpdateState::Checking => card.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(spinner(&t, 16.0))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(t.tx2)
                        .child(SharedString::from(crate::t!("update-checking"))),
                ),
        ),
        crate::update::UpdateState::UpToDate { .. } => {
            let recheck = button(
                &t,
                &crate::t!("update-check-now"),
                ButtonKind::Secondary,
                Some("refresh"),
                cx.listener(|this, _, _, cx| {
                    this.trigger_check_update(true, cx);
                }),
            );
            card.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(icon("check", 16.0, t.ac))
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .text_color(t.tx)
                                    .child(SharedString::from(crate::t!("update-up-to-date"))),
                            ),
                    )
                    .child(recheck),
            )
        }
        crate::update::UpdateState::Available(info) => {
            let info_clone = info.clone();
            let dl_btn = button(
                &t,
                &crate::t!("update-download"),
                ButtonKind::Primary,
                Some("download"),
                cx.listener(move |this, _, _, cx| {
                    this.trigger_download_update(info_clone.clone(), cx);
                }),
            );
            let mb = (info.file_size as f64) / (1024.0 * 1024.0);
            let size_text = if info.file_size > 0 {
                format!("({:.1} MB)", mb)
            } else {
                String::new()
            };
            let mut col = div().flex().flex_col().gap(px(10.0));
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(icon("arrow-up", 18.0, t.ac))
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(14.0))
                                    .text_color(t.tx)
                                    .child(SharedString::from(format!(
                                        "发现新版本 v{}",
                                        info.version
                                    ))),
                            )
                            .when(!size_text.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(t.tx3)
                                        .child(SharedString::from(size_text)),
                                )
                            }),
                    )
                    .child(dl_btn),
            );
            let cleaned_notes = crate::update::format_release_notes_plain(&info.release_notes);
            if !cleaned_notes.is_empty() {
                col = col.child(
                    div()
                        .id("update_release_notes")
                        .max_h(px(160.0))
                        .overflow_y_scroll()
                        .p(px(10.0))
                        .rounded(px(RADIUS_M))
                        .bg(t.surface)
                        .border_1()
                        .border_color(t.border_subtle)
                        .text_size(px(12.0))
                        .line_height(px(18.0))
                        .text_color(t.tx2)
                        .child(SharedString::from(cleaned_notes)),
                );
            }
            card.child(col)
        }
        crate::update::UpdateState::Downloading {
            version,
            progress,
            downloaded,
            total,
        } => {
            let dl_mb = (*downloaded as f64) / (1024.0 * 1024.0);
            let tot_mb = (*total as f64) / (1024.0 * 1024.0);
            let pct = (*progress * 100.0).clamp(0.0, 100.0);
            card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_size(px(13.5))
                                    .text_color(t.tx)
                                    .child(SharedString::from(format!(
                                        "正在下载更新 v{}…",
                                        version
                                    ))),
                            )
                            .child(div().text_size(px(12.0)).text_color(t.tx2).child(
                                SharedString::from(format!(
                                    "{:.1} MB / {:.1} MB ({:.0}%)",
                                    dl_mb, tot_mb, pct
                                )),
                            )),
                    )
                    .child(progress_bar(&t, *progress)),
            )
        }
        crate::update::UpdateState::Ready {
            version,
            installer_path,
        } => {
            let path_clone = installer_path.clone();
            let restart_btn = button(
                &t,
                &crate::t!("update-install"),
                ButtonKind::Primary,
                Some("arrow-up"),
                cx.listener(move |this, _, _, cx| {
                    this.trigger_apply_update(path_clone.clone(), cx);
                }),
            );
            let ready_title = crate::t!("update-ready", version = version.as_str());
            card.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .child(icon("check", 16.0, t.ac))
                                    .child(
                                        div()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_size(px(13.5))
                                            .text_color(t.tx)
                                            .child(SharedString::from(ready_title)),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(t.tx3)
                                    .child(SharedString::from(crate::t!("update-ready-desc"))),
                            ),
                    )
                    .child(restart_btn),
            )
        }
        crate::update::UpdateState::Failed(err) => {
            let err_clone = err.clone();
            let retry_btn = button(
                &t,
                &crate::t!("update-retry"),
                ButtonKind::Secondary,
                Some("refresh"),
                cx.listener(|this, _, _, cx| {
                    this.trigger_check_update(true, cx);
                }),
            );
            card.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(icon("alert-triangle", 16.0, t.danger))
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .text_color(t.tx2)
                                    .child(SharedString::from(format!("更新出错: {err_clone}"))),
                            ),
                    )
                    .child(retry_btn),
            )
        }
    }
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
        assert_eq!(CATEGORIES[CAT_MEMOS].0, "Memos");
        assert_eq!(CAT_SYNC, CAT_DAILY + 1);
        assert_eq!(CAT_AI, CAT_SYNC + 1);
        assert_eq!(CAT_MEMOS, CAT_AI + 1);
        assert_eq!(CATEGORIES[CAT_MEMOS + 1].0, "快捷键");
        assert_eq!(CATEGORIES.len(), 9);
    }
}
