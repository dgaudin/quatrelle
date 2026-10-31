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

/// A window tall enough for the dialogs with an address book.
const SIZE: egui::Vec2 = egui::vec2(1280.0, 900.0);

fn frames(ctx: &egui::Context, a: &mut MailApp, n: usize) {
    for _ in 0..n {
        frame_sized(ctx, a, vec![], SIZE);
    }
}

/// The dialog `d` open over the main window, a few frames on.
fn open_dialog(ctx: &egui::Context, a: &mut MailApp, d: Dialog) {
    frames(ctx, a, 1);
    a.ui.dialog = Some(d);
    frames(ctx, a, 3);
}

/// Type `text` in the dialog's field, its input given the keyboard first; a few frames after.
fn type_in(ctx: &egui::Context, a: &mut MailApp, field: FieldId, text: &str) {
    focus_field(ctx, field);
    frame_sized(ctx, a, vec![], SIZE);
    frame_sized(ctx, a, vec![Event::Text(text.into())], SIZE);
    frames(ctx, a, 3);
}

/// The dialog's button (or anything drawn on top) reading `label`, clicked.
fn press(ctx: &egui::Context, a: &mut MailApp, label: &str) {
    let at = settled_text_at(ctx, a, label, SIZE).unwrap_or_else(|| panic!("{label} is drawn"));
    click_at(ctx, a, at, 1, SIZE);
}

/// Whether the button `label` can be pressed, as a screen reader is told.
fn enabled(ctx: &egui::Context, a: &mut MailApp, label: &str) -> Option<bool> {
    ctx.enable_accesskit();
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    nodes.iter().find(|(_, n)| n.role() == egui::accesskit::Role::Button && n.label() == Some(label)).map(|(_, n)| !n.is_disabled())
}

/// The other people's calendars `account` has open.
fn shared(a: &MailApp, account: &str) -> Vec<String> {
    a.session.pim.get(account).map(|p| p.shared.clone()).unwrap_or_default()
}

fn calendar_open(a: &MailApp) -> bool {
    matches!(a.ui.dialog, Some(Dialog::OpenSharedCalendar(_)))
}

/// Add calendar takes several people: each is opened once; a list it refuses and the account's
/// own address stay, red, and the dialog with them. Pressed again, nothing is opened twice.
/// Taken out, they leave the field empty, with Open off.
#[test]
fn add_calendar_opens_each_entry_once_and_keeps_the_one_refused() {
    let (mut a, dir) = app("add-calendar-several");
    let gal = with_exchange(&mut a);
    team_is_a_list(&mut a, &gal);
    let ctx = egui::Context::default();
    open_dialog(&ctx, &mut a, Dialog::open_calendars());
    let f = dialog_field(0, "calendar");
    type_in(&ctx, &mut a, f, "ana@contoso.example; bo@contoso.example; team@contoso.example; riley@contoso.example; ");
    assert_eq!(enabled(&ctx, &mut a, "Open"), Some(true));
    press(&ctx, &mut a, "Open");
    assert_eq!(shared(&a, &gal), ["ana@contoso.example", "bo@contoso.example"]);
    assert!(calendar_open(&a), "the dialog stays for the entries left");
    let own = Piece::Text { text: "riley@contoso.example".into(), error: Some(PieceError::Failed("that's this account's own calendar".into())) };
    let left = a.ui.people.pieces(f).to_vec();
    assert_eq!(left.len(), 2, "{left:?}");
    assert_eq!(refusal(&a, f, "team@contoso.example"), Some(Some(NO_LIST_CALENDAR.into())), "{left:?}");
    assert_eq!(left[1], own);
    // Pressed again: nothing opened twice, the red entries as they were.
    press(&ctx, &mut a, "Open");
    assert_eq!(shared(&a, &gal), ["ana@contoso.example", "bo@contoso.example"]);
    assert_eq!(a.ui.people.pieces(f), left.as_slice());
    // Both taken out (Backspace selects the last, a second removes it).
    focus_field(&ctx, f);
    frames(&ctx, &mut a, 1);
    for _ in 0..4 {
        frame_sized(&ctx, &mut a, vec![key(Key::Backspace, Modifiers::NONE)], SIZE);
    }
    frames(&ctx, &mut a, 2);
    assert!(a.ui.people.pieces(f).is_empty(), "{:?}", a.ui.people.pieces(f));
    assert_eq!(enabled(&ctx, &mut a, "Open"), Some(false));
    assert!(calendar_open(&a));
    let _ = std::fs::remove_dir_all(dir);
}

