//! The contacts field wired to the session (`dialogs/people_field.rs`): the message's To, Cc,
//! Bcc and Replies to, the Select Names dialog's rows, and a Send that waits for the names still
//! being resolved.

use quatrelle_engine::FieldId;
use quatrelle_engine::model::scheduling::Room;
use quatrelle_engine::model::{Addressee, AddresseeKind, EmailAddress, Job};

use super::*;
use crate::dialogs::people_field::egui_id;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// The To of the message being written.
fn to(a: &MailApp) -> String {
    a.session.compose.as_ref().map(|c| c.to.clone()).unwrap_or_default()
}

/// A field of the message being written.
fn field(a: &MailApp, name: &'static str) -> FieldId {
    FieldId { window: a.session.compose.as_ref().map_or(0, |c| c.id), name }
}

/// Give a field's input the keyboard.
fn focus_field(ctx: &egui::Context, f: FieldId) {
    ctx.memory_mut(|m| m.request_focus(input_id(egui_id(f))));
}

/// A frame at `time` seconds (the clock a send's wait is measured by).
fn frame_at(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>, time: f64) {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)), time: Some(time), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
}

/// The suggestion list's rect once it stays put (the chips' line settles a frame after a chip
/// comes in, and the list follows the input).
fn settled_list_rect(ctx: &egui::Context, a: &mut MailApp) -> Option<egui::Rect> {
    let mut last = list_rect(ctx);
    for _ in 0..20 {
        frame(ctx, a, vec![]);
        let now = list_rect(ctx);
        if now.is_some() && now == last {
            return now;
        }
        last = now;
    }
    None
}

/// A backend that keeps the jobs handed to it (`NullBackend` drops them, and `logic` hands them
/// over every frame).
#[derive(Default)]
struct Taken(std::rc::Rc<std::cell::RefCell<Vec<Job>>>);

impl crate::Backend for Taken {
    fn submit(&mut self, job: Job) {
        self.0.borrow_mut().push(job);
    }

    fn poll(&mut self) -> Vec<MailEvent> {
        Vec::new()
    }
}

/// The jobs the app handed its backend so far, taken.
fn handed(taken: &std::rc::Rc<std::cell::RefCell<Vec<Job>>>) -> Vec<Job> {
    std::mem::take(&mut *taken.borrow_mut())
}

/// A click at `at` (press, release, a frame).
fn click(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    frame(ctx, a, vec![Event::PointerMoved(at)]);
    frame(ctx, a, vec![press(at, true)]);
    frame(ctx, a, vec![press(at, false)]);
    frame(ctx, a, vec![]);
}

fn chip_named(p: &Piece, name: &str) -> bool {
    matches!(p, Piece::Chip { who, .. } if who.display() == name)
}

fn text_with(p: &Piece, text: &str, error: Option<PieceError>) -> bool {
    matches!(p, Piece::Text { text: t, error: e } if t == text && *e == error)
}

/// The directory couldn't answer `query` (offline, unreachable).
fn directory_fails(a: &mut MailApp, account: &str, query: &str, error: &str) {
    a.session.apply(MailEvent::DirectoryPeople {
        account: account.into(),
        query: query.into(),
        people: Vec::new(),
        more: false,
        error: Some(error.into()),
    });
}

fn has_send(jobs: &[Job]) -> bool {
    jobs.iter().any(|j| matches!(j, Job::Send { .. }))
}

/// A message on the Exchange account, with a subject so that it can go.
fn new_message_on(a: &mut MailApp, account: &str, to: &str) {
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"account": account, "to": to, "subject": "Hello"}));
}

/// Typing in To opens suggestions; Enter takes the highlighted one as a chip; a click on a
/// suggestion keeps the cursor, so the next name is typed straight in; Escape hides the list, and
/// Enter with it hidden leaves the field, which closes what was typed.
#[test]
fn recipient_boxes_suggest_people() {
    let (mut a, dir) = app("suggest");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("zo".into())]);
    assert_eq!(to(&a), "zo");
    assert_eq!(a.ui.people.open_field(), Some(field(&a, "to")));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert_eq!(to(&a), "Zoe Park <zoe@studio.example>; ");
    // Picked with the mouse: the field keeps the cursor after the click, so the next address is
    // typed straight in.
    frame(&ctx, &mut a, vec![Event::Text("sh".into())]);
    frame(&ctx, &mut a, vec![]);
    let row = settled_list_rect(&ctx, &mut a).map(|r| r.left_top() + egui::vec2(20.0, 20.0)).expect("the suggestions show");
    click(&ctx, &mut a, row);
    frame(&ctx, &mut a, vec![Event::Text("x".into())]);
    let now = to(&a);
    assert!(now.starts_with("Zoe Park <zoe@studio.example>; ") && now.ends_with(">; x") && now.matches('@').count() == 2, "{now}");
    a.session.compose.as_mut().unwrap().to = "Zoe Park <zoe@studio.example>; ".into();

    // The next address starts fresh; Escape hides the list; Enter then leaves the field, and the
    // text typed is closed and checked as the field is left: "sh" fits one person here.
    frame(&ctx, &mut a, vec![Event::Text("sh".into())]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE), key(Key::Escape, Modifiers::NONE)]);
    assert!(list_rect(&ctx).is_none(), "Escape hid the list");
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "Zoe Park <zoe@studio.example>; Print Shop <orders@printshop.example>; ");
    assert!(chip_named(&a.ui.people.pieces(field(&a, "to"))[1], "Print Shop"), "{:?}", a.ui.people.pieces(field(&a, "to")));
    let _ = std::fs::remove_dir_all(dir);
}

