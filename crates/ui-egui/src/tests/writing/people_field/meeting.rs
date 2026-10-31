//! The contacts field in a meeting's window (`pim_dialogs/meeting.rs`): its Required, Optional and
//! Rooms lines, a room typed among the guests moved to Rooms, the Room Finder and the Rooms line
//! kept in step, and the Select Attendees and Resources dialog's rows.

use quatrelle_engine::FieldId;
use quatrelle_engine::model::{Addressee, AddresseeKind};

use super::message::{chip_named, focus_field, text_with};
use super::*;
use crate::dialogs::people_field::{accepts_of, egui_id, meeting_field};
use crate::pim_dialogs::{EventForm, Line, line_button_id};

pub(super) const SIZE: egui::Vec2 = egui::vec2(1280.0, 900.0);

pub(super) const LAB: &str = "sound.lab@studio.example";

pub(super) fn frames(ctx: &egui::Context, a: &mut MailApp, n: usize) {
    for _ in 0..n {
        frame_sized(ctx, a, vec![], SIZE);
    }
}

/// A new meeting with `guests` in Required, open in its window (`finder`: its Room Finder open);
/// the window's number.
pub(super) fn open_meeting(ctx: &egui::Context, a: &mut MailApp, guests: &str, finder: bool) -> u64 {
    frame_sized(ctx, a, vec![], SIZE);
    let mut d = Dialog::new_event(&a.session, Some(NOW + 2 * 3600));
    if let Dialog::Event(f) = &mut d {
        f.summary = "Foley review".into();
        f.inviting = true;
        f.guests = guests.into();
        if finder {
            f.finder = Some(crate::pim_dialogs::RoomFinderUi::default());
        }
    }
    a.ui.dialog = Some(d);
    frames(ctx, a, 3);
    a.ui.event_windows.first().map_or(0, |(w, _)| *w)
}

pub(super) fn form(a: &MailApp) -> &EventForm {
    &a.ui.event_windows[0].1
}

/// What the meeting books: its rooms' addresses.
pub(super) fn booked(a: &MailApp) -> Vec<String> {
    form(a).rooms.iter().map(|(_, e)| e.clone()).collect()
}

/// Type `text` in the field, its input given the keyboard first; a few frames after.
pub(super) fn type_in(ctx: &egui::Context, a: &mut MailApp, field: FieldId, text: &str) {
    focus_field(ctx, field);
    frame_sized(ctx, a, vec![], SIZE);
    frame_sized(ctx, a, vec![Event::Text(text.into())], SIZE);
    frames(ctx, a, 3);
}

/// Why the field's chip of `address` is refused (`Some(None)`: taken), if the field holds one.
fn refusal(a: &MailApp, field: FieldId, address: &str) -> Option<Option<String>> {
    a.ui.people.pieces(field).iter().find_map(|p| match p {
        Piece::Chip { who, refused } if who.address == address => Some(refused.clone()),
        _ => None,
    })
}

/// Whether the field's input stays where it is from a frame to the next: a line laid out wider
/// than its row grows the window it's in by as much every frame (an embedded window, as in the
/// browser).
fn steady(ctx: &egui::Context, a: &mut MailApp, field: FieldId) -> bool {
    let rect = |ctx: &egui::Context| ctx.read_response(input_id(egui_id(field))).map(|r| r.rect);
    frame_sized(ctx, a, vec![], SIZE);
    let before = rect(ctx);
    frame_sized(ctx, a, vec![], SIZE);
    before.is_some() && rect(ctx) == before
}

/// How far under its input the field's underline is drawn (the line under the field's lines,
/// from where its input starts when it holds nothing).
fn underline_gap(ctx: &egui::Context, a: &mut MailApp, field: FieldId) -> Option<f32> {
    let shapes = frame_shapes(ctx, a, vec![], SIZE);
    let input = ctx.read_response(input_id(egui_id(field))).map(|r| r.rect)?;
    let flat = |p: &[egui::Pos2; 2]| (p[0].y - p[1].y).abs() < 0.5 && (p[0].x - input.left()).abs() < 1.5 && p[0].y > input.top();
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::LineSegment { points, .. } if flat(points) => Some(points[0].y - input.bottom()),
            _ => None,
        })
        .min_by(f32::total_cmp)
}

