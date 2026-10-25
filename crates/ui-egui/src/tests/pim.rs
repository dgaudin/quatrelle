//! UI tests: calendar, people, notes, invitations and reminders.

use super::*;

/// People's list and A–Z index were a few pixels wider than the pane, so the pane (shared by
/// every module) grew each frame to its widest and Mail, Calendar and Notes stayed that wide.
#[test]
fn people_keep_the_pane_its_width() {
    let (mut a, dir) = app("pane-width");
    let ctx = egui::Context::default();
    // A panel's id is its parent's (the window's root) salted with its name.
    let mut root = egui::Id::NULL;
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        root = ui.scope_id();
        a.ui(ui);
    });
    out.textures_delta.clear();
    let nav = root.with_salt(egui::IdSalt::new("nav"));
    let width = |ctx: &egui::Context| egui::containers::panel::PanelState::load(ctx, nav).map_or(0.0, |p| p.size().x);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let before = width(&ctx);
    a.run("view.people", json!({}));
    for _ in 0..60 {
        frame(&ctx, &mut a, vec![]);
    }
    a.run("view.mail", json!({}));
    frame(&ctx, &mut a, vec![]);
    let after = width(&ctx);
    assert!(before > 0.0 && after <= before + 1.0, "the pane went from {before} to {after}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn calendar_and_people_forms_save_through_commands() {
    let (mut a, dir) = app("pim-forms");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    pim_tour(&mut a, &ctx);
    // A new event from the form.
    let mut d = Dialog::new_event(&a.session, Some(NOW + 3 * 86_400));
    let params = match &mut d {
        Dialog::Event(f) => {
            f.summary = "Grade the trailer".into();
            f.location = "Suite B".into();
            f.repeat = "weekly".into();
            f.params()
        }
        _ => serde_json::Value::Null,
    };
    a.ui.dialog = Some(d);
    a.act(crate::Action::Submit("event.create", params));
    assert!(a.ui.dialog.is_none(), "saved and closed");
    let (from, to) = (NOW, NOW + 30 * 86_400);
    let made: Vec<_> = a.session.agenda(from, to).into_iter().filter(|i| i.summary == "Grade the trailer").collect();
    assert!(made.len() >= 4 && made.iter().all(|i| i.recurring && i.location == "Suite B"), "{made:?}");
    // Reopened, it shows what was saved; a bad time keeps the form open with the reason.
    let mut d = Dialog::open_event(&a.session, &made[0].uid, made[0].occurrence).unwrap();
    let params = match &mut d {
        Dialog::Event(f) => {
            assert_eq!((f.summary.as_str(), f.repeat.as_str(), f.recurring), ("Grade the trailer", "weekly", true));
            f.start = "25:00".into();
            f.params()
        }
        _ => serde_json::Value::Null,
    };
    a.ui.dialog = Some(d);
    a.act(crate::Action::Submit("event.update", params));
    assert!(matches!(&a.ui.dialog, Some(Dialog::Event(f)) if f.error.is_some()));
    // A person from the form is picked afterwards.
    a.ui.dialog = Some(Dialog::new_contact());
    let mut f = crate::pim_dialogs::ContactForm { name: "Mira Chen".into(), ..Default::default() };
    f.emails[0] = "mira@colour.example".into();
    f.phones[2] = "+32 490 12 34 56".into();
    f.addresses[1].city = "Gent".into();
    f.birthday = "1990-07-12".into();
    a.act(crate::Action::Submit("contact.create", f.params()));
    let picked = a.ui.person.clone().unwrap();
    let card = a.session.find_person(&picked).unwrap().card;
    assert_eq!(card.display(), "Mira Chen");
    // Issue #1: each number keeps its kind, the address and birthday their fields, and the form
    // shows them where they were.
    assert_eq!((card.phones[0].kind.as_str(), card.addresses[0].kind.as_str(), card.birthday.as_str()), ("cell", "home", "1990-07-12"));
    let back = crate::pim_dialogs::ContactForm::edit(&card);
    assert_eq!((back.phones[2].as_str(), back.addresses[1].city.as_str(), back.address_shown), ("+32 490 12 34 56", "Gent", 1));
    f.emails[0] = "not an address".into();
    a.ui.dialog = Some(Dialog::new_contact());
    a.act(crate::Action::Submit("contact.create", f.params()));
    assert!(matches!(&a.ui.dialog, Some(Dialog::Contact(f)) if f.error.is_some()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn people_lay_out_as_a_list_cards_or_a_phone_list() {
    let (mut a, dir) = app("people-views");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "people"}));
    // One contact gets a picture (a 1×1 PNG): the business cards draw it rather than initials.
    let photo_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let pictured = a.session.people("").first().map(|p| p.card.uid.clone());
    if let Some(uid) = &pictured {
        let photo = quatrelle_engine::model::vcard::Photo { media: "image/png".into(), base64: photo_base64.into() };
        a.session.update_card(uid, |c| c.photo = Some(photo)).unwrap();
    }
    for v in crate::people::PeopleView::ALL {
        a.ui.people_view = v;
        frame(&ctx, &mut a, vec![]);
        frame(&ctx, &mut a, vec![]);
    }
    // The card's picture was decoded and drawn (egui keeps the texture while it's shown).
    if let Some(uid) = &pictured {
        let id = egui::Id::unique(("contact-photo", uid.as_str(), photo_base64.len()));
        let drawn = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(id)).flatten().is_some();
        assert!(drawn, "the business card draws the contact's picture");
    }
    // The layout is remembered with the window's other settings.
    // The list's columns and sort are kept too.
    a.ui.people_list.columns = vec![crate::people::Column::Mobile, crate::people::Column::Book];
    a.ui.people_list.sort = Some(crate::people::Column::Book);
    frame(&ctx, &mut a, vec![]);
    let saved = a.saved_state();
    assert_eq!(saved["peopleView"], "phone");
    a.ui.people_view = crate::people::PeopleView::People;
    a.ui.people_list = Default::default();
    a.restore_state(&saved);
    assert_eq!(a.ui.people_view, crate::people::PeopleView::Phone);
    assert_eq!(a.ui.people_list.columns, [crate::people::Column::Mobile, crate::people::Column::Book]);
    assert_eq!(a.ui.people_list.sort, Some(crate::people::Column::Book));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn notes_are_written_on_and_kept() {
    let (mut a, dir) = app("notes-ui");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "notes"}));
    frame(&ctx, &mut a, vec![]);
    // New note: made, and written on at once.
    a.act(crate::Action::NewNote);
    let uid = a.ui.note_edit.as_ref().map(|(u, _)| u.clone()).unwrap();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("Water the plants".into())]);
    assert_eq!(a.ui.note_edit.as_ref().map(|(_, t)| t.as_str()), Some("Water the plants"));
    // Escape leaves it: the text is kept.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.note_edit.is_none());
    assert!(a.session.notes("").iter().any(|n| n.note.uid == uid && n.note.text == "Water the plants"));
    // Ctrl+1 goes back to Mail.
    frame(&ctx, &mut a, vec![key(Key::Num1, Modifiers::COMMAND)]);
    assert_eq!(a.session.module, quatrelle_engine::Module::Mail);
    let _ = std::fs::remove_dir_all(dir);
}