/// Outlook's × beside a suggestion: red on the row pointed at, it takes the address out of the
/// suggestions (out of the address book until it's written to), the text typed staying as it
/// was; Delete does the same to the one highlighted.
#[test]
fn a_suggestion_is_taken_out_with_its_cross() {
    let (mut a, dir) = app("suggest-forget");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("zo".into())]);
    frame(&ctx, &mut a, vec![]);
    let suggested = |a: &mut MailApp| a.session.suggest_contacts_now("zo", 8).into_iter().map(|c| c.address.email).collect::<Vec<_>>();
    assert!(suggested(&mut a).contains(&"zoe@studio.example".to_string()));
    // Zoe's row, once the message's layout has settled; the × is at its right end.
    let list = settled_list_rect(&ctx, &mut a).expect("the suggestions show");
    let row = settled_text_at(&ctx, &mut a, "Zoe Park", SIZE).expect("Zoe suggested");
    let x = egui::pos2(list.right() - 4.0 - 13.0, row.y);
    click(&ctx, &mut a, x);
    assert_eq!(to(&a), "zo", "the text typed as it was, not the address picked");
    assert!(!suggested(&mut a).contains(&"zoe@studio.example".to_string()), "taken out");
    assert_eq!(a.ui.people.open_field(), Some(field(&a, "to")), "the field keeps the cursor");
    // Delete takes out the one highlighted.
    a.session.compose.as_mut().unwrap().to.clear();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("sh".into())]);
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    let first = a.ui.people.suggestions(f).first().map(|s| s.who.address.clone()).expect("someone for sh");
    frame(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(!a.session.suggest_contacts_now("sh", 8).iter().any(|c| c.address.email == first), "{first} taken out");
    let _ = std::fs::remove_dir_all(dir);
}

/// Text fed in (a draft reopened, a reply, a script) becomes chips at once, each with its kind
/// from what is known here: a room from the room lists, an address nobody knows as an address
/// alone; a name with a comma stays one chip; what isn't an address stays text. The text is
/// written back as the chips spell it.
#[test]
fn text_fed_in_becomes_chips_with_their_kinds() {
    let (mut a, dir) = app("fed-in");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let seine = Room { email: "room.seine@contoso.example".into(), name: "Salle Seine".into(), list: "Paris".into() };
    a.session.apply(MailEvent::Rooms { account: gal.clone(), lists: Vec::new(), rooms: vec![seine], error: None });
    a.run("mail.new", json!({}));
    let fed = "\"Dupont, Marc\" <m@contoso.example>; Dupont, Anne <a@contoso.example>; room.seine@contoso.example; nobody here";
    a.run("compose.update", json!({"account": gal, "to": fed}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let p = a.ui.people.pieces(field(&a, "to")).to_vec();
    assert_eq!(p.len(), 4, "{p:?}");
    let kind = |i: usize| match &p[i] {
        Piece::Chip { who, .. } => Some(who.kind),
        Piece::Text { .. } => None,
    };
    assert!(chip_named(&p[0], "Dupont, Marc") && kind(0) == Some(AddresseeKind::Unknown), "{p:?}");
    assert!(chip_named(&p[1], "Dupont, Anne") && kind(1) == Some(AddresseeKind::Unknown), "{p:?}");
    assert!(chip_named(&p[2], "Salle Seine") && kind(2) == Some(AddresseeKind::Room), "{p:?}");
    assert!(text_with(&p[3], "nobody here", None), "{p:?}");
    assert_eq!(
        to(&a),
        "\"Dupont, Marc\" <m@contoso.example>; \"Dupont, Anne\" <a@contoso.example>; Salle Seine <room.seine@contoso.example>; nobody here; "
    );
    assert_eq!(a.ui.people.syncs(field(&a, "to")), 1, "parsed once, written back once");
    let _ = std::fs::remove_dir_all(dir);
}

/// The lesson of the search bug of 2026-10-30: a field with 300 recipients parses its text when
/// it changes, never on a frame where nothing happened, and a key typed in its input asks for
/// one suggestion pass and parses nothing (the input isn't the text).
#[test]
fn a_field_parses_its_text_once_per_change_not_per_frame() {
    let (mut a, dir) = app("parse-once");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    let many: Vec<String> = (0..300).map(|i| format!("Person {i} <p{i}@studio.example>")).collect();
    a.run("compose.update", json!({"to": many.join("; ")}));
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    assert_eq!(a.ui.people.pieces(f).len(), 300);
    assert!(a.ui.people.pieces(f).iter().all(|p| matches!(p, Piece::Chip { .. })));
    assert_eq!(a.ui.people.syncs(f), 1);
    let passes = a.session.people_suggest_passes().0;
    for _ in 0..20 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(a.ui.people.syncs(f), 1, "idle frames parse nothing");
    assert_eq!(a.session.people_suggest_passes().0, passes, "idle frames suggest nothing");
    focus_field(&ctx, f);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("z".into())]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.people.syncs(f), 1, "the input isn't the text");
    assert_eq!(a.session.people_suggest_passes().0, passes + 1, "one pass for the key typed");
    assert!(to(&a).ends_with(">; z"), "{}", to(&a));
    let _ = std::fs::remove_dir_all(dir);
}

