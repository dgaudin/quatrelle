// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The permission to make Teams meetings: asked for apart from the chats', at a sign-in that asks
//! for it (Authorize Teams meetings) and at the renewals that follow; granted or not; taken back
//! at a renewal (the chats go on with their own); and a page for it closed or refused, with or
//! without a sign-in to go on from, with or without a keychain.

use super::sign_in::NoKeychain;
use super::*;

/// Signing in, asking for the permission to make Teams meetings too or not.
fn signing_in(meetings: bool) -> Job {
    Job::SignIn { client_id: CLIENT.into(), tenant: "organizations".into(), meetings }
}

/// What the sign-in service of the tests was asked, in order.
fn asked(desk: &Desk) -> Vec<(&'static str, bool)> {
    desk.asked.lock().unwrap().clone()
}

/// What the worker said of meetings, allowed or not, in order.
fn meetings_of(seen: &[model::Event]) -> Vec<bool> {
    seen.iter().filter_map(|e| if let model::Event::Meetings { allowed } = e { Some(*allowed) } else { None }).collect()
}

/// Whether the worker said whether meetings are allowed only after it said who signed in.
fn after_the_account(seen: &[model::Event]) -> bool {
    let account = seen.iter().position(|e| matches!(e, model::Event::Account { .. }));
    let meetings = seen.iter().position(|e| matches!(e, model::Event::Meetings { .. }));
    matches!((account, meetings), (Some(a), Some(m)) if a < m)
}

/// The access token held has run out: the next turn renews it.
fn run_out(w: &mut Worker) {
    w.tokens.as_mut().unwrap().expires_at = now_unix() - 1;
}

#[test]
fn the_meetings_permission_is_granted_under_either_of_its_names() {
    assert!(meetings_granted(""), "not said: not judged");
    assert!(meetings_granted(" \t "), "not said either");
    assert!(meetings_granted(&format!("{TEAMS_SCOPE} https://graph.microsoft.com/OnlineMeetings.ReadWrite")));
    // Microsoft may list it by its short name, in any case.
    assert!(meetings_granted("User.Read OnlineMeetings.ReadWrite Chat.ReadWrite"));
    assert!(meetings_granted("user.read onlinemeetings.readwrite"));
    assert!(meetings_granted("openid HTTPS://GRAPH.MICROSOFT.COM/ONLINEMEETINGS.READWRITE"));
    // A whole permission of the list, never a part of one, nor another's.
    for not in [
        TEAMS_SCOPE,
        "OnlineMeetings.Read",
        "OnlineMeetings.ReadWriteX",
        "OnlineMeetings.ReadWrite.All",
        "XOnlineMeetings.ReadWrite",
        "https://graph.microsoft.com/OnlineMeetings.Read",
        "https://other.example/OnlineMeetings.ReadWrite",
        "User.Read,OnlineMeetings.ReadWrite",
    ] {
        assert!(!meetings_granted(not), "{not}");
    }
}

#[test]
fn a_plain_sign_in_asks_the_chats_scope_alone() {
    let server = serve_teams(one_chat());
    let (mut w, desk, seen, _) = by_hand(&server, grant(Some(TENANT)));
    let t0 = Instant::now();
    w.job(signing_in(false));
    turn_until(&mut w, t0, "the account", |_| account(&teams_of(&seen)).is_some());
    assert_eq!(asked(&desk), [("authorize", false)]);
    let said_so = teams_of(&seen);
    assert_eq!(meetings_of(&said_so), [false], "{said_so:?}");
    assert!(after_the_account(&said_so), "{said_so:?}");
    assert!(!w.settings.meetings);
    // Its renewals ask for the chats' permissions alone too.
    run_out(&mut w);
    w.turn(t0 + Duration::from_secs(60));
    assert_eq!(asked(&desk), [("authorize", false), ("refresh", false)]);
}

#[test]
fn authorizing_meetings_asks_the_wider_scope_and_says_so() {
    let server = serve_teams(one_chat());
    // The permissions granted not said: not judged.
    let (mut w, desk, seen, _) = by_hand(&server, grant(Some(TENANT)));
    let t0 = Instant::now();
    w.job(signing_in(true));
    turn_until(&mut w, t0, "the account", |_| account(&teams_of(&seen)).is_some());
    assert_eq!(asked(&desk), [("authorize", true)]);
    let said_so = teams_of(&seen);
    assert_eq!(meetings_of(&said_so), [true], "{said_so:?}");
    assert!(after_the_account(&said_so), "{said_so:?}");
    assert!(w.settings.meetings);
    // From then on, the renewals ask for the meetings' permission too.
    run_out(&mut w);
    w.turn(t0 + Duration::from_secs(60));
    assert_eq!(asked(&desk), [("authorize", true), ("refresh", true)]);
    assert_eq!(meetings_of(&teams_of(&seen)), [true], "nothing lost");
}

