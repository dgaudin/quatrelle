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
            ..Default::default()
        }],
        error: None,
    });
    let drawn = texts(&frame_shapes(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0)));
    let line = drawn.iter().find(|t| t.contains("Away until Monday")).unwrap_or_else(|| panic!("{drawn:?}"));
    assert!(line.contains("ana@contoso.example") && line.contains("Mailbox full"), "{line}");
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