/// A pasted list becomes chips, the first 500 of it, and says how many were left out; a short
/// paste gives a chip per entry, named when the entry names it.
#[test]
fn pasting_a_list_gives_chips_and_keeps_the_first_500() {
    let (mut a, dir) = app("paste-list");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let many: Vec<String> = (0..503).map(|i| format!("p{i}@x.example")).collect();
    frame(&ctx, &mut a, vec![Event::Paste(many.join("; "))]);
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    let p = a.ui.people.pieces(f);
    assert_eq!(p.len(), 500);
    assert!(p.iter().all(|p| matches!(p, Piece::Chip { .. })), "every entry a chip");
    assert_eq!(a.ui.people.left_out(f), Some(3));
    assert!(settled_text_at(&ctx, &mut a, "and 3 more were left out", SIZE).is_some(), "said under the field");
    assert_eq!(to(&a).matches('@').count(), 500, "the text holds what the field holds");

    // A short paste in an empty field: two chips, the second named as pasted.
    a.run("compose.update", json!({"to": ""}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.people.left_out(f), None);
    frame(&ctx, &mut a, vec![Event::Paste("a@x.fr; John Doe <b@y.fr>".into())]);
    frame(&ctx, &mut a, vec![]);
    let p = a.ui.people.pieces(f);
    assert_eq!(p.len(), 2, "{p:?}");
    assert!(matches!(&p[0], Piece::Chip { who, .. } if who.address == "a@x.fr" && who.name.is_empty()), "{p:?}");
    assert!(matches!(&p[1], Piece::Chip { who, .. } if who.address == "b@y.fr" && who.name == "John Doe"), "{p:?}");
    assert_eq!(to(&a), "a@x.fr; John Doe <b@y.fr>; ");
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+K in the field checks what's typed: a known name becomes a chip, a piece with `@` that
/// isn't an address is red with "Not an address", a name waits on the directory and is red with
/// "Not found" when it answers nothing, and "The directory can't be reached" when it can't be
/// asked. (In the text, Ctrl+K is still a link: `ctrl_k_checks_the_names_typed`.)
#[test]
fn ctrl_k_in_the_field_resolves_and_marks_what_it_cant() {
    let (mut a, dir) = app("ctrl-k-field");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"account": gal}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    focus_field(&ctx, f);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("Zoe Park; a@; dupont".into())]);
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let p = a.ui.people.pieces(f).to_vec();
    assert_eq!(p.len(), 3, "{p:?}");
    assert!(chip_named(&p[0], "Zoe Park"), "{p:?}");
    assert!(text_with(&p[1], "a@", Some(PieceError::NotAnAddress)), "{p:?}");
    assert!(text_with(&p[2], "dupont", None), "{p:?}");
    assert_eq!(a.session.people_pending(f), 1, "dupont waits on the directory");
    assert!(a.session.directory_searching(&gal, "dupont"));
    assert_eq!(to(&a), "Zoe Park <zoe@studio.example>; a@; dupont; ");
    directory_answers(&mut a, &gal, "dupont", &[]);
    frame(&ctx, &mut a, vec![]);
    let p = a.ui.people.pieces(f).to_vec();
    assert!(text_with(&p[2], "dupont", Some(PieceError::NotFound)), "{p:?}");
    assert_eq!(a.session.people_pending(f), 0);

    // The directory can't be asked for the next name: said so, not "not found".
    frame(&ctx, &mut a, vec![Event::Text("smith".into())]);
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    directory_fails(&mut a, &gal, "smith", "offline");
    frame(&ctx, &mut a, vec![]);
    let p = a.ui.people.pieces(f).to_vec();
    assert!(text_with(&p[3], "smith", Some(PieceError::Unreached)), "{p:?}");
    let at = settled_text_at(&ctx, &mut a, "smith", SIZE).expect("the text is drawn");
    frame(&ctx, &mut a, vec![Event::PointerMoved(at)]);
    let mut tip = None;
    for _ in 0..60 {
        tip = tip.or_else(|| text_at(&frame_shapes(&ctx, &mut a, vec![], SIZE), "The directory can't be reached"));
    }
    assert!(tip.is_some(), "the reason on hover");
    let _ = std::fs::remove_dir_all(dir);
}

