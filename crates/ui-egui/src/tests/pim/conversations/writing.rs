// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: writing in a conversation. The box at its foot (Enter sends, Shift+Enter makes a new
//! line, Ctrl+B and Ctrl+I make bold and italic, markup typed goes as text, a message too long for
//! Teams or with too many waiting is kept and said, one the engine refuses goes back in it), none
//! for a chat left or unreadable; the messages going out, "Sending…" or "Not sent" with Retry and
//! Delete; and Sign out's question counting what wasn't sent. With `QUATRELLE_FRAME_PNG` set to a
//! folder, the pane is painted there, light and dark.

use quatrelle_engine::model::pim::teams::{ChatMessage, OUTBOX_CHAT_MAX, OUTBOX_MAX, OutKind, OutState, Outgoing, Unsent};
use serde_json::Value;

use super::chat::*;
use super::*;
use crate::Action;

pub(super) const SIZE: (usize, usize) = (1280, 800);

/// The support chat open, with `messages`.
pub(super) fn writing_app(name: &str, messages: Vec<ChatMessage>) -> (MailApp, std::path::PathBuf) {
    let (a, dir, _) = chat_app(name, group_row(), TEAMS_LINK, messages);
    (a, dir)
}

/// The box given the keyboard: a click on its hint.
pub(super) fn focus_box(ctx: &egui::Context, a: &mut MailApp) {
    settled_at(ctx, a, SIZE);
    let (_, texts) = pane(ctx, a, vec![], SIZE);
    let hint = rect_of(&texts, |t| t == "Write a message").expect("the box and its hint");
    click_pane(ctx, a, hint.center(), SIZE);
}