/// A click on the field's chip `i` opens its card; its Remove takes the chip out.
fn remove_chip(ctx: &egui::Context, a: &mut MailApp, field: FieldId, i: usize) {
    let chip = ctx.read_response(egui_id(field).with(("people-chip", i))).map(|r| r.rect.center()).expect("the chip is drawn");
    click_at(ctx, a, chip, 1, SIZE);
    let remove = settled_text_at(ctx, a, "Remove", SIZE).expect("the chip's card offers Remove");
    click_at(ctx, a, remove, 1, SIZE);
}

/// A meeting's Required line is the contacts field: its guests are chips, a name typed is
/// suggested, and Ctrl+K checks the line in the meeting's window (`people.resolve` with its text),
/// never Check Names on a message written meanwhile.
#[test]
fn meeting_lines_are_contacts_fields() {
    let (mut a, dir) = app("meeting-lines");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame_sized(&ctx, &mut a, vec![], SIZE);
    // A message being written, a name left in its To: Check Names on it would mark it.
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "nobody here"}));
    let compose_to = FieldId { window: a.session.compose.as_ref().map_or(0, |c| c.id), name: "to" };
    let w = open_meeting(&ctx, &mut a, "Zoe Park <zoe@studio.example>, sam@studio.example", false);
    let required = meeting_field(w, "required");
    let p = a.ui.people.pieces(required).to_vec();
    assert!(p.len() == 2 && p.iter().all(|p| matches!(p, Piece::Chip { refused: None, .. })), "two chips: {p:?}");
    assert!(chip_named(&p[0], "Zoe Park"), "{p:?}");
    assert!(steady(&ctx, &mut a, required) && steady(&ctx, &mut a, meeting_field(w, "rooms")), "the lines fit their rows");
    let gap = underline_gap(&ctx, &mut a, meeting_field(w, "optional"));
    assert!(gap.is_some_and(|g| g <= 8.0), "the line's underline is under it, not in the next row: {gap:?}");
    // Typing suggests.
    type_in(&ctx, &mut a, required, "zo");
    assert!(a.ui.people.suggestions(required).iter().any(|s| s.who.address == "zoe@studio.example"), "{:?}", a.ui.people.suggestions(required));
    // Ctrl+K checks what's typed in the line itself.
    frame_sized(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE), key(Key::Backspace, Modifiers::NONE)], SIZE);
    frame_sized(&ctx, &mut a, vec![Event::Text("nobody here".into())], SIZE);
    frame_sized(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)], SIZE);
    frames(&ctx, &mut a, 3);
    let p = a.ui.people.pieces(required).to_vec();
    assert!(p.len() == 3 && text_with(&p[2], "nobody here", Some(PieceError::NotFound)), "checked in the line: {p:?}");
    assert!(form(&a).guests.contains("nobody here"), "{}", form(&a).guests);
    let left = a.ui.people.pieces(compose_to).to_vec();
    assert!(left.len() == 1 && text_with(&left[0], "nobody here", None), "the message's To wasn't checked: {left:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The lesson of the search bug of 2026-10-30, in a meeting: thirty guests are parsed once, not
/// on every frame, and once more when a guest is typed and closed.
#[test]
fn meeting_lines_parse_once_per_change_not_per_frame() {
    let (mut a, dir) = app("meeting-parse-once");
    let ctx = egui::Context::default();
    let many: Vec<String> = (0..30).map(|i| format!("Person {i} <p{i}@studio.example>")).collect();
    let w = open_meeting(&ctx, &mut a, &many.join("; "), false);
    let required = meeting_field(w, "required");
    frames(&ctx, &mut a, 10);
    assert_eq!(a.ui.people.pieces(required).len(), 30);
    assert_eq!(a.ui.people.syncs(required), 1, "idle frames parse nothing");
    type_in(&ctx, &mut a, required, "zoe@studio.example;");
    frames(&ctx, &mut a, 5);
    assert_eq!(a.ui.people.pieces(required).len(), 31, "{:?}", a.ui.people.pieces(required).last());
    assert_eq!(a.ui.people.syncs(required), 2, "parsed once more, for the guest closed");
    let _ = std::fs::remove_dir_all(dir);
}

/// A room typed among the guests is no guest: it leaves Required for Rooms (the window asked its
/// account's rooms, so it's known without the Room Finder), the place follows, and the line says
/// where it went.
#[test]
fn a_room_typed_among_the_guests_moves_to_rooms() {
    let (mut a, dir) = app("meeting-room-moved");
    let ctx = egui::Context::default();
    let w = open_meeting(&ctx, &mut a, "zoe@studio.example", false);
    assert!(a.session.rooms_now("sample").is_some(), "the meeting asked for its account's rooms");
    let required = meeting_field(w, "required");
    type_in(&ctx, &mut a, required, "Sound Lab <sound.lab@studio.example>;");
    frames(&ctx, &mut a, 3);
    let f = form(&a);
    assert!(f.guests.contains("zoe@studio.example") && !f.guests.contains("sound.lab"), "{}", f.guests);
    assert_eq!(f.rooms, [("Sound Lab".to_string(), LAB.to_string())]);
    assert_eq!(f.location, "Sound Lab", "the place follows the rooms");
    assert_eq!(a.ui.people.note(required), Some("Sound Lab was moved to Rooms"));
    assert!(settled_text_at(&ctx, &mut a, "Sound Lab was moved to Rooms", SIZE).is_some(), "said under the line");
    let rooms = meeting_field(w, "rooms");
    assert!(a.ui.people.pieces(rooms).iter().any(|p| chip_named(p, "Sound Lab")), "{:?}", a.ui.people.pieces(rooms));
    let _ = std::fs::remove_dir_all(dir);
}

/// The Rooms line books a room or an address typed in full (an equipment, a mailbox only the
/// owner knows is one), and refuses a person picked by name and a list.
#[test]
fn the_rooms_field_takes_rooms_and_addresses_and_refuses_the_rest() {
    let (mut a, dir) = app("meeting-rooms-field");
    // A list the directory told about.
    let team = Addressee::new("Design Team", "team@contoso.example", AddresseeKind::DistributionList);
    a.session.apply(MailEvent::DirectoryPeople { account: "sample".into(), query: "team".into(), people: vec![team], more: false, error: None });
    let ctx = egui::Context::default();
    let w = open_meeting(&ctx, &mut a, "", false);
    let rooms = meeting_field(w, "rooms");
    let not_a_room = Some(Some("This isn't a room".to_string()));
    // A person picked by name (Ctrl+K settles "Zoe" as the Zoe known here): red, not booked.
    type_in(&ctx, &mut a, rooms, "Zoe");
    frame_sized(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)], SIZE);
    frames(&ctx, &mut a, 3);
    assert_eq!(refusal(&a, rooms, "zoe@studio.example"), not_a_room, "{:?}", a.ui.people.pieces(rooms));
    assert!(a.ui.people.blocked(rooms) && booked(&a).is_empty());
    // Taken out again (Backspace twice).
    frame_sized(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE)], SIZE);
    frame_sized(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE)], SIZE);
    frames(&ctx, &mut a, 2);
    assert!(a.ui.people.pieces(rooms).is_empty(), "{:?}", a.ui.people.pieces(rooms));
    // An address typed in full: booked, an equipment's as a mailbox's.
    type_in(&ctx, &mut a, rooms, "beamer@studio.example;");
    assert_eq!(booked(&a), ["beamer@studio.example"]);
    assert_eq!(form(&a).location, "beamer@studio.example", "the place follows the rooms");
    type_in(&ctx, &mut a, rooms, "zoe@studio.example;");
    assert_eq!(booked(&a), ["beamer@studio.example", "zoe@studio.example"]);
    assert_eq!(refusal(&a, rooms, "zoe@studio.example"), Some(None), "typed in full: taken");
    // A known list: red.
    type_in(&ctx, &mut a, rooms, "team@contoso.example;");
    assert_eq!(refusal(&a, rooms, "team@contoso.example"), not_a_room, "{:?}", a.ui.people.pieces(rooms));
    assert_eq!(booked(&a).len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

/// The Room Finder and the Rooms line book the same rooms: one booked in the finder is a chip on
/// the line, a chip removed frees it, and the place follows the rooms until it's typed by hand.
#[test]
fn the_rooms_field_and_the_room_finder_stay_in_step() {
    use crate::pim_dialogs::rooms::room_row_id;
    let (mut a, dir) = app("meeting-rooms-finder");
    let ctx = egui::Context::default();
    let w = open_meeting(&ctx, &mut a, "", true);
    let rooms = meeting_field(w, "rooms");
    assert!(steady(&ctx, &mut a, rooms), "the line and the Room Finder's toggle fit their row");
    let row = ctx.read_response(room_row_id(w, LAB)).map(|r| r.rect.center()).expect("the Sound Lab in the Room Finder");
    click_at(&ctx, &mut a, row, 2, SIZE);
    frames(&ctx, &mut a, 2);
    assert_eq!(booked(&a), [LAB]);
    assert_eq!(form(&a).location, "Sound Lab");
    assert!(a.ui.people.pieces(rooms).iter().any(|p| chip_named(p, "Sound Lab")), "{:?}", a.ui.people.pieces(rooms));
    // Its chip removed: freed, and the place follows.
    remove_chip(&ctx, &mut a, rooms, 0);
    assert!(booked(&a).is_empty() && a.ui.people.pieces(rooms).is_empty(), "{:?}", a.ui.people.pieces(rooms));
    assert_eq!(form(&a).location, "", "the place follows the rooms");
    // A place typed by hand stays when the room is booked again.
    a.ui.event_windows[0].1.location = "Online".into();
    let row = ctx.read_response(room_row_id(w, LAB)).map(|r| r.rect.center()).expect("the Sound Lab in the Room Finder");
    click_at(&ctx, &mut a, row, 2, SIZE);
    frames(&ctx, &mut a, 2);
    assert_eq!(booked(&a), [LAB]);
    assert_eq!(form(&a).location, "Online");
    assert!(a.ui.people.pieces(rooms).iter().any(|p| chip_named(p, "Sound Lab")), "{:?}", a.ui.people.pieces(rooms));
    let _ = std::fs::remove_dir_all(dir);
}

/// Select Attendees and Resources: its rows are the contacts field. A person put in Resources is
/// red and OK waits; a room picked there instead is booked at OK; Cancel leaves the meeting as it
/// was.
#[test]
fn the_attendees_dialog_rows_are_contacts_fields() {
    let (mut a, dir) = app("meeting-attendees-rows");
    let ctx = egui::Context::default();
    let w = open_meeting(&ctx, &mut a, "sam@studio.example", false);
    let open = |a: &MailApp| form(a).attendees_book.is_some();
    let at = ctx.read_response(line_button_id(w, Line::Required)).map(|r| r.rect.center()).expect("the Required button");
    click_at(&ctx, &mut a, at, 1, SIZE);
    assert_eq!(form(&a).attendees_book.as_ref().map(|b| b.target), Some(Line::Required));
    frames(&ctx, &mut a, 3);
    let (att_required, att_optional, att_rooms) = (meeting_field(w, "att-required"), meeting_field(w, "att-optional"), meeting_field(w, "att-rooms"));
    assert_eq!(refusal(&a, att_required, "sam@studio.example"), Some(None), "{:?}", a.ui.people.pieces(att_required));
    // Zoe picked in the list, then Resources ->: red, and OK waits.
    frame_sized(&ctx, &mut a, vec![Event::Text("zoe".into())], SIZE);
    let zoe = settled_text_at(&ctx, &mut a, "Zoe Park", SIZE).expect("Zoe in the list");
    click_at(&ctx, &mut a, zoe, 1, SIZE);
    let to_rooms = settled_text_at(&ctx, &mut a, "Resources ->", SIZE).expect("the Resources button");
    click_at(&ctx, &mut a, to_rooms, 1, SIZE);
    assert_eq!(refusal(&a, att_rooms, "zoe@studio.example"), Some(Some("This isn't a room".to_string())), "{:?}", a.ui.people.pieces(att_rooms));
    let ok = settled_text_at(&ctx, &mut a, "OK", SIZE).expect("OK");
    click_at(&ctx, &mut a, ok, 1, SIZE);
    assert!(open(&a) && booked(&a).is_empty(), "OK is disabled while a row holds what it doesn't take");
    // Taken out, and a room picked in its stead (a suggestion): OK books it.
    remove_chip(&ctx, &mut a, att_rooms, 0);
    assert!(a.ui.people.pieces(att_rooms).is_empty(), "{:?}", a.ui.people.pieces(att_rooms));
    type_in(&ctx, &mut a, att_rooms, "sound");
    assert_eq!(a.ui.people.suggestions(att_rooms).first().map(|s| s.who.address.as_str()), Some(LAB));
    frame_sized(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)], SIZE);
    frames(&ctx, &mut a, 2);
    assert_eq!(refusal(&a, att_rooms, LAB), Some(None), "{:?}", a.ui.people.pieces(att_rooms));
    let ok = settled_text_at(&ctx, &mut a, "OK", SIZE).expect("OK");
    click_at(&ctx, &mut a, ok, 1, SIZE);
    assert!(!open(&a), "closed");
    assert_eq!(booked(&a), [LAB]);
    assert!(form(&a).guests.contains("sam@studio.example"), "{}", form(&a).guests);
    // Cancel: the meeting as it was.
    let before = (form(&a).guests.clone(), form(&a).optional.clone(), form(&a).rooms.clone());
    let at = ctx.read_response(line_button_id(w, Line::Optional)).map(|r| r.rect.center()).expect("the Optional button");
    click_at(&ctx, &mut a, at, 1, SIZE);
    frames(&ctx, &mut a, 3);
    type_in(&ctx, &mut a, att_optional, "zoe@studio.example;");
    assert!(a.ui.people.pieces(att_optional).iter().any(|p| chip_named(p, "Zoe Park")), "{:?}", a.ui.people.pieces(att_optional));
    let cancel = settled_text_at(&ctx, &mut a, "Cancel", SIZE).expect("Cancel");
    click_at(&ctx, &mut a, cancel, 1, SIZE);
    assert!(!open(&a));
    assert_eq!((form(&a).guests.clone(), form(&a).optional.clone(), form(&a).rooms.clone()), before);
    let _ = std::fs::remove_dir_all(dir);
}