/// Send waits for a name still being looked up, then goes once the directory has answered; a
/// name the directory doesn't know stops the send with "Not found"; a message to addresses alone
/// goes without asking the directory at all.
#[test]
fn sending_waits_for_a_pending_name_then_stops_on_one_not_found() {
    let (mut a, dir) = app("send-waits");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "zoe@studio.example; dupont");
    frame(&ctx, &mut a, vec![]);
    handed(&jobs);
    a.run("compose.send", json!({}));
    assert!(!a.session.compose.as_ref().unwrap().sending, "held for the name");
    frame(&ctx, &mut a, vec![]);
    let out = handed(&jobs);
    assert!(!has_send(&out), "nothing sent: {out:?}");
    assert!(a.session.directory_searching(&gal, "dupont"), "asked of the directory");
    frame(&ctx, &mut a, vec![]);
    assert!(!has_send(&handed(&jobs)), "still held");
    directory_answers(&mut a, &gal, "dupont", &[("Marc Dupont", "marc@contoso.example")]);
    frame(&ctx, &mut a, vec![]);
    let out = handed(&jobs);
    assert!(has_send(&out), "sent once the name was found: {out:?}");
    assert_eq!(to(&a), "Zoe Park <zoe@studio.example>; Marc Dupont <marc@contoso.example>; ");
    assert!(a.session.compose.as_ref().unwrap().sending);
    assert!(a.ui.compose_alert.is_none(), "{:?}", a.ui.compose_alert);

    // A name nobody knows stops the send.
    new_message_on(&mut a, &gal, "nobody");
    frame(&ctx, &mut a, vec![]);
    a.run("compose.send", json!({}));
    directory_answers(&mut a, &gal, "nobody", &[]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.compose_alert.as_deref(), Some("Not found: nobody"));
    assert!(!has_send(&handed(&jobs)));
    assert!(!a.session.compose.as_ref().unwrap().sending);
    a.ui.compose_alert = None;

    // Addresses alone never need the directory: the message goes, offline or not.
    new_message_on(&mut a, &gal, "zoe@studio.example");
    for acc in a.session.accounts.iter_mut().filter(|x| x.config.id == gal) {
        acc.status = quatrelle_engine::model::SyncStatus::Offline;
    }
    frame(&ctx, &mut a, vec![]);
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    let out = handed(&jobs);
    assert!(has_send(&out), "{out:?}");
    assert!(!out.iter().any(|j| matches!(j, Job::SearchDirectory { .. })), "the directory wasn't asked: {out:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Several people fit a name at Send: the choice is asked before the message goes, and Cancel
/// (or Escape) there leaves the name as typed and the message unsent, with nothing to say.
#[test]
fn cancelling_the_choice_asked_at_send_leaves_the_message_unsent() {
    let (mut a, dir) = app("send-choice-cancel");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "dupont");
    frame(&ctx, &mut a, vec![]);
    a.run("compose.send", json!({}));
    directory_answers(&mut a, &gal, "dupont", &[("Anne Dupont", "anne@contoso.example"), ("Marc Dupont", "marc@contoso.example")]);
    assert!(settled_text_at(&ctx, &mut a, "More than one person fits “dupont”. Choose who you meant:", SIZE).is_some(), "asked before it goes");
    assert!(!has_send(&handed(&jobs)), "held for the choice");
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(settled_text_at(&ctx, &mut a, "More than one person fits “dupont”. Choose who you meant:", SIZE).is_none(), "the dialog closed");
    assert_eq!(a.ui.compose_alert, None, "nothing to say: the send was called off");
    assert!(!has_send(&handed(&jobs)), "not sent");
    assert!(!a.session.compose.as_ref().unwrap().sending);
    assert!(a.ui.people.send_wait_left(0.0).is_none(), "no send waits any more");
    let f = field(&a, "to");
    assert!(a.ui.people.pieces(f).iter().any(|p| text_with(p, "dupont", None)), "the name as typed: {:?}", a.ui.people.pieces(f));
    let _ = std::fs::remove_dir_all(dir);
}

/// A directory that never answers doesn't hold Send forever: past its timeout the send stops
/// with "The directory can't be reached".
#[test]
fn sending_gives_up_after_the_directorys_timeout() {
    let (mut a, dir) = app("send-timeout");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame_at(&ctx, &mut a, vec![], 1.0);
    new_message_on(&mut a, &gal, "dupont");
    frame_at(&ctx, &mut a, vec![], 2.0);
    a.run("compose.send", json!({}));
    frame_at(&ctx, &mut a, vec![], 30.0);
    assert!(a.ui.compose_alert.is_none(), "still waiting at 30 s");
    frame_at(&ctx, &mut a, vec![], 63.0);
    assert_eq!(a.ui.compose_alert.as_deref(), Some("The directory can't be reached"));
    assert!(!has_send(&handed(&jobs)));
    assert!(!a.session.compose.as_ref().unwrap().sending);
    let f = field(&a, "to");
    assert!(a.ui.people.pieces(f).iter().any(|p| text_with(p, "dupont", Some(PieceError::Unreached))), "{:?}", a.ui.people.pieces(f));
    let _ = std::fs::remove_dir_all(dir);
}

