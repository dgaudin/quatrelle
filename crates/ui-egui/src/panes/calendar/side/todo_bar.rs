// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Outlook's To-Do bar beside the mail: the calendar, the favorite people and the To-Do List
//! small, the parts View › To-Do Bar ticks.

use super::*;

/// Beside the mail, as Outlook's To-Do bar: the parts View › To-Do Bar ticks, in that order —
/// the calendar, People (the favorite people kept here), and the To-Do List small.
pub fn todo_bar(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let parts = s.prefs.todo_bar();
    crate::widgets::solid_scroll_bars(ui);
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("todo-bar").show(ui, |ui| {
        if parts.calendar {
            todo_heading(ui, &t, "Calendar");
            calendar_part(s, st, ui, acts);
        }
        if parts.people {
            todo_heading(ui, &t, "People");
            people_part(s, &t, ui, acts);
        }
        if parts.tasks {
            todo_heading(ui, &t, "Tasks");
            tasks_part(s, st, ui, acts);
        }
    });
}

/// A part's small heading, with a line above it (but for the first).
fn todo_heading(ui: &mut Ui, t: &Tokens, label: &str) {
    ui.add_space(6.0);
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment([r.left_top(), r.right_top()], egui::Stroke::new(1.0, t.border));
    ui.add_space(4.0);
    ui.label(RichText::new(tl!(label)).font(theme::bold(12.0)).color(t.dim));
}

/// The month, then the appointments of the coming week under their day (a click opens one).
fn calendar_part(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let shown = st.mini_month.unwrap_or_else(|| {
        let (y, m, _) = civil_from_days(today(s));
        y * 12 + i64::from(m) - 1
    });
    mini_month_to(s, st, ui, acts, shown, shown, true, true);
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
                    let title = Some(super::title(item)).filter(|x| !x.trim().is_empty()).unwrap_or_else(|| tl!("(no title)").to_string());
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
}

/// People: the favorite people kept on this computer, each with their picture (or initials). A
/// click opens the contact, or writes to someone with none; a right-click takes them off.
fn people_part(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    if s.prefs.favorite_people.is_empty() {
        ui.label(RichText::new(tl!("No favorite people yet: right-click a contact or a sender to add one.")).color(t.dim).small());
        return;
    }
    for email in &s.prefs.favorite_people {
        let person = s.person_by_email(email);
        let name = person.as_ref().map(|p| p.card.display()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| email.clone());
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect.shrink2(vec2(4.0, 1.0)), 4.0, t.hover);
        }
        let ar = egui::Rect::from_min_size(egui::pos2(rect.left() + 4.0, rect.center().y - 12.0), vec2(24.0, 24.0));
        let texture = person.as_ref().and_then(|p| {
            p.card
                .photo
                .as_ref()
                .and_then(|ph| crate::people::card_picture(ui, &p.card.uid, ph))
                .or_else(|| crate::people::address_picture(ui, s, email))
        });
        match texture {
            Some(tex) => {
                let side = ar.width().min(ar.height());
                let r = egui::Rect::from_center_size(ar.center(), vec2(side, side));
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(tex.id(), r, uv, egui::Color32::WHITE);
            }
            None => {
                let initials: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
                widgets::avatar(ui, ar.center(), 12.0, &initials, theme::avatar_color(email, t.dark));
            }
        }
        let room = (rect.width() - 40.0).max(40.0);
        let g = widgets::one_line(ui, &name, theme::regular(13.0), t.text, room);
        let size = g.size();
        ui.painter().galley(egui::pos2(rect.left() + 34.0, rect.center().y - size.y / 2.0), g, t.text);
        if resp.clicked() {
            match &person {
                Some(p) => acts.push(Action::OpenPerson { uid: p.card.uid.clone(), cards: false }),
                None => acts.push(Action::Run("mail.new", json!({"to": email}))),
            }
        }
        resp.context_menu(|ui| {
            if ui.button(tl!("Send email")).clicked() {
                acts.push(Action::Run("mail.new", json!({"to": email})));
                ui.close();
            }
            if let Some(p) = &person
                && ui.button(tl!("Open contact")).clicked()
            {
                acts.push(Action::OpenPerson { uid: p.card.uid.clone(), cards: false });
                ui.close();
            }
            if ui.button(tl!("Remove from Favorites")).clicked() {
                acts.push(Action::Run("contact.favorite", json!({"email": email, "on": false})));
                ui.close();
            }
        });
        ui.add_space(2.0);
    }
}

