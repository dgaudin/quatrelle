//! The Calendar: a month to pick days from and the calendars in the side pane; a day, a week
//! or a month in the main one. Click an empty hour to add an event there, an event to open it,
//! drag one to move it ([`drag`]).

use egui::{Align2, Color32, Key, Margin, Rect, RichText, Sense, Ui, pos2, vec2};
use quatrelle_engine::model::dates::{self, civil_from_days, days_from_civil};
use quatrelle_engine::{AgendaItem, CalMode, Session};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog, UiState, widgets};

mod days;
mod drag;
use days::days;
pub use drag::Drag;

/// Height of an hour in the day and week views.
pub(crate) const HOUR: f32 = 46.0;
pub(crate) const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
pub(crate) const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// A calendar's colour (the first one is the app's; then Quatrelle's eight categories' in turn).
pub fn color(i: u8, t: &Tokens) -> Color32 {
    if i == 0 { t.accent } else { theme::category_color(i.saturating_sub(1) % 8, t.dark) }
}

fn today(s: &Session) -> i64 {
    dates::today(&s.clock)
}

/// `HH:MM`, local.
pub fn hm(t: i64, s: &Session) -> String {
    let c = dates::civil(t, &s.clock);
    format!("{:02}:{:02}", c.hour, c.minute)
}

/// The local day a time falls on.
fn day_of(t: i64, s: &Session) -> i64 {
    s.clock.local(t).div_euclid(86_400)
}

/// `October 2026`, `12 – 18 October 2026`, `Monday, 12 October 2026`.
pub fn title(s: &Session) -> String {
    let (d0, d1) = s.cal_days();
    let month = |m: u32| crate::i18n::tc("month", MONTHS.get(m as usize - 1).copied().unwrap_or("")).to_string();
    match s.cal_mode {
        CalMode::Month => {
            let (y, m, _) = civil_from_days(s.cal_day());
            tf!("{month} {year}", month = month(m), year = y)
        }
        CalMode::Week | CalMode::WorkWeek => {
            let ((y0, m0, a), (y1, m1, b)) = (civil_from_days(d0), civil_from_days(d1 - 1));
            if (y0, m0) == (y1, m1) {
                tf!("{from} – {to} {month} {year}", from = a, to = b, month = month(m1), year = y1)
            } else {
                tf!("{from} {month_from} – {to} {month} {year}", from = a, month_from = month(m0), to = b, month = month(m1), year = y1)
            }
        }
        CalMode::Day => long_date(d0),
    }
}

/// A day written out: `Thursday, 22 October 2026`, in the interface's language.
pub fn long_date(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    let month = crate::i18n::tc("month", MONTHS.get(m as usize - 1).copied().unwrap_or("")).to_string();
    let wd = (day + 3).rem_euclid(7) as usize;
    let day_name = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].get(wd).copied().unwrap_or("");
    tf!("{weekday}, {day} {month} {year}", weekday = tl!(day_name), day = d, month = month, year = y)
}

