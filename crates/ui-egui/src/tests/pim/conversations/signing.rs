// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: where a Teams sign-in stands. A sign-in that failed or lacks a permission can be
//! tried again from the list and from File › Options › Teams; while the browser is waited for,
//! Microsoft's page can be opened again or its address copied; why Teams stopped is said in the
//! interface's language, and never in the words the worker didn't choose; Escape on Sign out's
//! question answers it and leaves the page open.

use super::*;

/// The reasons the worker may give, as it writes them (one of them with a code after it).
const STOPPED_UNEXPECTEDLY: &str = "The sign-in stopped unexpectedly; try again";
const SIGN_IN_PAGE: &str = "https://login.microsoftonline.com/contoso.example/oauth2/v2.0/authorize?client_id=x&state=y";

/// Services that remember the addresses opened in the default browser (no other is found).
struct Opening(Rc<RefCell<Vec<String>>>);

impl crate::Services for Opening {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("refused".into())
    }
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        self.0.borrow_mut().push(url.into());
        Ok(())
    }
}

/// What a click at `at` put on the clipboard.
fn copied_by_click(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, size: egui::Vec2) -> Vec<String> {
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let mut copied = Vec::new();
    for events in [vec![Event::PointerMoved(at)], vec![press(true)], vec![press(false)], vec![]] {
        let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| a.ui(ui));
        out.textures_delta.clear();
        copied.extend(out.platform_output.commands.into_iter().filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t),
            _ => None,
        }));
    }
    copied
}

