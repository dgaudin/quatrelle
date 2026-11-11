// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the scam check as it's shown. Options › Assistant (the box, the accounts, the line
//! naming the server, the cap, the senders taken as safe, the test on examples), the banner in the
//! reading pane with its reasons below the sender check's, « Not a scam », and the mark in the list.

use quatrelle_engine::model::AccountConfig;
use quatrelle_engine::model::scam::{ScamDoc, Verdict};

use super::senders::{SIZE, deliver, settle, spoken};
use super::*;

const SCAM_JSON: &str = "scam.json";

fn texts(shapes: &[egui::Shape]) -> Vec<String> {
    shapes.iter().filter_map(|s| if let egui::Shape::Text(t) = s { Some(t.galley.text().to_string()) } else { None }).collect()
}

/// The sample mailbox and one Exchange account of Zoe's beside it.
fn app_with_exchange(name: &str) -> (MailApp, std::path::PathBuf) {
    app_with_backend(name, Box::new(NullBackend))
}

/// What a backend was asked to do, kept for the test to read.
#[derive(Clone, Default)]
struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<quatrelle_engine::model::Job>>>);

impl crate::Backend for Recorder {
    fn submit(&mut self, job: quatrelle_engine::model::Job) {
        if let Ok(mut jobs) = self.0.lock() {
            jobs.push(job);
        }
    }
    fn poll(&mut self) -> Vec<quatrelle_engine::model::Event> {
        Vec::new()
    }
}

