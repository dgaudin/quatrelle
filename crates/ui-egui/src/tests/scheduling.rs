//! UI tests: the Room Finder and the Scheduling Assistant in the event form.

use super::*;

/// A new meeting's Scheduling Assistant: a row for the organizer, each guest and each room,
/// their hours read from the calendars here (the sample's rooms and a colleague's), and a click
/// on the timeline moving the meeting (its fields follow). Saved, the meeting is held in the
/// room, which is a resource; opened again, the room is back in its row, not among the guests.
#[test]
fn the_scheduling_assistant_shows_everyone_and_moves_the_meeting() {
    let (mut a, dir) = app("scheduling");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 900.0);
    frame_sized(&ctx, &mut a, vec![], size);
    let start = NOW + 2 * 3600;
    let mut d = Dialog::new_event(&a.session, Some(start));
    if let Dialog::Event(f) = &mut d {
        f.summary = "Foley review".into();
        f.guests = "Zoe Park <zoe@studio.example>, sam@studio.example".into();
        f.add_room("Sound Lab", "sound.lab@studio.example");
        f.scheduling = true;
        assert_eq!(f.location, "Sound Lab", "the place follows the room");
    }
    a.ui.dialog = Some(d);
    // The first frame asks for everyone's hours and places the dialog; the next draw them
    // where it settled (a response read back is the frame before's).
    for _ in 0..3 {
        frame_sized(&ctx, &mut a, vec![], size);
    }
    let day_from = NOW - NOW.rem_euclid(86_400);
    for who in ["riley@studio.example", "zoe@studio.example", "sam@studio.example", "sound.lab@studio.example"] {
        let known = a.session.availability_now("sample", who, day_from, day_from + 86_400);
        assert!(matches!(known, Some(quatrelle_engine::Fetched::Ready(_))), "{who}: {known:?}");
    }
    let room = a.session.availability_now("sample", "sound.lab@studio.example", day_from, day_from + 86_400);
    assert!(matches!(room, Some(quatrelle_engine::Fetched::Ready(x)) if x.slots.iter().any(|s| s.subject == "Foley session")));
    // The timeline: a click at nine o'clock puts the meeting there, as long as it was.
    let rect = ctx
        .read_response(crate::pim_dialogs::scheduling::timeline_id(a.ui.event_windows.first().map_or(0, |(w, _)| *w)))
        .map(|r| r.rect)
        .expect("the timeline is drawn");
    let (h0, h1) = crate::pim_dialogs::scheduling::hours_shown(start, start + 3600, day_from);
    let at = egui::pos2(rect.min.x + rect.width() * ((9 - h0) as f32 / (h1 - h0) as f32) + 1.0, rect.center().y);
    let press = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame_sized(&ctx, &mut a, vec![Event::PointerMoved(at)], size);
    frame_sized(&ctx, &mut a, vec![press(at, true)], size);
    frame_sized(&ctx, &mut a, vec![press(at, false)], size);
    frame_sized(&ctx, &mut a, vec![], size);
    let (s0, e0) = match a.ui.event_windows.first() {
        Some((_, f)) => (f.start.clone(), f.end.clone()),
        None => panic!("no event window"),
    };
    assert_eq!((s0.as_str(), e0.as_str()), ("09:00", "10:00"));
    // Another day, and back.
    if let Some((_, f)) = a.ui.event_windows.first_mut() {
        f.sched_day = Some(day_from.div_euclid(86_400) + 1);
    }
    frame_sized(&ctx, &mut a, vec![], size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert!(
        a.session.availability_now("sample", "riley@studio.example", day_from + 86_400, day_from + 2 * 86_400).is_some(),
        "the next day asked for"
    );
    if let Some((_, f)) = a.ui.event_windows.first_mut() {
        f.sched_day = None;
    }
    // Saved: held in the room, which is a resource.
    let params = match a.ui.event_windows.first() {
        Some((_, f)) => f.params(),
        None => panic!("no event window"),
    };
    assert_eq!(params["rooms"], "Sound Lab <sound.lab@studio.example>");
    let w = a.ui.event_windows.first().map_or(0, |(w, _)| *w);
    a.act(crate::Action::ForEvent(w, "event.create", params));
    assert!(a.ui.event_windows.is_empty(), "saved and closed");
    let made = a.session.agenda(day_from, day_from + 86_400).into_iter().find(|i| i.summary == "Foley review").unwrap();
    let e = a.session.find_event(&made.uid).unwrap().1.event.clone();
    assert!(e.attendees.iter().any(|x| x.is_resource() && x.email == "sound.lab@studio.example"), "{:?}", e.attendees);
    assert_eq!((made.location.as_str(), made.start - day_from), ("Sound Lab", 9 * 3600));
    let d = Dialog::open_event(&a.session, &made.uid, made.occurrence).unwrap();
    if let Dialog::Event(f) = &d {
        assert_eq!(f.rooms, [("Sound Lab".to_string(), "sound.lab@studio.example".to_string())]);
        assert!(!f.guests.contains("sound.lab") && f.guests.contains("zoe@studio.example"), "{}", f.guests);
    }
    a.ui.dialog = Some(d);
    if let Some((_, f)) = a.ui.event_windows.first_mut() {
        f.scheduling = true;
    }
    frame_sized(&ctx, &mut a, vec![], size);
    let _ = std::fs::remove_dir_all(dir);
}