/// The side pane: a month to jump around in, then the calendars.
pub fn nav(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("calendar-nav").show(ui, |ui| {
        months(s, st, ui, acts);
        ui.add_space(8.0);
        let several = s.pim.values().map(|p| p.calendars.len()).sum::<usize>() > 1;
        // The calendars in groups, as Outlook's: this user's own, the groups kept on the server
        // (a team's…), then those opened by their owner's address; a shared mailbox's own
        // calendar sits under its name, in the folder pane's order (issue #1).
        let sections = s.pane_sections();
        let mut groups: Vec<(String, Vec<_>)> = vec![(tl!("My calendars").to_string(), Vec::new())];
        let mut shared = Vec::new();
        for a in &s.accounts {
            let Some(p) = s.pim.get(&a.config.id) else { continue };
            for c in &p.calendars {
                if c.id.starts_with("shared:") {
                    if section_key(&sections, a, c).is_some() {
                        if let Some((_, list)) = groups.first_mut() {
                            list.push((a, c));
                        }
                    } else {
                        shared.push((a, c));
                    }
                } else if c.group.is_empty() {
                    if let Some((_, list)) = groups.first_mut() {
                        list.push((a, c));
                    }
                } else {
                    match groups.iter_mut().find(|(g, _)| *g == c.group) {
                        Some((_, list)) => list.push((a, c)),
                        None => groups.push((c.group.clone(), vec![(a, c)])),
                    }
                }
            }
        }
        groups.push((tl!("Shared calendars").to_string(), shared));
        for (title, list) in &groups {
            if list.is_empty() {
                continue;
            }
            egui::CollapsingHeader::new(RichText::new(title).font(theme::bold(13.0)).color(t.text))
                .id_salt(("calendar-group", title))
                .default_open(true)
                .show(ui, |ui| {
                    let mut last: Option<String> = None;
                    for (a, c) in list {
                        if sections.len() > 1 {
                            let key = section_key(&sections, a, c).unwrap_or_else(|| a.config.id.clone());
                            if last.as_deref() != Some(key.as_str()) {
                                section_heading(s, &t, ui, &key);
                                last = Some(key);
                            }
                        }
                        calendar_row(s, &t, ui, a, c, acts);
                    }
                });
        }
        // Each in a column of its own, or all together (Outlook's side-by-side view).
        if several {
            ui.add_space(4.0);
            let mut on = s.prefs.calendars_side_by_side;
            if ui.checkbox(&mut on, tl!("Side by side")).on_hover_text(tl!("Day and week show each calendar in a pane of its own")).changed() {
                acts.push(Action::Run("calendar.sideBySide", json!({"on": on})));
            }
        }
        // Outlook's Open Calendar: someone's calendar shared with an Exchange account.
        if s.accounts.iter().any(|a| a.config.graph.is_some() || a.config.ews.is_some()) {
            ui.add_space(6.0);
            if ui.link(RichText::new(tl!("Open shared calendar…")).small()).clicked() {
                acts.push(Action::Dialog(Dialog::OpenSharedCalendar { email: String::new(), error: None }));
            }
        }
        // Outlook's Add Calendar › From Internet (issue #7).
        if !s.accounts.is_empty() {
            ui.add_space(4.0);
            if ui.link(RichText::new(tl!("Subscribe to a calendar…")).small()).clicked() {
                acts.push(Action::Dialog(Dialog::SubscribeCalendar { url: String::new(), name: String::new(), error: None }));
            }
        }
    });
}

/// The mailbox a calendar belongs to, when it's a shared mailbox the folder pane opens
/// (`<account>|<address>`).
fn section_key(
    sections: &[quatrelle_engine::PaneSection],
    a: &quatrelle_engine::AccountState,
    c: &quatrelle_engine::model::pim::CalendarInfo,
) -> Option<String> {
    let owner = c.id.strip_prefix("shared:")?.to_ascii_lowercase();
    let key = format!("{}|{owner}", a.config.id);
    sections.iter().any(|x| x.key == key).then_some(key)
}

/// A mailbox's name above its calendars, with the icon of a shared one.
fn section_heading(s: &Session, t: &Tokens, ui: &mut Ui, key: &str) {
    let (shared, name) = match key.split_once('|') {
        Some((_, mailbox)) => (true, mailbox.to_string()),
        None => (false, s.account(key).map(|a| a.config.email.clone()).unwrap_or_else(|| key.to_string())),
    };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
        if shared {
            icons::paint(ui.painter(), r.expand(3.0), Icon::People, t.dim);
        } else if let Some(a) = s.account(key) {
            ui.painter().circle_filled(r.center(), 4.0, theme::account_color(a.config.color));
        }
        ui.add(egui::Label::new(RichText::new(name).font(theme::bold(12.0)).color(t.dim)).truncate());
    });
}