/// The To-Do List small: type a new task, then the tasks and flagged messages by follow-up due
/// day; a tick marks one done, a double click opens a message (or a task), a right-click gives
/// the Follow Up menu.
fn tasks_part(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let id = egui::Id::unique("todo-bar-new");
    let mut text: String = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
    let r =
        ui.add_enabled(!s.accounts.is_empty(), egui::TextEdit::singleline(&mut text).hint_text(tl!("Type a new task")).desired_width(f32::INFINITY));
    if r.changed() {
        ui.ctx().data_mut(|d| d.insert_temp(id, text.clone()));
    }
    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        let subject = text.trim().to_string();
        if !subject.is_empty() {
            acts.push(Action::Run("task.create", json!({"subject": subject})));
            text.clear();
            ui.ctx().data_mut(|d| d.insert_temp(id, String::new()));
            r.request_focus();
        }
    }
    let today = dates::today(&s.clock);
    // Folders still being read from disk in the background: repaint as they come in.
    if s.disk_scan_running() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    // Computed again only when mail or tasks changed, not at every image.
    let list = st.todo_memo.bar_list(s, today);
    let mut group = None;
    for row in list.iter().take(40) {
        let g = row.group(today);
        if group != Some(g) {
            group = Some(g);
            ui.add_space(4.0);
            let color = if g == quatrelle_engine::model::pim::tasks::DueGroup::Overdue { t.danger } else { t.faint };
            ui.label(RichText::new(tl!(g.label())).font(theme::bold(11.5)).color(color));
        }
        todo_mini_row(s, &t, ui, row, today, acts);
    }
}

/// One row of the small To-Do List.
fn todo_mini_row(s: &Session, t: &Tokens, ui: &mut Ui, row: &quatrelle_engine::TodoRef, today: i64, acts: &mut Vec<Action>) {
    let id = row.uid();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect.shrink2(vec2(4.0, 1.0)), 4.0, t.hover);
    }
    let done = row.is_complete();
    let late = !done && row.due().is_some_and(|d| d < today);
    let tick = egui::Rect::from_center_size(egui::pos2(rect.left() + 12.0, rect.center().y), vec2(12.0, 12.0));
    let tick_resp = ui.interact(tick.expand(3.0), ui.scope_id().with(("todo-tick", &id)), Sense::click());
    if done {
        ui.painter().circle_filled(tick.center(), 6.0, t.accent);
        icons::paint(ui.painter(), tick.shrink(2.0), Icon::Read, t.on_accent);
    } else {
        ui.painter().circle_stroke(tick.center(), 5.5, egui::Stroke::new(1.2, if late { t.danger } else { t.dim }));
    }
    if tick_resp.clicked() {
        match row {
            quatrelle_engine::TodoRef::Task(task) => {
                acts.push(Action::Run("task.complete", json!({"uid": task.task.uid, "done": !done})));
            }
            quatrelle_engine::TodoRef::Mail(mail) => {
                let p = if done { json!({"key": mail.key.to_id(), "on": true}) } else { json!({"key": mail.key.to_id(), "complete": true}) };
                acts.push(Action::Run("mail.flag", p));
            }
        }
    }
    let mut edge = 4.0;
    if let Some(due) = row.due() {
        let text = crate::tasks::short_day(due);
        let color = if late { t.danger } else { t.faint };
        let g = widgets::one_line(ui, &text, theme::regular(10.5), color, 80.0);
        let w = g.size().x;
        let x = rect.right() - edge - w;
        ui.painter().galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, color);
        edge += w + 8.0;
    }
    let color = if done {
        t.faint
    } else if late {
        t.danger
    } else {
        t.text
    };
    let room = (rect.width() - 30.0 - edge).max(30.0);
    let g = widgets::one_line(ui, row.subject(), theme::regular(12.5), color, room);
    let size = g.size();
    let top = egui::pos2(rect.left() + 24.0, rect.center().y - size.y / 2.0);
    if done {
        ui.painter().line_segment([egui::pos2(top.x, rect.center().y), egui::pos2(top.x + size.x, rect.center().y)], egui::Stroke::new(1.0, t.faint));
    }
    ui.painter().galley(top, g, color);
    if resp.double_clicked() {
        match row {
            quatrelle_engine::TodoRef::Task(task) => acts.push(Action::OpenTask(task.task.uid.clone())),
            quatrelle_engine::TodoRef::Mail(mail) => acts.push(Action::Open(mail.key.clone())),
        }
    }
    resp.context_menu(|ui| match row {
        quatrelle_engine::TodoRef::Task(task) => {
            if ui.button(tl!("Open")).clicked() {
                acts.push(Action::OpenTask(task.task.uid.clone()));
                ui.close();
            }
            if ui.button(tl!(if done { "Mark as not complete" } else { "Mark Complete" })).clicked() {
                acts.push(Action::Run("task.complete", json!({"uid": task.task.uid, "done": !done})));
                ui.close();
            }
        }
        quatrelle_engine::TodoRef::Mail(mail) => {
            if ui.button(tl!("Open")).clicked() {
                acts.push(Action::Open(mail.key.clone()));
                ui.close();
            }
            ui.menu_button(tl!("Follow up"), |ui| crate::organize::follow_up_menu(ui, s, t, acts, Some(&mail.key.to_id())));
            if ui.button(tl!("Remove from List")).clicked() {
                acts.push(Action::Run("mail.flag", json!({"key": mail.key.to_id(), "on": false})));
                ui.close();
            }
        }
    });
}