/// A contact group has no address: Send asks the server for its members first, and the message
/// goes to them once they're known.
#[test]
fn a_contact_group_is_expanded_before_the_message_goes() {
    let (mut a, dir) = app("send-group");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "Dupont family <group:AAMk=>");
    frame(&ctx, &mut a, vec![]);
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    let out = handed(&jobs);
    assert!(out.iter().any(|j| matches!(j, Job::ExpandList { item_id: Some(id), .. } if id == "AAMk=")), "{out:?}");
    assert!(!has_send(&out));
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
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    let p = a.ui.people.pieces(f).to_vec();
    assert!(p.len() == 2 && chip_named(&p[0], "Anne Dupont") && chip_named(&p[1], "Marc Dupont"), "{p:?}");
    let out = handed(&jobs);
    let sent = out.iter().find_map(|j| match j {
        Job::Send { recipients, .. } => Some(recipients.clone()),
        _ => None,
    });
    assert_eq!(sent, Some(vec!["anne@contoso.example".to_string(), "marc@contoso.example".to_string()]));
    assert_eq!(to(&a), "Anne Dupont <anne@contoso.example>; Marc Dupont <marc@contoso.example>; ");
    let _ = std::fs::remove_dir_all(dir);
}

/// The Select Names dialog's rows are the field: a double-click in the list adds a chip to the
/// row the dialog was opened for, typing in a row suggests, and OK hands the rows' text to the
/// message.
#[test]
fn the_select_names_rows_are_the_field() {
    let (mut a, dir) = app("select-names-rows");
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().unwrap();
    a.ui.dialog = Some(Dialog::select_names(&c, "to"));
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    f.search.query = "zoe".into();
    f.search.stale = true;
    let zoe = settled_text_at(&ctx, &mut a, "Zoe Park", SIZE).expect("Zoe listed");
    click_at(&ctx, &mut a, zoe, 2, SIZE);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_ref() else { panic!("no dialog") };
    assert_eq!(f.to, "Zoe Park <zoe@studio.example>; ");
    let book_to = FieldId { window: c.id, name: "book-to" };
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.people.pieces(book_to).iter().any(|p| chip_named(p, "Zoe Park")), "{:?}", a.ui.people.pieces(book_to));
    // Typing in the To row suggests (Sam, written to: Recent).
    focus_field(&ctx, book_to);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("sam".into())]);
    assert!(settled_text_at(&ctx, &mut a, "Recent", SIZE).is_some(), "suggestions under the row");
    // OK: the rows go to the message.
    let ok = settled_text_at(&ctx, &mut a, "OK", SIZE).expect("OK");
    click_at(&ctx, &mut a, ok, 1, SIZE);
    assert!(a.ui.dialog.is_none(), "closed");
    let now = to(&a);
    assert!(now.starts_with("Zoe Park <zoe@studio.example>; ") && now.contains("sam"), "{now}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+K in a Select Names row: the row hands its text to the engine and gets the name found
/// back as a chip (the message's own text is untouched until OK).
#[test]
fn ctrl_k_in_a_select_names_row_writes_the_name_found() {
    let (mut a, dir) = app("select-names-ctrl-k");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"account": gal}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().unwrap();
    a.ui.dialog = Some(Dialog::select_names(&c, "to"));
    frame(&ctx, &mut a, vec![]);
    let book_to = FieldId { window: c.id, name: "book-to" };
    focus_field(&ctx, book_to);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("Zoe Park; dupont".into())]);
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_ref() else { panic!("no dialog") };
    assert_eq!(f.to, "Zoe Park <zoe@studio.example>; dupont; ");
    let p = a.ui.people.pieces(book_to).to_vec();
    assert!(chip_named(&p[0], "Zoe Park") && text_with(&p[1], "dupont", None), "{p:?}");
    assert_eq!(a.session.people_pending(book_to), 1, "dupont waits on the directory");
    assert_eq!(to(&a), "", "the message's To waits for OK");
    directory_answers(&mut a, &gal, "dupont", &[("Marc Dupont", "marc@contoso.example")]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_ref() else { panic!("no dialog") };
    assert_eq!(f.to, "Zoe Park <zoe@studio.example>; Marc Dupont <marc@contoso.example>; ");
    let _ = std::fs::remove_dir_all(dir);
}

