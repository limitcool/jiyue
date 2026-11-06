//! Catalogue of the shared widgets, for checking them by eye and by scripted input.
//!
//! `render_demo(theme, window, cx)` returns one full-window element. The demo keeps its own state
//! in an entity (kept across frames with `Window::use_keyed_state`), so the owner needs no fields.
//! Show it from `ui::ext::overlays` (see the report for the exact lines).

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyElement, Context, Entity, FocusHandle, IntoElement, KeyDownEvent, MouseDownEvent, Render,
    Subscription, Window, div, prelude::*, px,
};

use super::controls::{self, BadgeKind, ButtonKind, ListAction};
use super::line_input::{LineInput, LineInputEvent};
use super::overlay::{self, MenuItem};
use crate::theme::ActiveTheme;

const SELECT_OPTIONS: [&str; 3] = ["日记", "笔记", "项目"];
const SEGMENT_OPTIONS: [&str; 3] = ["纸", "雾", "墨"];

/// Rows of the demo's context menu. Index 4 is disabled, index 2 is a separator.
fn demo_menu_items() -> Vec<MenuItem> {
    vec![
        MenuItem::new("新建笔记")
            .icon("square-rounded-plus")
            .shortcut("Ctrl N"),
        MenuItem::new("复制").icon("copy").shortcut("Ctrl C"),
        MenuItem::separator(),
        MenuItem::new("移到回收站").icon("trash").danger(),
        MenuItem::new("暂不可用").icon("settings").disabled(),
    ]
}

/// The demo's state and its render.
pub struct DemoView {
    theme: ActiveTheme,
    focus: FocusHandle,
    plain: Entity<LineInput>,
    password: Entity<LineInput>,
    limited: Entity<LineInput>,
    modal_input: Entity<LineInput>,
    _subs: Vec<Subscription>,
    last_event: String,
    toggle_on: bool,
    segment: usize,
    checked: bool,
    select_open: bool,
    select_sel: usize,
    select_hl: usize,
    menu_at: Option<(f32, f32)>,
    menu_hl: Option<usize>,
    menu_result: String,
    modal_open: bool,
    confirm_open: bool,
    confirm_result: String,
    progress: f32,
}

impl DemoView {
    fn new(cx: &mut Context<Self>) -> Self {
        let plain = cx.new(|cx| LineInput::new(cx, "输入中文或英文"));
        let password = cx.new(|cx| {
            let mut input = LineInput::new(cx, "密码");
            input.set_password(true, cx);
            input
        });
        let limited = cx.new(|cx| {
            let mut input = LineInput::new(cx, "最多 12 个字符");
            input.set_max_len(Some(12));
            input
        });
        let modal_input = cx.new(|cx| {
            let mut input = LineInput::new(cx, "新名称");
            input.set_text("沉积的想法", cx);
            input
        });
        let mut subs = Vec::new();
        for input in [&plain, &password, &limited, &modal_input] {
            subs.push(
                cx.subscribe(input, |this: &mut DemoView, _, ev: &LineInputEvent, cx| {
                    this.last_event = match ev {
                        LineInputEvent::Changed(s) => format!("Changed: {s}"),
                        LineInputEvent::Submit(s) => format!("Submit: {s}"),
                        LineInputEvent::Cancel => "Cancel".to_string(),
                    };
                    // Escape in a field closes the overlay that holds it.
                    if matches!(ev, LineInputEvent::Cancel) {
                        this.close_top();
                    }
                    cx.notify();
                }),
            );
        }
        Self {
            theme: ActiveTheme::dark(),
            focus: cx.focus_handle(),
            plain,
            password,
            limited,
            modal_input,
            _subs: subs,
            last_event: "无".to_string(),
            toggle_on: true,
            segment: 2,
            checked: false,
            select_open: false,
            select_sel: 0,
            select_hl: 0,
            menu_at: None,
            menu_hl: None,
            menu_result: "无".to_string(),
            modal_open: false,
            confirm_open: false,
            confirm_result: "无".to_string(),
            progress: 0.4,
        }
    }

