// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the MailTips strip over a message being written — what the server says about its
//! recipients, asked once per set as they change, shown as text, and nothing that holds it back.

use quatrelle_engine::model::Job;
use quatrelle_engine::model::scheduling::MailTip;

use super::*;

/// The jobs handed to the backend (`NullBackend` drops them; every frame hands them over).
type Handed = std::rc::Rc<std::cell::RefCell<Vec<Job>>>;

struct Recorder(Handed);

impl crate::Backend for Recorder {
    fn submit(&mut self, job: Job) {
        self.0.borrow_mut().push(job);
    }

    fn poll(&mut self) -> Vec<MailEvent> {
        Vec::new()
    }
}

/// The text drawn in the last frame, every shape taken apart.
fn texts(shapes: &[egui::Shape]) -> Vec<String> {
    shapes.iter().filter_map(|s| if let egui::Shape::Text(t) = s { Some(t.galley.text().to_string()) } else { None }).collect()
}

/// A message written from an Exchange account asks what the server says about its recipients
/// once, draws the answer over the form (an automatic reply, a full mailbox…), asks again when a
/// recipient is added or taken away, and never holds the message back.
#[test]
fn the_message_shows_what_the_server_says_about_its_recipients() {
    let (mut a, dir) = app("mail-tips-strip");
    let handed = Handed::default();
    a.backend = Box::new(Recorder(handed.clone()));
    let account = with_exchange(&mut a);
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({"account": account}));
    a.run("compose.update", json!({"to": "Ana <ana@contoso.example>"}));
    frame(&ctx, &mut a, vec![]);
    let asked = std::mem::take(&mut *handed.borrow_mut());
    let (ask, recipients) = match asked.iter().find(|j| matches!(j, Job::GetMailTips { .. })) {
        Some(Job::GetMailTips { ask, recipients, from, .. }) => {
            assert_eq!(from, "riley@contoso.example");
            (*ask, recipients.clone())
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(recipients, ["ana@contoso.example"], "asked about the recipients, once");
    // Another frame asks nothing more: this set was asked about.
    frame(&ctx, &mut a, vec![]);
    assert!(handed.borrow().iter().all(|j| !matches!(j, Job::GetMailTips { .. })), "one request per set");
    // The answer: the tip is drawn over the form, as words.
    a.session.apply(MailEvent::MailTips {
        account: account.clone(),
        ask,
        tips: vec![MailTip {
            email: "ana@contoso.example".into(),
            automatic_replies: "Away until Monday".into(),
            mailbox_full: true,
            max_message_size: 1,
            ..Default::default()
        }],
        error: None,
    });
    let drawn = texts(&frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0)));
    let line = drawn.iter().find(|t| t.contains("Away until Monday")).unwrap_or_else(|| panic!("{drawn:?}"));
    assert!(line.contains("ana@contoso.example") && line.contains("Mailbox full"), "{line}");
    assert!(line.contains("larger than the 1 B this recipient accepts"), "the message is larger than the 1 B it accepts: {line}");
    // Nothing holds the message back: it's only something told.
    assert!(a.session.compose.as_ref().is_some_and(|c| !c.sending));
    // A recipient added: asked again, and the strip from the previous set is gone until the answer.
    a.run("compose.update", json!({"cc": "bo@contoso.example"}));
    frame(&ctx, &mut a, vec![]);
    let again = std::mem::take(&mut *handed.borrow_mut());
    let (ask2, recipients) = match again.iter().find(|j| matches!(j, Job::GetMailTips { .. })) {
        Some(Job::GetMailTips { ask, recipients, .. }) => (*ask, recipients.clone()),
        other => panic!("{other:?}"),
    };
    assert_eq!(recipients, ["ana@contoso.example", "bo@contoso.example"]);
    assert_ne!(ask2, ask);
    let drawn = texts(&frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0)));
    assert!(!drawn.iter().any(|t| t.contains("Away until Monday")), "the previous set's tips are let go: {drawn:?}");
    // The answer to the new set.
    a.session.apply(MailEvent::MailTips {
        account,
        ask: ask2,
        tips: vec![MailTip { email: "bo@contoso.example".into(), custom: "Ring first".into(), ..Default::default() }],
        error: None,
    });
    let drawn = texts(&frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0)));
    assert!(drawn.iter().any(|t| t.contains("Ring first")), "{drawn:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The sample mailbox has no server that answers MailTips: writing a message there asks nothing
/// and draws no strip.
#[test]
fn a_message_from_an_account_with_no_server_for_tips_asks_nothing() {
    let (mut a, dir) = app("mail-tips-none");
    let handed = Handed::default();
    a.backend = Box::new(Recorder(handed.clone()));
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ana@soundlab.example"}));
    frame(&ctx, &mut a, vec![]);
    assert!(handed.borrow().iter().all(|j| !matches!(j, Job::GetMailTips { .. })), "{:?}", handed.borrow());
    assert!(a.session.mail_tips_now(a.session.compose.as_ref().unwrap().id).is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// Where the message's text `text` is drawn, if it is.
fn drawn_at(shapes: &[egui::Shape], text: &str) -> Option<egui::Rect> {
    shapes.iter().find_map(|s| match s {
        egui::Shape::Text(t) if t.galley.text() == text => Some(t.galley.rect.translate(t.pos.to_vec2())),
        _ => None,
    })
}

/// The MailTips asked for so far, taken.
fn asked(handed: &Handed) -> Vec<Vec<String>> {
    std::mem::take(&mut *handed.borrow_mut())
        .into_iter()
        .filter_map(|j| match j {
            Job::GetMailTips { recipients, .. } => Some(recipients),
            _ => None,
        })
        .collect()
}

/// An address typed letter by letter asks nothing while it's typed (`ana@c` is an address
/// already, and each letter asked the server again); the field left, it's a recipient, and the
/// server is asked about it once.
#[test]
fn an_address_being_typed_asks_nothing_until_it_is_written() {
    let (mut a, dir) = app("mail-tips-typing");
    let handed = Handed::default();
    a.backend = Box::new(Recorder(handed.clone()));
    let account = with_exchange(&mut a);
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({"account": account}));
    frame(&ctx, &mut a, vec![]);
    let to = quatrelle_engine::FieldId { window: a.session.compose.as_ref().map_or(0, |c| c.id), name: "to" };
    ctx.memory_mut(|m| m.request_focus(crate::look::people_field::input_id(crate::dialogs::people_field::egui_id(to))));
    frame(&ctx, &mut a, vec![]);
    for c in "ana@contoso.example".chars() {
        frame(&ctx, &mut a, vec![Event::Text(c.to_string())]);
    }
    assert!(a.session.compose.as_ref().is_some_and(|c| c.to.contains("ana@contoso.example")), "typed");
    let typing = asked(&handed);
    assert!(typing.is_empty(), "nothing asked while it's typed: {typing:?}");
    // The field left: one question, about it.
    frame(&ctx, &mut a, vec![key(Key::Tab, Modifiers::NONE)]);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(asked(&handed), [vec!["ana@contoso.example".to_string()]], "asked once it's written");
    let _ = std::fs::remove_dir_all(dir);
}

/// The strip is drawn only when it has a line to say: a size limit the message stays under (what
/// Exchange gives for nearly every recipient) draws nothing, the text where it was. A hundred
/// recipients with long tips keep it a few lines high, the rest scrolled to.
#[test]
fn the_strip_says_only_what_there_is_to_say_in_a_few_lines() {
    let (mut a, dir) = app("mail-tips-empty");
    let handed = Handed::default();
    a.backend = Box::new(Recorder(handed.clone()));
    let account = with_exchange(&mut a);
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 900.0);
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({"account": account}));
    a.run("compose.update", json!({"to": "ana@contoso.example", "body": "Hello there"}));
    let mut shapes = Vec::new();
    for _ in 0..3 {
        shapes = frame_shapes(&ctx, &mut a, vec![], size);
    }
    let alone = drawn_at(&shapes, "Hello there").unwrap_or_else(|| panic!("the text: {:?}", texts(&shapes)));
    let answer = |a: &mut MailApp, tips: Vec<MailTip>| {
        let ask = a.session.compose.as_ref().and_then(|c| a.session.mail_tips_now(c.id)).map_or(0, |s| s.ask);
        a.session.apply(MailEvent::MailTips { account: account.clone(), ask, tips, error: None });
    };
    answer(&mut a, vec![MailTip { email: "ana@contoso.example".into(), max_message_size: 10_485_760, ..Default::default() }]);
    for _ in 0..3 {
        shapes = frame_shapes(&ctx, &mut a, vec![], size);
    }
    let with_limit = drawn_at(&shapes, "Hello there").unwrap_or_else(|| panic!("the text: {:?}", texts(&shapes)));
    assert!((with_limit.top() - alone.top()).abs() < 0.5, "no empty strip over the text: {alone:?} then {with_limit:?}");
    // A hundred recipients (their field taller), each with a long tip.
    let people: Vec<String> = (0..100).map(|n| format!("p{n}@contoso.example")).collect();
    a.run("compose.update", json!({"to": people.join("; ")}));
    for _ in 0..3 {
        shapes = frame_shapes(&ctx, &mut a, vec![], size);
    }
    let alone = drawn_at(&shapes, "Hello there").unwrap_or_else(|| panic!("the text: {:?}", texts(&shapes)));
    let long = "Away for the season, writing back once home again. ".repeat(20);
    answer(&mut a, people.iter().map(|p| MailTip { email: p.clone(), custom: long.clone(), ..Default::default() }).collect());
    for _ in 0..3 {
        shapes = frame_shapes(&ctx, &mut a, vec![], size);
    }
    assert!(texts(&shapes).iter().any(|t| t.contains("Away for the season")), "the tips are drawn");
    let under = drawn_at(&shapes, "Hello there").map_or(f32::INFINITY, |r| r.top());
    assert!(under - alone.top() < 150.0, "a few lines high: the text moved {} down", under - alone.top());
    let _ = std::fs::remove_dir_all(dir);
}
