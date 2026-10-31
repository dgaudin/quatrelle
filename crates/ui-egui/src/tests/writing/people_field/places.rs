//! The contacts field in the dialogs (`dialogs/people_field.rs`): Add calendar's and Open another
//! mailbox's, whose OK waits for their names (`PeopleFields::gate_dialog`), and a rule's and a
//! Search Folder's, which keep a name as typed until Ctrl+K asks; all numbered apart from the
//! messages' and the meetings' windows. A dialog's field is drawn alone here, as its dialog will.

use std::collections::HashSet;

use quatrelle_engine::FieldId;
use quatrelle_engine::model::{Addressee, AddresseeKind};
use serde_json::Value;

use super::message::{chip_named, focus_field, text_with};
use super::*;
use crate::Action;
use crate::dialogs::people_field::{DIALOG_WINDOWS, MEETING_WINDOWS, accepts_of, dialog_field, egui_id, is_dialog_window};

/// The words a calendar's, a mailbox's and a rule's place refuse an entry with.
const NO_LIST_CALENDAR: &str = "A distribution list's calendar can't be opened";
const NOT_A_MAILBOX: &str = "Only a person's, a room's or a shared mailbox can be opened";
const GROUP_NO_ADDRESS: &str = "A contact group has no address: expand it";

/// One frame of a dialog's field alone, as its dialog draws it: made ready, then drawn; the
/// actions it asked for. (`focus_field` gives its input the keyboard.)
fn draw_field(ctx: &egui::Context, a: &mut MailApp, field: FieldId, account: Option<&str>, text: &mut String, events: Vec<Event>) -> Vec<Action> {
    let raw = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), events, ..Default::default() };
    let t = crate::theme::Tokens::light();
    let accepts = accepts_of(field.name);
    let mut acts = Vec::new();
    let mut out = ctx.run_ui(raw, |ui| {
        a.ui.people.prepare(&mut a.session, field, account, accepts, text);
        a.ui.people.draw(ui, &t, field, account, accepts, "From", "", text, false, false, false, &mut acts);
    });
    out.textures_delta.clear();
    acts
}

/// Whether the field asked the engine to check its names.
fn asks(acts: &[Action]) -> bool {
    acts.iter().any(|x| matches!(x, Action::Run("people.resolve", _)))
}

/// Why the field's chip of `address` is refused (`Some(None)`: taken), if the field holds one.
fn refusal(a: &MailApp, field: FieldId, address: &str) -> Option<Option<String>> {
    a.ui.people.pieces(field).iter().find_map(|p| match p {
        Piece::Chip { who, refused } if who.address == address => Some(refused.clone()),
        _ => None,
    })
}

/// The directory said what `team@contoso.example` is: a distribution list.
fn team_is_a_list(a: &mut MailApp, account: &str) {
    let team = Addressee::new("Team", "team@contoso.example", AddresseeKind::DistributionList);
    a.session.apply(MailEvent::DirectoryPeople { account: account.into(), query: "team".into(), people: vec![team], more: false, error: None });
}