/// A calendar in the side pane: ticked (in its colour) when shown, a click on the box hides or
/// shows it; someone else's closes from its right-click menu, one subscribed to by its link
/// (issue #7) is read again or removed from it.
fn calendar_row(
    s: &Session,
    t: &Tokens,
    ui: &mut Ui,
    a: &quatrelle_engine::AccountState,
    c: &quatrelle_engine::model::pim::CalendarInfo,
    acts: &mut Vec<Action>,
) {
    let shared = c.id.strip_prefix("shared:").map(str::to_string);
    let shown = s.calendar_shown(&a.config.id, &c.id);
    // Subscribed to by its link: where it's read from, and how that went.
    let web = s.pim.get(&a.config.id).and_then(|p| p.subscribed.iter().find(|w| w.id == c.id));
    let row = ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
        let c32 = color(c.color, t);
        if shown {
            ui.painter().rect_filled(r, 3.0, c32);
            let p = |x: f32, y: f32| pos2(r.min.x + r.width() * x, r.min.y + r.height() * y);
            ui.painter().line_segment([p(0.22, 0.52), p(0.42, 0.72)], egui::Stroke::new(1.8, egui::Color32::WHITE));
            ui.painter().line_segment([p(0.42, 0.72), p(0.78, 0.3)], egui::Stroke::new(1.8, egui::Color32::WHITE));
        } else {
            ui.painter().rect_stroke(r.shrink(0.5), 3.0, egui::Stroke::new(1.5, c32), egui::StrokeKind::Inside);
        }
        let tip = if shown { tl!("Shown: click to hide it") } else { tl!("Hidden: click to show it") };
        if resp.on_hover_text(tip).clicked() {
            acts.push(Action::Run("calendar.show", json!({"account": a.config.id, "calendar": c.id, "shown": !shown})));
        }
        let name = if c.id == quatrelle_engine::model::pim::LOCAL { tl!("Calendar").to_string() } else { c.name.clone() };
        let tip = match (&shared, s.pim.get(&a.config.id)) {
            // A shared mailbox's calendar comes with the mailbox; only one opened by hand closes.
            (Some(owner), Some(p)) if !p.shared.iter().any(|x| x.eq_ignore_ascii_case(owner)) => {
                tf!("Shared mailbox {address}, opened by {email}", address = owner, email = a.config.email)
            }
            (Some(_), _) => tf!("Shared with you, opened from {account}", account = a.config.email),
            (None, _) => match web {
                Some(w) => {
                    let state = match &w.error {
                        Some(e) => tf!("Couldn't be read: {error}", error = e),
                        None if w.fetched == 0 => tl!("Reading it…").to_string(),
                        None => tf!("Read {time}", time = crate::i18n::date(&dates::list_time(w.fetched, &s.clock))),
                    };
                    format!("{}\n{state}", w.url)
                }
                None => a.config.email.clone(),
            },
        };
        ui.add(egui::Label::new(RichText::new(name).color(t.text)).truncate()).on_hover_text(tip);
        if web.is_some_and(|w| w.error.is_some()) {
            ui.label(RichText::new(tl!("couldn't be read")).color(t.danger).small());
        } else if c.read_only {
            ui.label(RichText::new(tl!("read-only")).color(t.faint).small());
        }
    });
    // Subscribed to: right-click reads it again, or removes it.
    if web.is_some() {
        let id = c.id.clone();
        row.response.interact(Sense::click()).context_menu(|ui| {
            if ui.button(tl!("Refresh")).clicked() {
                acts.push(Action::Run("calendar.refreshSubscribed", json!({"calendar": id})));
                ui.close();
            }
            if ui.button(tl!("Remove this calendar")).clicked() {
                acts.push(Action::Run("calendar.unsubscribe", json!({"calendar": id})));
                ui.close();
            }
        });
    }
    if let Some(email) = shared
        && s.pim.get(&a.config.id).is_some_and(|p| p.shared.iter().any(|x| x.eq_ignore_ascii_case(&email)))
    {
        row.response.interact(Sense::click()).context_menu(|ui| {
            if ui.button(tl!("Remove this calendar")).clicked() {
                acts.push(Action::Run("calendar.closeShared", json!({"email": email})));
                ui.close();
            }
        });
    }
}

/// The months of the side pane, one under another (Outlook's date navigator, pulled down for
/// more), and a grip under them to drag for more or fewer.
fn months(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let shown = st.mini_month.unwrap_or_else(|| {
        let (y, m, _) = civil_from_days(s.cal_day());
        y * 12 + i64::from(m) - 1
    });
    let count = s.prefs.calendar_months.clamp(1, 6);
    for k in 0..i64::from(count) {
        if k > 0 {
            ui.add_space(8.0);
        }
        mini_month(s, st, ui, acts, shown, shown + k, k == 0);
    }
    // The grip: dragged down, one more month for each month's height; up, one fewer.
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::click_and_drag());
    let c = if resp.hovered() || resp.dragged() { t.dim } else { t.border };
    ui.painter().line_segment([pos2(r.center().x - 14.0, r.center().y), pos2(r.center().x + 14.0, r.center().y)], egui::Stroke::new(2.0, c));
    let resp = resp.on_hover_text(tl!("Drag down to show more months")).on_hover_cursor(egui::CursorIcon::ResizeVertical);
    if resp.drag_started() {
        st.months_drag = 0.0;
    }
    if resp.dragged() {
        st.months_drag += resp.drag_delta().y;
        let step = 170.0;
        if st.months_drag > step && count < 6 {
            st.months_drag -= step;
            acts.push(Action::Run("calendar.months", json!({"count": count + 1})));
        } else if st.months_drag < -step && count > 1 {
            st.months_drag += step;
            acts.push(Action::Run("calendar.months", json!({"count": count - 1})));
        }
    }
    resp.context_menu(|ui| {
        for n in 1..=6u8 {
            let label = if n == 1 { tl!("1 month").to_string() } else { tf!("{n} months", n = n) };
            if ui.radio(count == n, label).clicked() {
                acts.push(Action::Run("calendar.months", json!({"count": n})));
                ui.close();
            }
        }
    });
}