#[test]
fn a_grant_without_the_permission_is_not_allowed() {
    let server = serve_teams(one_chat());
    // Every permission of the chats, and not the meetings' one.
    let mut tokens = grant(Some(TENANT));
    tokens.granted = TEAMS_SCOPE.into();
    let (mut w, desk, seen, _) = by_hand(&server, tokens);
    let t0 = Instant::now();
    w.job(signing_in(true));
    turn_until(&mut w, t0, "the first list", |_| listed(&teams_of(&seen)) || failed(&teams_of(&seen)));
    let said_so = teams_of(&seen);
    assert!(account(&said_so).is_some() && !failed(&said_so), "signed in all the same: {said_so:?}");
    assert_eq!(meetings_of(&said_so), [false], "{said_so:?}");
    assert!(after_the_account(&said_so), "{said_so:?}");
    assert!(!w.settings.meetings);
    run_out(&mut w);
    w.turn(t0 + Duration::from_secs(60));
    assert_eq!(asked(&desk), [("authorize", true), ("refresh", false)], "the renewals ask for the chats' permissions alone");
}

#[test]
fn a_start_with_meetings_renews_wide() {
    let server = serve_teams(one_chat());
    let (mut w, desk, seen, creds) = by_hand(&server, grant(Some(TENANT)));
    creds.set_secret(&keyring(), REFRESH_KEY, REFRESH).unwrap();
    let t0 = Instant::now();
    w.job(Job::Start { settings: TeamsSettings { meetings: true, ..settings(ME) } });
    w.turn(t0);
    assert!(listed(&teams_of(&seen)), "{:?}", teams_of(&seen));
    assert!(w.settings.meetings);
    assert_eq!(asked(&desk), [("refresh", true)]);
    // Started again for the account going, the Options now without meetings: taken as they are,
    // nothing begun again, and the next renewal is the chats' alone.
    w.job(Job::Start { settings: settings(ME) });
    assert!(!w.settings.meetings && w.client.is_some());
    run_out(&mut w);
    w.turn(t0 + Duration::from_secs(60));
    assert_eq!(asked(&desk), [("refresh", true), ("refresh", false)]);
    assert_eq!(hits(&server, "me"), 1, "nothing begun again");
}

#[test]
fn a_start_without_meetings_renews_narrow() {
    let server = serve_teams(one_chat());
    let (mut w, desk, seen, creds) = by_hand(&server, grant(Some(TENANT)));
    creds.set_secret(&keyring(), REFRESH_KEY, REFRESH).unwrap();
    let t0 = Instant::now();
    w.job(Job::Start { settings: settings(ME) });
    w.turn(t0);
    assert!(listed(&teams_of(&seen)), "{:?}", teams_of(&seen));
    assert!(!w.settings.meetings);
    assert_eq!(asked(&desk), [("refresh", false)]);
    // The Options now with meetings, for the account going: the next renewal asks for them.
    w.job(Job::Start { settings: TeamsSettings { meetings: true, ..settings(ME) } });
    assert!(w.settings.meetings && w.client.is_some());
    run_out(&mut w);
    w.turn(t0 + Duration::from_secs(60));
    assert_eq!(asked(&desk), [("refresh", false), ("refresh", true)]);
}