/// In a rule's field, a name and a domain stay as typed when the field is left (nothing is asked
/// of the engine); Ctrl+K checks them: the name known becomes its chip, the domain stays text,
/// with no error.
#[test]
fn a_name_in_a_rule_field_stays_as_typed_until_ctrl_k() {
    let (mut a, dir) = app("rule-as-typed");
    a.session.run("contact.create", &json!({"name": "Jean Dupont", "emails": ["jean.dupont@proginov.com"]})).unwrap();
    // The book read (the window reads it a slice a frame).
    a.session.contacts_step(usize::MAX);
    let ctx = egui::Context::default();
    let f = dialog_field(0, "rule-any");
    let mut text = String::new();
    let mut acts = draw_field(&ctx, &mut a, f, Some("sample"), &mut text, vec![]);
    focus_field(&ctx, f);
    acts.extend(draw_field(&ctx, &mut a, f, Some("sample"), &mut text, vec![]));
    acts.extend(draw_field(&ctx, &mut a, f, Some("sample"), &mut text, vec![Event::Text("Dupont; @proginov.com; ".into())]));
    acts.extend(draw_field(&ctx, &mut a, f, Some("sample"), &mut text, vec![]));
    // The field left.
    ctx.memory_mut(|m| m.surrender_focus(input_id(egui_id(f))));
    for _ in 0..2 {
        acts.extend(draw_field(&ctx, &mut a, f, Some("sample"), &mut text, vec![]));
    }
    let p = a.ui.people.pieces(f).to_vec();
    assert!(p.len() == 2 && text_with(&p[0], "Dupont", None) && text_with(&p[1], "@proginov.com", None), "{p:?}");
    assert!(!asks(&acts), "nothing asked of the engine as the names were closed or the field left");
    assert!(!a.ui.people.blocked(f));
    // Ctrl+K checks them.
    focus_field(&ctx, f);
    draw_field(&ctx, &mut a, f, Some("sample"), &mut text, vec![]);
    let acts = draw_field(&ctx, &mut a, f, Some("sample"), &mut text, vec![key(Key::K, Modifiers::COMMAND)]);
    assert!(asks(&acts), "Ctrl+K asks");
    for x in acts {
        a.act(x);
    }
    for _ in 0..2 {
        draw_field(&ctx, &mut a, f, Some("sample"), &mut text, vec![]);
    }
    let p = a.ui.people.pieces(f).to_vec();
    assert!(p.len() == 2 && chip_named(&p[0], "Jean Dupont") && text_with(&p[1], "@proginov.com", None), "{p:?}");
    assert!(text.contains("jean.dupont@proginov.com") && text.contains("@proginov.com; "), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A name nothing matches, checked with Ctrl+K in a rule's field, stays as typed with no error,
/// and nothing says it wasn't found: it is looked for as text.
#[test]
fn a_rule_field_keeps_a_name_nothing_matches_without_error() {
    let (mut a, dir) = app("rule-nothing-matches");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    let f = dialog_field(0, "rule-one");
    let mut text = String::new();
    draw_field(&ctx, &mut a, f, Some(&gal), &mut text, vec![]);
    focus_field(&ctx, f);
    draw_field(&ctx, &mut a, f, Some(&gal), &mut text, vec![]);
    draw_field(&ctx, &mut a, f, Some(&gal), &mut text, vec![Event::Text("Zz".into())]);
    let notices = a.session.notices.len();
    let acts = draw_field(&ctx, &mut a, f, Some(&gal), &mut text, vec![key(Key::K, Modifiers::COMMAND)]);
    assert!(asks(&acts));
    for x in acts {
        a.act(x);
    }
    draw_field(&ctx, &mut a, f, Some(&gal), &mut text, vec![]);
    assert!(a.session.directory_searching(&gal, "Zz"), "asked of the directory");
    directory_answers(&mut a, &gal, "Zz", &[]);
    for _ in 0..2 {
        draw_field(&ctx, &mut a, f, Some(&gal), &mut text, vec![]);
    }
    let p = a.ui.people.pieces(f).to_vec();
    assert!(p.len() == 1 && text_with(&p[0], "Zz", None), "{p:?}");
    assert!(!a.ui.people.blocked(f), "nothing stops the rule");
    let said: Vec<&str> = a.session.notices[notices..].iter().map(|n| n.text.as_str()).collect();
    assert!(!said.contains(&quatrelle_engine::NAMES_NOT_FOUND), "no word of names not found: {said:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A contact group has no address to match: a rule's or a Search Folder's field refuses it, with
/// what to do.
#[test]
fn a_contact_group_is_refused_where_addresses_are_matched() {
    let (mut a, dir) = app("search-people-group");
    let f = dialog_field(0, "search-people");
    let group = Addressee { item_id: "AAMk=".into(), ..Addressee::new("Dupont family", "", AddresseeKind::ContactGroup) };
    a.ui.people.add(f, group);
    let p = a.ui.people.pieces(f);
    assert!(matches!(p, [Piece::Chip { refused: Some(why), .. }] if why == GROUP_NO_ADDRESS), "{p:?}");
    assert!(a.ui.people.blocked(f));
    let _ = std::fs::remove_dir_all(dir);
}

/// A distribution list has no calendar and no mailbox: Add calendar and Open another mailbox
/// refuse it, each in its own words.
#[test]
fn a_list_is_refused_where_calendars_open() {
    let (mut a, dir) = app("calendar-list");
    let list = Addressee::new("Team", "team@contoso.example", AddresseeKind::DistributionList);
    let calendar = dialog_field(0, "calendar");
    a.ui.people.add(calendar, list.clone());
    assert_eq!(refusal(&a, calendar, "team@contoso.example"), Some(Some(NO_LIST_CALENDAR.into())), "{:?}", a.ui.people.pieces(calendar));
    let mailbox = dialog_field(0, "mailbox");
    a.ui.people.add(mailbox, list);
    assert_eq!(refusal(&a, mailbox, "team@contoso.example"), Some(Some(NOT_A_MAILBOX.into())), "{:?}", a.ui.people.pieces(mailbox));
    assert!(a.ui.people.blocked(calendar) && a.ui.people.blocked(mailbox));
    let _ = std::fs::remove_dir_all(dir);
}

/// Open another mailbox's OK waits for the name typed: held while the directory is asked, it goes
/// once the answer is in, with the person found.
#[test]
fn a_dialog_ok_waits_for_its_names() {
    let (mut a, dir) = app("dialog-ok-waits");
    let gal = with_exchange(&mut a);
    let f = dialog_field(0, "mailbox");
    let mut text = "Ana".to_string();
    assert!(a.ui.people.gate_dialog(&mut a.session, "mailbox", Some(&gal), &mut text, false, 1.0), "held for the name");
    assert!(a.ui.people.holding(DIALOG_WINDOWS));
    assert!(a.session.directory_searching(&gal, "Ana"), "asked of the directory");
    assert!(a.ui.people.send_when_resolved(&mut a.session, 2.0).is_empty(), "still waiting");
    directory_answers(&mut a, &gal, "Ana", &[("Ana Lima", "ana@contoso.example")]);
    // The dialog's frame makes its field ready.
    a.ui.people.prepare(&mut a.session, f, Some(&gal), accepts_of("mailbox"), &text);
    assert_eq!(a.ui.people.send_when_resolved(&mut a.session, 3.0), vec![(DIALOG_WINDOWS, Ok(Value::Null))]);
    assert!(!a.ui.people.holding(DIALOG_WINDOWS));
    let who: Vec<String> = a.ui.people.addressees(f).into_iter().map(|w| w.address).collect();
    assert_eq!(who, ["ana@contoso.example"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Add calendar's OK, lenient, goes past an entry its place refuses (nothing waits); Open another
/// mailbox's, strict, stops on it with the refusal's words.
#[test]
fn a_lenient_ok_goes_with_a_refused_entry_and_a_strict_one_stops() {
    let (mut a, dir) = app("dialog-ok-lenient");
    let gal = with_exchange(&mut a);
    team_is_a_list(&mut a, &gal);
    let calendar = dialog_field(0, "calendar");
    let mut text = "ana@contoso.example; team@contoso.example".to_string();
    assert!(!a.ui.people.gate_dialog(&mut a.session, "calendar", Some(&gal), &mut text, true, 1.0), "nothing waits: it goes");
    assert!(!a.ui.people.holding(DIALOG_WINDOWS));
    assert_eq!(refusal(&a, calendar, "team@contoso.example"), Some(Some(NO_LIST_CALENDAR.into())), "{:?}", a.ui.people.pieces(calendar));
    let who: Vec<String> = a.ui.people.addressees(calendar).into_iter().map(|w| w.address).collect();
    assert_eq!(who, ["ana@contoso.example"], "the list isn't handed on");
    // Strict: the refused list stops it.
    let mut text = "team@contoso.example".to_string();
    assert!(a.ui.people.gate_dialog(&mut a.session, "mailbox", Some(&gal), &mut text, false, 2.0));
    assert_eq!(a.ui.people.send_when_resolved(&mut a.session, 3.0), vec![(DIALOG_WINDOWS, Err(NOT_A_MAILBOX.to_string()))]);
    assert!(!a.ui.people.holding(DIALOG_WINDOWS));
    // A lenient OK still waits for a name being asked.
    let mut text = "ana@contoso.example; dupont".to_string();
    assert!(a.ui.people.gate_dialog(&mut a.session, "calendar", Some(&gal), &mut text, true, 4.0), "waits for dupont");
    directory_answers(&mut a, &gal, "dupont", &[]);
    a.ui.people.prepare(&mut a.session, calendar, Some(&gal), accepts_of("calendar"), &text);
    assert_eq!(
        a.ui.people.send_when_resolved(&mut a.session, 5.0),
        vec![(DIALOG_WINDOWS, Ok(Value::Null))],
        "a name not found doesn't stop it: {:?}",
        a.ui.people.pieces(calendar)
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// After Add calendar's OK: a calendar that opened leaves the field, one that couldn't stays, red,
/// with why — and stays so when the book changes.
#[test]
fn an_entry_that_failed_stays_red_with_why_and_one_opened_goes() {
    let (mut a, dir) = app("dialog-opened-failed");
    let ctx = egui::Context::default();
    let f = dialog_field(0, "calendar");
    a.session.contacts_step(usize::MAX);
    let mut text = "ana@contoso.example; bo@contoso.example".to_string();
    draw_field(&ctx, &mut a, f, Some("sample"), &mut text, vec![]);
    assert_eq!(a.ui.people.pieces(f).len(), 2, "{:?}", a.ui.people.pieces(f));
    // Nothing to do for an address the field doesn't hold.
    a.ui.people.opened(f, "nobody@contoso.example");
    a.ui.people.failed(f, "nobody@contoso.example", "no access".into());
    a.ui.people.opened(f, "ana@contoso.example");
    a.ui.people.failed(f, "bo@contoso.example", "no access".into());
    draw_field(&ctx, &mut a, f, Some("sample"), &mut text, vec![]);
    assert!(!text.contains("ana"), "{text}");
    let failed = Piece::Text { text: "bo@contoso.example".into(), error: Some(PieceError::Failed("no access".into())) };
    assert_eq!(a.ui.people.pieces(f), std::slice::from_ref(&failed));
    assert!(a.ui.people.blocked(f));
    // The book changes: the failure stays.
    let before = a.session.contacts_revision();
    a.session.run("contact.create", &json!({"name": "Bo Berg", "emails": ["bo@contoso.example"]})).unwrap();
    a.session.contacts_step(usize::MAX);
    assert_ne!(a.session.contacts_revision(), before, "the book changed");
    for _ in 0..2 {
        draw_field(&ctx, &mut a, f, Some("sample"), &mut text, vec![]);
    }
    assert_eq!(a.ui.people.pieces(f), [failed]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A dialog's fields are numbered apart from a message's and a meeting's: they go with their
/// dialog and leave the message's alone. Reset, a dialog's field is made again from its text.
#[test]
fn a_dialog_field_is_numbered_apart_and_goes_with_its_dialog() {
    let (mut a, dir) = app("dialog-numbered-apart");
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "zoe@studio.example"}));
    let n = a.session.compose.as_ref().map_or(0, |c| c.id);
    let to = FieldId { window: n, name: "to" };
    let calendar = dialog_field(0, "calendar");
    assert_ne!(calendar.window, n);
    a.ui.people.prepare(&mut a.session, to, Some("sample"), accepts_of("to"), "zoe@studio.example");
    a.ui.people.prepare(&mut a.session, calendar, Some("sample"), accepts_of("calendar"), "ana@contoso.example");
    assert_eq!(a.ui.people.pieces(calendar).len(), 1);
    // Reset: made again from its text at the next prepare.
    a.ui.people.reset_window(DIALOG_WINDOWS);
    assert!(a.ui.people.pieces(calendar).is_empty());
    assert_eq!(a.ui.people.pieces(to).len(), 1, "the message's field stays");
    a.ui.people.prepare(&mut a.session, calendar, Some("sample"), accepts_of("calendar"), "ana@contoso.example");
    assert_eq!(a.ui.people.pieces(calendar).len(), 1);
    // The dialog closed: its field goes, the message's stays.
    a.ui.people.settle(&mut a.session, &[n, DIALOG_WINDOWS].into(), &HashSet::new());
    a.ui.people.settle(&mut a.session, &[n].into(), &HashSet::new());
    assert!(a.ui.people.pieces(calendar).is_empty());
    assert_eq!(a.ui.people.pieces(to).len(), 1);
    assert!(is_dialog_window(DIALOG_WINDOWS) && is_dialog_window(calendar.window));
    assert!(!is_dialog_window(MEETING_WINDOWS | 1) && !is_dialog_window(1 << 41) && !is_dialog_window(n));
    let _ = std::fs::remove_dir_all(dir);
}
