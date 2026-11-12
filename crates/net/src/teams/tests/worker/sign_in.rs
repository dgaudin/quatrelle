// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Signing in: the refresh token in the keychain only, whose organisation the account is of (never
//! a chat's own), the directory the Options name, the permissions granted, a session without a
//! keychain, a copy made for another account, and a sign-in nobody waits for any more.

use super::*;

#[test]
fn sign_in_keeps_the_refresh_token_in_the_keychain_only() {
    let mut teams = one_chat();
    teams.drive = Some("https://contoso-my.sharepoint.com/personal/morgan_contoso_example".into());
    let r = rig(teams, grant(Some(TENANT)));
    r.sign_in("organizations");
    let seen = r.wait("the first list", |e| said(e, &Status::UpToDate));
    // The page opens through the window (the worker launches nothing itself).
    assert!(r.seen.lock().unwrap().iter().any(|e| matches!(e, Event::OpenUrl { url } if url == SIGN_IN_PAGE)));
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
    let seen = r.wait("the permission said", |e| e.len() >= 2);
    assert!(said(&seen, &Status::PermissionMissing { scope: "https://graph.microsoft.com/Chat.ReadWrite".into() }), "{seen:?}");
    assert!(account(&seen).is_none(), "{seen:?}");
    assert_eq!(r.kept(), None);
    assert_eq!(r.all_hits(), 0, "said before anything is asked of Graph");
}

#[test]
fn ids_that_cant_be_ids_open_no_browser() {
    for (client_id, tenant) in [("", "organizations"), (CLIENT, ""), ("not an id", "organizations"), (CLIENT, "contoso/../evil")] {
        let r = rig(one_chat(), grant(Some(TENANT)));
        r.handle.submit(Job::SignIn { client_id: client_id.into(), tenant: tenant.into() });
        let seen = r.wait("the refusal", failed);
        assert!(!said(&seen, &Status::SigningIn), "{seen:?}");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(r.desk.authorized.load(Ordering::SeqCst), 0, "nothing is sent anywhere for them");
        assert!(!r.opened_the_browser());
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
        w.job(Job::SignIn { client_id: CLIENT.into(), tenant: "organizations".into() });
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
    assert!(teams_of(&seen).is_empty(), "{:?}", teams_of(&seen));
    server.data.lock().unwrap().teams.as_mut().unwrap().throttle_every = None;
    w.turn(t0 + Duration::from_secs(11));
    assert!(said(&teams_of(&seen), &Status::UpToDate), "{:?}", teams_of(&seen));
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
