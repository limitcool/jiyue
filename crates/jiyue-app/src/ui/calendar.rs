//! Month calendar for daily notes, drawn in the sidebar under the quick actions.

use gpui::{
    AnyElement, Context, Div, FontWeight, IntoElement, MouseButton, SharedString, div, prelude::*,
    px,
};

use super::icons::icon;
use crate::daily::{self, Date};
use crate::theme::ActiveTheme;
use crate::{AppCommand, RustdianApp};

/// State of the calendar widget. `year == 0 || month == 0` means "the current month".
pub struct CalendarState {
    /// Displayed month; (0, 0) means "the current month, resolved lazily".
    pub year: i32,
    pub month: u32,
    /// Folded state. Starts collapsed; `--open calendar` clears it.
    pub collapsed: bool,
    /// Set once the user clicks the 日历 label this session. Until then a daily note
    /// shows the calendar expanded.
    pub user_collapsed: bool,
}

impl Default for CalendarState {
    fn default() -> Self {
        Self {
            year: 0,
            month: 0,
            collapsed: true,
            user_collapsed: false,
        }
    }
}

/// One cell of the 6x7 month grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalCell {
    pub date: Date,
    /// False for the leading and trailing days that belong to a neighbouring month.
    pub in_month: bool,
}

const WEEKDAYS: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];

/// The 42 cells of the month grid for `year`/`month`, Monday first. Starts on the Monday
/// on or before the 1st. Empty for an invalid month.
pub fn month_grid(year: i32, month: u32) -> Vec<CalCell> {
    let Some(first) = Date::new(year, month, 1) else {
        return Vec::new();
    };
    let start = first.add_days(-(first.weekday() as i64));
    (0..42)
        .map(|i| {
            let date = start.add_days(i);
            CalCell {
                date,
                in_month: date.year == year && date.month == month,
            }
        })
        .collect()
}

/// Moves a (year, month) pair by `delta` months (negative goes back).
pub fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let index = year as i64 * 12 + (month as i64 - 1) + delta as i64;
    let y = index.div_euclid(12) as i32;
    let m = index.rem_euclid(12) as u32 + 1;
    (y, m)
}

/// The month the calendar shows: the stored month, or the month of `today` when unset.
pub fn shown_month(state: &CalendarState, today: Date) -> (i32, u32) {
    if state.year == 0 || state.month == 0 {
        (today.year, today.month)
    } else {
        (state.year, state.month)
    }
}

/// Whether the calendar is folded. A daily note shows it expanded unless the user has
/// clicked the 日历 label this session.
pub(crate) fn is_collapsed(app: &RustdianApp) -> bool {
    let state = &app.calendar;
    state.collapsed && !(!state.user_collapsed && active_daily_date(app).is_some())
}

/// The date of the active note when it is a daily note.
pub(crate) fn active_daily_date(app: &RustdianApp) -> Option<Date> {
    let note = app.vault.active_note()?;
    if !app.vault.is_daily_note(note) {
        return None;
    }
    let fmt = app.vault.daily_settings().date_format;
    daily::parse_date(&note.title, &fmt)
}

impl RustdianApp {
    /// Moves the calendar back to the current month (used by 今天 and 今日日记).
    pub(crate) fn reset_calendar_month(&mut self) {
        self.calendar.year = 0;
        self.calendar.month = 0;
    }

    /// The month calendar block shown in the sidebar under the quick actions.
    pub(crate) fn render_calendar(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let today = Date::today();
        let (year, month) = shown_month(&self.calendar, today);
        let collapsed = is_collapsed(self);
        let (prev_y, prev_m) = shift_month(year, month, -1);
        let (next_y, next_m) = shift_month(year, month, 1);

        let chevron = if collapsed {
            "chevron-right"
        } else {
            "chevron-down"
        };
        let label = div()
            .flex()
            .items_center()
            .justify_between()
            .pt(px(10.0))
            .pb(px(4.0))
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(t.tx3)
            .cursor_pointer()
            .hover(|s| s.text_color(t.tx2))
            .child("日历")
            .child(icon(chevron, 13.0, t.tx3))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.calendar.collapsed = !is_collapsed(this);
                    this.calendar.user_collapsed = true;
                    cx.notify();
                }),
            );

        let mut block = div()
            .flex()
            .flex_col()
            .flex_none()
            .px(px(8.0))
            .pb(px(4.0))
            .child(label);
        if collapsed {
            return block.into_any_element();
        }

        let nav_button = |name: &'static str| {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(22.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .child(icon(name, 14.0, t.tx3))
        };
        let header = div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .h(px(26.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.tx)
                    .child(SharedString::from(format!("{}年{}月", year, month))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(px(6.0))
                    .h(px(22.0))
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(t.ac)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .child("今天")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.reset_calendar_month();
                            cx.notify();
                        }),
                    ),
            )
            .child(nav_button("chevron-left").on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.calendar.year = prev_y;
                    this.calendar.month = prev_m;
                    cx.notify();
                }),
            ))
            .child(nav_button("chevron-right").on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.calendar.year = next_y;
                    this.calendar.month = next_m;
                    cx.notify();
                }),
            ));

        let mut weekdays = div()
            .flex()
            .items_center()
            .h(px(16.0))
            .text_size(px(11.0))
            .text_color(t.tx3);
        for name in WEEKDAYS {
            weekdays = weekdays.child(div().flex_1().flex().justify_center().child(name));
        }

        let dots = self.vault.daily_dates_in_month(year, month);
        let active = active_daily_date(self);
        let cells = month_grid(year, month);
        let mut grid = div().flex().flex_col().pt(px(2.0));
        for week in cells.chunks(7) {
            let mut row = div().flex().h(px(24.0));
            for cell in week {
                let has_note = cell.in_month && dots.contains(&cell.date.day);
                row = row.child(day_cell(&t, *cell, today, active, has_note, cx));
            }
            grid = grid.child(row);
        }

        block = block.child(header).child(weekdays).child(grid);
        block.into_any_element()
    }
}

