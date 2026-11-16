// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: the Teams pages in the meetings browser's app window. Microsoft's page to sign in on
//! and a chat in Teams open there, in the profile chosen for meetings; in the default browser when
//! no browser is found, when the default browser is the choice, or when the launch fails (which is
//! said). A page that isn't Microsoft's sign-in or a Teams chat is never handed to a browser.

use quatrelle_engine::model::pim::teams::{self, ChatKind, ChatRow};

use super::*;

const SIGN_IN: &str = "https://login.microsoftonline.com/contoso.example/oauth2/v2.0/authorize?client_id=x&state=y";
const CHAT: &str = "19:meet@thread.v2";
const CHAT_LINK: &str = "https://teams.microsoft.com/l/chat/19%3Ameet%40thread.v2/0?tenantId=00000000-0000-0000-0000-000000000000";
const CHROME: (&str, &str) = ("Google Chrome", "/opt/chrome");

/// The app of [`launching_with`] with Teams on and both ids, signing in.
fn signing_in(name: &str, browsers: &[(&str, &str)], fails: bool) -> (MailApp, std::path::PathBuf, Rc<RefCell<Vec<String>>>) {
    let (mut a, dir, o) = launching_with(name, browsers, &[], fails);
    let t = &mut a.session.prefs.teams;
    t.on = true;
    t.client_id = "11111111-2222-3333-4444-555555555555".into();
    t.tenant = "contoso.example".into();
    a.run("teams.signIn", json!({}));
    (a, dir, o)
}

/// The worker's page to sign in on, then a few frames (the first one only loads the fonts):
/// what the window opened.
fn sign_in_page(a: &mut MailApp, o: &Rc<RefCell<Vec<String>>>, url: &str) -> Vec<String> {
    a.session.teams_apply(teams::Event::SignInPage { url: url.into() });
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, a, vec![]);
    }
    opened(o)
}

#[test]
fn the_sign_in_opens_in_the_meetings_app_window() {
    let (mut a, dir, o) = signing_in("teams-page-app", &[CHROME], false);
    a.session.prefs.meeting_profile = Some(MEETINGS_PROFILE.into());
    assert_eq!(sign_in_page(&mut a, &o, SIGN_IN), [format!("app /opt/chrome --profile-directory={MEETINGS_PROFILE} {SIGN_IN}")]);
    // Opened again from the waiting panel (at once, without waiting for a frame): the same window.
    a.act(crate::Action::Run("teams.reopenSignIn", json!({})));
    assert_eq!(opened(&o).len(), 2, "{:?}", opened(&o));
    assert_eq!(opened(&o).last(), opened(&o).first());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn with_no_browser_found_it_opens_in_the_default_browser() {
    let (mut a, dir, o) = signing_in("teams-page-none", &[], false);
    assert_eq!(sign_in_page(&mut a, &o, SIGN_IN), [format!("url {SIGN_IN}")]);
    assert!(a.session.notices.is_empty(), "nothing to say: {:?}", a.session.notices);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_failed_launch_falls_back_to_the_default_browser_and_says_so() {
    let (mut a, dir, o) = signing_in("teams-page-fails", &[CHROME], true);
    assert_eq!(sign_in_page(&mut a, &o, SIGN_IN), [format!("app /opt/chrome {SIGN_IN}"), format!("url {SIGN_IN}")]);
    let last = a.session.notices.last();
    assert!(last.is_some_and(|n| n.error && n.text == "Google Chrome couldn't open the page: boom"), "{last:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_default_browser_chosen_is_used() {
    let (mut a, dir, o) = signing_in("teams-page-default", &[CHROME], false);
    a.session.prefs.meeting_browser = Some(String::new());
    assert_eq!(sign_in_page(&mut a, &o, SIGN_IN), [format!("url {SIGN_IN}")]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Another site's page, even one dressed as Microsoft's, opens nowhere.
#[test]
fn a_sign_in_page_elsewhere_is_never_opened() {
    let (mut a, dir, o) = signing_in("teams-page-evil", &[CHROME], false);
    for bad in ["https://evil.example/o/oauth2/v2.0/authorize?a=b", "https://login.microsoftonline.com:8443/o/oauth2/v2.0/authorize?a=b"] {
        assert!(sign_in_page(&mut a, &o, bad).is_empty(), "{bad}: {:?}", opened(&o));
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn open_in_teams_opens_the_chat_in_the_app_window() {
    let (mut a, dir, o) = launching_with("teams-page-chat", &[CHROME], &[], false);
    let t = &mut a.session.prefs.teams;
    t.on = true;
    t.signed_in = true;
    t.object_id = "00000000-0000-0000-0000-00000000000a".into();
    let row = ChatRow {
        id: CHAT.into(),
        name: "Support team".into(),
        kind: ChatKind::Group,
        last_line: String::new(),
        last_external: false,
        at: Some(NOW),
        unread: 0,
        hidden: false,
        left: false,
        unreadable: false,
        external: false,
        members: 3,
        person: None,
    };
    a.session.teams_apply(teams::Event::Chats { rows: vec![row] });
    a.run("teams.open", json!({"chat": CHAT}));
    a.session.teams_apply(teams::Event::Members { chat: CHAT.into(), members: vec![], web_url: Some(CHAT_LINK.into()), first: true });
    // At once, without waiting for a frame.
    a.act(crate::Action::Run("teams.openInTeams", json!({})));
    assert_eq!(opened(&o), [format!("app /opt/chrome {CHAT_LINK}")]);
    let _ = std::fs::remove_dir_all(dir);
}
