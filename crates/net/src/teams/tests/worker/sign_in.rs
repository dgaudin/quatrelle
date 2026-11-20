// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Signing in: the page it really opens, the refresh token in the keychain only, whose
//! organisation the account is of (never a chat's own), the directory the Options name, the
//! permissions granted, a session without a keychain, a copy made for another account, and a
//! sign-in nobody waits for any more.

use quatrelle_model::ical::is_microsoft_sign_in;

use super::*;
use crate::oauth::TEAMS_MEETINGS_SCOPE;
use crate::teams::worker::Microsoft;

#[test]
fn sign_in_keeps_the_refresh_token_in_the_keychain_only() {
    let mut teams = one_chat();
    teams.drive = Some("https://contoso-my.sharepoint.com/personal/morgan_contoso_example".into());
    let r = rig(teams, grant(Some(TENANT)));
    r.sign_in("organizations");
    let seen = r.wait("the first list", |e| said(e, &Status::UpToDate));
    // The page opens through the window (the worker launches nothing itself).
    assert!(seen.iter().any(|e| matches!(e, model::Event::SignInPage { url } if url == SIGN_IN_PAGE)));
    assert_eq!(seen.first(), Some(&status(Status::SigningIn)));
    assert_eq!(
        account(&seen),
        Some(&model::Event::Account {
            name: "Morgan Lee".into(),
            address: "morgan@contoso.example".into(),
            object_id: ME.into(),
            tenant_id: TENANT.into(),
            drive_host: Some("contoso-my.sharepoint.com".into()),
        })
    );
    assert!(!said(&seen, &Status::NoKeychain));
    assert_eq!(r.kept().as_deref(), Some(REFRESH), "the keychain holds teams/refresh");
    assert_eq!((r.desk.authorized.load(Ordering::SeqCst), r.desk.renewed.load(Ordering::SeqCst)), (1, 0));
    // The copy is begun, and no file of the store holds a token.
    let files = r.store.snapshot(|_| true);
    assert!(files.iter().any(|(path, _)| path.ends_with("identity.json")), "{:?}", files.iter().map(|(p, _)| p).collect::<Vec<_>>());
    for (path, bytes) in &files {
        for secret in [REFRESH, TOKEN] {
            assert!(!contains(bytes, secret.as_bytes()), "{path} holds a token");
        }
    }
}