/// Tasks: Ctrl+4 opens them; a task typed in the box at the top is added with Enter; the one
/// picked shows its form beside the list; ticked, it goes (and comes back with the completed
/// ones); its reminder's Open Item shows it.
#[test]
fn tasks_are_typed_in_picked_and_ticked() {
    let (mut a, dir) = app("tasks-ui");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Num4, Modifiers::COMMAND)]);
    assert_eq!(a.session.module, quatrelle_engine::Module::Tasks);
    frame(&ctx, &mut a, vec![]);
    let before = a.session.tasks("", false).len();
    assert!(before > 0, "the sample mailbox has tasks to show");
    a.ui.tasks.focus_new = true;
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("Call the printer".into())]);
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    let made = a.session.tasks("printer", false);
    assert_eq!(made.len(), 1, "added with Enter");
    assert!(a.ui.tasks.new_text.is_empty(), "the box is cleared for the next one");
    let uid = made[0].task.uid.clone();
    // Picked: its form beside the list, in both themes.
    a.ui.tasks.selected = Some(uid.clone());
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::SetDark(Some(true)));
    frame(&ctx, &mut a, vec![]);
    a.run("task.complete", json!({"uid": uid}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.tasks.selected.is_none(), "done, it leaves the list of what's to do");
    a.ui.tasks.completed = true;
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.tasks("printer", true).iter().any(|r| r.task.is_complete()));
    // From its reminder: Tasks, the task picked.
    a.run("view.module", json!({"name": "mail"}));
    a.act(crate::Action::OpenTask(uid.clone()));
    frame(&ctx, &mut a, vec![]);
    assert_eq!((a.session.module, a.ui.tasks.selected.as_deref()), (quatrelle_engine::Module::Tasks, Some(uid.as_str())));
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #1: an invitation opens with its notes to read, its links under their words, and the
/// dialog fits the window however long the notes are; Edit notes goes back to the box. Issue #4:
/// the list's footer and the close warning draw.
#[test]
fn invitations_read_and_the_new_footers_draw() {
    let (mut a, dir) = app("invitation");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut notes = String::from("Join with Google Meet<https://meet.google.com/abc-defg-hij>\nMeeting link\n");
    for i in 0..60 {
        notes.push_str(&format!("Line {i} More options<https://calendar.google.com/calendar/event?action=VIEW&eid={}>\n", "x".repeat(120)));
    }
    let mut d = Dialog::new_event(&a.session, None);
    if let Dialog::Event(f) = &mut d {
        f.description = notes.clone();
        f.reading_notes = true;
    }
    a.ui.dialog = Some(d);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    // However long, the appointment's window stays on the screen (its notes scroll).
    assert_eq!(a.ui.event_windows.len(), 1, "it opens in a window of its own");
    let screen = ctx.content_rect();
    let tallest = ctx.memory(|m| m.layer_ids().filter_map(|l| m.area_rect(l.id)).map(|r| r.height()).fold(0.0f32, f32::max));
    assert!(tallest <= screen.height() + 1.0, "{tallest} in {}", screen.height());
    if let Some((_, f)) = a.ui.event_windows.first_mut() {
        assert_eq!(f.description, notes, "reading changes nothing");
        f.reading_notes = false;
    }
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = None;

    // The list's footer: a search, before and after asking the server; the close warning.
    a.run("search.run", serde_json::json!({"query": "zzz-nothing", "scope": "all"}));
    frame(&ctx, &mut a, vec![]);
    a.session.server_search = Some(quatrelle_engine::ServerSearch { query: "zzz-nothing".into(), pending: 1, ..Default::default() });
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Some(Dialog::LostOnClose { unsent: 2, changes: 1 });
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reminders_pop_up_snooze_and_dismiss() {
    let (mut a, dir) = app("reminders");
    let now = std::sync::Arc::new(std::sync::Mutex::new(NOW));
    let shown = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let pressed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Ticking(now.clone(), shown.clone(), pressed.clone()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    assert!(!crate::reminders::showing(&a.ui.reminders), "the sample has none due");
    // A new event's form starts with a 15-minute reminder, and saves it.
    let start = NOW + 3600;
    let mut d = Dialog::new_event(&a.session, Some(start));
    let params = match &mut d {
        Dialog::Event(f) => {
            assert_eq!(f.reminder, Some(crate::pim_dialogs::DEFAULT_REMINDER));
            f.summary = "Colour review".into();
            f.location = "Suite B".into();
            f.params()
        }
        _ => serde_json::Value::Null,
    };
    a.ui.dialog = Some(d);
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Submit("event.create", params));
    let it = a.session.agenda(NOW, NOW + 86_400).into_iter().find(|i| i.summary == "Colour review").unwrap();
    let reopened = Dialog::open_event(&a.session, &it.uid, it.occurrence).unwrap();
    assert!(matches!(&reopened, Dialog::Event(f) if f.reminder == Some(15)));
    // Not yet; then due: the window and one desktop notification.
    frame(&ctx, &mut a, vec![]);
    assert!(!crate::reminders::showing(&a.ui.reminders));
    *now.lock().unwrap() = start - 15 * 60;
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(crate::reminders::showing(&a.ui.reminders));
    let key = a.ui.reminders.due[0].key.clone();
    let told: Vec<(String, String)> = shown.lock().unwrap().iter().filter(|(t, _)| t == "Colour review").cloned().collect();
    assert_eq!(told.len(), 1, "told once");
    assert!(told[0].1.contains(&format!("[{key}]")) && told[0].1.contains("Snooze 5 minutes/Dismiss"), "{told:?}");
    // Snoozed from the notification's button: gone until the snooze is over, then back.
    pressed.lock().unwrap().push((key.clone(), crate::ReminderAction::Snooze));
    frame(&ctx, &mut a, vec![]);
    assert!(!crate::reminders::showing(&a.ui.reminders));
    assert_eq!(a.session.prefs.reminders.get(&key), Some(&(start - 15 * 60 + 5 * 60)), "five minutes");
    let withdrawn =
        |shown: &std::sync::Mutex<Vec<(String, String)>>| shown.lock().unwrap().iter().filter(|(t, k)| t == "withdrawn" && *k == key).count();
    assert_eq!(withdrawn(&shown), 1, "its notification taken back while it sleeps");
    *now.lock().unwrap() = start - 10 * 60;
    frame(&ctx, &mut a, vec![]);
    assert!(crate::reminders::showing(&a.ui.reminders));
    // Clicked on the notification once the window was closed: the window comes back with it.
    a.ui.reminders.closed.insert(key.clone());
    assert!(!crate::reminders::showing(&a.ui.reminders));
    pressed.lock().unwrap().push((key.clone(), crate::ReminderAction::Open));
    frame(&ctx, &mut a, vec![]);
    assert!(crate::reminders::showing(&a.ui.reminders));
    assert_eq!(a.ui.reminders.selected.as_deref(), Some(key.as_str()));
    // The option off: the window is drawn inside the main one (as it always is in tests, where
    // windows are drawn inside the page); on, it would be a window of its own.
    a.run("view.remindersOnTop", json!({"on": false}));
    frame(&ctx, &mut a, vec![]);
    assert!(!a.session.prefs.reminders_on_top && crate::reminders::showing(&a.ui.reminders));
    a.run("view.remindersOnTop", json!({"on": true}));
    frame(&ctx, &mut a, vec![]);
    // Snoozed from the window: gone until the snooze is over, then back (and told again).
    a.act(crate::Action::Run("reminders.snooze", json!({"key": key, "minutes": 5})));
    frame(&ctx, &mut a, vec![]);
    assert!(!crate::reminders::showing(&a.ui.reminders));
    *now.lock().unwrap() = start - 4 * 60;
    frame(&ctx, &mut a, vec![]);
    assert!(crate::reminders::showing(&a.ui.reminders));
    // Closed with the X: hidden, not dismissed; a new one brings it back with both.
    a.ui.reminders.closed.insert(key.clone());
    frame(&ctx, &mut a, vec![]);
    assert!(!crate::reminders::showing(&a.ui.reminders));
    // Many at once, with long names, still draw.
    for n in 0..40 {
        let p = json!({"summary": format!("{} {n}", "Very long meeting name ".repeat(8)), "start": quatrelle_engine::model::dates::day_string((start).div_euclid(86_400)) + "T23:59", "reminder": 20160});
        a.run("event.create", p);
    }
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(crate::reminders::showing(&a.ui.reminders));
    assert!(a.ui.reminders.due.len() > 40);
    // Dismiss all: gone, and stays gone.
    a.act(crate::Action::Run("reminders.dismiss", json!({})));
    frame(&ctx, &mut a, vec![]);
    *now.lock().unwrap() += 3600;
    frame(&ctx, &mut a, vec![]);
    assert!(!crate::reminders::showing(&a.ui.reminders));
    // Dismissed in the window, their notifications were taken back; a button pressed on one all
    // the same (just before it went, or on a desktop that keeps it) has nothing left to do, and
    // says nothing (it said "no reminder … is due").
    assert!(withdrawn(&shown) >= 3, "{:?}", shown.lock().unwrap());
    let notices = a.session.notices.len();
    pressed.lock().unwrap().push((key.clone(), crate::ReminderAction::Dismiss));
    pressed.lock().unwrap().push((key.clone(), crate::ReminderAction::Snooze));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.notices.len(), notices, "{:?}", a.session.notices.last());
    assert_eq!(a.session.prefs.reminders.get(&key), Some(&0), "still dismissed, not snoozed");
    let _ = std::fs::remove_dir_all(dir);
}