/// A small month (`month`: months since year 0): click a day to show it. The first of them has
/// the arrows that move them all (`shown` is the first one's month).
fn mini_month(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, shown: i64, month: i64, arrows: bool) {
    mini_month_to(s, st, ui, acts, shown, month, arrows, false);
}

/// [`mini_month`], a click on a day opening the calendar there when `open` (from the mail).
#[allow(clippy::too_many_arguments)]
fn mini_month_to(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, shown: i64, month: i64, arrows: bool, open: bool) {
    let t = st.tokens;
    let (y, m) = (month.div_euclid(12), u32::try_from(month.rem_euclid(12) + 1).unwrap_or(1));
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(tf!("{month} {year}", month = crate::i18n::tc("month", MONTHS.get(m as usize - 1).copied().unwrap_or("")), year = y))
                .font(theme::bold(13.0))
                .color(t.text),
        );
        if !arrows {
            return;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::tiny_icon_button(ui, Icon::ChevronRight, "Next month", &t).clicked() {
                st.mini_month = Some(shown + 1);
            }
            if widgets::tiny_icon_button(ui, Icon::ChevronLeft, "Previous month", &t).clicked() {
                st.mini_month = Some(shown - 1);
            }
        });
    });
    let w = ((ui.available_width() - 4.0) / 7.0).clamp(20.0, 34.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for d in DAYS {
            let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                tl!(d).chars().next().map(String::from).unwrap_or_default(),
                theme::regular(11.0),
                t.faint,
            );
        }
    });
    let first = days_from_civil(y, m, 1);
    let start = first - (first + 3).rem_euclid(7);
    let (sel0, sel1) = s.cal_days();
    let now = today(s);
    for week in 0..6 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for i in 0..7 {
                let d = start + week * 7 + i;
                let (r, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::click());
                let (_, dm, dd) = civil_from_days(d);
                if d >= sel0 && d < sel1 && s.cal_mode != CalMode::Month {
                    ui.painter().rect_filled(r.shrink(1.0), 4.0, t.accent_soft);
                }
                if d == now {
                    ui.painter().circle_filled(r.center(), 10.0, t.accent);
                } else if resp.hovered() {
                    ui.painter().circle_filled(r.center(), 10.0, t.hover);
                }
                let c = if d == now {
                    t.on_accent
                } else if dm != m {
                    t.faint
                } else {
                    t.text
                };
                ui.painter().text(r.center(), Align2::CENTER_CENTER, dd.to_string(), theme::regular(12.0), c);
                if resp.clicked() {
                    let (yy, mm, ddd) = civil_from_days(d);
                    if open {
                        acts.push(Action::Run("view.module", json!({"name": "calendar"})));
                    }
                    acts.push(Action::Run("calendar.goto", json!({"date": format!("{yy:04}-{mm:02}-{ddd:02}")})));
                    st.mini_month = None;
                }
            }
        });
    }
}