/// D5: the meetings' permission taken back (by an administrator, a policy), the wide renewal is
/// refused: renewed once more with the chats' permissions alone, meetings no longer allowed, and
/// the chats go on without a new sign-in.
#[test]
fn a_wide_renewal_refused_falls_back_and_goes_on() {
    let server = serve_teams(one_chat());
    let (mut w, desk, seen, creds) = by_hand(&server, grant(Some(TENANT)));
    let t0 = Instant::now();
    w.job(signing_in(true));
    turn_until(&mut w, t0, "the first pass", |_| teams_of(&seen).iter().any(|e| matches!(e, model::Event::Chats { .. })));
    assert!(w.settings.meetings);
    let lists = hits(&server, "chats");
    desk.refuse_wide.store(true, Ordering::SeqCst);
    run_out(&mut w);
    let later = t0 + Duration::from_secs(3600);
    for i in 0..50 {
        if hits(&server, "chats") > lists {
            break;
        }
        w.turn(later + Duration::from_millis(200 * i));
    }
    let said_so = teams_of(&seen);
    assert_eq!(asked(&desk), [("authorize", true), ("refresh", true), ("refresh", false)]);
    assert_eq!(meetings_of(&said_so), [true, false], "{said_so:?}");
    assert!(!said(&said_so, &Status::SignInAgain) && !failed(&said_so), "{said_so:?}");
    assert!(!w.settings.meetings && w.polling());
    assert!(hits(&server, "chats") > lists, "the list is read after");
    assert_eq!(creds.get_secret(&keyring(), REFRESH_KEY).as_deref(), Some(REFRESH), "the sign-in kept");
    // From then on, the chats' permissions alone: the wide renewal isn't tried again.
    run_out(&mut w);
    w.turn(later + Duration::from_secs(60));
    assert_eq!(asked(&desk), [("authorize", true), ("refresh", true), ("refresh", false), ("refresh", false)]);
}

#[test]
fn a_narrow_renewal_refused_is_sign_in_again() {
    // The sign-in revoked: the wide renewal refused, then the narrow one too. "Sign in again", as
    // before, and nothing more is asked of Teams.
    for meetings in [true, false] {
        let server = serve_teams(one_chat());
        let (mut w, desk, seen, _) = by_hand(&server, grant(Some(TENANT)));
        let t0 = Instant::now();
        w.job(signing_in(meetings));
        turn_until(&mut w, t0, "the first pass", |_| teams_of(&seen).iter().any(|e| matches!(e, model::Event::Chats { .. })));
        desk.refuse.store(true, Ordering::SeqCst);
        run_out(&mut w);
        let later = t0 + Duration::from_secs(3600);
        w.turn(later);
        let renewals: &[(&str, bool)] = if meetings { &[("refresh", true), ("refresh", false)] } else { &[("refresh", false)] };
        assert_eq!(asked(&desk).get(1..), Some(renewals), "{meetings}");
        let said_so = teams_of(&seen);
        assert!(said(&said_so, &Status::SignInAgain), "{said_so:?}");
        assert!(!w.polling() && w.client.is_none() && w.tokens.is_none(), "{meetings}");
        let before = server.data.lock().unwrap().teams.as_ref().unwrap().hits.values().sum::<u32>();
        for i in 1..20 {
            w.turn(later + Duration::from_secs(i * 60));
        }
        assert_eq!(server.data.lock().unwrap().teams.as_ref().unwrap().hits.values().sum::<u32>(), before, "nothing more asked");
        assert_eq!(asked(&desk).len(), 1 + renewals.len(), "nor renewed");
    }
}

/// Review Focus 1: "Allow Teams meetings" signs in anew; its page closed or refused, the chats go
/// on from the sign-in kept, as before, without another sign-in, and meetings are said not
/// allowed (the window offers "Allow" again).
#[test]
fn a_consent_page_refused_goes_on_with_the_sign_in_kept() {
    let r = rig(one_chat(), grant(Some(TENANT)));
    r.sign_in("organizations");
    r.wait("the first list", |e| said(e, &Status::UpToDate));
    let (kept, lists) = (r.kept(), r.hits("chats"));
    assert_eq!(kept.as_deref(), Some(REFRESH));
    *r.desk.authorize_fails.lock().unwrap() = Some("Microsoft Teams: AADSTS65004: User declined to consent to access the app.".into());
    let before = teams_of(&r.seen).len();
    r.handle.submit(signing_in(true));
    let seen = r.wait("the list read again", |e| {
        let after = e.get(before..).unwrap_or_default();
        failed(after) || said(after, &Status::SignInAgain) || (r.hits("chats") > lists && listed(after))
    });
    let after = seen.get(before..).unwrap_or_default();
    assert_eq!(after.first(), Some(&status(Status::SigningIn)), "{after:?}");
    assert_eq!(meetings_of(after), [false], "{after:?}");
    assert!(!failed(after) && !said(after, &Status::SignInAgain), "no failure left: {after:?}");
    assert!(listed(after) && r.hits("chats") > lists, "the list is read again: {after:?}");
    assert!(account(after).is_none(), "the same sign-in, not a new one: {after:?}");
    assert_eq!(r.desk.authorized.load(Ordering::SeqCst), 2, "no sign-in in the browser after the refused one");
    assert_eq!(r.kept(), kept, "the token kept is the same");
    assert_eq!(asked(&r.desk), [("authorize", false), ("authorize", true), ("refresh", false)], "gone on with the chats' permissions alone");
}

