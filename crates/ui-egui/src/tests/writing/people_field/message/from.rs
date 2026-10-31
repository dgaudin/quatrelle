//! From's other address (`reading/sender.rs`): a one-entry contacts field in the From menu, a
//! person, a shared mailbox or a list, checked on Ctrl+K and never as it's typed.

use super::*;
use crate::dialogs::people_field::accepts_of;

/// The message's From menu, as last drawn: its Other email address section is open.
fn menu_open(ctx: &egui::Context, a: &mut MailApp) -> bool {
    settled_text_at(ctx, a, "Other email address", SIZE).is_some()
}

fn from(a: &MailApp) -> String {
    a.session.compose.as_ref().map(|c| c.from.clone()).unwrap_or_default()
}

/// Click Send from it.
fn send_from_it(ctx: &egui::Context, a: &mut MailApp) {
    let send = settled_text_at(ctx, a, "Send from it", SIZE).expect("Send from it");
    click_at(ctx, a, send, 1, SIZE);
}

/// A local contact typed in part, its suggestion clicked: the chip is there, and the menu (a
/// click on the list is outside it) stays open; Send from it takes the one picked.
#[test]
fn a_suggestion_clicked_in_from_is_taken() {
    let (mut a, dir) = app("from-suggestion");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    open_other_address(&ctx, &mut a, SIZE);
    let f = field(&a, "from");
    frame(&ctx, &mut a, vec![Event::Text("zo".into())]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.people.open_field(), Some(f), "the field has the keyboard");
    let list = crate::look::people_field::list_id(egui_id(f));
    let mut rect = None;
    for _ in 0..20 {
        frame(&ctx, &mut a, vec![]);
        rect = ctx.memory(|m| m.area_rect(list));
    }
    let row = rect.map(|r| r.left_top() + egui::vec2(20.0, 20.0)).expect("the suggestions show");
    click_at(&ctx, &mut a, row, 1, SIZE);
    let pieces = a.ui.people.pieces(f).to_vec();
    assert!(matches!(pieces.as_slice(), [p] if chip_named(p, "Zoe Park")), "{pieces:?}");
    assert!(menu_open(&ctx, &mut a), "the menu stays open");
    send_from_it(&ctx, &mut a);
    assert_eq!(from(&a), "Zoe Park <zoe@studio.example>");
    assert!(!menu_open(&ctx, &mut a), "the menu closed");
    let _ = std::fs::remove_dir_all(dir);
}

