//! The question a server's proposal of another domain raises, and Server settings' list of the
//! other domains allowed.

use super::*;
use quatrelle_engine::model::Job;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// The jobs the app handed its backend (`NullBackend` drops them, and every frame hands them over).
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

/// An app with one server account (its start-up job taken), online; the account's id, and the
/// jobs that go out from now on.
fn with_account(name: &str) -> (MailApp, std::path::PathBuf, String, Handed) {
    let (mut a, dir) = app(name);
    let handed = Handed::default();
    a.backend = Box::new(Recorder(handed.clone()));
    let id =
        a.session.run("account.add", &json!({"email": "zoe@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.apply(MailEvent::Status { account: id.clone(), status: SyncStatus::Online { at: NOW } });
    a.session.take_jobs();
    (a, dir, id, handed)
}

/// A pass of the app's `logic` (where the questions are taken from the engine), then a frame.
fn step(ctx: &egui::Context, a: &mut MailApp) {
    a.logic(ctx);
    frame(ctx, a, vec![]);
}

fn asked(id: &str, site: &str) -> MailEvent {
    MailEvent::TrustAsked { account: id.into(), site: site.into() }
}

/// The times the account was started again since the last look.
fn restarts(handed: &Handed) -> usize {
    std::mem::take(&mut *handed.borrow_mut()).iter().filter(|j| matches!(j, Job::Start { .. })).count()
}

fn trusted(a: &MailApp, id: &str) -> Vec<String> {
    a.session.account(id).unwrap().config.trusted_domains.clone()
}

fn is_question(a: &MailApp, site: &str) -> bool {
    matches!(&a.ui.dialog, Some(Dialog::TrustDomain { site: s, .. }) if s == site)
}

/// The question comes for an account that is online (a calendar's discovery raises it while the
/// mail works), one at a time, and never over another dialog or the File page.
#[test]
fn the_question_opens_for_an_online_account_and_not_over_another_dialog() {
    let (mut a, dir, id, _) = with_account("trust-opens");
    let ctx = egui::Context::default();
    a.session.apply(asked(&id, "icloud.com"));
    a.session.apply(asked(&id, "fastmail.com"));
    // Another dialog is open: the questions wait.
    a.ui.dialog = Some(Dialog::SaveChanges);
    step(&ctx, &mut a);
    assert!(matches!(a.ui.dialog, Some(Dialog::SaveChanges)), "the dialog stays");
    assert_eq!(a.session.ask_trust.len(), 2, "both wait");
    // So does the File page.
    a.ui.dialog = None;
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    step(&ctx, &mut a);
    assert!(a.ui.dialog.is_none() && a.session.ask_trust.len() == 2, "not over the File page");
    a.ui.backstage = None;
    // Free: the first, then, once it is answered, the next.
    step(&ctx, &mut a);
    assert!(is_question(&a, "icloud.com"), "{:?}", a.ui.dialog);
    assert_eq!(a.session.ask_trust.len(), 1, "one at a time");
    step(&ctx, &mut a);
    assert!(is_question(&a, "icloud.com"), "the second waits for the first's answer");
    let no = settled_text_at(&ctx, &mut a, "Don't allow", SIZE).expect("its buttons");
    click_at(&ctx, &mut a, no, 1, SIZE);
    step(&ctx, &mut a);
    assert!(is_question(&a, "fastmail.com"), "{:?}", a.ui.dialog);
    // The account is online the whole time.
    assert!(matches!(a.session.account(&id).unwrap().status, SyncStatus::Online { .. }));
    let _ = std::fs::remove_dir_all(dir);
}

/// Allow keeps the domain for the account and starts it again.
#[test]
fn allow_trusts_and_restarts() {
    let (mut a, dir, id, handed) = with_account("trust-allow");
    let ctx = egui::Context::default();
    a.session.apply(asked(&id, "icloud.com"));
    step(&ctx, &mut a);
    assert!(is_question(&a, "icloud.com"));
    // The words it says.
    assert!(settled_text_at(&ctx, &mut a, "Allow another server?", SIZE).is_some(), "its title");
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let said = shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .find(|t| t.starts_with("The server proposes"));
    assert_eq!(
        said.as_deref(),
        Some("The server proposes to set up zoe@studio.example with icloud.com. Your password or sign-in will be sent there. Allow it?")
    );
    let yes = settled_text_at(&ctx, &mut a, "Allow", SIZE).expect("its buttons");
    click_at(&ctx, &mut a, yes, 1, SIZE);
    step(&ctx, &mut a);
    assert!(a.ui.dialog.is_none(), "answered");
    assert_eq!(trusted(&a, &id), ["icloud.com"]);
    assert_eq!(restarts(&handed), 1, "the account starts again");
    let _ = std::fs::remove_dir_all(dir);
}

/// Don't allow says so, leaves the account as it is, and nothing is asked again for that domain.
#[test]
fn dont_allow_says_so_and_asks_no_more() {
    let (mut a, dir, id, handed) = with_account("trust-refuse");
    let ctx = egui::Context::default();
    a.session.apply(asked(&id, "icloud.com"));
    step(&ctx, &mut a);
    assert!(is_question(&a, "icloud.com"));
    let no = settled_text_at(&ctx, &mut a, "Don't allow", SIZE).expect("its buttons");
    click_at(&ctx, &mut a, no, 1, SIZE);
    step(&ctx, &mut a);
    assert!(a.ui.dialog.is_none(), "answered");
    assert!(trusted(&a, &id).is_empty());
    assert_eq!(restarts(&handed), 0, "the account is left as it is");
    assert!(
        a.session.notices.iter().any(|n| n.text == "icloud.com wasn't allowed: zoe@studio.example won't use it"),
        "{:?}",
        a.session.notices.iter().map(|n| &n.text).collect::<Vec<_>>()
    );
    // The network says it again (it asks at every try): nothing comes.
    a.session.apply(asked(&id, "icloud.com"));
    a.session.apply(MailEvent::Status { account: id.clone(), status: SyncStatus::NeedsTrust { site: "icloud.com".into() } });
    step(&ctx, &mut a);
    assert!(a.ui.dialog.is_none() && a.session.ask_trust.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Escape, or a click outside, is a no as well.
#[test]
fn escape_is_a_no() {
    let (mut a, dir, id, _) = with_account("trust-escape");
    let ctx = egui::Context::default();
    a.session.apply(asked(&id, "icloud.com"));
    step(&ctx, &mut a);
    assert!(is_question(&a, "icloud.com"));
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    step(&ctx, &mut a);
    assert!(a.ui.dialog.is_none());
    assert!(trusted(&a, &id).is_empty());
    assert!(a.session.notices.iter().any(|n| n.text.starts_with("icloud.com wasn't allowed")), "it says so");
    let _ = std::fs::remove_dir_all(dir);
}

/// An account waiting for the answer says so in the status bar, and the link there brings the
/// question back if it was closed.
#[test]
fn the_status_bar_waits_for_the_answer_and_brings_the_question_back() {
    let (mut a, dir, id, _) = with_account("trust-status-bar");
    let ctx = egui::Context::default();
    a.session.apply(MailEvent::Status { account: id.clone(), status: SyncStatus::NeedsTrust { site: "office365.com".into() } });
    step(&ctx, &mut a);
    assert!(is_question(&a, "office365.com"));
    a.ui.dialog = None;
    let words = "office365.com is waiting for your answer";
    let shapes = frame_shapes(&ctx, &mut a, vec![], SIZE);
    let at = shapes
        .iter()
        .find_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text().contains(words) => Some(t.pos + t.galley.rect.center().to_vec2()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the status bar says: {words}"));
    click_at(&ctx, &mut a, at, 1, SIZE);
    step(&ctx, &mut a);
    assert!(is_question(&a, "office365.com"), "{:?}", a.ui.dialog);
    let _ = std::fs::remove_dir_all(dir);
}

/// Server settings list the other domains allowed, each with Remove, and a field with Add; both
/// start the account again.
#[test]
fn server_settings_list_add_and_remove_domains() {
    let (mut a, dir, id, handed) = with_account("trust-settings");
    let ctx = egui::Context::default();
    a.session.run("account.trustDomain", &json!({"account": id, "domain": "icloud.com"})).unwrap();
    a.session.take_jobs();
    let config = a.session.account(&id).unwrap().config.clone();
    a.ui.dialog = Some(Dialog::edit_account(&config));
    let size = egui::vec2(1280.0, 1400.0);
    assert!(settled_text_at(&ctx, &mut a, "Other servers allowed", size).is_some(), "its title");
    assert!(settled_text_at(&ctx, &mut a, "icloud.com", size).is_some(), "the domain allowed is listed");
    // Add: typed, then the button.
    let Some(Dialog::EditAccount(f)) = a.ui.dialog.as_mut() else { panic!("the server settings") };
    f.trust_add = " https://api.Fastmail.com/jmap/session ".into();
    let add = settled_text_at(&ctx, &mut a, "Add", size).expect("its Add button");
    click_at(&ctx, &mut a, add, 1, size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert_eq!(trusted(&a, &id), ["icloud.com", "fastmail.com"], "its site, normalised by the command");
    assert_eq!(restarts(&handed), 1);
    let Some(Dialog::EditAccount(f)) = a.ui.dialog.as_ref() else { panic!("the dialog stays open") };
    assert!(f.trust_add.is_empty(), "the field is emptied");
    assert!(settled_text_at(&ctx, &mut a, "fastmail.com", size).is_some(), "listed at once");
    // Not a domain: refused by the command, said in a notice; nothing changes.
    let Some(Dialog::EditAccount(f)) = a.ui.dialog.as_mut() else { panic!() };
    f.trust_add = "nodot".into();
    let add = settled_text_at(&ctx, &mut a, "Add", size).expect("its Add button");
    click_at(&ctx, &mut a, add, 1, size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert_eq!(trusted(&a, &id).len(), 2);
    assert!(a.session.notices.iter().any(|n| n.error && n.text.contains("not a domain: nodot")), "{:?}", a.session.notices);
    restarts(&handed);
    // Remove the first one (the first Remove button drawn is the first domain's).
    let removes: Vec<egui::Pos2> = frame_shapes(&ctx, &mut a, vec![], size)
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) if t.galley.text() == "Remove" => Some(t.pos + t.galley.rect.center().to_vec2()),
            _ => None,
        })
        .collect();
    assert_eq!(removes.len(), 2, "one Remove for each");
    let first = removes.iter().copied().min_by(|p, q| p.y.total_cmp(&q.y)).unwrap();
    click_at(&ctx, &mut a, first, 1, size);
    frame_sized(&ctx, &mut a, vec![], size);
    assert_eq!(trusted(&a, &id), ["fastmail.com"]);
    assert_eq!(restarts(&handed), 1);
    // Saving the form leaves the list alone.
    let Some(Dialog::EditAccount(f)) = a.ui.dialog.as_ref() else { panic!() };
    a.act(crate::Action::Submit("account.update", f.params()));
    assert_eq!(trusted(&a, &id), ["fastmail.com"]);
    let _ = std::fs::remove_dir_all(dir);
}