/// Beside the mail, as Outlook's To-Do bar: this month, and the appointments of the coming
/// week under their day (a click opens one).
pub fn todo_bar(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("todo-bar").show(ui, |ui| {
        let shown = st.mini_month.unwrap_or_else(|| {
            let (y, m, _) = civil_from_days(today(s));
            y * 12 + i64::from(m) - 1
        });
        mini_month_to(s, st, ui, acts, shown, shown, true, true);
        ui.add_space(8.0);
        let r = ui.available_rect_before_wrap();
        ui.painter().line_segment([r.left_top(), r.right_top()], egui::Stroke::new(1.0, t.border));
        ui.add_space(8.0);
        let now = s.clock.now;
        let mut items: Vec<_> = s.agenda(now, now + 7 * 86_400).into_iter().filter(|i| i.end > now).collect();
        items.sort_by_key(|i| (i.start, i.all_day));
        if items.is_empty() {
            ui.label(RichText::new(tl!("Nothing planned in the coming week.")).color(t.dim).small());
            return;
        }
        let now_day = today(s);
        let mut last_day = None;
        for item in items.iter().take(20) {
            let day = day_of(item.start, s).max(now_day);
            if last_day != Some(day) {
                last_day = Some(day);
                let label = match day - now_day {
                    0 => tl!("Today").to_string(),
                    1 => tl!("Tomorrow").to_string(),
                    _ => crate::i18n::date(&dates::day_label(day)),
                };
                ui.add_space(4.0);
                ui.label(RichText::new(label).font(theme::bold(12.5)).color(t.text));
            }
            // An all-day event says so under its title (the time column is narrow).
            let when = if item.all_day { String::new() } else { hm(item.start, s) };
            let under = match (item.all_day, item.location.is_empty()) {
                (true, true) => tl!("All day").to_string(),
                (true, false) => format!("{} · {}", tl!("All day"), item.location),
                (false, _) => item.location.clone(),
            };
            let resp = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.add_sized(vec2(40.0, 16.0), egui::Label::new(RichText::new(when).color(t.dim).small()));
                    let (bar, _) = ui.allocate_exact_size(vec2(3.0, 30.0), Sense::hover());
                    ui.painter().rect_filled(bar, 1.0, color(item.color, &t));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let title = if item.summary.trim().is_empty() { tl!("(no title)").to_string() } else { item.summary.clone() };
                        ui.add(egui::Label::new(RichText::new(title).font(theme::bold(12.0)).color(t.text)).truncate());
                        if !under.is_empty() {
                            ui.add(egui::Label::new(RichText::new(&under).color(t.dim).small()).truncate());
                        }
                    });
                })
                .response
                .interact(Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(resp.rect.expand(2.0), 4.0, t.hover_row.gamma_multiply(0.6));
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
                && let Some(d) = Dialog::open_event(s, &item.uid, item.occurrence)
            {
                acts.push(Action::Dialog(d));
            }
            ui.add_space(4.0);
        }
    });
}

/// The main pane.
pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    egui::Frame::new().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.button(tl!("Today")).clicked() {
                acts.push(Action::Run("calendar.goto", json!({"date": "today"})));
            }
            if widgets::tiny_icon_button(ui, Icon::ChevronLeft, "Previous", &t).clicked() {
                acts.push(Action::Run("calendar.goto", json!({"step": -1})));
            }
            if widgets::tiny_icon_button(ui, Icon::ChevronRight, "Next", &t).clicked() {
                acts.push(Action::Run("calendar.goto", json!({"step": 1})));
            }
            ui.add_space(6.0);
            ui.label(RichText::new(title(s)).font(theme::bold(18.0)).color(t.text));
            // The view, as a drop-down at the right, as Outlook's.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let views = [
                    (CalMode::Day, "Day", "day"),
                    (CalMode::WorkWeek, "Work week", "workWeek"),
                    (CalMode::Week, "Week", "week"),
                    (CalMode::Month, "Month", "month"),
                ];
                let current = views.iter().find(|(m, _, _)| *m == s.cal_mode).map_or("Week", |(_, l, _)| *l);
                egui::ComboBox::from_id_salt("calendar-view").selected_text(tl!(current)).show_ui(ui, |ui| {
                    for (mode, label, code) in views {
                        if ui.selectable_label(s.cal_mode == mode, tl!(label)).clicked() {
                            acts.push(Action::Run("calendar.view", json!({"mode": code})));
                        }
                    }
                });
            });
        });
    });
    let (d0, d1) = s.cal_days();
    let began = web_time::Instant::now();
    let items = s.agenda(s.local_midnight(d0), s.local_midnight(d1));
    let found = began.elapsed();
    match s.cal_mode {
        CalMode::Month => month(s, st, ui, &items, d0, acts),
        CalMode::Day | CalMode::WorkWeek | CalMode::Week => days(s, st, ui, &items, d0, d1, acts),
    }
    slow_drawing(ui, began.elapsed(), found, items.len());
    // A drag whose chip is no longer drawn (the view changed under it) is let go of.
    if st.cal_drag.is_some() && !ui.input(|i| i.pointer.primary_down()) {
        st.cal_drag = None;
    }
}