fn app_with_backend(name: &str, backend: Box<dyn crate::Backend>) -> (MailApp, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("quatrelle-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    sample::seed_if_empty(&store, NOW).unwrap();
    let mut accounts = store.load_accounts().unwrap();
    accounts.push(AccountConfig {
        id: "work".into(),
        name: "Zoe Park".into(),
        email: "zoe@work.example".into(),
        username: "zoe".into(),
        ews: Some("https://mail.work.example/EWS/Exchange.asmx".into()),
        ..Default::default()
    });
    accounts.push(AccountConfig {
        id: "home".into(),
        name: "Zoe Park".into(),
        email: "zoe.park@home.example".into(),
        username: "zoe.park".into(),
        imap: quatrelle_engine::model::ServerConfig { host: "imap.home.example".into(), ..Default::default() },
        ..Default::default()
    });
    store.save_accounts(&accounts).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    (MailApp::new(s, backend, Box::new(NullServices(NOW))), dir)
}

fn verdict(score: u8, reasons: &[&str]) -> Verdict {
    Verdict { score, reasons: reasons.iter().map(|r| r.to_string()).collect(), model: "glm".into(), at: NOW, judged: Vec::new() }
}

/// Keep the verdicts for the messages `kept` in the sample account's `scam.json`, before the app has
/// read it.
fn keep(a: &MailApp, kept: &[(&MessageKey, Verdict)], trusted: &[&str]) {
    let mut doc = ScamDoc::default();
    for (key, v) in kept {
        let id = a.session.summary(key).map(|m| m.message_id.clone()).expect("the message is listed");
        doc.keep(&id, v.clone());
    }
    for t in trusted {
        doc.trust(t);
    }
    a.session.store.as_ref().unwrap().save_account_doc("sample", SCAM_JSON, &doc).unwrap();
}

fn open_options(a: &mut MailApp) {
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
}

const TALL: egui::Vec2 = egui::vec2(1280.0, 5000.0);

/// The option greyed with a line until a server and a model are set; then the line naming the
/// server's host alone (never a path, a key or a port's user), the Exchange account ticked and the
/// other greyed with its reason, the cap, and the senders taken as safe with Remove.
#[test]
fn the_scam_option_is_greyed_without_a_model_and_lists_the_accounts() {
    let (mut a, dir) = app_with_exchange("scam-option");
    let ctx = egui::Context::default();
    // A sender taken as safe earlier, in the Exchange account's document.
    a.session
        .store
        .as_ref()
        .unwrap()
        .save_account_doc("work", SCAM_JSON, &{
            let mut d = ScamDoc::default();
            d.trust("sam.doe@elsewhere.example");
            d
        })
        .unwrap();
    frame(&ctx, &mut a, vec![]);
    open_options(&mut a);
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], TALL));
    assert!(shown.iter().any(|t| t == "Spot scams in the external mail that arrives"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "Set a server and a model above first."), "greyed with its line: {shown:?}");
    assert!(!shown.iter().any(|t| t.starts_with("External mail from unknown senders")), "no server yet: {shown:?}");
    // Server and model set: the host is named, nothing else of the address.
    a.run("assistant.settings", json!({"url": "https://user:hunter2@llm.studio.example:8443/v1/secret?k=1", "model": "glm"}));
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], TALL));
    assert!(!shown.iter().any(|t| t == "Set a server and a model above first."), "{shown:?}");
    let line = shown.iter().find(|t| t.starts_with("External mail from unknown senders")).cloned().unwrap_or_default();
    assert_eq!(line, "External mail from unknown senders is sent to llm.studio.example to be read.");
    assert!(!shown.iter().any(|t| t.contains("hunter2") || t.contains("/v1/secret") || t.contains("8443")), "no key, no path: {shown:?}");
    // Accounts: those with a server by their address (Exchange and IMAP alike); the sample, kept
    // on this computer only, isn't listed.
    assert!(shown.iter().any(|t| t == "zoe@work.example"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "zoe.park@home.example"), "the IMAP account can be ticked: {shown:?}");
    let sample = a.session.accounts.iter().find(|x| x.is_local()).map(|x| x.config.email.clone()).unwrap_or_default();
    assert!(!sample.is_empty() && !shown.contains(&sample), "the sample receives no mail: {shown:?}");
    assert!(shown.iter().any(|t| t == "At most 200 messages a day"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "Today: 0 messages checked, 0 flagged"), "what it did today: {shown:?}");
    assert!(shown.iter().any(|t| t == "Try it on examples"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "sam.doe@elsewhere.example"), "the safe sender is listed: {shown:?}");
    assert!(shown.iter().any(|t| t == "Remove"), "{shown:?}");
    // The box turned on is saved through the command, with the Exchange account ticked.
    a.run("assistant.scamCheck", json!({"enabled": true, "dailyCap": 50}));
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], TALL));
    assert!(shown.iter().any(|t| t == "At most 50 messages a day"), "{shown:?}");
    assert!(a.session.prefs.scam_check && a.session.prefs.scam_accounts.contains("work"));
    // Remove takes the sender back.
    let at = settled_text_at(&ctx, &mut a, "Remove", TALL).expect("a Remove button");
    click_at(&ctx, &mut a, at, 1, TALL);
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], TALL));
    assert!(!shown.iter().any(|t| t == "sam.doe@elsewhere.example"), "removed: {shown:?}");
    // In French, as the owner reads it.
    a.ui.language = "fr".into();
    frame(&ctx, &mut a, vec![]);
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], TALL));
    assert!(shown.iter().any(|t| t == "Repérer les arnaques dans le courrier externe reçu"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "Les messages externes d'expéditeurs inconnus sont envoyés à llm.studio.example pour être lus."), "{shown:?}");
    assert!(shown.iter().any(|t| t == "Au plus 50 messages par jour"), "{shown:?}");
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let _ = std::fs::remove_dir_all(dir);
}