/// The Room Finder: opened from its row, it offers the sample's rooms (those its calendar
/// knows), a click books one, another frees it, a room no list offers goes in by its address,
/// and the place follows the rooms until it's typed by hand. Loading and refused states draw.
#[test]
fn the_room_finder_offers_and_books_rooms() {
    let (mut a, dir) = app("room-finder");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut d = Dialog::new_event(&a.session, Some(NOW + 3600));
    if let Dialog::Event(f) = &mut d {
        f.finder = Some(crate::pim_dialogs::RoomFinderUi::default());
    }
    a.ui.dialog = Some(d);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let rooms = a.session.rooms_now("sample").cloned();
    assert!(matches!(&rooms, Some(quatrelle_engine::Fetched::Ready(f)) if f.rooms.len() == 2 && f.from_calendar), "{rooms:?}");
    if let Some((_, f)) = a.ui.event_windows.first_mut() {
        if let Some(st) = f.finder.as_mut() {
            st.filter = "sound".into();
        }
        f.add_room("Sound Lab", "sound.lab@studio.example");
        f.add_room("Sound Lab", "sound.lab@studio.example");
        assert_eq!(f.rooms.len(), 1, "once");
        assert_eq!(f.location, "Sound Lab");
        f.add_room("", "projector@studio.example");
        assert_eq!(f.location, "Sound Lab; projector@studio.example", "the place follows the rooms");
        f.location = "Online".into();
        f.remove_room("sound.lab@studio.example");
        assert_eq!((f.rooms.len(), f.location.as_str()), (1, "Online"), "a place typed by hand stays");
        f.add_room("Sound Lab", "not an address");
        assert_eq!(f.rooms.len(), 1);
    }
    frame(&ctx, &mut a, vec![]);
    // Still loading, and refused: drawn too.
    a.session.rooms.insert("sample".into(), quatrelle_engine::Fetched::Loading);
    frame(&ctx, &mut a, vec![]);
    a.session.rooms.insert("sample".into(), quatrelle_engine::Fetched::Failed("no directory".into()));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.dialog.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// The hours shown: a working day, stretched to hold a meeting outside it, and left alone for a
/// meeting on another day.
#[test]
fn the_hours_shown_hold_the_meeting() {
    use crate::pim_dialogs::scheduling::hours_shown;
    let day = 1_000_000 * 86_400;
    assert_eq!(hours_shown(day + 9 * 3600, day + 10 * 3600, day), (7, 20));
    assert_eq!(hours_shown(day + 6 * 3600, day + 22 * 3600 + 1800, day), (6, 23));
    assert_eq!(hours_shown(day + 86_400 + 3600, day + 86_400 + 7200, day), (7, 20), "another day's meeting");
    assert_eq!(hours_shown(day, day + 86_400, day), (0, 24));
}

/// An appointment opens in a window of its own (Outlook's), the main window free meanwhile:
/// Invite Attendees makes it a meeting, with Required and Optional lines; Show As, Private and
/// the Response Options go with it when it's sent; the window closes once the engine has it.
/// Opened again, it says what was sent; two can be open at once, each with its own Scheduling
/// Assistant.
#[test]
fn a_meeting_is_written_in_a_window_of_its_own() {
    use quatrelle_engine::model::ical::ShowAs;
    let (mut a, dir) = app("meeting-window");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 900.0);
    frame_sized(&ctx, &mut a, vec![], size);
    a.ui.dialog = Some(Dialog::new_event(&a.session, Some(NOW + 2 * 3600)));
    frame_sized(&ctx, &mut a, vec![], size);
    assert!(a.ui.dialog.is_none(), "not a dialog");
    assert_eq!(a.ui.event_windows.len(), 1, "a window of its own");
    let w = a.ui.event_windows[0].0;
    {
        let f = &mut a.ui.event_windows[0].1;
        assert!(!f.inviting && f.responses && f.new_times, "an appointment, answers asked once it's a meeting");
        f.summary = "Budget review".into();
        f.inviting = true;
        f.guests = "Zoe Park <zoe@studio.example>".into();
        f.optional = "sam@studio.example".into();
        f.show_as = ShowAs::OutOfOffice;
        f.private = true;
        f.new_times = false;
    }
    // Both tabs draw, the Scheduling Assistant with everyone.
    frame_sized(&ctx, &mut a, vec![], size);
    a.ui.event_windows[0].1.scheduling = true;
    frame_sized(&ctx, &mut a, vec![], size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert!(ctx.read_response(crate::pim_dialogs::scheduling::timeline_id(w)).is_some(), "its own timeline");
    a.ui.event_windows[0].1.scheduling = false;
    // A second one at the same time: its own window, its own timeline.
    a.ui.dialog = Some(Dialog::new_event(&a.session, Some(NOW + 5 * 3600)));
    frame_sized(&ctx, &mut a, vec![], size);
    assert_eq!(a.ui.event_windows.len(), 2);
    let second = a.ui.event_windows[1].0;
    a.ui.event_windows[1].1.scheduling = true;
    a.ui.event_windows[0].1.scheduling = true;
    frame_sized(&ctx, &mut a, vec![], size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert!(ctx.read_response(crate::pim_dialogs::scheduling::timeline_id(second)).is_some());
    a.ui.event_windows[0].1.scheduling = false;
    // Sent: kept by the engine, and the window goes (the other stays).
    let params = a.ui.event_windows[0].1.params();
    assert_eq!(
        (params["showAs"].as_str(), params["private"].as_bool(), params["newTimes"].as_bool()),
        (Some("outOfOffice"), Some(true), Some(false))
    );
    a.act(crate::Action::ForEvent(w, "event.create", params));
    assert_eq!(a.ui.event_windows.len(), 1, "sent and closed; the other stays");
    let made = a.session.agenda(NOW, NOW + 86_400).into_iter().find(|i| i.summary == "Budget review").unwrap();
    let e = a.session.find_event(&made.uid).unwrap().1.event.clone();
    assert_eq!((e.show_as, e.private, e.no_new_times), (ShowAs::OutOfOffice, true, true));
    assert!(e.attendees.iter().any(|x| x.is_optional() && x.email == "sam@studio.example"), "{:?}", e.attendees);
    // A bad time keeps the window, with the reason.
    a.ui.event_windows[0].1.summary = "Mix".into();
    a.ui.event_windows[0].1.start = "25:00".into();
    let bad = a.ui.event_windows[0].1.params();
    a.act(crate::Action::ForEvent(second, "event.create", bad));
    assert!(a.ui.event_windows[0].1.error.is_some(), "the reason, in its window");
    // Opened again: what was sent.
    a.ui.dialog = Dialog::open_event(&a.session, &made.uid, made.occurrence);
    frame_sized(&ctx, &mut a, vec![], size);
    let f = &a.ui.event_windows.last().unwrap().1;
    assert!(f.inviting && f.private && !f.new_times && f.show_as == ShowAs::OutOfOffice);
    assert!(f.optional.contains("sam@studio.example") && !f.guests.contains("sam@"), "{} / {}", f.guests, f.optional);
    frame_sized(&ctx, &mut a, vec![], size);
    let _ = std::fs::remove_dir_all(dir);
}