/// From holds one entry: a second address replaces the first. A room (the sample's Sound Lab,
/// known once its account's rooms are) can't send: its chip is red with why, and Send from it
/// takes nothing.
#[test]
fn from_holds_one_entry_and_refuses_a_room() {
    let (mut a, dir) = app("from-one");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.session.request_rooms("sample", false).unwrap();
    assert!(a.session.rooms_now("sample").is_some(), "the sample's rooms are known");
    a.run("mail.new", json!({}));
    open_other_address(&ctx, &mut a, SIZE);
    let f = field(&a, "from");
    frame(&ctx, &mut a, vec![Event::Text("ann@one.example;".into())]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("bob@two.example;".into())]);
    frame(&ctx, &mut a, vec![]);
    let pieces = a.ui.people.pieces(f).to_vec();
    assert!(matches!(pieces.as_slice(), [Piece::Chip { who, refused: None }] if who.address == "bob@two.example"), "{pieces:?}");
    assert_eq!(a.ui.people.left_out(f), None, "the first isn't left out: it's replaced");
    frame(&ctx, &mut a, vec![Event::Text("sound.lab@studio.example;".into())]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let pieces = a.ui.people.pieces(f).to_vec();
    let why = "Send from a person, a shared mailbox or a list";
    assert!(
        matches!(pieces.as_slice(), [Piece::Chip { who, refused: Some(r) }] if who.address == "sound.lab@studio.example" && r == why),
        "{pieces:?}"
    );
    assert!(a.ui.people.blocked(f));
    send_from_it(&ctx, &mut a);
    assert_eq!(from(&a), "", "nothing taken");
    assert!(menu_open(&ctx, &mut a), "the menu is still open");
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+K in the From field checks that field's name, not the message's: To keeps the name it
/// has as it was (Check Names would have written it out).
#[test]
fn ctrl_k_in_from_checks_that_field_only() {
    let (mut a, dir) = app("from-ctrl-k");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "Zoe Park"}));
    frame(&ctx, &mut a, vec![]);
    open_other_address(&ctx, &mut a, SIZE);
    let f = field(&a, "from");
    frame(&ctx, &mut a, vec![Event::Text("Print Shop".into())]);
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "Zoe Park", "Check Names didn't run");
    let pieces = a.ui.people.pieces(f).to_vec();
    assert!(matches!(pieces.as_slice(), [p] if chip_named(p, "Print Shop")), "From's name was checked: {pieces:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The From menu closed with its field holding the keyboard (its button clicked again): the
/// field isn't drawn any more, so it has let go, and Ctrl+K in the message checks its names.
#[test]
fn ctrl_k_checks_the_message_again_once_from_is_closed() {
    let (mut a, dir) = app("from-closed");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "Zoe Park"}));
    let button = open_other_address(&ctx, &mut a, SIZE);
    let f = field(&a, "from");
    frame(&ctx, &mut a, vec![Event::Text("x".into())]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.people.open_field(), Some(f));
    click_at(&ctx, &mut a, button, 1, SIZE);
    assert!(!menu_open(&ctx, &mut a), "the menu closed");
    assert_eq!(a.ui.people.open_field(), None, "the field let go");
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "Zoe Park <zoe@studio.example>; ", "Check Names ran");
    let _ = std::fs::remove_dir_all(dir);
}

/// A name several people in the directory fit, in From: the choice dialog asks which one (over
/// the menu, which stays open), and the one picked is the field's chip, taken with Send from it.
#[test]
fn a_name_several_fit_in_from_is_asked_with_the_menu_open() {
    let (mut a, dir) = app("from-choice");
    let gal = with_exchange(&mut a);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    open_other_address(&ctx, &mut a, SIZE);
    let f = field(&a, "from");
    frame(&ctx, &mut a, vec![Event::Text("dupont".into())]);
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    directory_answers(&mut a, &gal, "dupont", &[("Anne Dupont", "anne.dupont@contoso.example"), ("Marc Dupont", "marc.dupont@contoso.example")]);
    assert!(settled_text_at(&ctx, &mut a, "More than one person fits “dupont”. Choose who you meant:", SIZE).is_some(), "the choice is asked");
    let marc = settled_text_at(&ctx, &mut a, "Marc Dupont", SIZE).expect("Marc offered");
    click_at(&ctx, &mut a, marc, 2, SIZE);
    frame(&ctx, &mut a, vec![]);
    let pieces = a.ui.people.pieces(f).to_vec();
    assert!(matches!(pieces.as_slice(), [p] if chip_named(p, "Marc Dupont")), "{pieces:?}");
    assert!(menu_open(&ctx, &mut a), "the menu stays open");
    send_from_it(&ctx, &mut a);
    assert_eq!(from(&a), "Marc Dupont <marc.dupont@contoso.example>");
    assert!(a.session.compose.as_ref().is_some_and(|c| c.to.is_empty()), "nothing went to To");
    let _ = std::fs::remove_dir_all(dir);
}

/// A text fed into a one-entry field with several entries (a paste that reached it whole, a text
/// set from outside) keeps the last one, and the others aren't counted as left out.
#[test]
fn a_one_entry_field_fed_several_keeps_the_last() {
    let (mut a, dir) = app("from-fed");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let f = field(&a, "from");
    a.ui.people.prepare(&mut a.session, f, None, accepts_of("from"), "ann@one.example; Bob <bob@two.example>");
    let pieces = a.ui.people.pieces(f).to_vec();
    assert!(matches!(pieces.as_slice(), [p] if chip_named(p, "Bob")), "{pieces:?}");
    assert_eq!(a.ui.people.left_out(f), None);
    let _ = std::fs::remove_dir_all(dir);
}