/// The model's reasons, as plain lines below the sender check's own banner when both apply, red from
/// 70 and orange from 40, nothing under 40; the banner's first line is also the list's.
#[test]
fn the_scam_banner_shows_its_reasons_below_the_sender_checks() {
    let (mut a, dir) = app_with_exchange("scam-banner");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let red = deliver(&mut a, 9810, ("Zoe Park", "zoe.park.director@gmail.com"), "Quick favour", "Are you at your desk?");
    let orange = deliver(&mut a, 9812, ("Sam Doe", "sam.doe@elsewhere.example"), "Odd request", "Hello.");
    let calm = deliver(&mut a, 9813, ("Sam Doe", "sam.doe@elsewhere.example"), "Plain question", "Hello again.");
    keep(
        &a,
        &[
            (&red, verdict(85, &["Urgent transfer to a new account", "Asks to keep it secret", "<b>Click</b> https://x.example", "A fourth"])),
            (&orange, verdict(55, &["Odd request"])),
            (&calm, verdict(39, &["Not shown"])),
        ],
        &[],
    );
    frame(&ctx, &mut a, vec![]);
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": red.to_id()}));
    settle(&ctx, &mut a);
    let shown = spoken(&ctx, &mut a, SIZE);
    let at = |what: &str| shown.iter().position(|l| l.starts_with(what));
    assert!(at("Careful: you know Zoe Park").is_some() && at("This message looks like a scam.").is_some(), "{shown:?}");
    // Top to bottom: the sender check's banner, the scam banner's first line, then its reasons.
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let y = |what: &str| {
        shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) if t.galley.text().starts_with(what) => Some(t.pos.y),
                _ => None,
            })
            .fold(f32::MAX, f32::min)
    };
    let (sender, scam, reason) = (y("Careful: you know Zoe Park"), y("This message looks like a scam."), y("Urgent transfer to a new account"));
    assert!(sender < scam && scam < reason && y("Asks to keep it secret") > reason, "{sender} {scam} {reason}");
    assert!(at("<b>Click</b> https://x.example").is_some(), "reasons are plain text, never markup: {shown:?}");
    assert!(at("A fourth").is_none(), "three at most: {shown:?}");
    assert!(at("Not a scam").is_some(), "{shown:?}");
    let (advice, careful) = (
        shown.iter().filter(|l| l.starts_with("Don't open its links or attachments")).count(),
        shown.iter().filter(|l| l.starts_with("Careful: you know Zoe Park")).count(),
    );
    assert_eq!(advice, careful, "the advice once for the two banners: {shown:?}");
    // The list's row says it too.
    let row = shown.iter().find(|l| l.contains(", Quick favour, ")).cloned().unwrap_or_default();
    assert!(row.contains("This message looks like a scam."), "{row}");
    assert_eq!(a.session.sender_marks(a.session.summary(&red).unwrap()).scam, Some(85));
    // 40 to 69: its own words.
    a.run("list.open", json!({"key": orange.to_id()}));
    settle(&ctx, &mut a);
    let shown = spoken(&ctx, &mut a, SIZE);
    assert!(shown.iter().any(|l| l == "This message could be a scam."), "{shown:?}");
    assert!(shown.iter().any(|l| l == "Odd request"), "{shown:?}");
    assert!(!shown.iter().any(|l| l.starts_with("This message looks like a scam.")), "{shown:?}");
    // Under 40: nothing, in the pane or the list.
    a.run("list.open", json!({"key": calm.to_id()}));
    settle(&ctx, &mut a);
    let shown = spoken(&ctx, &mut a, SIZE);
    // (The list's other rows keep their own marks.)
    let shown: Vec<String> = shown.into_iter().filter(|l| !l.contains(", 12:00")).collect();
    assert!(!shown.iter().any(|l| l.contains("be a scam") || l.contains("like a scam") || l == "Not a scam" || l == "Not shown"), "{shown:?}");
    assert_eq!(a.session.sender_marks(a.session.summary(&calm).unwrap()).scam, None);
    // In French.
    a.run("list.open", json!({"key": red.to_id()}));
    a.ui.language = "fr".into();
    settle(&ctx, &mut a);
    let shown = spoken(&ctx, &mut a, SIZE);
    assert!(shown.iter().any(|l| l == "Ce message ressemble à une arnaque."), "{shown:?}");
    assert!(shown.iter().any(|l| l == "Ce n'est pas une arnaque"), "{shown:?}");
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let _ = std::fs::remove_dir_all(dir);
}