    /// Closes the topmost overlay. Returns true when one was open.
    fn close_top(&mut self) -> bool {
        if self.confirm_open {
            self.confirm_open = false;
            self.confirm_result = "按 Esc 取消".to_string();
        } else if self.modal_open {
            self.modal_open = false;
        } else if self.select_open {
            self.select_open = false;
        } else if self.menu_at.is_some() {
            self.menu_at = None;
        } else {
            return false;
        }
        true
    }

    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.select_open {
            match controls::select_action(
                ev.keystroke.key.as_str(),
                self.select_hl,
                SELECT_OPTIONS.len(),
            ) {
                ListAction::Highlight(i) => self.select_hl = i,
                ListAction::Pick(i) => {
                    self.select_sel = i;
                    self.select_open = false;
                }
                ListAction::Close => self.select_open = false,
                ListAction::Ignore => {}
            }
        } else if self.menu_at.is_some() {
            let items = demo_menu_items();
            match overlay::menu_action(ev.keystroke.key.as_str(), &items, self.menu_hl) {
                ListAction::Highlight(i) => self.menu_hl = Some(i),
                ListAction::Pick(i) => {
                    self.menu_result = items[i].label.clone();
                    self.menu_at = None;
                }
                ListAction::Close => self.menu_at = None,
                ListAction::Ignore => {}
            }
        } else if overlay::is_escape(ev) {
            self.close_top();
        }
        cx.notify();
    }
}

fn section(t: &ActiveTheme, title: &str, children: Vec<AnyElement>) -> gpui::Div {
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(controls::section_title(t, title));
    for c in children {
        col = col.child(c);
    }
    col
}