/// Outlook's drag and drop in the Day view: an appointment dragged an hour down moves an hour
/// later (and Undo puts it back), one dragged by its bottom edge ends later, Esc lets go, a
/// recurring one asks whether to move the occurrence or the series, and a meeting someone else
/// organizes doesn't move at all.
#[test]
fn appointments_are_dragged_to_another_time() {
    use crate::calendar::{HOUR, chip_id};
    let (mut a, dir) = app("drag-event");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "calendar"}));
    a.run("calendar.view", json!({"mode": "day"}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let agenda = |a: &MailApp| a.session.agenda(NOW - 86_400, NOW + 2 * 86_400);
    let lunch = agenda(&a).into_iter().find(|i| i.summary == "Lunch with Jun").unwrap();
    assert!(lunch.movable && !lunch.organized && !lunch.recurring);
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let chip =
        |ctx: &egui::Context, item: &quatrelle_engine::AgendaItem| ctx.read_response(chip_id(item)).map(|r| r.rect).expect("the chip is drawn");
    // Once the day has scrolled to the working hours (the chips stay put).
    for _ in 0..20 {
        let before = chip(&ctx, &lunch);
        frame(&ctx, &mut a, vec![]);
        if chip(&ctx, &lunch) == before {
            break;
        }
    }
    // Taken by its middle, dragged an hour down, let go.
    let from = chip(&ctx, &lunch).center();
    let to = from + egui::vec2(0.0, HOUR);
    frame(&ctx, &mut a, vec![Event::PointerMoved(from)]);
    frame(&ctx, &mut a, vec![press(from, true)]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(from + egui::vec2(0.0, 14.0))]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(to)]);
    let going = a.ui.cal_drag.as_ref().and_then(|d| d.to);
    assert_eq!(going, Some((lunch.start + 3600, lunch.end + 3600)), "the ghost an hour down");
    frame(&ctx, &mut a, vec![press(to, false)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.cal_drag.is_none());
    let moved = a.session.find_event(&lunch.uid).unwrap().1.event.clone();
    assert_eq!((moved.start, moved.end), (lunch.start + 3600, lunch.end + 3600));
    assert!(a.session.notices.last().is_some_and(|n| n.text.contains("Moved to")), "{:?}", a.session.notices.last());
    a.run("edit.undo", json!({}));
    assert_eq!(a.session.find_event(&lunch.uid).unwrap().1.event.start, lunch.start, "Undo puts it back");
    frame(&ctx, &mut a, vec![]);
    // Its bottom edge, dragged an hour down: ends an hour later, starts when it did.
    let r = chip(&ctx, &lunch);
    let edge = egui::pos2(r.center().x, r.max.y - 3.0);
    frame(&ctx, &mut a, vec![Event::PointerMoved(edge)]);
    frame(&ctx, &mut a, vec![press(edge, true)]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(edge + egui::vec2(0.0, 14.0))]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(edge + egui::vec2(0.0, HOUR))]);
    assert!(a.ui.cal_drag.as_ref().is_some_and(|d| d.resize), "{:?}", a.ui.cal_drag);
    frame(&ctx, &mut a, vec![press(edge + egui::vec2(0.0, HOUR), false)]);
    frame(&ctx, &mut a, vec![]);
    let longer = a.session.find_event(&lunch.uid).unwrap().1.event.clone();
    assert_eq!((longer.start, longer.end), (lunch.start, lunch.end + 3600));
    a.run("edit.undo", json!({}));
    for _ in 0..24 {
        frame(&ctx, &mut a, vec![]);
    }
    // Its top edge, dragged half an hour up: starts half an hour earlier, ends when it did.
    let r = chip(&ctx, &lunch);
    let top = egui::pos2(r.center().x, r.min.y + 2.0);
    frame(&ctx, &mut a, vec![Event::PointerMoved(top)]);
    frame(&ctx, &mut a, vec![press(top, true)]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(top - egui::vec2(0.0, 14.0))]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(top - egui::vec2(0.0, HOUR / 2.0))]);
    assert!(a.ui.cal_drag.as_ref().is_some_and(|d| d.resize && d.top), "{:?}", a.ui.cal_drag);
    frame(&ctx, &mut a, vec![press(top - egui::vec2(0.0, HOUR / 2.0), false)]);
    frame(&ctx, &mut a, vec![]);
    let earlier = a.session.find_event(&lunch.uid).unwrap().1.event.clone();
    assert_eq!((earlier.start, earlier.end), (lunch.start - 1800, lunch.end));
    a.run("edit.undo", json!({}));
    // (A moment later: a press soon after a release is a double-click in the making, not a drag.)
    for _ in 0..24 {
        frame(&ctx, &mut a, vec![]);
    }
    // Esc lets go: nothing moves.
    let from = chip(&ctx, &lunch).center();
    frame(&ctx, &mut a, vec![Event::PointerMoved(from)]);
    frame(&ctx, &mut a, vec![press(from, true)]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(from + egui::vec2(0.0, 14.0))]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(from + egui::vec2(0.0, HOUR))]);
    assert!(a.ui.cal_drag.is_some());
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![press(from + egui::vec2(0.0, HOUR), false)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.find_event(&lunch.uid).unwrap().1.event.start, lunch.start);
    // A recurring one asks first; "This occurrence" makes an instance of its own.
    let day = quatrelle_engine::model::dates::day_string(NOW.div_euclid(86_400));
    a.run("event.create", json!({"summary": "Daily check", "start": format!("{day}T15:00"), "end": format!("{day}T15:30"), "repeat": "daily"}));
    frame(&ctx, &mut a, vec![]);
    let check = agenda(&a).into_iter().find(|i| i.summary == "Daily check" && i.start >= NOW - 86_400).unwrap();
    assert!(check.recurring);
    let from = chip(&ctx, &check).center();
    frame(&ctx, &mut a, vec![Event::PointerMoved(from)]);
    frame(&ctx, &mut a, vec![press(from, true)]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(from + egui::vec2(0.0, 14.0))]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(from + egui::vec2(0.0, HOUR))]);
    frame(&ctx, &mut a, vec![press(from + egui::vec2(0.0, HOUR), false)]);
    frame(&ctx, &mut a, vec![]);
    let params = match a.ui.dialog.as_mut() {
        Some(Dialog::MoveEvent(f)) => {
            assert!(f.recurring && f.occurrence_allowed && !f.meeting);
            f.this_occurrence = Some(true);
            f.params(true)
        }
        other => panic!("asked nothing: {other:?}"),
    };
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Submit("event.move", params));
    assert!(a.ui.dialog.is_none());
    let instances =
        a.session.pim["sample"].events.iter().filter(|e| e.event.uid == check.uid && e.event.recurrence_id == Some(check.occurrence)).count();
    assert_eq!(instances, 1, "an instance of its own");
    assert!(agenda(&a).iter().any(|i| i.summary == "Daily check" && i.start == check.start + 3600));
    // Someone else's meeting can't be taken hold of.
    a.run("calendar.goto", json!({"step": 1}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let mix = agenda(&a).into_iter().find(|i| i.summary == "Sound mix playback").unwrap();
    assert!(!mix.movable);
    let from = chip(&ctx, &mix).center();
    frame(&ctx, &mut a, vec![Event::PointerMoved(from)]);
    frame(&ctx, &mut a, vec![press(from, true)]);
    frame(&ctx, &mut a, vec![Event::PointerMoved(from + egui::vec2(0.0, HOUR))]);
    assert!(a.ui.cal_drag.is_none());
    frame(&ctx, &mut a, vec![press(from + egui::vec2(0.0, HOUR), false)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.find_event(&mix.uid).unwrap().1.event.start, mix.start);
    let _ = std::fs::remove_dir_all(dir);
}