/// Review Focus 1, without a keychain: the sign-in is held in memory only, and still goes on.
#[test]
fn a_consent_page_refused_without_a_keychain_goes_on_too() {
    // The token is stale at once: renewed from memory, it rotates, so what goes on after can only
    // be what the worker held.
    const ROTATED: &str = "rotated-in-memory-0002";
    let mut tokens = grant(Some(TENANT));
    tokens.expires_at = now_unix() - 5;
    let r = rig_with(one_chat(), tokens, Store::memory(), Arc::new(NoKeychain), Hooks::default());
    *r.desk.rotate.lock().unwrap() = Some(ROTATED.into());
    r.sign_in("organizations");
    r.wait("the first list", |e| failed(e) || said(e, &Status::SignInAgain) || listed(e));
    assert_eq!(*r.desk.presented.lock().unwrap(), [REFRESH]);
    assert_eq!(r.kept(), None);
    let lists = r.hits("chats");
    *r.desk.authorize_fails.lock().unwrap() = Some("Microsoft Teams: AADSTS65004: User declined to consent to access the app.".into());
    let before = teams_of(&r.seen).len();
    r.handle.submit(signing_in(true));
    let seen = r.wait("the list read again", |e| {
        let after = e.get(before..).unwrap_or_default();
        failed(after) || said(after, &Status::SignInAgain) || (r.hits("chats") > lists && listed(after))
    });
    let after = seen.get(before..).unwrap_or_default();
    assert!(!said(after, &Status::SignInAgain) && !failed(after), "{after:?}");
    assert_eq!(meetings_of(after), [false], "{after:?}");
    assert!(listed(after) && r.hits("chats") > lists, "the list is read again: {after:?}");
    assert!(said(after, &Status::NoKeychain), "still lasting until Quatrelle closes: {after:?}");
    assert_eq!(*r.desk.presented.lock().unwrap(), [REFRESH, ROTATED], "the next renewal goes with the token held");
    assert_eq!(asked(&r.desk), [("authorize", false), ("refresh", false), ("authorize", true), ("refresh", false)]);
    assert_eq!(r.kept(), None);
    for (path, bytes) in r.store.snapshot(|_| true) {
        assert!(!contains(&bytes, ROTATED.as_bytes()), "{path} holds the refresh token");
    }
}

/// D6: a sign-in asking for the meetings' permission refused, with no sign-in to go on from (nobody
/// signed in, or "sign in again"), fails as any sign-in does, and lets the permission go: the
/// worker says meetings aren't allowed, for the next plain sign-in to ask for the chats' alone.
#[test]
fn a_wide_sign_in_refused_with_nothing_kept_lets_the_permission_go() {
    let refused = "Microsoft Teams: AADSTS65004: User declined to consent to access the app.";
    let why = Status::Failed { why: "The sign-in stopped unexpectedly; try again (AADSTS65004)".into() };
    // Nobody signed in.
    let r = rig(one_chat(), grant(Some(TENANT)));
    *r.desk.authorize_fails.lock().unwrap() = Some(refused.into());
    r.handle.submit(signing_in(true));
    let seen = r.wait("the refusal", failed);
    assert!(said(&seen, &why), "{seen:?}");
    assert!(account(&seen).is_none(), "{seen:?}");
    assert_eq!(meetings_of(&seen), [false], "{seen:?}");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!((r.desk.renewed.load(Ordering::SeqCst), r.all_hits()), (0, 0));
    assert_eq!(asked(&r.desk), [("authorize", true)]);
    assert_eq!(r.kept(), None);

    // Signed in before with the permission, but nothing kept to go on from ("sign in again").
    let server = serve_teams(one_chat());
    let (mut w, desk, seen, _) = by_hand(&server, grant(Some(TENANT)));
    let t0 = Instant::now();
    w.job(Job::Start { settings: TeamsSettings { meetings: true, ..settings(ME) } });
    assert!(said(&teams_of(&seen), &Status::SignInAgain) && w.settings.meetings);
    *desk.authorize_fails.lock().unwrap() = Some(refused.into());
    w.job(signing_in(true));
    turn_until(&mut w, t0, "the refusal", |_| failed(&teams_of(&seen)));
    let said_so = teams_of(&seen);
    assert!(said(&said_so, &why), "{said_so:?}");
    assert_eq!(meetings_of(&said_so), [false], "{said_so:?}");
    assert!(!w.settings.meetings && !w.polling());
    assert_eq!((desk.renewed.load(Ordering::SeqCst), hits(&server, "me")), (0, 0));
    assert_eq!(asked(&desk), [("authorize", true)]);
}

