//! A meeting's Send and its lines (`app/sending.rs`): Send waits for the names they're still
//! checking ("Checking names…", Send off meanwhile), stops in the meeting's window on an entry it
//! can't invite or book, asks there which person a name means, expands a contact group first, and
//! goes at once with addresses alone; a room taken out of the guests is never lost.

use egui::accesskit::Role;
use quatrelle_engine::model::{Addressee, AddresseeKind};

use super::meeting::{LAB, SIZE, booked, form, frames, open_meeting, type_in};
use super::message::text_with;
use super::*;
use crate::dialogs::people_field::{MEETING_WINDOWS, meeting_field};
use crate::pim_dialogs::{Line, line_button_id};

const CHOOSE: &str = "More than one person fits “Dupont”. Choose who you meant:";

/// What the meeting called `summary` invites once it's in the calendar: each attendee's address,
/// and whether it's a resource (a room booked).
fn made(a: &MailApp, summary: &str) -> Option<Vec<(String, bool)>> {
    let it = a.session.agenda(NOW - 86_400, NOW + 7 * 86_400).into_iter().find(|i| i.summary == summary)?;
    let e = &a.session.find_event(&it.uid)?.1.event;
    Some(e.attendees.iter().map(|x| (x.email.clone(), x.is_resource())).collect())
}

/// The guests (not the rooms) of what was made.
fn guests_of(made: &[(String, bool)]) -> Vec<&str> {
    made.iter().filter(|(_, room)| !room).map(|(e, _)| e.as_str()).collect()
}

/// The meeting's `label` button (Send, Send Update) pressed.
fn press(ctx: &egui::Context, a: &mut MailApp, label: &str) {
    let at = settled_text_at(ctx, a, label, SIZE).expect("the meeting's Send");
    click_at(ctx, a, at, 1, SIZE);
}

/// Whether the meeting's Send can be pressed, as a screen reader is told.
fn send_enabled(ctx: &egui::Context, a: &mut MailApp) -> Option<bool> {
    ctx.enable_accesskit();
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    nodes.iter().find(|(_, n)| n.role() == Role::Button && n.label() == Some("Send")).map(|(_, n)| !n.is_disabled())
}

/// A name left in Required: Send holds the meeting while the directory is asked, says so in the
/// window with Send off, and sends it once the name is found.
#[test]
fn a_meeting_waits_for_a_name_then_is_sent() {
    let (mut a, dir) = app("meeting-send-waits");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    let w = open_meeting(&ctx, &mut a, "Dupont", false);
    assert_eq!(send_enabled(&ctx, &mut a), Some(true));
    press(&ctx, &mut a, "Send");
    assert!(a.session.directory_searching(&gal, "dupont"), "asked of the directory");
    assert!(made(&a, "Foley review").is_none(), "nothing created yet");
    assert!(a.ui.people.holding(MEETING_WINDOWS | w));
    assert!(settled_text_at(&ctx, &mut a, "Checking names…", SIZE).is_some(), "said in the window");
    assert_eq!(send_enabled(&ctx, &mut a), Some(false), "Send is off meanwhile");
    directory_answers(&mut a, &gal, "dupont", &[("Anne Dupont", "anne.dupont@contoso.example")]);
    frames(&ctx, &mut a, 3);
    let made = made(&a, "Foley review").expect("created once the name was found");
    assert_eq!(guests_of(&made), ["anne.dupont@contoso.example"]);
    assert!(a.ui.event_windows.is_empty(), "sent, and the window closed");
    assert!(!a.ui.people.holding(MEETING_WINDOWS | w));
    let _ = std::fs::remove_dir_all(dir);
}