/// `events` given to the pane a frame each, then one more: what it asked for.
pub(super) fn typed(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Vec<Action> {
    let mut acts = Vec::new();
    for e in events {
        acts.extend(pane(ctx, a, vec![e], SIZE).0);
    }
    acts.extend(pane(ctx, a, vec![], SIZE).0);
    acts
}

/// The params of the messages the box sent.
pub(super) fn sends(acts: &[Action]) -> Vec<Value> {
    acts.iter()
        .filter_map(|x| match x {
            Action::Run("teams.send", p) => Some(p.clone()),
            _ => None,
        })
        .collect()
}

/// The params of the commands `id` asked for.
pub(super) fn asked(acts: &[Action], id: &str) -> Vec<Value> {
    acts.iter()
        .filter_map(|x| match x {
            Action::Run(i, p) if *i == id => Some(p.clone()),
            _ => None,
        })
        .collect()
}

/// What the box of the open chat holds.
pub(super) fn in_box(a: &MailApp) -> String {
    a.ui.conversations.drafts.get(CHAT).map(|d| d.text.clone()).unwrap_or_default()
}

pub(super) fn enter() -> Event {
    key(Key::Enter, Modifiers::NONE)
}

fn outgoing(seq: u64, local: &str, text: &str, state: OutState) -> Outgoing {
    Outgoing {
        seq,
        local: local.into(),
        chat: CHAT.into(),
        body: para(vec![words(text)]),
        mentions: Vec::new(),
        what: OutKind::Send,
        state,
        parts: Vec::new(),
    }
}

fn outbox(items: Vec<Outgoing>) -> teams::Event {
    teams::Event::Outbox { chat: CHAT.into(), items, took: None }
}

/// "hi", Shift+Enter, "there", Enter: one message of two lines, nothing bold; the box empty after
/// and still with the keyboard.
#[test]
fn enter_sends_and_shift_enter_makes_a_new_line() {
    let (mut a, dir) = writing_app("teams-box-enter", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    let acts = typed(&ctx, &mut a, vec![Event::Text("hi".into()), key(Key::Enter, Modifiers::SHIFT), Event::Text("there".into()), enter()]);
    assert_eq!(sends(&acts), vec![json!({"chat": CHAT, "text": "hi\nthere", "bold": [], "italic": []})]);
    assert_eq!(in_box(&a), "", "the box empty after");
    typed(&ctx, &mut a, vec![Event::Text("next".into())]);
    assert_eq!(in_box(&a), "next", "the box kept the keyboard");
    let _ = std::fs::remove_dir_all(dir);
}

/// "hello" (and "été", whose ranges are in bytes) selected, Ctrl+B: sent bold, end to end; Ctrl+I
/// with nothing selected makes what's typed next italic.
#[test]
fn ctrl_b_makes_the_selection_bold_and_it_goes_so() {
    for (word, bytes) in [("hello", 5), ("été", 5)] {
        let (mut a, dir) = writing_app("teams-box-bold", vec![]);
        let ctx = egui::Context::default();
        focus_box(&ctx, &mut a);
        let acts = typed(&ctx, &mut a, vec![Event::Text(word.into()), key(Key::A, Modifiers::COMMAND), key(Key::B, Modifiers::COMMAND), enter()]);
        assert_eq!(sends(&acts), vec![json!({"chat": CHAT, "text": word, "bold": [[0, bytes]], "italic": []})], "{word}");
        let _ = std::fs::remove_dir_all(dir);
    }
    let (mut a, dir) = writing_app("teams-box-italic", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    let acts = typed(
        &ctx,
        &mut a,
        vec![
            Event::Text("so ".into()),
            key(Key::I, Modifiers::COMMAND),
            Event::Text("very".into()),
            key(Key::I, Modifiers::COMMAND),
            Event::Text(" ".into()),
            enter(),
        ],
    );
    assert_eq!(sends(&acts), vec![json!({"chat": CHAT, "text": "so very ", "bold": [], "italic": [[3, 7]]})]);
    let _ = std::fs::remove_dir_all(dir);
}

/// 90 KiB pasted (Review Focus 2): the box says it's too long for Teams, Enter sends nothing, and
/// the text is still there.
#[test]
fn a_message_too_long_is_kept_and_said() {
    let (mut a, dir) = writing_app("teams-box-long", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    let long = "words & <more> ".repeat(90 * 1024 / 15 + 1);
    assert!(long.len() >= 90 * 1024);
    typed(&ctx, &mut a, vec![Event::Paste(long.clone())]);
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(
        has_text(&texts, "This message is too long for Teams"),
        "{:?}",
        texts.iter().map(|(t, _)| t.chars().take(40).collect::<String>()).collect::<Vec<_>>()
    );
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert!(sends(&acts).is_empty(), "nothing sent");
    assert_eq!(in_box(&a), long, "the text kept");
    let _ = std::fs::remove_dir_all(dir);
}

/// Markup typed is the text sent, as it was typed: nothing is made bold of it.
#[test]
fn markup_typed_is_text() {
    let (mut a, dir) = writing_app("teams-box-markup", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    let acts = typed(&ctx, &mut a, vec![Event::Text("<b>x</b>".into()), enter()]);
    assert_eq!(sends(&acts), vec![json!({"chat": CHAT, "text": "<b>x</b>", "bold": [], "italic": []})]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Spaces, blank lines or control characters alone are nothing to send: Enter leaves them.
#[test]
fn a_blank_message_isnt_sent() {
    let (mut a, dir) = writing_app("teams-box-blank", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    let acts = typed(&ctx, &mut a, vec![Event::Text("  ".into()), key(Key::Enter, Modifiers::SHIFT), Event::Text("\u{7}\u{1b}".into()), enter()]);
    assert!(sends(&acts).is_empty(), "{:?}", sends(&acts));
    let _ = std::fs::remove_dir_all(dir);
}

/// A frame's events in one go, then `more` frames: what the pane asked for.
pub(super) fn one_frame(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>, more: usize) -> Vec<Action> {
    let mut acts = pane(ctx, a, events, SIZE).0;
    for _ in 0..more {
        acts.extend(pane(ctx, a, vec![], SIZE).0);
    }
    acts
}

/// Enter acts where it comes in the frame's events (P13): what was typed before it in the same
/// frame goes with the message, what follows it stays in the box.
#[test]
fn enter_sends_what_was_typed_before_it_in_the_same_frame() {
    let (mut a, dir) = writing_app("teams-box-order-enter", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    let acts = one_frame(&ctx, &mut a, vec![Event::Text("o".into()), Event::Text("k".into()), enter()], 2);
    assert_eq!(sends(&acts), vec![json!({"chat": CHAT, "text": "ok", "bold": [], "italic": []})]);
    assert_eq!(in_box(&a), "", "nothing left behind");
    typed(&ctx, &mut a, vec![Event::Text("first".into())]);
    let acts = one_frame(&ctx, &mut a, vec![enter(), Event::Text("x".into())], 2);
    assert_eq!(sends(&acts), vec![json!({"chat": CHAT, "text": "first", "bold": [], "italic": []})]);
    assert_eq!(in_box(&a), "x", "typed after Enter: for the box");
    // A paste and Enter in one frame: the paste goes.
    let acts = one_frame(&ctx, &mut a, vec![Event::Paste(" and more".into()), enter()], 2);
    assert_eq!(sends(&acts), vec![json!({"chat": CHAT, "text": "x and more", "bold": [], "italic": []})]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+B and Ctrl+I act where they come in the frame's events: on the text and the selection as
/// they stand then.
#[test]
fn ctrl_b_and_ctrl_i_act_where_they_come_in_the_frame() {
    let (mut a, dir) = writing_app("teams-box-order-bold", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    let acts =
        one_frame(&ctx, &mut a, vec![Event::Text("hello".into()), key(Key::A, Modifiers::COMMAND), key(Key::B, Modifiers::COMMAND), enter()], 3);
    assert_eq!(sends(&acts), vec![json!({"chat": CHAT, "text": "hello", "bold": [[0, 5]], "italic": []})]);
    let events = vec![
        Event::Text("so ".into()),
        key(Key::I, Modifiers::COMMAND),
        Event::Text("very".into()),
        key(Key::I, Modifiers::COMMAND),
        Event::Text(" ".into()),
        enter(),
    ];
    let acts = one_frame(&ctx, &mut a, events, 5);
    assert_eq!(sends(&acts), vec![json!({"chat": CHAT, "text": "so very ", "bold": [], "italic": [[3, 7]]})]);
    let _ = std::fs::remove_dir_all(dir);
}

/// The outbox full (I3), for this chat or in all: Enter does nothing, the text stays and the box
/// says why; room again, Enter sends it.
#[test]
fn too_many_waiting_keeps_the_text() {
    let (mut a, dir) = writing_app("teams-box-full", vec![]);
    let ctx = egui::Context::default();
    focus_box(&ctx, &mut a);
    typed(&ctx, &mut a, vec![Event::Text("hello".into())]);
    let waiting = |n: u64| (1..=n).map(|i| outgoing(i, &format!("l-{i}"), "wait", OutState::Waiting)).collect::<Vec<_>>();
    for (chat_full, all) in [(true, 1), (false, OUTBOX_MAX as u32)] {
        let n = if chat_full { OUTBOX_CHAT_MAX as u64 } else { 1 };
        a.session.teams_apply(outbox(waiting(n)));
        a.session.teams_apply(teams::Event::Unsent { count: all });
        let acts = typed(&ctx, &mut a, vec![enter()]);
        assert!(sends(&acts).is_empty(), "chat full {chat_full}: nothing sent");
        assert_eq!(in_box(&a), "hello", "chat full {chat_full}: the text kept");
        let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
        assert!(has_text(&texts, "Too many messages are waiting"), "chat full {chat_full}");
    }
    a.session.teams_apply(outbox(vec![]));
    a.session.teams_apply(teams::Event::Unsent { count: 0 });
    let acts = typed(&ctx, &mut a, vec![enter()]);
    assert_eq!(sends(&acts).len(), 1, "room again: sent");
    let _ = std::fs::remove_dir_all(dir);
}

/// A message the engine refuses all the same goes back in its box, its bold and italic too, and
/// why is said under the box in the interface's language, never in the engine's English; a
/// refusal the window has no sentence of its own for is said in its own words.
#[test]
fn a_refused_message_goes_back_in_its_box() {
    let (mut a, dir) = writing_app("teams-box-refused", vec![]);
    let ctx = egui::Context::default();
    a.ui.language = "fr".into();
    settled_at(&ctx, &mut a, SIZE);
    // The worker says the outbox is full only once the box let the message go.
    a.session.teams_apply(teams::Event::Unsent { count: OUTBOX_MAX as u32 });
    a.act(Action::Run("teams.send", json!({"chat": CHAT, "text": "été ok", "bold": [[0, 5]], "italic": [[6, 8]]})));
    let d = a.ui.conversations.drafts.get(CHAT).expect("the box's draft");
    assert_eq!(d.text, "été ok");
    let bold: Vec<bool> = (0..6).map(|i| d.styles.style(i).bold).collect();
    let italic: Vec<bool> = (0..6).map(|i| d.styles.style(i).italic).collect();
    assert_eq!(bold, [true, true, true, false, false, false]);
    assert_eq!(italic, [false, false, false, false, true, true]);
    assert!(a.session.notices.iter().all(|n| !n.text.contains("Too many")), "no notice in English");
    a.session.teams_apply(teams::Event::Unsent { count: 0 });
    let texts = settled_at(&ctx, &mut a, SIZE);
    let said = crate::i18n::t("Too many messages are waiting");
    assert_ne!(said, "Too many messages are waiting", "translated in French");
    assert!(has_text(&texts, said), "{texts:?}");
    // Typed before the refusal came back: kept after the message.
    a.ui.conversations.drafts.get_mut(CHAT).expect("draft").text = "more".into();
    a.session.teams_apply(teams::Event::Unsent { count: OUTBOX_MAX as u32 });
    a.act(Action::Run("teams.send", json!({"chat": CHAT, "text": "first", "bold": [], "italic": []})));
    assert_eq!(in_box(&a), "first\nmore");
    // The chat gone from the list in between: its draft keeps the text, and nothing is said in
    // English.
    a.act(Action::Run("teams.send", json!({"chat": "19:gone@thread.v2", "text": "lost?"})));
    let gone = a.ui.conversations.drafts.get("19:gone@thread.v2").expect("its draft");
    assert_eq!(gone.text, "lost?");
    assert!(a.session.notices.iter().all(|n| !n.text.contains("isn't listed")), "no notice in English");
    // Delete refused while the message may be in Teams already: said under the box, in French.
    a.session.teams_apply(teams::Event::Unsent { count: 0 });
    a.run("teams.send", json!({"chat": CHAT, "text": "going"}));
    let local = a.session.teams.outgoing.last().map(|o| o.local.clone()).expect("shown at once");
    a.act(Action::Run("teams.discard", json!({"seq": 0, "local": local})));
    let texts = settled_at(&ctx, &mut a, SIZE);
    let said = crate::i18n::t("This message may already be in Teams");
    assert_ne!(said, "This message may already be in Teams", "translated in French");
    assert!(has_text(&texts, said), "{texts:?}");
    assert!(a.session.notices.iter().all(|n| !n.text.contains("already be in Teams")), "no notice in English");
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let _ = std::fs::remove_dir_all(dir);
}

/// A message Teams refused: "Not sent", why and Teams' code; Retry and Delete name it by its
/// number, or by its local id for one the outbox refused to take (number 0).
#[test]
fn a_not_sent_message_offers_retry_and_delete() {
    let (mut a, dir) = writing_app("teams-out-not-sent", vec![]);
    let ctx = egui::Context::default();
    let refused = OutState::NotSent { reason: Unsent::Refused, code: Some("HTTP 403 Forbidden".into()) };
    a.session.teams_apply(outbox(vec![outgoing(7, "l-7", "Hello there", refused)]));
    settled_at(&ctx, &mut a, SIZE);
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    for w in ["Hello there", "Not sent", "Teams refused this message (HTTP 403 Forbidden)", "Retry", "Delete"] {
        assert!(has_text(&texts, w), "{w}: {texts:?}");
    }
    let retry = rect_of(&texts, |t| t == "Retry").expect("Retry");
    assert_eq!(asked(&click_pane(&ctx, &mut a, retry.center(), SIZE), "teams.retry"), vec![json!({"seq": 7})]);
    let delete = rect_of(&texts, |t| t == "Delete").expect("Delete");
    assert_eq!(asked(&click_pane(&ctx, &mut a, delete.center(), SIZE), "teams.discard"), vec![json!({"seq": 7})]);
    // Refused by the outbox: known by its local id alone.
    let too_many = OutState::NotSent { reason: Unsent::TooMany, code: None };
    a.session.teams_apply(outbox(vec![outgoing(0, "l-0", "Later", too_many)]));
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    assert!(has_text(&texts, "Too many messages are waiting"), "{texts:?}");
    let retry = rect_of(&texts, |t| t == "Retry").expect("Retry");
    assert_eq!(asked(&click_pane(&ctx, &mut a, retry.center(), SIZE), "teams.retry"), vec![json!({"seq": 0, "local": "l-0"})]);
    // Each reason in its words.
    for (reason, said) in [
        (Unsent::TooLong, "Too long for Teams"),
        (Unsent::Left, "You're no longer in this conversation"),
        (Unsent::Unconfirmed, "Quatrelle couldn't confirm it was sent"),
    ] {
        a.session.teams_apply(outbox(vec![outgoing(9, "l-9", "Again", OutState::NotSent { reason, code: None })]));
        let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
        assert!(has_text(&texts, said), "{reason:?}: {texts:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A message being sent (Review Focus 5), or sent from here and not yet taken by the outbox, says
/// "Sending…" and offers neither Delete nor Retry: it may be in Teams already. One waiting in the
/// outbox (P12) can be deleted before it goes: stuck offline, it needn't fail first.
#[test]
fn a_message_being_sent_offers_no_delete() {
    let (mut a, dir) = writing_app("teams-out-sending", vec![]);
    let ctx = egui::Context::default();
    a.session.teams_apply(outbox(vec![outgoing(3, "l-3", "First", OutState::Waiting), outgoing(4, "l-4", "Second", OutState::Sending)]));
    settled_at(&ctx, &mut a, SIZE);
    a.run("teams.send", json!({"chat": CHAT, "text": "Third"}));
    let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
    for w in ["First", "Second", "Third"] {
        assert!(has_text(&texts, w), "{w}: {texts:?}");
    }
    assert_eq!(texts.iter().filter(|(t, _)| t == "Sending…").count(), 3, "{texts:?}");
    assert!(!has_text(&texts, "Retry"), "{texts:?}");
    // Delete once: under the one waiting in the outbox, above the one being sent.
    let deletes = texts_with(&texts, "Delete");
    let (first, second) = (rect_of(&texts, |t| t == "First").expect("First"), rect_of(&texts, |t| t == "Second").expect("Second"));
    assert_eq!(deletes.len(), 1, "{texts:?}");
    assert!(deletes[0].top() > first.bottom() && deletes[0].bottom() < second.top(), "{deletes:?} {first:?} {second:?}");
    assert_eq!(asked(&click_pane(&ctx, &mut a, deletes[0].center(), SIZE), "teams.discard"), vec![json!({"seq": 3})]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A chat the user left, or one Teams won't let be read, has no box.
#[test]
fn no_box_for_a_left_or_unreadable_chat() {
    let ctx = egui::Context::default();
    for (r, shown) in [(group_row(), true), (ChatRow { left: true, ..group_row() }, false), (ChatRow { unreadable: true, ..group_row() }, false)] {
        let what = format!("left {}, unreadable {}", r.left, r.unreadable);
        let (mut a, dir, _) = chat_app("teams-box-none", r, TEAMS_LINK, vec![]);
        settled_at(&ctx, &mut a, SIZE);
        let (_, texts) = pane(&ctx, &mut a, vec![], SIZE);
        assert_eq!(has_text(&texts, "Write a message"), shown, "{what}: {texts:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Sign out's question says how many messages not sent will be lost; with none, nothing.
#[test]
fn the_sign_out_question_counts_the_unsent() {
    for (count, said) in [(3, Some("3 messages not sent yet will be lost")), (0, None)] {
        let (mut a, dir, _, _) = teams_app("teams-sign-out-unsent", true);
        a.session.teams_apply(teams::Event::Unsent { count });
        let ctx = egui::Context::default();
        let size = egui::vec2(PAGE.0 as f32, PAGE.1 as f32);
        a.act(crate::Action::Backstage(crate::backstage::Page::Options));
        let at = settled_text_at(&ctx, &mut a, "Sign out", size).expect("Sign out");
        click_at(&ctx, &mut a, at, 1, size);
        settled_text_at(&ctx, &mut a, "Keep the copy", size).expect("the question");
        let texts: Vec<String> = frame_shapes(&ctx, &mut a, vec![], size)
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect();
        match said {
            Some(said) => assert!(has(&texts, said), "{count}: {texts:?}"),
            None => assert!(!texts.iter().any(|t| t.contains("not sent yet")), "{count}: {texts:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The pane with the box written in (bold and italic), a message being sent, one not sent and one
/// waiting its turn, light and dark.
#[test]
fn the_box_and_what_goes_out_in_light_and_dark() {
    let messages = vec![
        said("m1", user("u-ana", "Ana Lima"), NOW - 600, para(vec![words("Can you send the figures?")])),
        said("m2", user(ME, "Riley Moreno"), NOW - 300, para(vec![words("On it")])),
    ];
    let (mut a, dir) = writing_app("teams-box-frames", messages);
    let refused = OutState::NotSent { reason: Unsent::Refused, code: Some("HTTP 403 Forbidden".into()) };
    a.session.teams_apply(outbox(vec![
        outgoing(5, "l-5", "Here they are", OutState::Sending),
        outgoing(6, "l-6", "And the chart", refused),
        outgoing(7, "l-7", "Then the notes", OutState::Waiting),
    ]));
    let ctx = egui::Context::default();
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    let hint = settled_text_at(&ctx, &mut a, "Write a message", size).expect("the box");
    click_at(&ctx, &mut a, hint, 1, size);
    for e in [
        Event::Text("Totals ".into()),
        key(Key::B, Modifiers::COMMAND),
        Event::Text("by Friday".into()),
        key(Key::B, Modifiers::COMMAND),
        key(Key::I, Modifiers::COMMAND),
        Event::Text(" at the latest".into()),
    ] {
        frame_sized(&ctx, &mut a, vec![e], size);
    }
    assert_eq!(in_box(&a), "Totals by Friday at the latest");
    let pictures_dir = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
    let mut tex = Textures::default();
    for (dark, theme) in [(false, "light"), (true, "dark")] {
        a.ui.dark = Some(dark);
        for _ in 0..6 {
            frame_with_size(&ctx, &mut a, &mut tex, vec![], None, SIZE);
        }
        let png = pictures_dir.as_ref().map(|d| d.join(format!("teams-box-{theme}.png")));
        let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, SIZE);
        let texts = texts_of(&shapes);
        for w in ["Here they are", "And the chart", "Then the notes", "Sending…", "Not sent", "Retry", "Delete", "Totals by Friday at the latest"] {
            assert!(has_text(&texts, w), "{theme}: {w}: {texts:?}");
        }
        // The box under the messages, what goes out after the chat's own.
        let on_it = rect_of(&texts, |t| t == "On it").expect("the user's message");
        let going = rect_of(&texts, |t| t == "Here they are").expect("the one going out");
        let typed = rect_of(&texts, |t| t == "Totals by Friday at the latest").expect("the box's text");
        assert!(on_it.bottom() <= going.top() && going.bottom() <= typed.top(), "{theme}: {on_it:?} {going:?} {typed:?}");
        assert!(going.right() > 1000.0, "{theme}: on the user's side: {going:?}");
        // "Not sent" right under its message, Retry right under it.
        let chart = rect_of(&texts, |t| t == "And the chart").expect("the one not sent");
        let not_sent = rect_of(&texts, |t| t == "Not sent").expect("Not sent");
        let retry = rect_of(&texts, |t| t == "Retry").expect("Retry");
        assert!(not_sent.top() >= chart.bottom() && not_sent.top() - chart.bottom() < 30.0, "{theme}: {chart:?} {not_sent:?}");
        assert!(retry.top() >= not_sent.bottom() && retry.top() - not_sent.bottom() < 30.0, "{theme}: {not_sent:?} {retry:?}");
        // The box draws its text's bold and italic.
        let galley = shapes
            .iter()
            .find_map(|s| match s {
                egui::Shape::Text(t) if t.galley.text() == "Totals by Friday at the latest" => Some(t.galley.clone()),
                _ => None,
            })
            .expect("the box's text");
        let looks: Vec<(&str, bool, bool)> = galley
            .job
            .sections
            .iter()
            .filter_map(|x| {
                let part = galley.job.text.get(x.byte_range.start.0..x.byte_range.end.0)?;
                Some((part, x.format.font_id == crate::theme::bold(14.0), x.format.italics))
            })
            .collect();
        assert_eq!(looks, [("Totals ", false, false), ("by Friday", true, false), (" at the latest", false, true)], "{theme}");
    }
    a.ui.dark = Some(false);
    let _ = std::fs::remove_dir_all(dir);
}

/// Where the control named `label` is, in the middle of it.
fn control_at(ctx: &egui::Context, a: &mut MailApp, label: &str) -> Option<egui::Pos2> {
    ctx.enable_accesskit();
    let input =
        RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE.0 as f32, SIZE.1 as f32))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    let b = nodes.iter().find(|(_, n)| n.label() == Some(label)).and_then(|(_, n)| n.bounds())?;
    Some(egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
}

/// The emoji button opens the mail editor's picker; the emoji clicked goes in at the cursor, and
/// it's the first of the recent ones.
#[test]
fn the_emoji_button_puts_one_at_the_cursor() {
    let (mut a, dir) = writing_app("teams-box-emoji", vec![]);
    let ctx = egui::Context::default();
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    let hint = settled_text_at(&ctx, &mut a, "Write a message", size).expect("the box");
    click_at(&ctx, &mut a, hint, 1, size);
    frame_sized(&ctx, &mut a, vec![Event::Text("Thanks ".into())], size);
    let button = control_at(&ctx, &mut a, "Emoji").expect("the emoji button");
    click_at(&ctx, &mut a, button, 1, size);
    let smiley = settled_text_at(&ctx, &mut a, "😀", size).expect("the picker's smileys");
    click_at(&ctx, &mut a, smiley, 1, size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert_eq!(in_box(&a), "Thanks 😀");
    assert_eq!(a.session.prefs.recent_emoji.first().map(String::as_str), Some("😀"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Drafts are kept for 20 chats at most: past them the one written in longest ago goes, an
/// empty one first.
#[test]
fn drafts_are_kept_for_twenty_chats_an_empty_one_first() {
    let mut c = crate::conversations::ConversationsUi::default();
    let refusal = quatrelle_engine::CmdError::Disabled("Too many messages are waiting".into());
    let chat = |i: usize| format!("19:c{i:02}@thread.v2");
    let send = |c: &mut crate::conversations::ConversationsUi, chat: &str| {
        crate::conversations::refused(c, None, "teams.send", &json!({"chat": chat, "text": "kept"}), &refusal);
    };
    for i in 0..19 {
        send(&mut c, &chat(i));
    }
    // A Delete refused in a chat whose box is empty: its draft, empty, is the newest.
    crate::conversations::refused(&mut c, Some("19:empty@thread.v2"), "teams.discard", &json!({"seq": 3}), &refusal);
    assert_eq!(c.drafts.len(), 20);
    send(&mut c, &chat(19));
    assert_eq!(c.drafts.len(), 20);
    assert!(!c.drafts.contains_key("19:empty@thread.v2"), "the empty one went first");
    send(&mut c, &chat(20));
    assert_eq!(c.drafts.len(), 20);
    assert!(!c.drafts.contains_key(&chat(0)) && c.drafts.contains_key(&chat(1)), "then the oldest");
    assert!(c.drafts.values().all(|d| d.text == "kept"));
}

/// The box having the keyboard, no key sends the mail being written (P13): Ctrl+Enter and Alt+S
/// are the mail's Send, and a reply in a conversation is never an e-mail.
#[test]
fn ctrl_enter_in_the_box_never_sends_the_mail_being_written() {
    let (mut a, dir) = writing_app("teams-box-no-mail", vec![]);
    let ctx = egui::Context::default();
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Hi", "body": "Hello"}));
    a.session.module = quatrelle_engine::Module::Conversations;
    let hint = settled_text_at(&ctx, &mut a, "Write a message", size).expect("the box");
    click_at(&ctx, &mut a, hint, 1, size);
    frame_sized(&ctx, &mut a, vec![Event::Text("draft".into())], size);
    assert_eq!(in_box(&a), "draft", "the box has the keyboard");
    for (k, m) in [(Key::Enter, Modifiers::COMMAND), (Key::S, Modifiers::ALT)] {
        frame_sized(&ctx, &mut a, vec![Event::ModifiersChanged(m), key(k, m)], size);
        frame_sized(&ctx, &mut a, vec![Event::ModifiersChanged(Modifiers::NONE)], size);
        assert!(a.session.compose.as_ref().is_some_and(|c| !c.sending), "{k:?}: the mail wasn't sent");
    }
    assert_eq!(in_box(&a), "draft", "nor the box's text");
    let _ = std::fs::remove_dir_all(dir);
}

/// B-I1: in Conversations none of the mail's fields is on screen, so no key there sends the mail
/// being written, wherever the keyboard is: on nothing, on a message's bubble, in the
/// conversations' search.
#[test]
fn in_conversations_no_key_sends_the_mail_being_written() {
    let bubble = said("m1", user("u-ana", "Ana Lima"), NOW - 60, para(vec![words("Can you look at the agenda?")]));
    let (mut a, dir) = writing_app("teams-no-mail-anywhere", vec![bubble]);
    let ctx = egui::Context::default();
    let size = egui::vec2(SIZE.0 as f32, SIZE.1 as f32);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Hi", "body": "Hello"}));
    a.session.module = quatrelle_engine::Module::Conversations;
    let send_keys = |a: &mut MailApp, place: &str| {
        for (k, m) in [(Key::Enter, Modifiers::COMMAND), (Key::S, Modifiers::ALT)] {
            frame_sized(&ctx, a, vec![Event::ModifiersChanged(m), key(k, m)], size);
            frame_sized(&ctx, a, vec![Event::ModifiersChanged(Modifiers::NONE)], size);
            assert!(a.session.compose.as_ref().is_some_and(|c| !c.sending), "{place}, {k:?}: the mail wasn't sent");
        }
    };
    let text = settled_text_at(&ctx, &mut a, "Can you look at the agenda?", size).expect("the bubble");
    send_keys(&mut a, "nothing has the keyboard");
    click_at(&ctx, &mut a, text, 1, size);
    send_keys(&mut a, "a bubble clicked");
    a.ui.conversations.focus_search = true;
    frame_sized(&ctx, &mut a, vec![], size);
    frame_sized(&ctx, &mut a, vec![Event::Text("plan".into())], size);
    assert_eq!(a.ui.conversations.query, "plan", "the search has the keyboard");
    send_keys(&mut a, "in the search");
    let _ = std::fs::remove_dir_all(dir);
}

/// The engine's refusals are known by the constants it exports (P12), each said in the window's
/// words: too long, too many waiting, the chat left, a Delete while it may be in Teams; any other
/// in the box's own.
#[test]
fn the_engines_refusals_are_said_in_the_windows_words() {
    let (mut a, dir) = writing_app("teams-box-refusals", vec![]);
    let said = |a: &MailApp, chat: &str| a.ui.conversations.drafts.get(chat).and_then(|d| d.said());
    let long = "<&> ".repeat(30 * 1024);
    a.act(Action::Run("teams.send", json!({"chat": CHAT, "text": long})));
    assert_eq!(said(&a, CHAT), Some("This message is too long for Teams"));
    a.session.teams_apply(teams::Event::Unsent { count: OUTBOX_MAX as u32 });
    a.act(Action::Run("teams.send", json!({"chat": CHAT, "text": "hi"})));
    assert_eq!(said(&a, CHAT), Some("Too many messages are waiting"));
    a.session.teams_apply(teams::Event::Unsent { count: 0 });
    a.run("teams.send", json!({"chat": CHAT, "text": "going"}));
    let local = a.session.teams.outgoing.last().map(|o| o.local.clone()).expect("shown at once");
    a.act(Action::Run("teams.discard", json!({"seq": 0, "local": local})));
    assert_eq!(said(&a, CHAT), Some("This message may already be in Teams"));
    a.session.teams_apply(teams::Event::Chats { rows: vec![ChatRow { left: true, ..group_row() }] });
    a.act(Action::Run("teams.send", json!({"chat": CHAT, "text": "still there?"})));
    assert_eq!(said(&a, CHAT), Some("You're no longer in this conversation"));
    a.act(Action::Run("teams.send", json!({"chat": "19:gone@thread.v2", "text": "lost?"})));
    assert_eq!(said(&a, "19:gone@thread.v2"), Some("This message couldn't be sent"));
    let _ = std::fs::remove_dir_all(dir);
}