/// The page to sign in on goes to the engine as Teams' own, once: the engine opens it only when
/// it's Microsoft's, in the meetings browser's window. Never as the mail's `OpenUrl`, which the
/// window would open as it is.
#[test]
fn the_sign_in_page_goes_to_the_engine_as_teams_own() {
    let r = rig(one_chat(), grant(Some(TENANT)));
    r.sign_in("organizations");
    r.wait("the first list", |e| said(e, &Status::UpToDate));
    let seen = r.seen.lock().unwrap();
    let pages: Vec<&str> = seen
        .iter()
        .filter_map(|e| match e {
            Event::Teams { event: model::Event::SignInPage { url } } => Some(url.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(pages, [SIGN_IN_PAGE]);
    assert!(!seen.iter().any(|e| matches!(e, Event::OpenUrl { .. })), "{seen:?}");
}

#[test]
fn the_tenant_is_never_a_chats_own() {
    // A chat made in another organisation: its own `tenantId` is its creator's.
    let foreign = |members: Vec<FakeUser>| {
        let mut c = FakeChat::new(CHAT, "group", Some("Plans"), members);
        c.tenant = Some("other".into());
        c.messages.push(FakeMessage::new(&T0.to_string(), Some("u-ana"), &rfc3339(T0), "<p>Hello</p>"));
        teams_with(vec![c])
    };
    let (ana, bruno) = (FakeUser::new("u-ana", "Ana Lima"), FakeUser::new("u-bruno", "Bruno Rossi"));
    // The sign-in names no organisation: the user's own entry among the members does.
    let r = rig(foreign(vec![ana.clone(), me()]), grant(None));
    r.sign_in("organizations");
    let seen = r.wait("the account", |e| account(e).is_some() || failed(e));
    assert_eq!(tenant_of(&seen), Some(TENANT), "{seen:?}");
    // Once to find the organisation, once more for the copy (its first pass over).
    r.wait("the first pass", |e| e.iter().any(|e| matches!(e, model::Event::Chats { .. })));
    assert_eq!(r.hits("members"), 2);

    // No entry of the user's own: nothing says the organisation, and the chat's isn't taken.
    let r = rig(foreign(vec![ana, bruno]), grant(None));
    r.sign_in("organizations");
    let seen = r.wait("the refusal", |e| account(e).is_some() || failed(e));
    assert!(said(&seen, &Status::Failed { why: NO_ORGANISATION.into() }), "{seen:?}");
    assert!(account(&seen).is_none(), "{seen:?}");
    assert_eq!(r.kept(), None, "nothing is kept of a sign-in refused");
    assert!(r.store.snapshot(|_| true).is_empty(), "and no copy is begun");
}

#[test]
fn a_tid_other_than_the_guid_in_options_is_refused() {
    const OURS: &str = "aaaaaaaa-1111-2222-3333-bbbbbbbbbbbb";
    const THEIRS: &str = "cccccccc-4444-5555-6666-dddddddddddd";
    let r = rig(one_chat(), grant(Some(THEIRS)));
    r.sign_in(OURS);
    let seen = r.wait("the refusal", |e| account(e).is_some() || failed(e));
    assert!(said(&seen, &Status::Failed { why: NO_ORGANISATION.into() }), "{seen:?}");
    assert!(account(&seen).is_none(), "{seen:?}");
    assert_eq!(r.kept(), None);
    assert_eq!(r.all_hits(), 0, "refused before anything is asked of Graph");

    // The directory asked for, however its letters are cased, is taken.
    let r = rig(one_chat(), grant(Some(&OURS.to_uppercase())));
    r.sign_in(OURS);
    let seen = r.wait("the account", |e| account(e).is_some() || failed(e));
    assert_eq!(tenant_of(&seen), Some(OURS.to_uppercase().as_str()), "{seen:?}");

    // Without a `tid`, the organisation the user's own member entry names must be it too.
    let r = rig(one_chat(), grant(None));
    r.sign_in(OURS);
    let seen = r.wait("the refusal", |e| account(e).is_some() || failed(e));
    assert!(said(&seen, &Status::Failed { why: NO_ORGANISATION.into() }) && account(&seen).is_none(), "{seen:?}");
    assert_eq!(r.kept(), None);
}

#[test]
fn a_missing_scope_is_said() {
    let mut tokens = grant(Some(TENANT));
    tokens.granted = TEAMS_SCOPE.split(' ').filter(|s| !s.ends_with("/Chat.ReadWrite")).collect::<Vec<_>>().join(" ");
    let r = rig(one_chat(), tokens);
    r.sign_in("organizations");
    // The status after "signing in" (the page to sign in on comes between them).
    let seen = r.wait("the permission said", |e| e.iter().filter(|x| matches!(x, model::Event::Status { .. })).count() >= 2);
    assert!(said(&seen, &Status::PermissionMissing { scope: "https://graph.microsoft.com/Chat.ReadWrite".into() }), "{seen:?}");
    assert!(account(&seen).is_none(), "{seen:?}");
    assert_eq!(r.kept(), None);
    assert_eq!(r.all_hits(), 0, "said before anything is asked of Graph");
}

#[test]
fn ids_that_cant_be_ids_open_no_browser() {
    for (client_id, tenant) in [("", "organizations"), (CLIENT, ""), ("not an id", "organizations"), (CLIENT, "contoso/../evil")] {
        let r = rig(one_chat(), grant(Some(TENANT)));
        r.handle.submit(Job::SignIn { client_id: client_id.into(), tenant: tenant.into(), meetings: false });
        let seen = r.wait("the refusal", failed);
        assert!(!said(&seen, &Status::SigningIn), "{seen:?}");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(r.desk.authorized.load(Ordering::SeqCst), 0, "nothing is sent anywhere for them");
        assert!(!r.opened_the_browser());
    }
}

/// A-m1 (B-m1): the page the Teams sign-in really opens (Microsoft's, at the directory the Options
/// name, with Teams' permissions and every parameter it sends) is one the engine lets open; else a
/// permission added or a parameter left unencoded would stop the sign-in from opening, every other
/// test green. The longest directory name the Options take too.
#[test]
fn the_sign_in_page_really_opened_is_one_the_engine_lets_open() {
    let longest = format!("{}.example", "a".repeat(245));
    assert!(longest.len() == 253 && quatrelle_model::valid_oauth_tenant(&longest));
    for tenant in ["organizations", "contoso.onmicrosoft.com", "00000000-0000-0000-0000-000000000001", longest.as_str()] {
        // The meetings' permission asked for too ("Allow Teams meetings"): the page still opens.
        for meetings in [false, true] {
            let mut opened = None;
            let mut open = |url: &str| {
                opened = Some(url.to_string());
                Err("the test opens nothing".to_string())
            };
            assert!(Microsoft.authorize(CLIENT, tenant, meetings, &mut open).is_err(), "stopped where the browser would open");
            let url = opened.unwrap_or_else(|| panic!("no page opened for {tenant}"));
            assert!(is_microsoft_sign_in(&url), "{} bytes: {url}", url.len());
            let scope = if meetings { format!("{TEAMS_SCOPE} {TEAMS_MEETINGS_SCOPE}") } else { TEAMS_SCOPE.to_string() };
            assert!(url.contains(&format!("scope={}", crate::http::pct(&scope))), "Teams' permissions asked: {url}");
            assert_eq!(url.contains("OnlineMeetings"), meetings, "{url}");
        }
    }
}

#[test]
fn a_scope_is_granted_under_either_of_its_names() {
    let chat = Some("https://graph.microsoft.com/Chat.ReadWrite".to_string());
    assert_eq!(missing_scope(""), None, "not said: not judged");
    assert_eq!(missing_scope(TEAMS_SCOPE), None);
    // Microsoft may list them by their short names, and never lists `offline_access`.
    let short = "User.Read User.ReadBasic.All Chat.ReadWrite Presence.Read.All Files.ReadWrite Files.Read.All";
    assert_eq!(missing_scope(short), None);
    assert_eq!(missing_scope(&short.to_lowercase()), None);
    assert_eq!(missing_scope(&format!("openid email {}", short.replace("Chat.ReadWrite", "Chat.Read"))), chat, "Chat.Read isn't Chat.ReadWrite");
    assert_eq!(
        missing_scope(&short.replace("Files.ReadWrite ", "Files.ReadWrite.All ")),
        Some("https://graph.microsoft.com/Files.ReadWrite".to_string()),
        "nor a longer name a shorter one"
    );
}

/// A session without a secret service: nothing can be kept.
struct NoKeychain;

impl Credentials for NoKeychain {
    fn get(&self, _: &AccountConfig) -> Option<String> {
        None
    }
    fn set(&self, _: &AccountConfig, _: &str) -> std::result::Result<(), String> {
        Err("no keychain".into())
    }
    fn delete(&self, _: &AccountConfig) {}
    fn get_smtp(&self, _: &AccountConfig) -> Option<String> {
        None
    }
    fn set_smtp(&self, _: &AccountConfig, _: &str) -> std::result::Result<(), String> {
        Err("no keychain".into())
    }
    fn delete_smtp(&self, _: &AccountConfig) {}
}

#[test]
fn without_a_keychain_the_sign_in_lasts_the_session() {
    // The token is stale at once, so the session has to renew it from what it holds in memory.
    let mut tokens = grant(Some(TENANT));
    tokens.expires_at = now_unix() - 5;
    let r = rig_with(one_chat(), tokens, Store::memory(), Arc::new(NoKeychain), Hooks::default());
    r.sign_in("organizations");
    let seen = r.wait("the first list", |e| failed(e) || r.hits("chats") >= 1);
    assert!(account(&seen).is_some(), "signed in all the same: {seen:?}");
    assert!(said(&seen, &Status::NoKeychain), "and told it won't be remembered: {seen:?}");
    assert!(!said(&seen, &Status::SignInAgain) && !failed(&seen), "{seen:?}");
    assert_eq!(*r.desk.presented.lock().unwrap(), [REFRESH], "renewed from memory");
    assert_eq!(r.kept(), None);
    for (path, bytes) in r.store.snapshot(|_| true) {
        assert!(!contains(&bytes, REFRESH.as_bytes()), "{path} holds the refresh token");
    }
}

#[test]
fn a_copy_made_for_another_account_is_set_aside_and_said() {
    let t = temp("set-aside");
    let store = Store::open_packed(t.join("app"), t.join("files")).unwrap();
    // Someone else's copy, holding a chat (not one this account has: the sync copies those).
    let other = Identity { tenant_id: TENANT.into(), object_id: "u-someone-else".into() };
    let (copy, _) = Copy::open(store.clone(), &other).unwrap();
    copy.put_chat(&ChatRecord::new(chat_of(&json!({"id": "19:theirs@thread.v2", "chatType": "group"})).unwrap())).unwrap();
    drop(copy);

    let r = rig_with(one_chat(), grant(Some(TENANT)), store.clone(), Arc::new(Kept::default()), Hooks::default());
    r.sign_in("organizations");
    let seen = r.wait("the first list", |e| said(e, &Status::UpToDate));
    let aside = seen.iter().find_map(|e| if let model::Event::SetAside { path } = e { Some(PathBuf::from(path)) } else { None });
    let aside = aside.unwrap_or_else(|| panic!("not said: {seen:?}"));
    assert!(aside.exists() && aside.to_string_lossy().ends_with(".quatrelle.old"), "{aside:?}");
    drop(r);
    // The copy is this account's now, with nothing of the other's.
    let (copy, opened) = Copy::open(store, &Identity { tenant_id: TENANT.into(), object_id: ME.into() }).unwrap();
    assert_eq!(opened, Opened::Same);
    assert!(copy.chat("19:theirs@thread.v2").is_none());
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn a_sign_in_nobody_waits_for_any_more_keeps_nothing() {
    // Signed out (or switched off, or signed in anew) while the browser's page was open: the
    // answer that comes after is dropped with its tokens.
    for then in [Job::SignOut { delete_copy: false }, Job::Stop] {
        let server = serve_teams(one_chat());
        let (mut w, desk, seen, creds) = by_hand(&server, grant(Some(TENANT)));
        let t0 = Instant::now();
        w.job(Job::SignIn { client_id: CLIENT.into(), tenant: "organizations".into(), meetings: false });
        w.job(then);
        let until = Instant::now() + WAIT;
        while desk.authorized.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(5));
        }
        for i in 0..20 {
            w.turn(t0 + Duration::from_millis(50 * i));
            std::thread::sleep(Duration::from_millis(5));
        }
        let seen = teams_of(&seen);
        assert!(account(&seen).is_none(), "{seen:?}");
        assert_eq!(creds.get_secret(&keyring(), REFRESH_KEY), None);
        assert!(w.client.is_none() && w.tokens.is_none() && w.copy.is_none());
        assert_eq!(hits(&server, "me"), 0, "its token is never used");
    }
}

#[test]
fn a_directory_id_is_told_from_a_domain() {
    assert!(is_guid("aaaaaaaa-1111-2222-3333-bbbbbbbbbbbb") && is_guid("AAAAAAAA-1111-2222-3333-BBBBBBBBBBBB"));
    for not in [
        "",
        "organizations",
        "contoso.onmicrosoft.com",
        "aaaaaaaa-1111-2222-3333-bbbbbbbbbbb",
        "aaaaaaaa-1111-2222-3333-bbbbbbbbbbbbb",
        "aaaaaaaa11112222-3333-bbbbbbbbbbbb-",
        "gaaaaaaa-1111-2222-3333-bbbbbbbbbbbb",
    ] {
        assert!(!is_guid(not), "{not}");
    }
}

#[test]
fn the_keychain_entry_is_teams_refresh() {
    let k = keyring();
    assert_eq!((k.id.as_str(), k.email.as_str(), REFRESH_KEY), ("teams", "teams", "refresh"));
    assert_eq!((crate::teams::keyring().id, crate::teams::REFRESH_KEY), ("teams".to_string(), "refresh"));
}

#[test]
fn a_busy_graph_is_never_sat_out_in_the_loop() {
    // Graph answers 429 to everything: the sign-in says so at once, asking once (a wait in the
    // loop would keep it deaf to its jobs).
    let busy = || FakeTeams { throttle_every: Some(1), ..one_chat() };
    let r = rig(busy(), grant(Some(TENANT)));
    r.sign_in("organizations");
    let seen = r.wait("the refusal", |e| account(e).is_some() || failed(e));
    assert!(seen.iter().any(|e| matches!(e, model::Event::Status { status: Status::Failed { why } } if why.contains("HTTP 429"))), "{seen:?}");
    assert_eq!(r.hits("me"), 1);
    assert_eq!(r.kept(), None);

    // At a start it is tried again later, the sign-in still good.
    let server = serve_teams(busy());
    let (mut w, _, seen, creds) = by_hand(&server, grant(Some(TENANT)));
    creds.set_secret(&keyring(), REFRESH_KEY, REFRESH).unwrap();
    let t0 = Instant::now();
    w.job(Job::Start { settings: settings(ME) });
    w.turn(t0);
    w.turn(t0 + Duration::from_secs(1));
    assert_eq!(hits(&server, "me"), 1);
    // Nothing said but the outbox's count, from the copy opened.
    assert!(teams_of(&seen).iter().all(|e| *e == model::Event::Unsent { count: 0 }), "{:?}", teams_of(&seen));
    server.data.lock().unwrap().teams.as_mut().unwrap().throttle_every = None;
    w.turn(t0 + Duration::from_secs(11));
    assert!(listed(&teams_of(&seen)), "{:?}", teams_of(&seen));
}

#[test]
fn credentials_that_keep_nothing_say_so() {
    // The portable app's credentials: held in memory, so taking the token isn't keeping it.
    let r = rig_with(one_chat(), grant(Some(TENANT)), Store::memory(), Arc::new(MemoryCredentials::default()), Hooks::default());
    r.sign_in("organizations");
    let seen = r.wait("the first list", |e| failed(e) || r.hits("chats") >= 1);
    assert!(account(&seen).is_some(), "signed in: {seen:?}");
    assert!(said(&seen, &Status::NoKeychain), "and told it won't be remembered: {seen:?}");
    assert!(!said(&seen, &Status::SignInAgain) && !failed(&seen), "{seen:?}");
    // Taken for the run all the same: switched off and on, it goes on without a new sign-in, and
    // still says it won't be remembered.
    assert_eq!(r.kept().as_deref(), Some(REFRESH));
    r.handle.submit(Job::Stop);
    r.handle.submit(Job::Start { settings: settings(ME) });
    let pages = r.hits("chats");
    r.wait("the list again", |e| failed(e) || said(e, &Status::SignInAgain) || r.hits("chats") > pages);
    let seen = teams_of(&r.seen);
    assert!(!said(&seen, &Status::SignInAgain) && !said(&seen, &Status::UpToDate), "{seen:?}");
    assert_eq!(seen.iter().filter(|e| **e == status(Status::NoKeychain)).count(), 2, "{seen:?}");
    assert_eq!(r.desk.authorized.load(Ordering::SeqCst), 1);
    // Said once at each start, then the status goes on as for anyone: the copy, up to date.
    let goes_on = |e: &[model::Event]| {
        let last = e.iter().rposition(|e| *e == status(Status::NoKeychain));
        last.is_some_and(|i| listed(&e[i..]))
    };
    r.wait("the status after it won't be remembered", goes_on);

    // What says it keeps: the system's keychain (the trait's own answer), not memory.
    assert!(!MemoryCredentials::default().persists());
    assert!(Kept::default().persists() && NoKeychain.persists());
}

#[test]
fn a_panic_while_signing_in_says_it_failed() {
    let hooks = Hooks { trip_sign_in: Arc::new(AtomicU32::new(1)), ..Hooks::default() };
    let r = rig_with(one_chat(), grant(Some(TENANT)), Store::memory(), Arc::new(Kept::default()), hooks);
    r.sign_in("organizations");
    let seen = r.wait("the failure", |e| failed(e) || account(e).is_some());
    assert!(failed(&seen) && account(&seen).is_none(), "not signing in for good: {seen:?}");
    assert_eq!(r.kept(), None, "nothing kept of it");
    // The next sign-in goes through.
    r.sign_in("organizations");
    r.wait("the account", |e| account(e).is_some());
}

#[test]
fn a_copy_that_panics_on_opening_is_set_aside_and_made_again() {
    let t = temp("panic-open");
    let files = t.join("files");
    let store = Store::open_packed(t.join("app"), &files).unwrap();
    // This account's copy, holding a chat.
    let who = Identity { tenant_id: TENANT.into(), object_id: ME.into() };
    let (copy, _) = Copy::open(store.clone(), &who).unwrap();
    copy.put_chat(&ChatRecord::new(chat_of(&json!({"id": "19:kept@thread.v2", "chatType": "group"})).unwrap())).unwrap();
    drop(copy);

    // Reading it panics (a parser's bug, a file's damage).
    let hooks = Hooks { trip_open: Arc::new(AtomicU32::new(1)), ..Hooks::default() };
    let r = rig_with(one_chat(), grant(Some(TENANT)), store.clone(), Arc::new(Kept::default()), hooks);
    r.sign_in("organizations");
    let seen = r.wait("the first list", |e| said(e, &Status::UpToDate) || failed(e));
    assert!(said(&seen, &Status::UpToDate), "signed in all the same: {seen:?}");
    assert!(files.join("Teams.quatrelle.damaged").exists(), "the copy set aside as damaged");
    r.wait("the first pass", |e| e.iter().any(|e| matches!(e, model::Event::Chats { .. })));
    let (copy, opened) = Copy::open(store, &who).unwrap();
    assert_eq!(opened, Opened::Same);
    assert!(copy.chat("19:kept@thread.v2").is_none() && copy.chat(CHAT).is_some(), "made again from the server");
    drop(r);
    let _ = std::fs::remove_dir_all(&t);
}

/// Final review R42: why a sign-in failed is one of the worker's own sentences, with Microsoft's
/// AADSTS code or the HTTP status at most: never Microsoft's words, which can name the account.
#[test]
fn a_sign_in_refused_says_a_sentence_of_its_own_and_a_code_only() {
    let why = |seen: &[model::Event]| {
        seen.iter().rev().find_map(|e| match e {
            model::Event::Status { status: Status::Failed { why } } => Some(why.clone()),
            _ => None,
        })
    };
    let r = rig(one_chat(), grant(Some(TENANT)));
    *r.desk.authorize_fails.lock().unwrap() = Some(
        "Microsoft Teams: AADSTS50020: User account 'zoe@contoso.example' from identity provider 'live.com' does not exist in tenant 'Contoso Ltd'. Trace ID: 0f1e2d3c".into(),
    );
    r.sign_in("organizations");
    let seen = r.wait("the refusal", failed);
    assert_eq!(why(&seen).as_deref(), Some("The sign-in stopped unexpectedly; try again (AADSTS50020)"), "{seen:?}");

    // Graph refusing the token at once: the HTTP status, not the error's words.
    let r = rig(one_chat(), Tokens { access: "not-the-fakes".into(), ..grant(Some(TENANT)) });
    r.sign_in("organizations");
    let seen = r.wait("the refusal", failed);
    assert_eq!(why(&seen).as_deref(), Some("Teams didn't say who signed in (HTTP 401)"), "{seen:?}");
}

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

#[test]
fn a_wide_sign_in_refused_with_nothing_kept_fails_as_before() {
    let refused = "Microsoft Teams: AADSTS65004: User declined to consent to access the app.";
    let why = Status::Failed { why: "The sign-in stopped unexpectedly; try again (AADSTS65004)".into() };
    // Nobody signed in.
    let r = rig(one_chat(), grant(Some(TENANT)));
    *r.desk.authorize_fails.lock().unwrap() = Some(refused.into());
    r.handle.submit(signing_in(true));
    let seen = r.wait("the refusal", failed);
    assert!(said(&seen, &why), "{seen:?}");
    assert!(account(&seen).is_none() && meetings_of(&seen).is_empty(), "{seen:?}");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!((r.desk.renewed.load(Ordering::SeqCst), r.all_hits()), (0, 0));
    assert_eq!(asked(&r.desk), [("authorize", true)]);
    assert_eq!(r.kept(), None);

    // Signed in before, but nothing kept to go on from ("sign in again"): the same.
    let r = rig(one_chat(), grant(Some(TENANT)));
    r.handle.submit(Job::Start { settings: settings(ME) });
    r.wait("sign in again", |e| said(e, &Status::SignInAgain));
    *r.desk.authorize_fails.lock().unwrap() = Some(refused.into());
    r.handle.submit(signing_in(true));
    let seen = r.wait("the refusal", failed);
    assert!(said(&seen, &why) && meetings_of(&seen).is_empty(), "{seen:?}");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!((r.desk.renewed.load(Ordering::SeqCst), r.all_hits()), (0, 0));
    assert_eq!(asked(&r.desk), [("authorize", true)]);
}