/// T6: credentials that keep the token for this run only (the portable app's) hold the sign-in's,
/// never the newer ones the renewals hand out: going on after the page is refused, the newest,
/// held by the worker, is the one presented.
#[test]
fn a_consent_page_refused_with_credentials_in_memory_goes_on_with_the_newest_token() {
    const ROTATED: &str = "rotated-in-memory-0002";
    let mut tokens = grant(Some(TENANT));
    tokens.expires_at = now_unix() - 5;
    let r = rig_with(one_chat(), tokens, Store::memory(), Arc::new(MemoryCredentials::default()), Hooks::default());
    *r.desk.rotate.lock().unwrap() = Some(ROTATED.into());
    r.sign_in("organizations");
    r.wait("the first list", |e| failed(e) || said(e, &Status::SignInAgain) || listed(e));
    assert_eq!(*r.desk.presented.lock().unwrap(), [REFRESH]);
    assert_eq!(r.kept().as_deref(), Some(REFRESH), "taken for the run, as the sign-in gave it");
    let lists = r.hits("chats");
    *r.desk.authorize_fails.lock().unwrap() = Some("Microsoft Teams: AADSTS65004: User declined to consent to access the app.".into());
    let before = teams_of(&r.seen).len();
    r.handle.submit(signing_in(true));
    let seen = r.wait("the list read again", |e| {
        let after = e.get(before..).unwrap_or_default();
        failed(after) || said(after, &Status::SignInAgain) || (r.hits("chats") > lists && listed(after))
    });
    let after = seen.get(before..).unwrap_or_default();
    assert!(!said(after, &Status::SignInAgain) && !failed(after), "{after:?}");
    assert_eq!(*r.desk.presented.lock().unwrap(), [REFRESH, ROTATED], "the token held, not the sign-in's");
}

/// T6: the system gives no thread for the sign-in's page: it fails as a sign-in that stops does,
/// and the token the worker held isn't kept for a sign-in that never began.
#[test]
fn a_sign_in_whose_thread_cant_start_keeps_no_token() {
    let server = serve_teams(one_chat());
    let (mut w, _, seen, _) = by_hand(&server, grant(Some(TENANT)));
    let t0 = Instant::now();
    w.job(signing_in(false));
    turn_until(&mut w, t0, "the first pass", |_| teams_of(&seen).iter().any(|e| matches!(e, model::Event::Chats { .. })));
    w.hooks.no_sign_in_thread = true;
    w.job(signing_in(true));
    assert!(failed(&teams_of(&seen)), "{:?}", teams_of(&seen));
    assert!(!w.holds_a_resume(), "nothing to go on from is kept");
}

/// T6: what the Options changed while the page for the permission was open (the announcements)
/// is what the chats go on with once it's refused, not what they said before it opened.
#[test]
fn a_consent_page_refused_goes_on_with_the_options_changed_meanwhile() {
    let server = serve_teams(one_chat());
    let (mut w, desk, seen, _) = by_hand(&server, grant(Some(TENANT)));
    let t0 = Instant::now();
    w.job(signing_in(false));
    turn_until(&mut w, t0, "the first pass", |_| teams_of(&seen).iter().any(|e| matches!(e, model::Event::Chats { .. })));
    assert!(w.settings.notify && !w.settings.notify_only_direct);
    *desk.authorize_fails.lock().unwrap() = Some("Microsoft Teams: AADSTS65004: User declined to consent to access the app.".into());
    w.job(signing_in(true));
    let changed = TeamsSettings { notify: false, notify_only_direct: true, ..w.settings.clone() };
    w.job(Job::Start { settings: changed });
    turn_until(&mut w, t0, "going on", |w| w.polling());
    assert!(!w.settings.notify && w.settings.notify_only_direct, "the Options as they are now");
    assert!(!w.settings.meetings);
}