/// A frame of the calendar this long or longer is said in the log.
const SLOW_DRAWING: web_time::Duration = web_time::Duration::from_millis(50);

/// The calendar drawn slowly (`took`, of which `found` finding what it shows): said in the log,
/// once every ten seconds at most, to tell where a slow calendar spends its time.
fn slow_drawing(ui: &Ui, took: web_time::Duration, found: web_time::Duration, shown: usize) {
    if took < SLOW_DRAWING {
        return;
    }
    let id = egui::Id::unique("calendar-slow-drawing");
    let now = ui.input(|i| i.time);
    let last = ui.ctx().data(|d| d.get_temp::<f64>(id));
    if last.is_some_and(|t| now - t < 10.0) {
        return;
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, now));
    log::info!("calendar: drawn in {} ms ({} ms finding the {shown} appointments shown)", took.as_millis(), found.as_millis());
}

/// Open an item (its event).
fn open(s: &Session, item: &AgendaItem) -> Option<Action> {
    Dialog::open_event(s, &item.uid, item.occurrence).map(Action::Dialog)
}

/// The id a chip is drawn under: the same meeting can be in two calendars (two accounts', or an
/// account's and a shared mailbox's) at the same time, so it's told apart by whose it is, or
/// the click goes to neither. (Fixed, not the pane's, so a test can find the chip.)
pub(crate) fn chip_id(item: &AgendaItem) -> egui::Id {
    egui::Id::unique(("calendar-chip", &item.account, &item.calendar, &item.uid, item.start))
}

fn chip(ui: &mut Ui, s: &Session, t: &Tokens, rect: Rect, item: &AgendaItem, text: &str) -> egui::Response {
    // One that can move is taken hold of as well as clicked.
    let sense = if item.movable { Sense::click_and_drag() } else { Sense::click() };
    let resp = ui.interact(rect, chip_id(item), sense);
    let c = item_color(item, t);
    let tentative = item.needs_reply || item.status == "TENTATIVE";
    // Stronger under the pointer, with its colour round it.
    let hovered = resp.hovered();
    let fill = match (tentative, hovered) {
        (true, false) => t.surface,
        (true, true) => t.hover,
        (false, h) => c.gamma_multiply(match (t.dark, h) {
            (true, false) => 0.35,
            (true, true) => 0.5,
            (false, false) => 0.18,
            (false, true) => 0.3,
        }),
    };
    ui.painter().rect(rect, 4.0, fill, egui::Stroke::new(if tentative || hovered { 1.0 } else { 0.0 }, c), egui::StrokeKind::Inside);
    ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 2.0, c);
    // As many whole lines as the box holds, the last one ending in "…" when more is left (a line
    // cut in half by the box's edge read badly).
    let font = theme::regular(12.0);
    let row = ui.fonts_mut(|f| f.row_height(&font)).max(1.0);
    let mut job = egui::text::LayoutJob::simple(text.to_string(), font, t.text, (rect.width() - 8.0).max(4.0));
    job.wrap.max_rows = ((rect.height() - 4.0) / row).floor().max(1.0) as usize;
    job.wrap.overflow_character = Some('…');
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().with_clip_rect(rect.shrink(1.0)).galley(rect.min + vec2(6.0, 2.0), galley, t.text);
    // Outlook's card when the pointer rests on it: what it is, when, where, and its calendar
    // (not while something is dragged: the card would hide where it's going).
    if !resp.dragged() && !ui.input(|i| i.pointer.primary_down()) {
        hover_card(&resp, s, t, item);
    }
    resp
}

/// The colour of an appointment: its first category's, as Outlook draws a categorized one; else
/// its calendar's.
fn item_color(item: &AgendaItem, t: &Tokens) -> egui::Color32 {
    item.category.map_or_else(|| color(item.color, t), |i| theme::category_color(i, t.dark))
}

