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

/// Each Teams page leaves a line in the log: what it is, where it opened (the browser's shown
/// name and which profile, never a folder's name), and the link's shape, never the chat's id,
/// the organisation's or a query's value.
#[test]
fn each_page_leaves_a_line_with_no_id_in_it() {
    use crate::app::{PageOutcome, ProfileKind, page_line};
    use quatrelle_engine::TeamsPage;
    let built = "https://teams.microsoft.com/l/chat/19%3Ameet%40thread.v2/conversations?context=%7B%22contextType%22%3A%22chat%22%7D&tenantId=00000000-0000-0000-0000-000000000000";
    let shape = "teams.microsoft.com/l/chat/<id>/conversations?context&tenantId";
    let chat = TeamsPage::Chat(built.into());
    let ours = ProfileKind::of(Some(MEETINGS_PROFILE));
    let lines = [
        (page_line(&chat, &PageOutcome::AppWindow { browser: "Google Chrome", profile: ours }), format!("teams: a chat opened in Google Chrome, in Quatrelle's own profile: {shape}")),
        (page_line(&chat, &PageOutcome::DefaultBrowser), format!("teams: a chat opened in the default browser: {shape}")),
        (
            page_line(&chat, &PageOutcome::InsteadOf { browser: "Google Chrome", error: "boom" }),
            format!("teams: a chat couldn't be opened in Google Chrome (boom); the default browser instead: {shape}"),
        ),
        (page_line(&chat, &PageOutcome::Failed { error: "boom" }), format!("teams: a chat couldn't be opened: boom: {shape}")),
        (page_line(&TeamsPage::Chat(CHAT_LINK.into()), &PageOutcome::Refused), "teams: a page that isn't Teams' own was not opened (a chat): teams.microsoft.com/l/chat/<id>/0?tenantId".into()),
        (
            page_line(&TeamsPage::SignIn(SIGN_IN.into()), &PageOutcome::AppWindow { browser: "Chromium", profile: ProfileKind::of(None) }),
            "teams: the sign-in page opened in Chromium, in the profile it used last: login.microsoftonline.com/<id>/oauth2/<id>/authorize?client_id&state".into(),
        ),
    ];
    for (line, want) in &lines {
        assert_eq!(line, want);
        for secret in ["meet", "thread", "0000", "%7B", "contextType", "contoso", "=x", "=y"] {
            assert!(!line.contains(secret), "{secret} in {line}");
        }
    }
    // Another profile chosen: said so, never by its folder's name.
    let chosen = page_line(&chat, &PageOutcome::AppWindow { browser: "Google Chrome", profile: ProfileKind::of(Some("Profile 3")) });
    assert_eq!(chosen, format!("teams: a chat opened in Google Chrome, in another profile chosen: {shape}"));
    assert!(!chosen.contains("Profile 3"));
}

/// A launcher's error that names the page (macOS' `open` prints its whole command line) is never
/// written in the log: neither the chat's id, the organisation's, nor a query's value. An ordinary
/// error is written as it is.
#[test]
fn a_launchers_error_naming_the_page_is_withheld() {
    use crate::app::{PageOutcome, page_line};
    use quatrelle_engine::TeamsPage;
    let guid = "00000000-0000-0000-0000-000000000000";
    let chat = format!(
        "https://teams.microsoft.com/l/chat/19%3Aquokka7%40thread.v2/conversations?context=%7B%22contextType%22%3A%22chat%22%7D&tenantId={guid}"
    );
    let sign_in = format!("https://login.microsoftonline.com/{guid}/oauth2/v2.0/authorize?client_id=abcd1234-wombat&state=x");
    for page in [TeamsPage::Chat(chat.clone()), TeamsPage::SignIn(sign_in.clone())] {
        let url = if matches!(page, TeamsPage::Chat(_)) { &chat } else { &sign_in };
        let error = format!("Launcher \"/usr/bin/open\" \"--\" \"{url}\" failed with ExitStatus(unix_wait_status(256))");
        for line in
            [page_line(&page, &PageOutcome::Failed { error: &error }), page_line(&page, &PageOutcome::InsteadOf { browser: "Safari", error: &error })]
        {
            for secret in ["quokka7", "19:", "thread", "0000", "abcd1234", "wombat", "%", "="] {
                assert!(!line.contains(secret), "{secret} in {line}");
            }
            assert!(line.contains("(withheld: it names the address)"), "{line}");
        }
    }
    let ordinary = page_line(&TeamsPage::Chat(chat), &PageOutcome::Failed { error: "No such file or directory (os error 2)" });
    assert_eq!(
        ordinary,
        "teams: a chat couldn't be opened: No such file or directory (os error 2): teams.microsoft.com/l/chat/<id>/conversations?context&tenantId"
    );
}