/// Signing in, Microsoft's page known: "A Microsoft sign-in window has opened" with Reopen the
/// page and Copy the address, on the card (a first sign-in and a sign-in again) and above the
/// list, light and dark, in place of the plain waiting line. Copy puts the page's address on the
/// clipboard; Reopen opens it again.
#[test]
fn the_waiting_panel_offers_reopen_and_copy() {
    for (case, card, signed_in) in [("first", true, false), ("again", true, true), ("list", false, true)] {
        let (mut a, dir) = if card {
            let (a, dir, _, _) = teams_app(&format!("teams-waiting-{case}"), signed_in);
            (a, dir)
        } else {
            let (a, dir, _) = module_app(&format!("teams-waiting-{case}"), vec![row("19:g@thread.v2", 0)]);
            (a, dir)
        };
        let opened = Rc::new(RefCell::new(Vec::new()));
        a.services = Box::new(Opening(opened.clone()));
        a.run("teams.signIn", json!({}));
        a.session.teams_apply(teams::Event::SignInPage { url: SIGN_IN_PAGE.into() });
        let ctx = egui::Context::default();
        let mut tex = Textures::default();
        let (texts, size) = if card {
            a.act(crate::Action::Backstage(crate::backstage::Page::Options));
            (drawn(&ctx, &mut a, &mut tex, &format!("waiting-{case}")), egui::vec2(PAGE.0 as f32, PAGE.1 as f32))
        } else {
            let pictures = std::env::var_os("QUATRELLE_FRAME_PNG").map(std::path::PathBuf::from);
            let mut light = Vec::new();
            for (dark, theme) in [(false, "light"), (true, "dark")] {
                a.ui.dark = Some(dark);
                for _ in 0..6 {
                    frame_with_size(&ctx, &mut a, &mut tex, vec![], None, (1280, 800));
                }
                let png = pictures.as_ref().map(|d| d.join(format!("teams-list-waiting-{theme}.png")));
                let (_, shapes) = frame_with_size(&ctx, &mut a, &mut tex, vec![], png, (1280, 800));
                let texts: Vec<String> = texts_of(&shapes).into_iter().map(|(t, _)| t).collect();
                for w in ["A Microsoft sign-in window has opened", "Reopen the page", "Copy the address"] {
                    assert!(has(&texts, w), "{theme}: {w}: {texts:?}");
                }
                if !dark {
                    light = texts;
                }
            }
            a.ui.dark = Some(false);
            (light, egui::vec2(1280.0, 800.0))
        };
        assert_eq!(opened.borrow().as_slice(), [SIGN_IN_PAGE], "{case}: opened once, in the default browser");
        for w in ["A Microsoft sign-in window has opened", "Reopen the page", "Copy the address"] {
            assert!(has(&texts, w), "{case}: {w}: {texts:?}");
        }
        for plain in ["Waiting for the browser…", "Signing in…"] {
            assert!(!has(&texts, plain), "{case}: {plain}: {texts:?}");
        }
        let at = settled_text_at(&ctx, &mut a, "Copy the address", size).expect("Copy the address");
        assert_eq!(copied_by_click(&ctx, &mut a, at, size), [SIGN_IN_PAGE], "{case}");
        let at = settled_text_at(&ctx, &mut a, "Reopen the page", size).expect("Reopen the page");
        click_at(&ctx, &mut a, at, 1, size);
        assert_eq!(opened.borrow().as_slice(), [SIGN_IN_PAGE, SIGN_IN_PAGE], "{case}: opened again");
        // The sign-in over: the panel goes.
        a.session.teams_apply(teams::Event::Status { status: Status::UpToDate });
        assert!(settled_text_at(&ctx, &mut a, "Reopen the page", size).is_none(), "{case}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Signed in, a sign-in again that failed (cancelled in the browser), or that lacks a
/// permission, still offers Sign in, in the list and on the card; waiting for the browser says so.
#[test]
fn a_failed_sign_in_again_can_be_tried_again() {
    let failed = Status::Failed { why: STOPPED_UNEXPECTEDLY.into() };
    let missing = Status::PermissionMissing { scope: "Chat.Read".into() };
    for status in [failed, missing, Status::SignInAgain] {
        let (mut a, dir, jobs) = module_app("teams-retry-list", vec![row("19:g@thread.v2", 0)]);
        a.session.teams_apply(teams::Event::Status { status: status.clone() });
        let ctx = egui::Context::default();
        let size = egui::vec2(1280.0, 800.0);
        let at = settled_text_at(&ctx, &mut a, "Sign in", size);
        let Some(at) = at else { panic!("{status:?}: no Sign in in the list: {:?}", settled(&ctx, &mut a)) };
        click_at(&ctx, &mut a, at, 1, size);
        assert!(jobs.borrow().iter().any(|j| matches!(j, Job::Teams { job: teams::Job::SignIn { .. } })), "{status:?}: {:?}", jobs.borrow());
        let _ = std::fs::remove_dir_all(dir);
        // The card.
        let (mut a, dir, jobs, _) = teams_app("teams-retry-card", true);
        a.session.teams_apply(teams::Event::Status { status: status.clone() });
        a.act(crate::Action::Backstage(crate::backstage::Page::Options));
        let ctx = egui::Context::default();
        let size = egui::vec2(PAGE.0 as f32, PAGE.1 as f32);
        let Some(at) = settled_text_at(&ctx, &mut a, "Sign in", size) else { panic!("{status:?}: no Sign in on the card") };
        click_at(&ctx, &mut a, at, 1, size);
        assert!(jobs.borrow().iter().any(|j| matches!(j, Job::Teams { job: teams::Job::SignIn { .. } })), "{status:?}: {:?}", jobs.borrow());
        let _ = std::fs::remove_dir_all(dir);
    }
    // Waiting for the browser: said, and no Sign in to click twice.
    let (mut a, dir, _, _) = teams_app("teams-retry-waiting", true);
    a.session.teams_apply(teams::Event::Status { status: Status::SigningIn });
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    let t = drawn(&ctx, &mut a, &mut tex, "waiting");
    assert!(has(&t, "Waiting for the browser…"), "{t:?}");
    assert!(!has(&t, "Sign in"), "{t:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Why Teams stopped, in French: the worker's sentence in French with its code kept; anything
/// else the worker might pass on (Microsoft's own words, an address in them) is never drawn, a
/// sentence of Quatrelle's instead.
#[test]
fn why_teams_stopped_is_said_in_the_interfaces_language() {
    let cases = [
        (format!("{STOPPED_UNEXPECTEDLY} (AADSTS50058)"), "La connexion s’est arrêtée de façon inattendue ; réessayez (AADSTS50058)"),
        (
            "Microsoft gave no lasting sign-in; try again (HTTP 400)".to_string(),
            "Microsoft n’a pas donné de connexion durable ; réessayez (HTTP 400)",
        ),
        (
            "AADSTS50126: Error validating credentials due to invalid username or password for riley@contoso.example. Trace ID: 0b1c".to_string(),
            "Teams s’est arrêté : reconnectez-vous",
        ),
        (format!("{STOPPED_UNEXPECTEDLY} (riley@contoso.example)"), "Teams s’est arrêté : reconnectez-vous"),
    ];
    for (why, wanted) in cases {
        for card in [false, true] {
            let (mut a, dir, _) = module_app("teams-why", vec![row("19:g@thread.v2", 0)]);
            a.ui.language = "fr".into();
            a.session.teams_apply(teams::Event::Status { status: Status::Failed { why: why.clone() } });
            let ctx = egui::Context::default();
            let texts: Vec<String> = if card {
                a.act(crate::Action::Backstage(crate::backstage::Page::Options));
                let mut tex = Textures::default();
                drawn(&ctx, &mut a, &mut tex, "why")
            } else {
                settled(&ctx, &mut a).into_iter().map(|(t, _)| t).collect()
            };
            crate::i18n::set_current(crate::i18n::Lang::EN);
            assert!(texts.iter().any(|t| t == wanted), "{why} (card: {card}): {texts:?}");
            // The address only where the card says who is signed in (the directory's domain is
            // in its box); nothing of Microsoft's words.
            let leaked = |t: &String| {
                (t.contains("riley@") && !t.starts_with("Connecté"))
                    || ["AADSTS50126", "Trace", "validating", "password"].iter().any(|w| t.contains(w))
            };
            assert!(!texts.iter().any(leaked), "{why}: {texts:?}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// Escape on Sign out's question is Cancel: the File page stays, the question goes and doesn't
/// come back when the page is opened again; a second Escape leaves the page.
#[test]
fn escape_on_the_sign_out_question_leaves_the_page_open() {
    let (mut a, dir, jobs, _) = teams_app("teams-sign-out-escape", true);
    let ctx = egui::Context::default();
    let size = egui::vec2(PAGE.0 as f32, PAGE.1 as f32);
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let at = settled_text_at(&ctx, &mut a, "Sign out", size).expect("Sign out");
    click_at(&ctx, &mut a, at, 1, size);
    assert!(settled_text_at(&ctx, &mut a, "Keep the copy", size).is_some(), "the question is asked");
    frame_sized(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)], size);
    assert!(a.ui.backstage.is_some(), "the page stays");
    assert!(settled_text_at(&ctx, &mut a, "Keep the copy", size).is_none(), "the question went");
    frame_sized(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)], size);
    assert!(a.ui.backstage.is_none(), "a second Escape leaves the page");
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    assert!(settled_text_at(&ctx, &mut a, "Keep the copy", size).is_none(), "not asked again");
    assert!(a.session.prefs.teams.signed_in);
    assert!(!jobs.borrow().iter().any(|j| matches!(j, Job::Teams { job: teams::Job::SignOut { .. } })));
    let _ = std::fs::remove_dir_all(dir);
}