/// What the card says, one line each: when the appointment is, where, and the calendar it's in
/// (nothing for what it doesn't have). As Outlook's card, which also shows the reminder (Quatrelle
/// doesn't read `VALARM` yet).
pub(crate) fn card_rows(s: &Session, item: &AgendaItem) -> Vec<(Icon, String)> {
    let mut out = vec![(Icon::Clock, when(s, item))];
    let location = item.location.trim();
    if !location.is_empty() {
        out.push((Icon::Location, location.to_string()));
    }
    out.push((Icon::Calendar, calendar_name(s, item)));
    if let Some(m) = item.reminder {
        out.push((Icon::Reminder, reminder_text(m)));
    }
    out
}

/// When the reminder sounds, as Outlook's card says it: `15 minutes before`, `1 hour before`.
pub(crate) fn reminder_text(m: i64) -> String {
    let n = m.unsigned_abs();
    if n == 0 {
        return tl!("When it starts").to_string();
    }
    // Days and hours when they divide it, minutes otherwise.
    let (unit, count) = if n.is_multiple_of(1440) {
        (1440, n / 1440)
    } else if n.is_multiple_of(60) {
        (60, n / 60)
    } else {
        (1, n)
    };
    match (m > 0, count == 1, unit) {
        (true, true, 1440) => tl!("1 day before").to_string(),
        (true, false, 1440) => tf!("{n} days before", n = count),
        (true, true, 60) => tl!("1 hour before").to_string(),
        (true, false, 60) => tf!("{n} hours before", n = count),
        (true, true, _) => tl!("1 minute before").to_string(),
        (true, false, _) => tf!("{n} minutes before", n = count),
        (false, true, 1440) => tl!("1 day after").to_string(),
        (false, false, 1440) => tf!("{n} days after", n = count),
        (false, true, 60) => tl!("1 hour after").to_string(),
        (false, false, 60) => tf!("{n} hours after", n = count),
        (false, true, _) => tl!("1 minute after").to_string(),
        (false, false, _) => tf!("{n} minutes after", n = count),
    }
}

/// When an appointment is, as the Day view says it: `Friday, 23 October 2026 · 12:30 – 13:30`
/// (both days for something over several, `All day` for an all-day one).
fn when(s: &Session, item: &AgendaItem) -> String {
    let (d0, d1) = (day_of(item.start, s), day_of(item.end.max(item.start + 1) - 1, s));
    let date = if d1 > d0 { tf!("{from} – {to}", from = day_text(d0), to = day_text(d1)) } else { day_text(d0) };
    let time = if item.all_day { tl!("All day").to_string() } else { format!("{} – {}", hm(item.start, s), hm(item.end, s)) };
    format!("{date} · {time}")
}

/// A local day written out, as the Day view's heading has it.
fn day_text(d: i64) -> String {
    let (y, m, dd) = civil_from_days(d);
    let wd = (d + 3).rem_euclid(7) as usize;
    let name = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].get(wd).copied().unwrap_or("");
    tf!(
        "{weekday}, {day} {month} {year}",
        weekday = tl!(name),
        day = dd,
        month = crate::i18n::tc("month", MONTHS.get(m as usize - 1).copied().unwrap_or("")),
        year = y
    )
}

/// The calendar an appointment is in, by the name the calendar's pane gives it.
fn calendar_name(s: &Session, item: &AgendaItem) -> String {
    if item.calendar == quatrelle_engine::model::pim::LOCAL {
        return tl!("Calendar").to_string();
    }
    s.shown_calendars()
        .into_iter()
        .find(|(a, c, _, _)| *a == item.account && *c == item.calendar)
        .map(|(_, _, n, _)| n)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| tl!("Calendar").to_string())
}

/// The card itself: the title on the appointment's colour, then its lines, as Outlook's.
fn hover_card(resp: &egui::Response, s: &Session, t: &Tokens, item: &AgendaItem) {
    let c = item_color(item, t);
    let title = if item.summary.trim().is_empty() { tl!("(no title)").to_string() } else { item.summary.clone() };
    let rows = card_rows(s, item);
    // The tooltip's own frame is replaced by the card's: the header's band on top, square
    // corners where the body meets it.
    let frame = egui::Frame::new().fill(t.surface).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(5.0);
    let mut tip = egui::Tooltip::for_enabled(resp);
    tip.popup = tip.popup.frame(frame).gap(6.0).width(340.0);
    tip.at_pointer().show(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
        egui::Frame::new()
            .fill(c.gamma_multiply(if t.dark { 0.45 } else { 0.22 }))
            .corner_radius(egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 })
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.set_width(300.0);
                ui.label(RichText::new(title).font(theme::bold(13.5)).color(t.text));
            });
        egui::Frame::new().corner_radius(egui::CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 }).inner_margin(egui::Margin::symmetric(10, 8)).show(
            ui,
            |ui| {
                ui.set_width(300.0);
                ui.spacing_mut().item_spacing.y = 5.0;
                for (icon, text) in &rows {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        icons::paint(ui.painter(), r, *icon, t.dim);
                        ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
                    });
                }
            },
        );
    });
}

