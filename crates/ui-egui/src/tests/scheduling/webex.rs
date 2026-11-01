//! UI tests: File › Options › Calendar › Webex.

use super::*;
use quatrelle_engine::model::{Job, WEBEX_REDIRECT_URI};

/// The File page on Options, drawn tall enough for every card.
fn options_page(name: &str) -> (MailApp, std::path::PathBuf, egui::Context, egui::Vec2) {
    let (mut a, dir) = app(name);
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 5000.0);
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    frame_sized(&ctx, &mut a, vec![], size);
    (a, dir, ctx, size)
}

/// The card draws signed out (the sign-in waiting on a client id, a typed one enabling it) and
/// signed in, with or without a name; a sign-in's refusal is said under it.
#[test]
fn the_webex_card_draws_signed_out_and_in() {
    let (mut a, dir, ctx, size) = options_page("webex-card");
    for _ in 0..3 {
        frame_sized(&ctx, &mut a, vec![], size);
    }
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    for t in ["Webex", "Site", "Client ID", "Client secret", "Redirect URI", "Sign in to Webex"] {
        assert!(text_at(&shapes, t).is_some(), "{t}");
    }
    assert!(text_at(&shapes, "Sign out").is_none());
    a.session.webex.signing_in = true;
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(text_at(&shapes, "Waiting for the browser…").is_some());
    a.session.webex.signing_in = false;
    a.session.webex.sign_in_error = Some(quatrelle_engine::model::WebexError::SignIn);
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(text_at(&shapes, "Sign in to Webex again").is_some(), "the refusal is said");
    a.session.webex.sign_in_error = None;
    a.session.prefs.webex_signed_in = true;
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(text_at(&shapes, "Signed in").is_some() && text_at(&shapes, "Sign out").is_some());
    a.session.prefs.webex_user = "Ana Diaz".into();
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(text_at(&shapes, "Signed in as Ana Diaz").is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// A backend that keeps the jobs it is given.
struct Recorder(std::rc::Rc<std::cell::RefCell<Vec<Job>>>);

impl crate::Backend for Recorder {
    fn submit(&mut self, job: Job) {
        self.0.borrow_mut().push(job);
    }
    fn poll(&mut self) -> Vec<MailEvent> {
        Vec::new()
    }
}

/// The secret goes to the keychain once the field is left, and the card forgets it.
#[test]
fn the_secret_is_never_shown_again() {
    let (mut a, dir, ctx, size) = options_page("webex-secret");
    // The secret's box lines up with the site's, which shows its hint.
    let x = settled_text_at(&ctx, &mut a, "acme.webex.com", size).expect("the site's hint").x;
    // Its row has a note under the name: the box is centred a little lower.
    let y = settled_text_at(&ctx, &mut a, "Client secret", size).expect("its row").y + 12.0;
    let jobs = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    a.backend = Box::new(Recorder(jobs.clone()));
    click_at(&ctx, &mut a, egui::pos2(x, y), 1, size);
    frame_sized(&ctx, &mut a, vec![egui::Event::Text("s3cret".into())], size);
    frame_sized(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)], size);
    frame_sized(&ctx, &mut a, vec![], size);
    let sent = jobs.borrow().iter().any(|j| matches!(j, Job::SetWebexSecret { secret } if secret.0 == "s3cret"));
    assert!(sent, "webex.settings ran with the secret");
    let kept = ctx.data_mut(|d| d.get_temp::<String>(egui::Id::unique(("options-webex", "secret"))));
    assert_eq!(kept.as_deref(), Some(""), "the typed text is cleared");
    let _ = std::fs::remove_dir_all(dir);
}

/// The address to register with the integration is shown, for copying.
#[test]
fn the_redirect_uri_is_shown_to_copy() {
    let (mut a, dir, ctx, size) = options_page("webex-redirect");
    let shapes = frame_shapes(&ctx, &mut a, vec![], size);
    assert!(text_at(&shapes, WEBEX_REDIRECT_URI).is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// Task 8's way in: the action opens File › Options.
#[test]
fn webex_options_opens_the_card() {
    let (mut a, dir) = app("webex-options");
    a.act(crate::Action::WebexOptions);
    assert!(a.ui.backstage.as_ref().is_some_and(|b| b.page == crate::backstage::Page::Options));
    let _ = std::fs::remove_dir_all(dir);
}
