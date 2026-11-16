// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the Teams pages the window opens in the meetings browser's window. Microsoft's
//! page to sign in on, only when it's Microsoft's and while signing in, opened again on asking
//! and forgotten once the sign-in is over; a chat in Teams, only by a Teams chat link.

use crate::TeamsPage;

use super::*;

const SIGN_IN: &str = "https://login.microsoftonline.com/contoso.example/oauth2/v2.0/authorize?client_id=x&state=y";
const CHAT: &str = "19:chat1@thread.v2";

/// Teams on, nobody signed in, and the sign-in asked for.
fn signing_in() -> Session {
    let mut s = Session::blank();
    s.prefs.teams.on = true;
    s.prefs.teams.client_id = CLIENT.into();
    s.prefs.teams.tenant = TENANT.into();
    s.run("teams.signIn", &json!({})).unwrap();
    s.take_jobs();
    s
}

fn page(s: &mut Session, url: &str) {
    s.apply(teams_event(teams::Event::SignInPage { url: url.into() }));
}

fn disabled(s: &mut Session, id: &str) -> bool {
    matches!(s.run(id, &json!({})), Err(CmdError::Disabled(_)))
}

/// The worker's page to sign in on: kept and opened only when it's Microsoft's own, and only while
/// signing in; opened again on asking; forgotten once the sign-in is over, a page not yet opened
/// with it.
#[test]
fn only_a_microsoft_sign_in_page_is_kept() {
    let mut s = signing_in();
    assert!(disabled(&mut s, "teams.reopenSignIn"), "no page yet");
    for bad in [
        "https://evil.example/o/oauth2/v2.0/authorize?a=b",
        "https://login.microsoftonline.com.evil.example/o/oauth2/v2.0/authorize?a=b",
        "https://login.microsoftonline.com:8443/o/oauth2/v2.0/authorize?a=b",
        "http://login.microsoftonline.com/o/oauth2/v2.0/authorize?a=b",
        "https://login.microsoftonline.com/o/oauth2/v2.0/logout?a=b",
    ] {
        page(&mut s, bad);
        assert_eq!(s.take_teams_page(), None, "{bad}");
        assert_eq!(s.teams.sign_in_page, None, "{bad}");
        assert!(disabled(&mut s, "teams.reopenSignIn"), "{bad}");
    }
    page(&mut s, SIGN_IN);
    assert_eq!(s.take_teams_page(), Some(TeamsPage::SignIn(SIGN_IN.into())));
    assert_eq!(s.take_teams_page(), None, "opened once");
    assert_eq!(s.teams.sign_in_page.as_deref(), Some(SIGN_IN), "kept while signing in");
    s.run("teams.reopenSignIn", &json!({})).unwrap();
    assert_eq!(s.take_teams_page(), Some(TeamsPage::SignIn(SIGN_IN.into())), "once more");
    assert_eq!(s.take_teams_page(), None);
    // The sign-in over: forgotten, and the command can't open it again.
    s.apply(teams_event(teams::Event::Status { status: Status::UpToDate }));
    assert_eq!(s.teams.sign_in_page, None);
    assert!(disabled(&mut s, "teams.reopenSignIn"));
    assert_eq!(s.take_teams_page(), None);
    // A page that comes after it isn't opened.
    page(&mut s, SIGN_IN);
    assert_eq!(s.take_teams_page(), None, "not signing in any more");
    // Signing in again, and failing before the window took the page: it isn't opened.
    s.run("teams.signIn", &json!({})).unwrap();
    page(&mut s, SIGN_IN);
    s.apply(teams_event(teams::Event::Status { status: Status::Failed { why: "The sign-in stopped unexpectedly; try again".into() } }));
    assert_eq!(s.take_teams_page(), None, "the sign-in stopped first");
    assert_eq!(s.teams.sign_in_page, None);
}

/// "Open in Teams": the open chat's link, only when it's a Teams chat link; nothing else is
/// handed to the browser.
#[test]
fn open_in_teams_takes_only_a_teams_chat_link() {
    let mut s = signed_in(vec![row(1, 10)]);
    assert!(disabled(&mut s, "teams.openInTeams"), "no chat open");
    open(&mut s, CHAT);
    s.take_jobs();
    let link = |s: &mut Session, url: Option<&str>| {
        s.apply(teams_event(teams::Event::Members { chat: CHAT.into(), members: vec![], web_url: url.map(str::to_string), first: true }));
    };
    for bad in [
        None,
        Some("https://evil.example/l/chat/x"),
        Some("https://teams.microsoft.com.evil.example/l/chat/x"),
        Some("https://teams.microsoft.com:8443/l/chat/x"),
        Some("http://teams.microsoft.com/l/chat/x"),
        Some("https://teams.microsoft.com/l/meetup-join/x"),
    ] {
        link(&mut s, bad);
        assert!(disabled(&mut s, "teams.openInTeams"), "{bad:?}");
        assert_eq!(s.take_teams_page(), None, "{bad:?}");
    }
    let good = "https://teams.microsoft.com/l/chat/19%3Aa%40thread.v2/0";
    link(&mut s, Some(good));
    s.run("teams.openInTeams", &json!({})).unwrap();
    assert_eq!(s.take_teams_page(), Some(TeamsPage::Chat(good.into())));
    assert_eq!(s.take_teams_page(), None, "opened once");
    assert!(teams_jobs(&mut s).is_empty(), "nothing asked of the worker");
}