/// A list's members are entries like any other: among a meeting's guests, a room in a contact
/// group expanded at sending moves to Rooms, and a member the line doesn't take is red.
#[test]
fn a_lists_members_are_checked_as_any_entry() {
    let (mut a, dir) = app("meeting-members");
    let gal = with_exchange(&mut a);
    let wid = 4;
    let required = meeting_field(wid, "required");
    let (mut guests, mut optional, mut rooms) = ("Dupont family <group:AAMk=>".to_string(), String::new(), String::new());
    let lines = [("required", &mut guests), ("optional", &mut optional), ("rooms", &mut rooms)];
    assert!(a.ui.people.gate_meeting(&mut a.session, wid, &gal, lines, 1.0), "held for the group's members");
    let members = vec![
        Addressee::new("Anne Dupont", "anne@contoso.example", AddresseeKind::Person),
        Addressee::new("Salle Seine", "room.seine@contoso.example", AddresseeKind::Room),
        Addressee::new("News", "news@contoso.example", AddresseeKind::PublicFolder),
    ];
    a.session.apply(MailEvent::ListExpanded {
        account: gal.clone(),
        address: String::new(),
        item_id: Some("AAMk=".into()),
        members,
        more: false,
        total: 3,
        error: None,
    });
    a.ui.people.prepare(&mut a.session, required, Some(&gal), accepts_of("required"), &guests);
    assert_eq!(refusal(&a, required, "anne@contoso.example"), Some(None));
    assert_eq!(refusal(&a, required, "news@contoso.example"), Some(Some("Only people, lists and groups can be invited".to_string())));
    assert_eq!(refusal(&a, required, "room.seine@contoso.example"), None, "out of the guests: {:?}", a.ui.people.pieces(required));
    let moved = a.ui.people.take_moved(required);
    assert!(moved.len() == 1 && moved[0].address == "room.seine@contoso.example", "{moved:?}");
    let _ = std::fs::remove_dir_all(dir);
}