/// A name typed into a line while "Checking names…" is up, not closed: once the names waited on
/// are found, it's checked in turn, never sent as typed.
#[test]
fn a_name_typed_while_the_names_are_checked_is_checked_too() {
    let (mut a, dir) = app("meeting-typed-while-held");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    let w = open_meeting(&ctx, &mut a, "Dupont", false);
    press(&ctx, &mut a, "Send");
    assert!(a.ui.people.holding(MEETING_WINDOWS | w));
    let required = meeting_field(w, "required");
    type_in(&ctx, &mut a, required, "Zoe Park");
    assert_eq!(a.ui.people.typing(required), "Zoe Park", "typed, not closed");
    assert!(a.ui.people.holding(MEETING_WINDOWS | w), "still held");
    directory_answers(&mut a, &gal, "dupont", &[("Anne Dupont", "anne.dupont@contoso.example")]);
    frames(&ctx, &mut a, 3);
    if a.session.directory_searching(&gal, "zoe park") {
        directory_answers(&mut a, &gal, "zoe park", &[("Zoe Park", "zoe@studio.example")]);
        frames(&ctx, &mut a, 3);
    }
    assert_eq!(form_error(&a), None, "nothing refused");
    let made = made(&a, "Foley review").expect("sent once both are known");
    assert_eq!(guests_of(&made), ["anne.dupont@contoso.example", "zoe@studio.example"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// The window's error, while it's open.
fn form_error(a: &MailApp) -> Option<String> {
    a.ui.event_windows.first().and_then(|(_, f)| f.error.clone())
}

/// A name the directory doesn't know stops the meeting's Send, said in its own window.
#[test]
fn a_meeting_stops_on_a_name_not_found_and_says_why_in_its_window() {
    let (mut a, dir) = app("meeting-send-not-found");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    let w = open_meeting(&ctx, &mut a, "Dupont", false);
    press(&ctx, &mut a, "Send");
    directory_answers(&mut a, &gal, "dupont", &[]);
    frames(&ctx, &mut a, 3);
    assert!(made(&a, "Foley review").is_none(), "not sent");
    assert_eq!(form(&a).error.as_deref(), Some("Not found: Dupont"));
    assert!(!a.ui.people.holding(MEETING_WINDOWS | w), "the wait is over");
    assert!(settled_text_at(&ctx, &mut a, "Not found: Dupont", SIZE).is_some(), "said in the window");
    assert_eq!(send_enabled(&ctx, &mut a), Some(true), "Send can be pressed again");
    let _ = std::fs::remove_dir_all(dir);
}

/// Two people fit the name: which one is asked over the meeting (not in the main window), and
/// the meeting goes with the one picked.
#[test]
fn an_ambiguous_name_asks_in_the_meeting_window_then_sends() {
    let (mut a, dir) = app("meeting-send-choice");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    open_meeting(&ctx, &mut a, "Dupont", false);
    press(&ctx, &mut a, "Send");
    directory_answers(&mut a, &gal, "dupont", &[("Anne Dupont", "anne@contoso.example"), ("Marc Dupont", "marc@contoso.example")]);
    assert!(settled_text_at(&ctx, &mut a, CHOOSE, SIZE).is_some(), "asked before it goes");
    assert!(a.ui.dialog.is_none(), "over the meeting, not a dialog of the main window");
    assert!(made(&a, "Foley review").is_none(), "held for the choice");
    let marc = settled_text_at(&ctx, &mut a, "Marc Dupont", SIZE).expect("Marc offered");
    click_at(&ctx, &mut a, marc, 2, SIZE);
    frames(&ctx, &mut a, 3);
    let made = made(&a, "Foley review").expect("sent once the choice was made");
    assert_eq!(guests_of(&made), ["marc@contoso.example"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A contact group has no address: the meeting goes to its members, asked for at Send.
#[test]
fn a_contact_group_is_expanded_before_the_meeting_is_sent() {
    let (mut a, dir) = app("meeting-send-group");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    open_meeting(&ctx, &mut a, "Team <group:AAMk=>", false);
    press(&ctx, &mut a, "Send");
    assert!(made(&a, "Foley review").is_none(), "held for the members");
    let members = vec![
        Addressee::new("Anne Dupont", "anne@contoso.example", AddresseeKind::Person),
        Addressee::new("Marc Dupont", "marc@contoso.example", AddresseeKind::Person),
    ];
    a.session.apply(MailEvent::ListExpanded {
        account: gal.clone(),
        address: String::new(),
        item_id: Some("AAMk=".into()),
        members,
        more: false,
        total: 2,
        error: None,
    });
    frames(&ctx, &mut a, 3);
    let made = made(&a, "Foley review").expect("sent once the members came");
    assert_eq!(guests_of(&made), ["anne@contoso.example", "marc@contoso.example"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A person picked by name into Rooms is no room: Send stops on it, in the window.
#[test]
fn a_refused_room_stops_the_send() {
    let (mut a, dir) = app("meeting-send-refused-room");
    let ctx = egui::Context::default();
    let w = open_meeting(&ctx, &mut a, "sam@studio.example", false);
    a.ui.people.add(meeting_field(w, "rooms"), Addressee::new("Zoe Park", "zoe@studio.example", AddresseeKind::Person));
    frames(&ctx, &mut a, 3);
    assert!(form(&a).rooms_line.contains("zoe@studio.example"), "{}", form(&a).rooms_line);
    press(&ctx, &mut a, "Send");
    assert!(made(&a, "Foley review").is_none(), "not sent");
    assert_eq!(form(&a).error.as_deref(), Some("This isn't a room"));
    assert_eq!(a.ui.event_windows.len(), 1, "the window stays");
    let _ = std::fs::remove_dir_all(dir);
}

/// A meeting sent before, opened again: its attendees are a shared mailbox, an address nothing
/// knows and a room booked by its address. Send Update with nothing changed goes at once, the
/// directory not asked, with the same attendees and rooms.
#[test]
fn a_meeting_reopened_sends_its_update_as_before() {
    let (mut a, dir) = app("meeting-send-update");
    let gal = with_exchange(&mut a);
    for acc in a.session.accounts.iter_mut().filter(|x| x.config.id == "sample") {
        acc.config.shared_mailboxes.push("support@studio.example".into());
    }
    let day = quatrelle_engine::model::dates::day_string(NOW.div_euclid(86_400) + 1);
    let p = json!({
        "summary": "Mix review",
        "start": format!("{day}T10:00"),
        "end": format!("{day}T11:00"),
        "attendees": "support@studio.example, Ola Berg <ola@elsewhere.example>",
        "rooms": "beamer@studio.example",
    });
    a.session.run("event.create", &p).unwrap();
    let before = made(&a, "Mix review").unwrap();
    assert!(before.contains(&("beamer@studio.example".to_string(), true)), "{before:?}");
    let it = a.session.agenda(NOW, NOW + 3 * 86_400).into_iter().find(|i| i.summary == "Mix review").unwrap();
    let ctx = egui::Context::default();
    frame_sized(&ctx, &mut a, vec![], SIZE);
    a.ui.dialog = Dialog::open_event(&a.session, &it.uid, it.occurrence);
    frames(&ctx, &mut a, 3);
    assert!(form(&a).can_invite && form(&a).inviting, "the organizer's meeting");
    press(&ctx, &mut a, "Send Update");
    assert!(a.ui.event_windows.is_empty(), "updated at once, and closed");
    assert!(!a.session.directory_searching(&gal, "support@studio.example") && !a.session.directory_searching(&gal, "ola@elsewhere.example"));
    let mut after = made(&a, "Mix review").unwrap();
    let mut before = before;
    after.sort();
    before.sort();
    assert_eq!(after, before);
    let _ = std::fs::remove_dir_all(dir);
}

/// No directory anywhere: a meeting to addresses alone needs none, and goes at once.
#[test]
fn addresses_alone_send_a_meeting_offline() {
    let (mut a, dir) = app("meeting-send-offline");
    let ctx = egui::Context::default();
    open_meeting(&ctx, &mut a, "zoe@studio.example; sam@studio.example", false);
    assert!(a.session.directory_account(Some("sample")).is_none(), "no directory");
    press(&ctx, &mut a, "Send");
    let made = made(&a, "Foley review").expect("created at once");
    assert_eq!(guests_of(&made), ["zoe@studio.example", "sam@studio.example"]);
    assert!(a.ui.event_windows.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// The meeting books 50 rooms at most: a room typed among the guests past that stays among them,
/// saying why, rather than being taken out for Rooms to drop; Send stops on it.
#[test]
fn a_room_that_cant_be_booked_stays_among_the_guests() {
    let (mut a, dir) = app("meeting-room-not-booked");
    let ctx = egui::Context::default();
    let w = open_meeting(&ctx, &mut a, "zoe@studio.example", false);
    if let Some((_, f)) = a.ui.event_windows.first_mut() {
        for i in 0..50 {
            f.add_room("", &format!("room{i}@studio.example"));
        }
    }
    frames(&ctx, &mut a, 3);
    assert_eq!(booked(&a).len(), 50);
    let required = meeting_field(w, "required");
    type_in(&ctx, &mut a, required, "Sound Lab <sound.lab@studio.example>;");
    frames(&ctx, &mut a, 3);
    let entry = "Sound Lab <sound.lab@studio.example>";
    assert!(form(&a).guests.contains(LAB), "kept among the guests: {}", form(&a).guests);
    assert!(!booked(&a).iter().any(|e| e == LAB) && booked(&a).len() == 50);
    let why = "Sound Lab <sound.lab@studio.example> can't be booked: a meeting books 50 rooms at most";
    assert!(a.ui.people.pieces(required).iter().any(|p| text_with(p, entry, Some(PieceError::RoomsFull(50)))), "{:?}", a.ui.people.pieces(required));
    assert_ne!(a.ui.people.note(required), Some("Sound Lab was moved to Rooms"));
    frames(&ctx, &mut a, 3);
    assert!(form(&a).guests.contains(LAB), "still there a few frames on: {}", form(&a).guests);
    press(&ctx, &mut a, "Send");
    assert!(made(&a, "Foley review").is_none(), "not sent");
    assert_eq!(form(&a).error.as_deref(), Some(why));
    let _ = std::fs::remove_dir_all(dir);
}

/// The room kept among the guests while the meeting booked 50 is checked again at Send once one
/// of them is freed: it's moved into Rooms, booked, and the meeting goes.
#[test]
fn a_room_kept_among_the_guests_is_booked_once_a_room_is_freed() {
    let (mut a, dir) = app("meeting-room-freed");
    let ctx = egui::Context::default();
    let w = open_meeting(&ctx, &mut a, "zoe@studio.example", false);
    if let Some((_, f)) = a.ui.event_windows.first_mut() {
        for i in 0..50 {
            f.add_room("", &format!("room{i}@studio.example"));
        }
    }
    frames(&ctx, &mut a, 3);
    let required = meeting_field(w, "required");
    type_in(&ctx, &mut a, required, "Sound Lab <sound.lab@studio.example>;");
    frames(&ctx, &mut a, 3);
    let entry = "Sound Lab <sound.lab@studio.example>";
    assert!(a.ui.people.pieces(required).iter().any(|p| text_with(p, entry, Some(PieceError::RoomsFull(50)))), "{:?}", a.ui.people.pieces(required));
    if let Some((_, f)) = a.ui.event_windows.first_mut() {
        f.remove_room("room0@studio.example");
    }
    frames(&ctx, &mut a, 3);
    assert_eq!(booked(&a).len(), 49, "a room freed");
    press(&ctx, &mut a, "Send");
    frames(&ctx, &mut a, 3);
    let made = made(&a, "Foley review").expect("sent, the room booked");
    assert!(made.contains(&(LAB.to_string(), true)), "booked: {made:?}");
    assert_eq!(guests_of(&made), ["zoe@studio.example"], "not invited as a guest");
    let _ = std::fs::remove_dir_all(dir);
}

/// The same room typed but not closed when Send is pressed: the check closes it, takes it out of
/// the guests for Rooms, which can't book it, and the send stops on it rather than inviting a
/// room as a guest.
#[test]
fn a_room_left_typed_past_the_rooms_limit_stops_the_send() {
    let (mut a, dir) = app("meeting-room-typed-not-booked");
    let ctx = egui::Context::default();
    let w = open_meeting(&ctx, &mut a, "zoe@studio.example", false);
    if let Some((_, f)) = a.ui.event_windows.first_mut() {
        for i in 0..50 {
            f.add_room("", &format!("room{i}@studio.example"));
        }
    }
    frames(&ctx, &mut a, 3);
    type_in(&ctx, &mut a, meeting_field(w, "required"), "Sound Lab <sound.lab@studio.example>");
    let p = form(&a).params();
    a.act(crate::Action::ForEvent(w, "event.create", p));
    frames(&ctx, &mut a, 2);
    assert!(made(&a, "Foley review").is_none(), "not sent");
    assert_eq!(form(&a).error.as_deref(), Some("Sound Lab <sound.lab@studio.example> can't be booked: a meeting books 50 rooms at most"));
    assert!(form(&a).guests.contains(LAB), "kept among the guests: {}", form(&a).guests);
    let _ = std::fs::remove_dir_all(dir);
}

/// In Select Attendees and Resources, a room typed in the Required row goes to the Resources row
/// (said under Required), and OK books it.
#[test]
fn a_room_typed_in_the_attendees_rows_moves_to_resources() {
    let (mut a, dir) = app("meeting-attendees-room-moved");
    let ctx = egui::Context::default();
    let w = open_meeting(&ctx, &mut a, "sam@studio.example", false);
    let at = ctx.read_response(line_button_id(w, Line::Required)).map(|r| r.rect.center()).expect("the Required button");
    click_at(&ctx, &mut a, at, 1, SIZE);
    frames(&ctx, &mut a, 3);
    let (att_required, att_rooms) = (meeting_field(w, "att-required"), meeting_field(w, "att-rooms"));
    type_in(&ctx, &mut a, att_required, "Sound Lab <sound.lab@studio.example>;");
    frames(&ctx, &mut a, 3);
    let has_lab = |a: &MailApp, field| a.ui.people.pieces(field).iter().any(|p| matches!(p, Piece::Chip { who, .. } if who.address == LAB));
    assert!(!has_lab(&a, att_required), "{:?}", a.ui.people.pieces(att_required));
    assert!(has_lab(&a, att_rooms), "{:?}", a.ui.people.pieces(att_rooms));
    assert_eq!(a.ui.people.note(att_required), Some("Sound Lab was moved to Rooms"));
    let ok = settled_text_at(&ctx, &mut a, "OK", SIZE).expect("OK");
    click_at(&ctx, &mut a, ok, 1, SIZE);
    assert!(form(&a).attendees_book.is_none(), "closed");
    assert_eq!(booked(&a), [LAB]);
    assert!(form(&a).guests.contains("sam@studio.example") && !form(&a).guests.contains(LAB), "{}", form(&a).guests);
    let _ = std::fs::remove_dir_all(dir);
}

/// The Scheduling Assistant doesn't draw the lines, so the form's text may still hold a room
/// just taken out of the guests: Send there sends what the lines hold, the room booked and no
/// guest.
#[test]
fn a_meeting_sent_from_the_scheduling_assistant_sends_the_lines_as_they_are() {
    let (mut a, dir) = app("meeting-send-assistant");
    let ctx = egui::Context::default();
    frame_sized(&ctx, &mut a, vec![], SIZE);
    let mut d = Dialog::new_event(&a.session, Some(NOW + 2 * 3600));
    if let Dialog::Event(f) = &mut d {
        f.summary = "Foley review".into();
        f.inviting = true;
        f.guests = "zoe@studio.example; Sound Lab <sound.lab@studio.example>".into();
        f.scheduling = true;
    }
    a.ui.dialog = Some(d);
    frames(&ctx, &mut a, 3);
    assert_eq!(booked(&a), [LAB], "taken out of the guests for Rooms");
    press(&ctx, &mut a, "Send");
    let made = made(&a, "Foley review").expect("sent");
    assert_eq!(guests_of(&made), ["zoe@studio.example"], "{made:?}");
    assert!(made.contains(&(LAB.to_string(), true)), "{made:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// An appointment books no rooms through the Room Finder until it's asked for, nor invites: its
/// window asks for no room list (on Exchange, a request to the server); a meeting's does, and so
/// does an appointment's Select Attendees and Resources (its Resources row takes rooms).
#[test]
fn an_appointment_asks_for_no_rooms() {
    let (mut a, dir) = app("appointment-no-rooms");
    let ctx = egui::Context::default();
    frame_sized(&ctx, &mut a, vec![], SIZE);
    a.ui.dialog = Some(Dialog::new_event(&a.session, Some(NOW + 2 * 3600)));
    frames(&ctx, &mut a, 3);
    assert!(!form(&a).inviting && form(&a).can_invite);
    assert!(a.session.rooms_now("sample").is_none(), "an appointment asks for no rooms");
    a.ui.event_windows[0].1.inviting = true;
    frames(&ctx, &mut a, 2);
    assert!(a.session.rooms_now("sample").is_some(), "a meeting does");
    let _ = std::fs::remove_dir_all(dir);
    // Another appointment, its Select Attendees opened.
    let (mut a, dir) = app("appointment-attendees-rooms");
    let ctx = egui::Context::default();
    frame_sized(&ctx, &mut a, vec![], SIZE);
    a.ui.dialog = Some(Dialog::new_event(&a.session, Some(NOW + 2 * 3600)));
    frames(&ctx, &mut a, 3);
    assert!(a.session.rooms_now("sample").is_none(), "an appointment asks for no rooms");
    a.ui.event_windows[0].1.select_attendees();
    frames(&ctx, &mut a, 2);
    assert!(!form(&a).inviting && form(&a).attendees_book.is_some(), "still an appointment, the book open");
    assert!(a.session.rooms_now("sample").is_some(), "its Select Attendees does");
    let _ = std::fs::remove_dir_all(dir);
}
