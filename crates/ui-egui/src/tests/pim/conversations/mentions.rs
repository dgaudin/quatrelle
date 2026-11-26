// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: @mentions in a conversation's box. "@" offers the chat's members of the user's
//! organisation (never the user, nor someone of another one), above the box; ↑ and ↓ choose,
//! Enter, Tab or a click picks: the name goes in as one line, then a space, and is sent as that
//! member's mention (byte ranges of the text); a name the user changes is text again; mentions are
//! drawn in the box as in a bubble. With `QUATRELLE_FRAME_PNG` set to a folder, the box offering
//! is painted there, light and dark.

use quatrelle_engine::model::pim::teams::Mention;
use serde_json::Value;

use super::chat::*;
use super::writing::{SIZE, enter, focus_box, in_box, one_frame, sends, typed, writing_app};
use super::*;
use crate::Action;

fn text(s: &str) -> Event {
    Event::Text(s.into())
}

fn down() -> Event {
    key(Key::ArrowDown, Modifiers::NONE)
}

fn up() -> Event {
    key(Key::ArrowUp, Modifiers::NONE)
}

fn tab() -> Event {
    key(Key::Tab, Modifiers::NONE)
}

fn backspace() -> Event {
    key(Key::Backspace, Modifiers::NONE)
}

/// The mentions a send carries (none when it has no `mentions`, as the engine reads it).
fn mentions_of(p: &Value) -> Value {
    p.get("mentions").cloned().unwrap_or_else(|| json!([]))
}

/// The people @mentioned in the open chat's box.
fn in_box_mentions(a: &MailApp) -> Vec<(std::ops::Range<usize>, String)> {
    a.ui.conversations.drafts.get(CHAT).map(|d| d.mentions.iter().map(|m| (m.chars.clone(), m.user.clone())).collect()).unwrap_or_default()
}

/// What the box's actions do in the engine (as the window runs them at the frame's end).
fn run_all(a: &mut MailApp, acts: Vec<Action>) {
    for x in acts {
        a.act(x);
    }
}