impl Render for DemoView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        for input in [
            self.plain.clone(),
            self.password.clone(),
            self.limited.clone(),
            self.modal_input.clone(),
        ] {
            input.update(cx, |i, cx| i.set_theme(t, cx));
        }
        let viewport = window.viewport_size();
        let win = (f32::from(viewport.width), f32::from(viewport.height));

        let inputs = section(
            &t,
            "输入框 LineInput",
            vec![
                div()
                    .w(px(360.0))
                    .child(self.plain.clone())
                    .into_any_element(),
                div()
                    .w(px(360.0))
                    .child(self.password.clone())
                    .into_any_element(),
                div()
                    .w(px(360.0))
                    .child(self.limited.clone())
                    .into_any_element(),
                div()
                    .text_size(px(12.0))
                    .text_color(t.tx3)
                    .child(format!("最近事件：{}", self.last_event))
                    .into_any_element(),
            ],
        );

        let buttons = section(
            &t,
            "按钮 button / icon_button",
            vec![
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(controls::button(
                        &t,
                        "保存",
                        ButtonKind::Primary,
                        None,
                        |_, _, _| {},
                    ))
                    .child(controls::button(
                        &t,
                        "取消",
                        ButtonKind::Secondary,
                        None,
                        |_, _, _| {},
                    ))
                    .child(controls::button(
                        &t,
                        "更多",
                        ButtonKind::Ghost,
                        None,
                        |_, _, _| {},
                    ))
                    .child(controls::button(
                        &t,
                        "删除",
                        ButtonKind::Danger,
                        Some("trash"),
                        |_, _, _| {},
                    ))
                    .child(controls::button(
                        &t,
                        "搜索",
                        ButtonKind::Secondary,
                        Some("search"),
                        |_, _, _| {},
                    ))
                    .child(controls::icon_button(&t, "x", |_, _, _| {}))
                    .into_any_element(),
            ],
        );

        let on_toggle = cx.listener(|this, _: &MouseDownEvent, _, cx| {
            this.toggle_on = !this.toggle_on;
            cx.notify();
        });
        let on_seg = Rc::new(cx.listener(|this, i: &usize, _, cx| {
            this.segment = *i;
            cx.notify();
        }));
        let on_check = cx.listener(|this, _: &MouseDownEvent, _, cx| {
            this.checked = !this.checked;
            cx.notify();
        });
        let controls_section = section(
            &t,
            "开关、分段、复选",
            vec![
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.0))
                    .child(controls::toggle(&t, self.toggle_on, on_toggle))
                    .child(controls::segmented(
                        &t,
                        &SEGMENT_OPTIONS,
                        self.segment,
                        on_seg,
                    ))
                    .child(controls::checkbox(&t, self.checked, on_check))
                    .child(div().text_size(px(12.5)).text_color(t.tx2).child(format!(
                        "复选框：{}",
                        if self.checked { "开" } else { "关" }
                    )))
                    .into_any_element(),
            ],
        );

        let on_open = cx.listener(|this, _: &MouseDownEvent, _, cx| {
            this.select_open = true;
            this.select_hl = this.select_sel;
            cx.notify();
        });
        let on_close = cx.listener(|this, _: &MouseDownEvent, _, cx| {
            this.select_open = false;
            cx.notify();
        });
        let on_pick = Rc::new(cx.listener(|this, i: &usize, _, cx| {
            this.select_sel = *i;
            this.select_open = false;
            cx.notify();
        }));
        let select_section = section(
            &t,
            "下拉 select",
            vec![
                div()
                    .h(px(120.0))
                    .child(controls::select(
                        &t,
                        &SELECT_OPTIONS,
                        self.select_sel,
                        self.select_open,
                        self.select_hl,
                        180.0,
                        on_open,
                        on_close,
                        on_pick,
                    ))
                    .into_any_element(),
            ],
        );

        let badges = section(
            &t,
            "标签与按键 badge / kbd",
            vec![
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(controls::badge(&t, "中", BadgeKind::Neutral))
                    .child(controls::badge(&t, "已同步", BadgeKind::Ok))
                    .child(controls::badge(&t, "冲突", BadgeKind::Danger))
                    .child(controls::badge(&t, "草稿", BadgeKind::Warn))
                    .child(controls::badge(&t, "AI", BadgeKind::Accent))
                    .child(controls::kbd(&t, "Ctrl"))
                    .child(controls::kbd(&t, "K"))
                    .into_any_element(),
            ],
        );

        let on_more = cx.listener(|this, _: &MouseDownEvent, _, cx| {
            this.progress = (this.progress + 0.1).min(1.0);
            cx.notify();
        });
        let progress = section(
            &t,
            "进度 progress_bar / spinner",
            vec![
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .w(px(360.0))
                    .child(
                        div()
                            .flex_1()
                            .child(controls::progress_bar(&t, self.progress)),
                    )
                    .child(controls::button(
                        &t,
                        "+10%",
                        ButtonKind::Secondary,
                        None,
                        on_more,
                    ))
                    .child(controls::spinner(&t, 16.0))
                    .into_any_element(),
            ],
        );

        let rows = section(
            &t,
            "设置行 form_row",
            vec![
                controls::form_row(
                    &t,
                    "同步到云端",
                    "开启后按计划上传笔记",
                    controls::toggle(&t, self.toggle_on, |_, _, _| {}),
                    false,
                )
                .into_any_element(),
                controls::form_row(
                    &t,
                    "界面字号",
                    "正文编辑区的文字大小",
                    controls::badge(&t, "15", BadgeKind::Neutral),
                    true,
                )
                .into_any_element(),
            ],
        );

        let empty = section(
            &t,
            "空状态 empty_state",
            vec![
                controls::empty_state(
                    &t,
                    "search",
                    "没有匹配的笔记",
                    "换个关键词再试，或新建一篇笔记。",
                )
                .into_any_element(),
            ],
        );

        let on_modal = cx.listener(|this, _: &MouseDownEvent, window, cx| {
            this.modal_open = true;
            // Focus after this click: the click's own focus change would otherwise win.
            let input = this.modal_input.clone();
            window.on_next_frame(move |window, cx| input.read(cx).focus(window));
            cx.notify();
        });
        let on_confirm_open = cx.listener(|this, _: &MouseDownEvent, window, cx| {
            this.confirm_open = true;
            window.focus(&this.focus);
            cx.notify();
        });
        let dialogs = section(
            &t,
            "弹窗 modal / confirm_dialog",
            vec![
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(controls::button(
                        &t,
                        "打开弹窗",
                        ButtonKind::Secondary,
                        None,
                        on_modal,
                    ))
                    .child(controls::button(
                        &t,
                        "确认删除",
                        ButtonKind::Danger,
                        None,
                        on_confirm_open,
                    ))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(t.tx3)
                            .child(format!("确认结果：{}", self.confirm_result)),
                    )
                    .into_any_element(),
                div()
                    .text_size(px(12.0))
                    .text_color(t.tx3)
                    .child(format!("菜单结果：{}", self.menu_result))
                    .into_any_element(),
            ],
        );

        let title = div()
            .font_family(crate::ui::fonts::ui_name())
            .text_size(px(20.0))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(t.tx)
            .child("共享控件 Widgets");
        // Two scrolling columns: inputs and controls on the left, status and dialogs on the right.
        let left = div()
            .id("widgets-demo-left")
            .absolute()
            .left(px(32.0))
            .top(px(28.0))
            .bottom(px(28.0))
            .w(px(380.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .child(title)
            .child(inputs)
            .child(buttons)
            .child(controls_section)
            .child(select_section);
        let right = div()
            .id("widgets-demo-right")
            .absolute()
            .left(px(452.0))
            .top(px(28.0))
            .bottom(px(28.0))
            .w(px(440.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(18.0))
            .child(badges)
            .child(progress)
            .child(rows)
            .child(empty)
            .child(dialogs);
        let column = div().absolute().inset_0().child(left).child(right);

        // Context menu trigger, bottom right, so the menu has to flip.
        let on_menu = cx.listener(|this, ev: &MouseDownEvent, window, cx| {
            this.menu_at = Some((f32::from(ev.position.x), f32::from(ev.position.y)));
            this.menu_hl = None;
            window.focus(&this.focus);
            cx.notify();
        });
        let menu_trigger =
            div()
                .absolute()
                .right(px(24.0))
                .bottom(px(24.0))
                .child(controls::button(
                    &t,
                    "右键菜单",
                    ButtonKind::Primary,
                    None,
                    on_menu,
                ));

        let mut root = div()
            .absolute()
            .inset_0()
            .overflow_hidden()
            .bg(t.sheet)
            .text_color(t.tx)
            .font_family(crate::ui::fonts::ui_name())
            .text_size(px(13.0))
            .track_focus(&self.focus)
            // The demo covers the window: keep clicks away from the page underneath.
            .occlude()
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key(ev, cx)))
            .child(column)
            .child(menu_trigger);

        if self.modal_open {
            let on_close = cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.modal_open = false;
                cx.notify();
            });
            let body = div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(t.tx2)
                        .child("重命名笔记。Esc 关闭，Enter 确认。"),
                )
                .child(self.modal_input.clone());
            let close_btn = cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.modal_open = false;
                cx.notify();
            });
            let footer = vec![
                controls::button(&t, "取消", ButtonKind::Secondary, None, close_btn)
                    .into_any_element(),
                controls::button(&t, "保存", ButtonKind::Primary, None, |_, _, _| {})
                    .into_any_element(),
            ];
            root = root.child(overlay::modal(&t, "重命名", body, footer, 420.0, on_close));
        }

        if self.confirm_open {
            let on_ok = cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.confirm_open = false;
                this.confirm_result = "已删除".to_string();
                cx.notify();
            });
            let on_cancel = cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.confirm_open = false;
                this.confirm_result = "已取消".to_string();
                cx.notify();
            });
            root = root.child(overlay::confirm_dialog(
                &t,
                "删除这篇笔记？",
                "删除后会移到回收站，可以在 .trash 中找回。",
                "删除",
                true,
                on_ok,
                on_cancel,
            ));
        }

        if let Some(pos) = self.menu_at {
            let on_select = Rc::new(cx.listener(|this, i: &usize, _, cx| {
                let items = demo_menu_items();
                this.menu_result = items
                    .get(*i)
                    .map_or_else(String::new, |it| it.label.clone());
                this.menu_at = None;
                cx.notify();
            }));
            let on_close = cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.menu_at = None;
                cx.notify();
            });
            root = root.child(overlay::context_menu(
                &t,
                &demo_menu_items(),
                self.menu_hl,
                pos,
                win,
                on_select,
                on_close,
            ));
        }

        root
    }
}

thread_local! {
    /// The demo's entity, created by the first `render_demo` call and kept for the app's life.
    static DEMO: RefCell<Option<Entity<DemoView>>> = const { RefCell::new(None) };
}

/// The catalogue as an element, for `ui::ext::overlays`. The first call registers the widget key
/// bindings and creates the demo state; later calls reuse it. `theme` is the app theme.
pub fn render_demo<V: 'static>(theme: ActiveTheme, cx: &mut Context<V>) -> AnyElement {
    let existing = DEMO.with(|d| d.borrow().clone());
    let demo = match existing {
        Some(demo) => demo,
        None => {
            super::bind_keys(cx);
            let demo = cx.new(DemoView::new);
            DEMO.with(|d| *d.borrow_mut() = Some(demo.clone()));
            demo
        }
    };
    demo.update(cx, |d, _| d.theme = theme);
    demo.into_any_element()
}