fn month(s: &Session, st: &mut UiState, ui: &mut Ui, items: &[AgendaItem], d0: i64, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let t = &t;
    let (_, shown_month, _) = civil_from_days(s.cal_day());
    let now = today(s);
    let area = ui.available_rect_before_wrap().shrink2(vec2(10.0, 4.0));
    let head = 22.0;
    let cw = area.width() / 7.0;
    let rh = ((area.height() - head) / 6.0).max(60.0);
    // Dragged to another day, as long as it was, at the same time.
    let drag_area = drag::Area::Month { area, head, cw, rh, day0: d0 };
    for (i, d) in DAYS.iter().enumerate() {
        ui.painter().text(pos2(area.min.x + cw * (i as f32 + 0.5), area.min.y + 10.0), Align2::CENTER_CENTER, tl!(d), theme::regular(12.0), t.dim);
    }
    for w in 0..6 {
        for i in 0..7 {
            let d = d0 + w * 7 + i;
            let r = Rect::from_min_size(pos2(area.min.x + cw * i as f32, area.min.y + head + rh * w as f32), vec2(cw, rh));
            if ui.rect_contains_pointer(r) {
                ui.painter().rect_filled(r, 0.0, t.hover_row);
            }
            ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            let (_, m, dd) = civil_from_days(d);
            let num_pos = r.min + vec2(14.0, 12.0);
            if d == now {
                ui.painter().circle_filled(num_pos, 10.0, t.accent);
            }
            ui.painter().text(
                num_pos,
                Align2::CENTER_CENTER,
                dd.to_string(),
                theme::regular(12.0),
                if d == now {
                    t.on_accent
                } else if m == shown_month {
                    t.text
                } else {
                    t.faint
                },
            );
            let bg = ui.interact(r, ui.scope_id().with(("month-day", d)), Sense::click());
            let todays: Vec<&AgendaItem> =
                items.iter().filter(|it| day_of(it.start, s) <= d && day_of(it.end.max(it.start + 1) - 1, s) >= d).collect();
            let fit = ((rh - 26.0) / 19.0).floor().max(0.0) as usize;
            let shown = if todays.len() > fit { fit.saturating_sub(1) } else { todays.len() };
            for (k, it) in todays.iter().take(shown).enumerate() {
                let cr = Rect::from_min_size(r.min + vec2(3.0, 24.0 + 19.0 * k as f32), vec2(cw - 6.0, 17.0));
                let text = if it.all_day { it.summary.clone() } else { format!("{} {}", hm(it.start, s), it.summary) };
                let resp = chip(ui, s, t, cr, it, &text);
                // Only on the day it starts: a chip drawn on each day it spans would start a
                // drag each.
                if day_of(it.start, s) == d {
                    drag::chip(ui, st, s, &resp, cr, it, &drag_area, acts);
                }
                if resp.clicked()
                    && let Some(a) = open(s, it)
                {
                    acts.push(a);
                }
            }
            if todays.len() > shown {
                let more = ui.put(
                    Rect::from_min_size(r.min + vec2(6.0, 24.0 + 19.0 * shown as f32), vec2(cw - 12.0, 17.0)),
                    egui::Link::new(RichText::new(tf!("{n} more", n = todays.len() - shown)).small()),
                );
                if more.clicked() {
                    let (y, mm, ddd) = civil_from_days(d);
                    acts.push(Action::Run("calendar.view", json!({"mode": "day"})));
                    acts.push(Action::Run("calendar.goto", json!({"date": format!("{y:04}-{mm:02}-{ddd:02}")})));
                }
            }
            if bg.double_clicked() {
                acts.push(Action::Dialog(Dialog::new_event(s, Some(s.local_midnight(d) + 9 * 3600))));
            }
        }
    }
    drag::paint(ui, st, s, t, &drag_area, 0, &|_| 0);
    ui.allocate_rect(area, Sense::hover());
}