/// "@" offers the organisation's members of the chat (Ana, Bruno), not the user, not Sam of
/// another organisation; "@a" Ana alone, above the box. Enter picks her: her name and a space in
/// the box, the offer gone; Enter sends it as her mention, which the engine takes.
#[test]
fn at_offers_the_organisations_members_and_enter_picks() {
    let (mut a, dir) = writing_app("teams-at-offer", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![text("@")]);
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(has_text(&texts, "Ana Exemple") && has_text(&texts, "Bruno Rossi"), "{texts:?}");
    assert!(!has_text(&texts, "Riley Moreno") && !has_text(&texts, "Sam Okafor"), "{texts:?}");
    typed(&ctx, &mut a, vec![text("a")]);
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    let ana = rect_of(&texts, |t| t == "Ana Exemple").expect("Ana offered");
    let written = rect_of(&texts, |t| t == "@a").expect("the box's text");
    assert!(ana.bottom() <= written.top(), "above the box: {ana:?} {written:?}");
    for not in ["Bruno Rossi", "Sam Okafor", "Riley Moreno"] {
        assert!(!has_text(&texts, not), "{not}: {texts:?}");
    }
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert!(sends(&acts).is_empty(), "Enter picked: nothing sent");
    assert_eq!(in_box(&a), "Ana Exemple ");
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(!has_text(&texts, "Ana Exemple"), "the offer gone: {texts:?}");
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert_eq!(sends(&acts), vec![json!({"chat": CHAT, "text": "Ana Exemple ", "bold": [], "italic": [], "mentions": [[0, 11, "u-ana"]]})]);
    run_all(&mut a, acts);
    let sent = a.session.teams.outgoing.last().expect("taken by the engine, shown at once");
    assert_eq!(sent.mentions, vec![Mention { id: 0, text: "Ana Exemple".into(), user_id: Some("u-ana".into()) }]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A member's name with a bidi mark and a double space (Review Focus 4): offered and put in the
/// box as one line, exactly as the engine and the writer take a mention's name, so the send
/// carries the mention and the engine keeps it.
#[test]
fn a_name_with_a_bidi_mark_goes_in_as_one_line_and_stays_a_mention() {
    let (mut a, dir) = writing_app("teams-at-bidi", vec![]);
    let mut people = members();
    people[1].name = "Ana\u{202E}  Exemple".into();
    a.session.teams_apply(teams::Event::Members { chat: CHAT.into(), members: people, web_url: Some(TEAMS_LINK.into()), first: true });
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![text("@"), text("a")]);
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(has_text(&texts, "Ana Exemple"), "offered on one line: {texts:?}");
    typed(&ctx, &mut a, vec![enter()]);
    assert_eq!(in_box(&a), "Ana Exemple ");
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert_eq!(sends(&acts).iter().map(mentions_of).collect::<Vec<_>>(), vec![json!([[0, 11, "u-ana"]])]);
    run_all(&mut a, acts);
    let sent = a.session.teams.outgoing.last().expect("taken by the engine");
    assert_eq!(sent.mentions, vec![Mention { id: 0, text: "Ana Exemple".into(), user_id: Some("u-ana".into()) }]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Decision 11: a member is found by the start of their name, of a word of it, or of their
/// address, whatever the capitals; at most eight are offered, in the members' order.
#[test]
fn a_member_is_found_by_a_word_of_the_name_or_the_address() {
    let (mut a, dir) = writing_app("teams-at-match", vec![]);
    let mut people = members();
    people[2].address = "b.rossi@contoso.example".into();
    people.extend((0..12).map(|i| member(&format!("u-{i}"), &format!("Zed {i:02}"), TENANT_ID)));
    a.session.teams_apply(teams::Event::Members { chat: CHAT.into(), members: people, web_url: Some(TEAMS_LINK.into()), first: true });
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    for (query, offered) in [("@RO", vec!["Bruno Rossi"]), ("@b.r", vec!["Bruno Rossi"]), ("@ex", vec!["Ana Exemple"]), ("@zz", vec![])] {
        typed(&ctx, &mut a, vec![text(query)]);
        let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
        for name in ["Ana Exemple", "Bruno Rossi"] {
            assert_eq!(has_text(&texts, name), offered.contains(&name), "{query}: {name}: {texts:?}");
        }
        typed(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND), backspace()]);
    }
    typed(&ctx, &mut a, vec![text("@z")]);
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    let zeds: Vec<&str> = texts.iter().filter(|(t, _)| t.starts_with("Zed ")).map(|(t, _)| t.as_str()).collect();
    assert_eq!(zeds, ["Zed 00", "Zed 01", "Zed 02", "Zed 03", "Zed 04", "Zed 05", "Zed 06", "Zed 07"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Words with accents before a mention (Ruling 2): its range is sent in bytes of the text, on its
/// characters, as bold and italic are.
#[test]
fn a_mention_after_accented_words_is_sent_in_bytes() {
    let (mut a, dir) = writing_app("teams-at-bytes", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![text("été "), text("@"), text("b"), enter()]);
    assert_eq!(in_box(&a), "été Bruno Rossi ");
    assert_eq!(in_box_mentions(&a), vec![(4..15, "u-bruno".to_string())], "by character in the box");
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert_eq!(sends(&acts).iter().map(mentions_of).collect::<Vec<_>>(), vec![json!([[6, 17, "u-bruno"]])]);
    let _ = std::fs::remove_dir_all(dir);
}

/// "@" offers Ana then Bruno, in the members' order: ↓ then Tab picks Bruno, and Tab leaves the
/// keyboard in the box; ↓ then ↑ comes back to Ana.
#[test]
fn arrows_and_tab_choose_in_the_offer() {
    let (mut a, dir) = writing_app("teams-at-arrows", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![text("@"), down(), tab()]);
    assert_eq!(in_box(&a), "Bruno Rossi ");
    assert_eq!(in_box_mentions(&a), vec![(0..11, "u-bruno".to_string())]);
    typed(&ctx, &mut a, vec![text("and "), text("@"), down(), up(), tab(), text("x")]);
    assert_eq!(in_box(&a), "Bruno Rossi and Ana Exemple x", "Tab picked, the keyboard kept");
    assert_eq!(in_box_mentions(&a), vec![(0..11, "u-bruno".to_string()), (16..27, "u-ana".to_string())]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A click on a member offered picks them, and the box keeps the keyboard.
#[test]
fn a_click_on_a_member_offered_picks_them() {
    let (mut a, dir) = writing_app("teams-at-click", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![text("@")]);
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    let bruno = rect_of(&texts, |t| t == "Bruno Rossi").expect("Bruno offered");
    click_pane(&ctx, &mut a, bruno.center(), SIZE);
    assert_eq!(in_box(&a), "Bruno Rossi ");
    typed(&ctx, &mut a, vec![text("x")]);
    assert_eq!(in_box(&a), "Bruno Rossi x", "the keyboard back in the box");
    let _ = std::fs::remove_dir_all(dir);
}

/// Review Focus 4: a mention whose name the user changes is text again, never a mention over
/// other words. Backspace once takes the space after it (the mention kept), twice the name's last
/// letter: the send carries no mention. A letter typed inside a name does the same.
#[test]
fn typing_into_a_mention_makes_it_text() {
    let (mut a, dir) = writing_app("teams-at-typed-into", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![text("@"), text("a"), enter()]);
    assert_eq!(in_box_mentions(&a).len(), 1);
    typed(&ctx, &mut a, vec![backspace()]);
    assert_eq!((in_box(&a).as_str(), in_box_mentions(&a).len()), ("Ana Exemple", 1), "the space only: the mention kept");
    typed(&ctx, &mut a, vec![backspace()]);
    assert_eq!(in_box(&a), "Ana Exempl");
    assert!(in_box_mentions(&a).is_empty(), "the name changed: text");
    let acts = typed(&ctx, &mut a, vec![enter()]);
    let sent = sends(&acts);
    assert_eq!(sent.iter().map(|p| p.get("text").cloned()).collect::<Vec<_>>(), vec![Some(json!("Ana Exempl"))]);
    assert_eq!(sent.iter().map(mentions_of).collect::<Vec<_>>(), vec![json!([])]);
    // A letter typed inside the name.
    typed(&ctx, &mut a, vec![text("@"), text("b"), enter()]);
    assert_eq!(in_box(&a), "Bruno Rossi ");
    let left = key(Key::ArrowLeft, Modifiers::NONE);
    typed(&ctx, &mut a, vec![left.clone(), left.clone(), left, text("X")]);
    assert_eq!(in_box(&a), "Bruno RosXsi ");
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert_eq!(sends(&acts).iter().map(mentions_of).collect::<Vec<_>>(), vec![json!([])]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A message the engine refuses comes back in the box with its mentions, before what was typed
/// since (whose mentions move after it).
#[test]
fn a_refused_send_comes_back_with_its_mentions() {
    let (mut a, dir) = writing_app("teams-at-refused", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![text("@"), text("b"), enter()]);
    assert_eq!(in_box(&a), "Bruno Rossi ");
    a.session.teams_apply(teams::Event::Unsent { count: quatrelle_engine::model::pim::teams::OUTBOX_MAX as u32 });
    a.act(Action::Run("teams.send", json!({"chat": CHAT, "text": "Ana Exemple hi", "bold": [], "italic": [], "mentions": [[0, 11, "u-ana"]]})));
    assert_eq!(in_box(&a), "Ana Exemple hi\nBruno Rossi ");
    assert_eq!(in_box_mentions(&a), vec![(0..11, "u-ana".to_string()), (15..26, "u-bruno".to_string())]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Enter acts where it comes in the frame (P13), on the offer as the frame found it: "@" then
/// "a" and Enter in one frame sends what was typed (no offer was shown yet); with the offer shown,
/// "n" and Enter in one frame picks for "@an".
#[test]
fn enter_in_the_frame_acts_on_the_offer_shown() {
    let (mut a, dir) = writing_app("teams-at-one-frame", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    let acts = one_frame(&ctx, &mut a, vec![text("@"), text("a"), enter()], 2);
    assert_eq!(sends(&acts).iter().map(|p| p.get("text").cloned()).collect::<Vec<_>>(), vec![Some(json!("@a"))]);
    typed(&ctx, &mut a, vec![text("@"), text("a")]);
    let acts = one_frame(&ctx, &mut a, vec![text("n"), enter()], 2);
    assert!(sends(&acts).is_empty(), "picked, not sent");
    assert_eq!(in_box(&a), "Ana Exemple ");
    let _ = std::fs::remove_dir_all(dir);
}

/// Mentions are drawn in the box as in a bubble: the name in the accent on its soft tint, the
/// words around it as typed.
#[test]
fn mentions_are_highlighted_in_the_box() {
    let (mut a, dir) = writing_app("teams-at-highlight", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![text("Hi "), text("@"), text("a"), enter()]);
    assert_eq!(in_box(&a), "Hi Ana Exemple ");
    let shapes = frame_shapes(&ctx, &mut a, vec![], egui::vec2(SIZE.0 as f32, SIZE.1 as f32));
    let t = a.ui.tokens;
    let galley = shapes
        .iter()
        .find_map(|s| match s {
            egui::Shape::Text(x) if x.galley.text() == "Hi Ana Exemple " => Some(x.galley.clone()),
            _ => None,
        })
        .expect("the box's text");
    let looks: Vec<(&str, egui::Color32, egui::Color32)> = galley
        .job
        .sections
        .iter()
        .filter_map(|x| Some((galley.job.text.get(x.byte_range.start.0..x.byte_range.end.0)?, x.format.color, x.format.background)))
        .collect();
    let plain = egui::Color32::TRANSPARENT;
    assert_eq!(looks, [("Hi ", t.text, plain), ("Ana Exemple", t.accent, t.accent_soft), (" ", t.text, plain)]);
    let _ = std::fs::remove_dir_all(dir);
}

/// The box offering two members, light and dark: the offer above the box, inside its panel.
#[test]
fn the_offer_in_light_and_dark() {
    let messages = vec![said("m1", user("u-ana", "Ana Exemple"), NOW - 600, para(vec![words("Who has the figures?")]))];
    let (mut a, dir) = writing_app("teams-at-frames", messages);
    let mut people = members();
    // A guest of the organisation: offered, tagged External.
    let mut guest = member("u-gale", "Gale Stone", TENANT_ID);
    guest.roles = vec!["guest".into()];
    people.push(guest);
    for m in &mut people {
        m.address = format!("{}@contoso.example", m.name.to_lowercase().replace(' ', "."));
    }
    a.session.teams_apply(teams::Event::Members { chat: CHAT.into(), members: people, web_url: Some(TEAMS_LINK.into()), first: true });
    let ctx = egui::Context::default();
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    let hint = settled_text_at(&ctx, &mut a, "Write a message", size).expect("the box");
    click_at(&ctx, &mut a, hint, 1, size);
    for e in [text("Thanks "), text("@")] {
        frame_sized(&ctx, &mut a, vec![e], size);
    }
    let pictures_dir = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut tex = Textures::default();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..6 {
            frame_with_size(&ctx, &mut a, &mut tex, vec![], None, SIZE);
        }
        let png = pictures_dir.as_ref().map(|d| d.join(format!("teams-box-offer-{theme}.png")));
        let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, SIZE);
        let texts = texts_of(&shapes);
        let written = rect_of(&texts, |t| t == "Thanks @").expect("the box's text");
        let bubble = rect_of(&texts, |t| t == "Who has the figures?").expect("the message");
        let ana = texts.iter().filter(|(t, _)| t == "Ana Exemple").map(|(_, r)| *r).max_by(|x, y| x.top().total_cmp(&y.top())).expect("Ana");
        let bruno = rect_of(&texts, |t| t == "Bruno Rossi").expect("Bruno");
        assert!(bubble.bottom() <= ana.top(), "{theme}: the offer under the messages: {bubble:?} {ana:?}");
        assert!(ana.bottom() <= bruno.top() && bruno.bottom() <= written.top(), "{theme}: {ana:?} {bruno:?} {written:?}");
        let address = rect_of(&texts, |t| t == "ana.exemple@contoso.example").expect("Ana's address");
        assert!(address.left() > ana.right() && (address.center().y - ana.center().y).abs() < 4.0, "{theme}: after the name: {address:?}");
        let gale = rect_of(&texts, |t| t == "Gale Stone").expect("the guest offered");
        let tag = texts.iter().filter(|(t, _)| t == "External").map(|(_, r)| *r).find(|r| (r.center().y - gale.center().y).abs() < 6.0);
        assert!(tag.is_some_and(|r| r.right() <= gale.left()), "{theme}: External before the guest: {tag:?} {gale:?}");
    }
    // Picked: the mention in the box, as a bubble draws one.
    a.ui.dark = Some(false);
    frame_sized(&ctx, &mut a, vec![enter()], size);
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..6 {
            frame_with_size(&ctx, &mut a, &mut tex, vec![], None, SIZE);
        }
        let png = pictures_dir.as_ref().map(|d| d.join(format!("teams-box-mention-{theme}.png")));
        let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, SIZE);
        let texts = texts_of(&shapes);
        assert!(has_text(&texts, "Thanks Ana Exemple "), "{theme}: {texts:?}");
        assert!(!has_text(&texts, "Bruno Rossi"), "{theme}: the offer gone");
    }
    a.ui.dark = Some(false);
    let _ = std::fs::remove_dir_all(dir);
}

/// Security review 1.5 (content n° 4): a guest of the organisation can be mentioned, and is
/// offered with "External" before their name, so they don't pass for the colleague they're named
/// like; a member of the organisation has no mark.
#[test]
fn a_guest_is_offered_with_external() {
    let (mut a, dir) = writing_app("teams-at-guest", vec![]);
    let mut guest = member("u-gale", "Ana Exemple Guest", TENANT_ID);
    guest.roles = vec!["guest".into()];
    let all = members().into_iter().chain([guest]).collect();
    a.session.teams_apply(teams::Event::Members { chat: CHAT.into(), members: all, web_url: Some(TEAMS_LINK.into()), first: true });
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![text("@ana")]);
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    let (ana, gale) =
        (rect_of(&texts, |t| t == "Ana Exemple").expect("Ana offered"), rect_of(&texts, |t| t == "Ana Exemple Guest").expect("the guest offered"));
    let on = |row: egui::Rect| texts.iter().filter(|(t, _)| t == "External").map(|(_, r)| *r).find(|r| (r.center().y - row.center().y).abs() < 6.0);
    assert!(on(gale).is_some_and(|r| r.right() <= gale.left()), "External before the guest's name: {texts:?}");
    assert!(on(ana).is_none(), "not on a member of the organisation: {texts:?}");
    let _ = std::fs::remove_dir_all(dir);
}