/// Two messages written at once keep their own fields: Ctrl+K in the second's To rewrites the
/// second's text and marks the second's pieces, and the first's stay as they were.
#[test]
fn two_messages_keep_their_own_fields() {
    let (mut a, dir) = app("two-messages-fields");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "Zoe Park; nobody"}));
    frame(&ctx, &mut a, vec![]);
    let first = field(&a, "to");
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.people.pieces(first).len(), 2);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "Zoe Park; nobody"}));
    frame(&ctx, &mut a, vec![]);
    let second = field(&a, "to");
    assert_ne!(first.window, second.window);
    focus_field(&ctx, second);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "Zoe Park <zoe@studio.example>; nobody; ");
    let p = a.ui.people.pieces(second).to_vec();
    assert!(chip_named(&p[0], "Zoe Park") && text_with(&p[1], "nobody", Some(PieceError::NotFound)), "{p:?}");
    let other = a.session.all_composes().find(|c| c.id == first.window).map(|c| c.to.clone());
    assert_eq!(other.as_deref(), Some("Zoe Park; nobody"), "the first's text as it was");
    let p = a.ui.people.pieces(first).to_vec();
    assert!(text_with(&p[0], "Zoe Park", None) && text_with(&p[1], "nobody", None), "the first's pieces untouched: {p:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A draft holding a contact group not yet expanded (Ruling 2): the message being written keeps
/// its reference, and the copy saved to the server carries the valid addresses alone, with
/// nothing else lost and no error. The group comes back only here, while the message is open:
/// a draft reopened from Drafts has its addresses.
#[test]
fn a_draft_keeps_a_contact_group_here_and_the_server_gets_its_addresses() {
    let (mut a, dir) = app("draft-group");
    let gal = with_exchange(&mut a);
    for acc in a.session.accounts.iter_mut().filter(|x| x.config.id == gal) {
        acc.folders.push(quatrelle_engine::model::Folder {
            account: gal.clone(),
            path: "Drafts".into(),
            role: quatrelle_engine::model::FolderRole::Drafts,
            selectable: true,
            ..Default::default()
        });
    }
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    let to_text = "\"Dupont, Marc\" <m@contoso.example>; Zoe Park <zoe@studio.example>; Dupont family <group:AAMk=>";
    a.run("compose.update", json!({"account": gal, "to": to_text, "subject": "Draft"}));
    frame(&ctx, &mut a, vec![]);
    a.session.take_jobs();
    a.run("compose.saveDraft", json!({}));
    assert_eq!(a.ui.compose_alert, None, "saved without an error");
    let raw = a.session.take_jobs().into_iter().find_map(|j| match j {
        Job::SaveDraft { raw, .. } => Some(raw),
        _ => None,
    });
    let parsed = quatrelle_engine::mime::parse(&raw.expect("the draft goes to the server")).unwrap();
    let kept = [EmailAddress::new("Dupont, Marc", "m@contoso.example"), EmailAddress::new("Zoe Park", "zoe@studio.example")];
    assert_eq!(parsed.to, kept, "the valid addresses alone, a comma name whole");
    assert_eq!(parsed.subject, "Draft", "nothing else lost");
    assert!(to(&a).contains("Dupont family <group:AAMk=>"), "the message being written keeps the group: {}", to(&a));
    let _ = std::fs::remove_dir_all(dir);
}

/// A name left with Tab, or closed with `;`, is checked as Ctrl+K would check it: the directory is
/// asked at once, not only at Send.
#[test]
fn a_name_left_or_closed_is_asked_of_the_directory_at_once() {
    let (mut a, dir) = app("resolve-on-close");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"account": gal}));
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    focus_field(&ctx, f);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("dupont".into())]);
    frame(&ctx, &mut a, vec![key(Key::Tab, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.directory_searching(&gal, "dupont"), "asked as the field was left");
    assert_eq!(a.session.people_pending(f), 1);
    assert!(a.ui.people.pieces(f).iter().any(|p| text_with(p, "dupont", None)), "{:?}", a.ui.people.pieces(f));
    // Back in the field, a name closed with `;` is asked too; what's settled isn't asked again.
    focus_field(&ctx, f);
    frame(&ctx, &mut a, vec![]);
    directory_answers(&mut a, &gal, "dupont", &[("Marc Dupont", "marc@contoso.example")]);
    frame(&ctx, &mut a, vec![]);
    handed(&jobs);
    frame(&ctx, &mut a, vec![Event::Text("smith;".into())]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.directory_searching(&gal, "smith"), "asked as the name was closed");
    let asked: Vec<String> = handed(&jobs)
        .into_iter()
        .filter_map(|j| match j {
            Job::SearchDirectory { query, .. } => Some(query),
            _ => None,
        })
        .collect();
    assert_eq!(asked, ["smith"], "Marc, found already, isn't asked again");
    assert_eq!(to(&a), "Marc Dupont <marc@contoso.example>; smith; ");
    let _ = std::fs::remove_dir_all(dir);
}

/// The directory was down when Ctrl+K asked ("The directory can't be reached"), and it is back
/// when Send asks again a minute later: the name is pending again, not stopped on its old error.
#[test]
fn an_old_unreached_name_asked_again_at_send_waits_instead_of_stopping() {
    let (mut a, dir) = app("send-retries-unreached");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "smith");
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    focus_field(&ctx, f);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    directory_fails(&mut a, &gal, "smith", "offline");
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.people.pieces(f).iter().any(|p| text_with(p, "smith", Some(PieceError::Unreached))), "{:?}", a.ui.people.pieces(f));
    // A minute later the directory is asked again at Send.
    a.session.clock.now += 61;
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.directory_searching(&gal, "smith"), "asked again");
    assert_eq!(a.ui.compose_alert, None, "pending again, not stopped on the old error");
    assert!(!has_send(&handed(&jobs)));
    directory_answers(&mut a, &gal, "smith", &[("Jo Smith", "jo@contoso.example")]);
    frame(&ctx, &mut a, vec![]);
    assert!(has_send(&handed(&jobs)), "sent once the directory answered");
    assert_eq!(to(&a), "Jo Smith <jo@contoso.example>; ");
    let _ = std::fs::remove_dir_all(dir);
}