/// A name typed in Add calendar: Open waits for the directory ("Checking names…", nothing
/// opened), then opens the person found and closes.
#[test]
fn add_calendar_waits_for_a_name_then_opens_it() {
    let (mut a, dir) = app("add-calendar-waits");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    open_dialog(&ctx, &mut a, Dialog::open_calendars());
    // A name the sample's people don't know (its Ana Lima would settle "Ana" at once).
    type_in(&ctx, &mut a, dialog_field(0, "calendar"), "Dupont");
    press(&ctx, &mut a, "Open");
    assert!(a.ui.people.holding(DIALOG_WINDOWS), "held for the name");
    assert!(settled_text_at(&ctx, &mut a, "Checking names…", SIZE).is_some(), "said in the dialog");
    assert!(shared(&a, &gal).is_empty(), "nothing opened yet");
    directory_answers(&mut a, &gal, "Dupont", &[("Ana Dupont", "ana@contoso.example")]);
    frames(&ctx, &mut a, 2);
    assert_eq!(shared(&a, &gal), ["ana@contoso.example"], "the name answered is opened: {:?}", a.ui.dialog.is_some());
    assert!(a.ui.dialog.is_none(), "and the dialog closed");
    let _ = std::fs::remove_dir_all(dir);
}

/// Add calendar's address book: the person picked goes into the field with "Calendar ->", or with
/// a double-click on their row.
#[test]
fn add_calendar_takes_a_person_from_its_book() {
    let (mut a, dir) = app("add-calendar-book");
    for (name, email) in [("Jean Dupont", "jean.dupont@proginov.com"), ("Jeanne Martin", "jeanne.martin@proginov.com")] {
        a.session.run("contact.create", &json!({"name": name, "emails": [email]})).unwrap();
    }
    a.session.contacts_step(usize::MAX);
    let ctx = egui::Context::default();
    open_dialog(&ctx, &mut a, Dialog::open_calendars());
    // The search box has the keyboard as the dialog opens.
    frame_sized(&ctx, &mut a, vec![Event::Text("Jean".into())], SIZE);
    frames(&ctx, &mut a, 3);
    let f = dialog_field(0, "calendar");
    press(&ctx, &mut a, "Jean Dupont");
    press(&ctx, &mut a, "Calendar ->");
    frames(&ctx, &mut a, 2);
    let p = a.ui.people.pieces(f).to_vec();
    assert!(p.len() == 1 && chip_named(&p[0], "Jean Dupont"), "{p:?}");
    let row = settled_text_at(&ctx, &mut a, "Jeanne Martin", SIZE).expect("the second row");
    click_at(&ctx, &mut a, row, 2, SIZE);
    frames(&ctx, &mut a, 2);
    let p = a.ui.people.pieces(f).to_vec();
    assert!(p.len() == 2 && chip_named(&p[1], "Jeanne Martin"), "{p:?}");
    let Some(Dialog::OpenSharedCalendar(form)) = &a.ui.dialog else { panic!("the dialog is open") };
    assert!(form.text.contains("jean.dupont@proginov.com") && form.text.contains("jeanne.martin@proginov.com"), "{}", form.text);
    let _ = std::fs::remove_dir_all(dir);
}

fn open_mailbox(account: &str) -> Dialog {
    Dialog::OpenSharedMailbox { account: account.into(), email: String::new(), calendar: false, error: None }
}

fn mailboxes(a: &MailApp, account: &str) -> Vec<String> {
    a.session.account(account).map(|x| x.config.shared_mailboxes.clone()).unwrap_or_default()
}

/// Open another mailbox's field: a name typed, Enter waits for the directory, then the mailbox
/// found opens and the dialog closes.
#[test]
fn open_mailbox_waits_for_a_typed_name() {
    let (mut a, dir) = app("open-mailbox-waits");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    open_dialog(&ctx, &mut a, open_mailbox(&gal));
    type_in(&ctx, &mut a, dialog_field(0, "mailbox"), "Support");
    frame_sized(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)], SIZE);
    frames(&ctx, &mut a, 1);
    assert!(matches!(a.ui.dialog, Some(Dialog::OpenSharedMailbox { .. })), "still open");
    assert!(a.ui.people.holding(DIALOG_WINDOWS), "held for the name");
    assert!(mailboxes(&a, &gal).is_empty());
    directory_answers(&mut a, &gal, "Support", &[("Support", "support@contoso.example")]);
    frames(&ctx, &mut a, 2);
    assert_eq!(mailboxes(&a, &gal), ["support@contoso.example"]);
    assert!(a.ui.dialog.is_none(), "opened, and the dialog closed");
    let _ = std::fs::remove_dir_all(dir);
}

/// A list is no mailbox: Open another mailbox stops on it, says why, and opens nothing.
#[test]
fn open_mailbox_stops_on_a_list_and_says_why() {
    let (mut a, dir) = app("open-mailbox-list");
    let gal = with_exchange(&mut a);
    team_is_a_list(&mut a, &gal);
    let ctx = egui::Context::default();
    open_dialog(&ctx, &mut a, open_mailbox(&gal));
    type_in(&ctx, &mut a, dialog_field(0, "mailbox"), "team@contoso.example");
    press(&ctx, &mut a, "Open");
    frames(&ctx, &mut a, 2);
    let Some(Dialog::OpenSharedMailbox { error, .. }) = &a.ui.dialog else { panic!("the dialog stays open") };
    assert_eq!(error.as_deref(), Some(NOT_A_MAILBOX));
    assert!(mailboxes(&a, &gal).is_empty(), "nothing opened");
    let _ = std::fs::remove_dir_all(dir);
}