/// One day of the grid: number, then a 4px dot when that day has a daily note.
fn day_cell(
    t: &ActiveTheme,
    cell: CalCell,
    today: Date,
    active: Option<Date>,
    has_note: bool,
    cx: &mut Context<RustdianApp>,
) -> Div {
    let date = cell.date;
    let is_today = date == today;
    let is_active = active == Some(date);
    let text = if is_today {
        t.ac_on
    } else if is_active {
        t.ac
    } else if cell.in_month {
        t.tx2
    } else {
        let mut dim = t.tx3;
        dim.a = 0.5;
        dim
    };

    let mut el = div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(1.0))
        .flex_1()
        .h(px(24.0))
        .rounded(px(7.0))
        .text_size(px(12.0))
        .text_color(text)
        .cursor_pointer();
    if is_today {
        el = el.bg(t.ac);
    } else if is_active {
        el = el.bg(t.ac_soft);
    } else {
        el = el.hover(|s| s.bg(t.hover));
    }

    let dot_color = if is_today { t.ac_on } else { t.ac };
    let dot = div().size(px(4.0)).rounded(px(2.0));
    let dot = if has_note { dot.bg(dot_color) } else { dot };
    el.child(SharedString::from(date.day.to_string()))
        .child(dot)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.run_command(AppCommand::OpenDaily(date));
                cx.notify();
            }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> Date {
        Date::new(y, m, day).expect("valid test date")
    }

    #[test]
    fn november_2026_grid() {
        // 1 November 2026 is a Sunday, so six leading days (26-31 October) come first.
        let cells = month_grid(2026, 11);
        assert_eq!(cells.len(), 42);
        assert_eq!(cells[0].date, d(2026, 10, 26));
        assert!(!cells[0].in_month);
        assert_eq!(cells[6].date, d(2026, 11, 1));
        assert!(cells[6].in_month);
        assert_eq!(cells[41].date, d(2026, 12, 6));
        assert_eq!(cells.iter().filter(|c| c.in_month).count(), 30);
        // Cells are consecutive days, Monday first.
        for (i, cell) in cells.iter().enumerate() {
            assert_eq!(cell.date, d(2026, 10, 26).add_days(i as i64));
        }
    }

    #[test]
    fn leap_february_grid() {
        // 1 February 2024 is a Thursday: three leading days (29-31 January).
        let cells = month_grid(2024, 2);
        assert_eq!(cells.len(), 42);
        assert_eq!(cells[0].date, d(2024, 1, 29));
        assert_eq!(cells[3].date, d(2024, 2, 1));
        assert_eq!(cells.iter().filter(|c| c.in_month).count(), 29);
        assert!(cells[3..32].iter().all(|c| c.in_month));
    }

    #[test]
    fn monday_start_month_has_no_leading_days() {
        // 1 June 2026 is a Monday.
        let cells = month_grid(2026, 6);
        assert_eq!(cells[0].date, d(2026, 6, 1));
        assert!(cells[0].in_month);
        assert_eq!(cells.iter().filter(|c| c.in_month).count(), 30);
    }

    #[test]
    fn shift_wraps_years() {
        assert_eq!(shift_month(2026, 12, 1), (2027, 1));
        assert_eq!(shift_month(2026, 1, -1), (2025, 12));
        assert_eq!(shift_month(2026, 11, -12), (2025, 11));
        assert_eq!(shift_month(2026, 11, 0), (2026, 11));
    }

    #[test]
    fn unset_state_shows_current_month() {
        let today = d(2026, 11, 4);
        let unset = CalendarState::default();
        assert_eq!(shown_month(&unset, today), (2026, 11));
        let set = CalendarState {
            year: 2025,
            month: 3,
            collapsed: false,
            user_collapsed: false,
        };
        assert_eq!(shown_month(&set, today), (2025, 3));
    }
}