/// Several messages can wait at once: one sent meanwhile (addresses alone) doesn't lose the
/// others' waits, and each goes exactly once when its own names settle.
#[test]
fn two_messages_waiting_each_go_once_when_their_names_settle() {
    let (mut a, dir) = app("two-waits");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "dupont");
    let first = a.session.compose.as_ref().unwrap().id;
    frame(&ctx, &mut a, vec![]);
    a.run("compose.send", json!({}));
    new_message_on(&mut a, &gal, "zoe@studio.example");
    let second = a.session.compose.as_ref().unwrap().id;
    frame(&ctx, &mut a, vec![]);
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    let sent_ids = |jobs: &[Job]| {
        jobs.iter()
            .filter_map(|j| match j {
                Job::Send { id, .. } => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(sent_ids(&handed(&jobs)), [second], "addresses alone go at once");
    new_message_on(&mut a, &gal, "smith");
    let third = a.session.compose.as_ref().unwrap().id;
    frame(&ctx, &mut a, vec![]);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert!(sent_ids(&handed(&jobs)).is_empty(), "both wait");
    directory_answers(&mut a, &gal, "dupont", &[("Marc Dupont", "marc@contoso.example")]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(sent_ids(&handed(&jobs)), [first], "the first goes when its name is found");
    directory_answers(&mut a, &gal, "smith", &[("Jo Smith", "jo@contoso.example")]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(sent_ids(&handed(&jobs)), [third], "the third too, once");
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(sent_ids(&handed(&jobs)).is_empty(), "nothing goes twice");
    assert_eq!(a.ui.compose_alert, None);
    let _ = std::fs::remove_dir_all(dir);
}

/// While the choice dialog is open, the directory's timeout doesn't run: a slow choice never reads
/// "The directory can't be reached", and the message goes once the person is picked.
#[test]
fn the_timeout_waits_while_the_choice_dialog_is_open() {
    let (mut a, dir) = app("send-choice-slow");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame_at(&ctx, &mut a, vec![], 1.0);
    new_message_on(&mut a, &gal, "dupont");
    frame_at(&ctx, &mut a, vec![], 2.0);
    a.run("compose.send", json!({}));
    directory_answers(&mut a, &gal, "dupont", &[("Anne Dupont", "anne@contoso.example"), ("Marc Dupont", "marc@contoso.example")]);
    frame_at(&ctx, &mut a, vec![], 3.0);
    frame_at(&ctx, &mut a, vec![], 70.0);
    frame_at(&ctx, &mut a, vec![], 130.0);
    assert_eq!(a.ui.compose_alert, None, "the clock waits for the choice");
    assert!(settled_text_at(&ctx, &mut a, "More than one person fits “dupont”. Choose who you meant:", SIZE).is_some(), "still asked");
    let marc = settled_text_at(&ctx, &mut a, "Marc Dupont", SIZE).expect("Marc offered");
    click_at(&ctx, &mut a, marc, 2, SIZE);
    frame(&ctx, &mut a, vec![]);
    assert!(has_send(&handed(&jobs)), "sent once chosen");
    assert_eq!(to(&a), "Marc Dupont <marc@contoso.example>; ");
    let _ = std::fs::remove_dir_all(dir);
}

/// A stop that isn't the user's doing is said: the choice answered "as typed" by a command (not by
/// the dialog's Cancel) leaves a name nothing waits for, and the send stops with a word.
#[test]
fn a_name_left_unsettled_by_something_else_stops_the_send_with_a_word() {
    let (mut a, dir) = app("send-unsettled");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "dupont");
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    a.run("compose.send", json!({}));
    directory_answers(&mut a, &gal, "dupont", &[("Anne Dupont", "anne@contoso.example"), ("Marc Dupont", "marc@contoso.example")]);
    frame(&ctx, &mut a, vec![]);
    a.run("people.choose", json!({"window": f.window, "field": "to", "token": "dupont"}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.compose_alert.as_deref(), Some("Some names need checking before sending"));
    assert!(!has_send(&handed(&jobs)));
    assert!(a.ui.people.send_wait_left(0.0).is_none(), "the wait is off");
    let _ = std::fs::remove_dir_all(dir);
}

/// The sample with one more message in its Inbox: from Zoe's address with no name on it (an
/// automated sender's way), as a reply finds it.
fn app_with_bare_sender(name: &str) -> (MailApp, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("quatrelle-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    sample::seed_if_empty(&store, NOW).unwrap();
    let raw = "From: zoe@studio.example\r\nTo: Riley Moreno <riley@studio.example>\r\nSubject: Bare from Zoe\r\n\
         Date: Wed, 22 Oct 2026 10:00:00 +0200\r\nMessage-ID: <bare-zoe@studio.example>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\nNo name on this one.";
    let account = sample::SAMPLE_ACCOUNT;
    let mut fs = store.load_folder(account, "INBOX").unwrap();
    let uid = fs.uid_next.max(1);
    let key = MessageKey { account: account.into(), folder: "INBOX".into(), validity: fs.validity.max(1), uid };
    store.write_body(&key, raw.as_bytes()).unwrap();
    let flags = quatrelle_engine::model::Flags { seen: true, ..Default::default() };
    fs.messages.push(quatrelle_engine::mime::summary_from_full(raw.as_bytes(), key, flags, raw.len() as u32, NOW));
    fs.uid_next = uid + 1;
    store.save_folder(account, "INBOX", &fs).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    (MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(NOW))), dir)
}

/// A reply to a sender the book knows by name but whose From had none: the field spells the name
/// it knows, with no word from the owner, and the reply left as it came still simply goes when
/// another message is read — it isn't filed as a draft (the old box never wrote the text itself).
#[test]
fn an_untouched_reply_to_a_bare_sender_is_not_filed_as_a_draft() {
    let (mut a, dir) = app_with_bare_sender("untouched-reply");
    let ctx = egui::Context::default();
    a.session.prefs.reply_window = false;
    frame(&ctx, &mut a, vec![]);
    let bare = a.session.list.iter().find(|m| m.subject == "Bare from Zoe").map(|m| m.key.clone()).expect("the bare message");
    let other = a.session.list.iter().find(|m| m.subject != "Bare from Zoe").map(|m| m.key.clone()).expect("another message");
    a.run("list.open", json!({"key": bare.to_id()}));
    frame(&ctx, &mut a, vec![]);
    a.run("mail.reply", json!({}));
    assert_eq!(to(&a), "zoe@studio.example; ", "the original's From had no name");
    for _ in 0..6 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(to(&a), "Zoe Park <zoe@studio.example>; ", "the field spells the name the book knows");
    assert!(a.session.compose.as_ref().is_some_and(|c| c.untouched()), "nothing the owner did");
    let drafts = a
        .session
        .account(sample::SAMPLE_ACCOUNT)
        .and_then(|x| x.folder(quatrelle_engine::model::FolderRole::Drafts))
        .map(|f| f.path.clone())
        .unwrap();
    let before = a.session.folder_state(sample::SAMPLE_ACCOUNT, &drafts).messages.len();
    a.run("list.open", json!({"key": other.to_id()}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.is_none(), "the reply left as it came simply goes");
    assert!(!a.session.notices.iter().any(|n| n.text == "Draft saved"), "{:?}", a.session.notices.last());
    assert_eq!(a.session.folder_state(sample::SAMPLE_ACCOUNT, &drafts).messages.len(), before, "no draft filed");
    let _ = std::fs::remove_dir_all(dir);
}

/// A contact group with more members than the field holds, expanded at Send: the message doesn't
/// go to part of the list; it stops and says so.
#[test]
fn a_contact_group_with_more_members_than_the_field_holds_stops_the_send() {
    let (mut a, dir) = app("send-group-partial");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "Dupont family <group:AAMk=>");
    frame(&ctx, &mut a, vec![]);
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert!(handed(&jobs).iter().any(|j| matches!(j, Job::ExpandList { .. })));
    let members = vec![
        Addressee::new("Anne Dupont", "anne@contoso.example", AddresseeKind::Person),
        Addressee::new("Marc Dupont", "marc@contoso.example", AddresseeKind::Person),
    ];
    a.session.apply(MailEvent::ListExpanded {
        account: gal.clone(),
        address: String::new(),
        item_id: Some("AAMk=".into()),
        members,
        more: true,
        total: 700,
        error: None,
    });
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.compose_alert.as_deref(), Some("Some recipients were left out: send to fewer at once, or to the list itself"));
    assert!(!has_send(&handed(&jobs)), "not sent to part of the list");
    assert!(!a.session.compose.as_ref().unwrap().sending);
    assert!(a.ui.people.send_wait_left(0.0).is_none(), "the wait is off");
    let _ = std::fs::remove_dir_all(dir);
}

/// A field fed more entries than it holds (a long list pasted) keeps the first 500 and says so;
/// Send doesn't go to those 500 alone.
#[test]
fn a_field_fed_more_entries_than_it_holds_stops_the_send() {
    let (mut a, dir) = app("send-over-cap");
    let taken = Taken::default();
    let jobs = taken.0.clone();
    a.backend = Box::new(taken);
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    new_message_on(&mut a, &gal, "");
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let many: Vec<String> = (0..503).map(|i| format!("p{i}@x.example")).collect();
    frame(&ctx, &mut a, vec![Event::Paste(many.join("; "))]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "to");
    assert_eq!(a.ui.people.pieces(f).len(), 500);
    assert_eq!(a.ui.people.left_out(f), Some(3));
    handed(&jobs);
    a.run("compose.send", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.compose_alert.as_deref(), Some("Some recipients were left out: send to fewer at once, or to the list itself"));
    assert!(!has_send(&handed(&jobs)), "not sent to the first 500 alone");
    assert!(!a.session.compose.as_ref().unwrap().sending);
    let _ = std::fs::remove_dir_all(dir);
}

/// A first start over a big mailbox, the book being read a slice per frame: an address closed in
/// To (checked at once) must not read the mail that is left in one go on the window's thread.
#[test]
fn closing_an_address_never_reads_a_big_mailbox_whole() {
    let (mut a, dir) = app_big("resolve-big", 3, 12_000);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.contacts_pending(), "the book is being read a slice per frame");
    let before = a.session.cached_folders();
    frame(&ctx, &mut a, vec![Event::Text("ann@x.example;".into())]);
    let read = a.session.cached_folders().iter().filter(|k| !before.contains(k)).count();
    assert!(a.session.contacts_pending(), "closing an address didn't read the book whole");
    assert!(read < 2, "the frame read its slice, not the mailbox: {read} folders");
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "ann@x.example; ");
    let _ = std::fs::remove_dir_all(dir);
}
