// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The calendar's side pane: a month (or several) to jump around in, then the calendars in their
//! groups; and beside the mail, Outlook's To-Do bar.

use super::*;

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
                acts.push(Action::Dialog(Dialog::open_calendars()));
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
pub(super) fn section_key(
    sections: &[quatrelle_engine::PaneSection],
    a: &quatrelle_engine::AccountState,
    c: &quatrelle_engine::model::pim::CalendarInfo,
) -> Option<String> {
    let owner = c.id.strip_prefix("shared:")?.to_ascii_lowercase();
    let key = format!("{}|{owner}", a.config.id);
    sections.iter().any(|x| x.key == key).then_some(key)
}

/// A mailbox's name above its calendars, with the icon of a shared one.
pub(super) fn section_heading(s: &Session, t: &Tokens, ui: &mut Ui, key: &str) {
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
pub(super) fn calendar_row(
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
pub(super) fn months(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
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
pub(super) fn mini_month(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, shown: i64, month: i64, arrows: bool) {
    mini_month_to(s, st, ui, acts, shown, month, arrows, false);
}

/// [`mini_month`], a click on a day opening the calendar there when `open` (from the mail).
#[allow(clippy::too_many_arguments)]
pub(super) fn mini_month_to(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, shown: i64, month: i64, arrows: bool, open: bool) {
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