/// The link takes the banner away, trusts the sender (listed in Options with Remove) and keeps the
/// message without a verdict.
#[test]
fn not_a_scam_takes_the_banner_away() {
    let (mut a, dir) = app_with_exchange("scam-not");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let key = deliver(&mut a, 9811, ("Sam Doe", "sam.doe@elsewhere.example"), "Overdue invoice", "Pay now to the new account.");
    keep(&a, &[(&key, verdict(80, &["Payment to a new account"]))], &[]);
    frame(&ctx, &mut a, vec![]);
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    a.run("list.open", json!({"key": key.to_id()}));
    settle(&ctx, &mut a);
    assert!(spoken(&ctx, &mut a, SIZE).iter().any(|l| l == "This message looks like a scam."));
    let at = settled_text_at(&ctx, &mut a, "Not a scam", SIZE).expect("the link");
    click_at(&ctx, &mut a, at, 1, SIZE);
    let shown = spoken(&ctx, &mut a, SIZE);
    assert!(!shown.iter().any(|l| l.contains("a scam") && !l.starts_with("Not a scam")), "the banner is gone: {shown:?}");
    assert_eq!(a.session.run("scam.verdict", &json!({"key": key.to_id()})).ok(), Some(serde_json::Value::Null));
    open_options(&mut a);
    a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "glm"}));
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], TALL));
    assert!(shown.iter().any(|t| t == "sam.doe@elsewhere.example"), "the sender is trusted, and listed: {shown:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// The test's button starts the twenty examples; its result line says what it found, in the
/// words of the spec, with the ones it got wrong.
#[test]
fn the_test_on_examples_shows_its_result() {
    use quatrelle_engine::model::{Event as MailEvent, Job};
    let jobs = Recorder::default();
    let (mut a, dir) = app_with_backend("scam-trial", Box::new(jobs.clone()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("assistant.settings", json!({"url": "http://localhost:11434/v1", "model": "glm"}));
    open_options(&mut a);
    let at = settled_text_at(&ctx, &mut a, "Try it on examples", TALL).expect("the button");
    click_at(&ctx, &mut a, at, 1, TALL);
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], TALL));
    assert!(shown.iter().any(|t| t == "Trying it on the examples: 0 of 20"), "{shown:?}");
    // Answered one at a time (the window's own clock moves with every frame, the seconds are the engine's test): the third scam let through.
    for i in 0..20 {
        frame(&ctx, &mut a, vec![]);
        let asked: Vec<u64> = jobs.0.lock().unwrap().drain(..).filter_map(|j| if let Job::AskLlm { id, .. } = j { Some(id) } else { None }).collect();
        assert_eq!(asked.len(), 1, "one question at a time ({i})");
        let id = asked[0];
        a.session.clock.now += 4;
        let text = if i == 2 || i >= 10 { r#"{"score": 2, "reasons": []}"# } else { r#"{"score": 90, "reasons": ["x"]}"# };
        a.session.apply(MailEvent::LlmAnswered { id, text: text.into(), error: None });
    }
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], TALL));
    let line: Vec<&String> = shown.iter().filter(|t| t.contains("spotted") || t.contains("Trying")).collect();
    assert!(
        shown.iter().any(|t| t.starts_with("9 of 10 scams spotted, 10 of 10 normal messages left alone, ") && t.ends_with(" s a message")),
        "{line:?}"
    );
    let third = quatrelle_engine::model::scam::EXAMPLES[2].subject;
    assert!(shown.iter().any(|t| *t == format!("Scam not spotted: {third} (score 2)")), "each miss on its line, with its score: {shown:?}");
    a.ui.language = "fr".into();
    frame(&ctx, &mut a, vec![]);
    let shown = texts(&frame_shapes(&ctx, &mut a, vec![], TALL));
    assert!(
        shown
            .iter()
            .any(|t| t.starts_with("9 arnaques sur 10 repérées, 10 messages normaux sur 10 laissés tranquilles, ") && t.ends_with(" s par message")),
        "{shown:?}"
    );
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let _ = std::fs::remove_dir_all(dir);
}
