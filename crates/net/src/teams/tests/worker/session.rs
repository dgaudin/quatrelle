// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A session: the token renewed once when stale or refused, going on from a kept sign-in (and
//! never another account's), no network, switching off, signing out, a panic in a step, and what
//! the log says.

use super::*;

#[test]
fn a_401_renews_once_and_retries() {
    // The token refused once: renewed, and the request made again.
    let mut teams = one_chat();
    teams.refuse_once = Some("chats");
    let r = rig(teams, grant(Some(TENANT)));
    r.sign_in("organizations");
    let seen = r.wait("the list read", |e| said(e, &Status::UpToDate) || said(e, &Status::SignInAgain));
    assert!(!said(&seen, &Status::SignInAgain), "{seen:?}");
    assert_eq!(r.desk.renewed.load(Ordering::SeqCst), 1, "one renewal");
    assert_eq!(*r.desk.presented.lock().unwrap(), [REFRESH]);
    assert_eq!(r.hits("chats"), 2, "refused, then read");

    // The renewed token refused too: sign in again, and nothing more is asked of Teams.
    let mut teams = one_chat();
    teams.token_expires = Some(("chats", 1, "a-token-nobody-has".into()));
    let r = rig(teams, grant(Some(TENANT)));
    r.sign_in("organizations");
    let seen = r.wait("sign in again", |e| said(e, &Status::UpToDate) || said(e, &Status::SignInAgain));
    assert!(said(&seen, &Status::SignInAgain) && !said(&seen, &Status::UpToDate), "{seen:?}");
    assert_eq!((r.desk.renewed.load(Ordering::SeqCst), r.hits("chats")), (1, 2), "one renewal, one retry");
    let before = r.all_hits();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(r.all_hits(), before, "polling stopped");
    assert_eq!(r.desk.renewed.load(Ordering::SeqCst), 1);

    // A renewal refused: the same, without a retry.
    let mut teams = one_chat();
    teams.refuse_once = Some("chats");
    let r = rig(teams, grant(Some(TENANT)));
    r.desk.refuse.store(true, Ordering::SeqCst);
    r.sign_in("organizations");
    let seen = r.wait("sign in again", |e| said(e, &Status::UpToDate) || said(e, &Status::SignInAgain));
    assert!(said(&seen, &Status::SignInAgain), "{seen:?}");
    assert_eq!((r.desk.renewed.load(Ordering::SeqCst), r.hits("chats")), (1, 1));
}

#[test]
fn a_stale_token_is_renewed_once_after_a_sleep() {
    let server = serve_teams(one_chat());
    let (mut w, desk, seen, _) = by_hand(&server, grant(Some(TENANT)));
    let t0 = Instant::now();
    w.job(Job::SignIn { client_id: CLIENT.into(), tenant: "organizations".into() });
    turn_until(&mut w, t0, "the first list", |_| said(&teams_of(&seen), &Status::UpToDate));
    assert_eq!((desk.renewed.load(Ordering::SeqCst), hits(&server, "chats")), (0, 1));

    // The computer slept the night: the wall clock went on, so the token has run out, and every
    // cadence is due at once.
    w.tokens.as_mut().unwrap().expires_at = now_unix() - 8 * 3600;
    let woke = t0 + Duration::from_secs(8 * 3600);
    w.turn(woke);
    assert_eq!(desk.renewed.load(Ordering::SeqCst), 1, "renewed at the first tick, before anything is asked");
    for i in 1..6 {
        w.turn(woke + Duration::from_millis(200 * i));
    }
    assert_eq!(desk.renewed.load(Ordering::SeqCst), 1, "and once");
    assert_eq!(hits(&server, "chats"), 2, "one list for the night, not the ones missed");
    assert!(!said(&teams_of(&seen), &Status::SignInAgain));

    // A token within its last minute is renewed too, before it runs out mid-request.
    w.tokens.as_mut().unwrap().expires_at = now_unix() + 30;
    w.turn(woke + Duration::from_secs(60));
    assert_eq!(desk.renewed.load(Ordering::SeqCst), 2);

    // A sign-in service whose tokens have run out as they come isn't asked at every turn.
    desk.short.store(true, Ordering::SeqCst);
    w.tokens.as_mut().unwrap().expires_at = now_unix() - 1;
    let later = woke + Duration::from_secs(120);
    for i in 0..100 {
        w.turn(later + Duration::from_millis(100 * i));
    }
    assert_eq!(desk.renewed.load(Ordering::SeqCst), 3, "one renewal in ten seconds of turns");
    w.turn(later + Duration::from_secs(31));
    assert_eq!(desk.renewed.load(Ordering::SeqCst), 4, "and the next half a minute on");
    assert!(!said(&teams_of(&seen), &Status::SignInAgain));
}

#[test]
fn switching_off_stops_within_a_second() {
    // 300 chats: six pages of the list, a second apart.
    let chats = (0..300).map(|i| chat(&format!("19:c{i}@thread.v2"), T0 + i * MINUTE, 1)).collect();
    let mut r = rig(teams_with(chats), grant(Some(TENANT)));
    r.sign_in("organizations");
    r.wait("the second page of the list", |e| failed(e) || r.hits("chats") >= 2);
    // The third page waits for its second (one list read a second): the switch comes during
    // that wait, which a sleep wouldn't hear.
    std::thread::sleep(Duration::from_millis(300));
    let asked = Instant::now();
    r.handle.submit(Job::Stop);
    assert!(r.handle.finished_within(Duration::from_secs(1)), "still running a second after the switch");
    assert!(asked.elapsed() < Duration::from_secs(1));
    let (pages, all) = (r.hits("chats"), r.all_hits());
    assert!(pages < 6, "stopped in the middle of the list: {pages} pages");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(r.all_hits(), all, "no request after the switch");
}

/// A keychain that notes, each time the refresh token is deleted, whether the copy's file was
/// still there.
struct Watched {
    inner: MemoryCredentials,
    file: PathBuf,
    there: Mutex<Vec<bool>>,
}

impl Credentials for Watched {
    fn get(&self, a: &AccountConfig) -> Option<String> {
        self.inner.get(a)
    }
    fn set(&self, a: &AccountConfig, p: &str) -> std::result::Result<(), String> {
        self.inner.set(a, p)
    }
    fn delete(&self, a: &AccountConfig) {
        self.inner.delete(a)
    }
    fn get_smtp(&self, a: &AccountConfig) -> Option<String> {
        self.inner.get_smtp(a)
    }
    fn set_smtp(&self, a: &AccountConfig, p: &str) -> std::result::Result<(), String> {
        self.inner.set_smtp(a, p)
    }
    fn delete_smtp(&self, a: &AccountConfig) {
        self.inner.delete_smtp(a)
    }
    fn get_secret(&self, a: &AccountConfig, what: &str) -> Option<String> {
        self.inner.get_secret(a, what)
    }
    fn set_secret(&self, a: &AccountConfig, what: &str, secret: &str) -> std::result::Result<(), String> {
        self.inner.set_secret(a, what, secret)
    }
    fn delete_secret(&self, a: &AccountConfig, what: &str) {
        self.there.lock().unwrap().push(self.file.exists());
        self.inner.delete_secret(a, what)
    }
}

#[test]
fn sign_out_deletes_the_token_then_the_copy_if_asked() {
    let t = temp("sign-out");
    let files = t.join("files");
    let file = files.join("Teams.quatrelle");
    let creds = Arc::new(Watched { inner: MemoryCredentials::default(), file: file.clone(), there: Mutex::new(Vec::new()) });
    let store = Store::open_packed(t.join("app"), &files).unwrap();
    let r = rig_with(one_chat(), grant(Some(TENANT)), store.clone(), creds.clone(), Hooks::default());
    r.sign_in("organizations");
    r.wait("the first list", |e| said(e, &Status::UpToDate));
    assert!(file.exists() && r.kept().is_some());

    // Signed out, the copy kept: the token goes, the file stays, and Teams is asked nothing more.
    r.handle.submit(Job::SignOut { delete_copy: false });
    r.wait("signed out", |e| said(e, &Status::SignedOut));
    assert_eq!(r.kept(), None);
    assert!(file.exists(), "the copy is kept");
    let before = r.all_hits();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(r.all_hits(), before, "nothing more is asked once signed out");

    // The copy deleted, by a worker that never opened it (the pool lets a worker go once signed
    // out): the file goes, and every copy of it set aside as damaged, whichever name it had;
    // another account's files stay, and so does a copy set aside for the user to keep.
    let gone = [files.join("Teams.quatrelle.damaged"), files.join("Teams (2).quatrelle.damaged")];
    let stay = [files.join("Notes.quatrelle.damaged"), files.join("Teams-1a2b3c4d.quatrelle.old"), files.join("Teams (x).quatrelle.damaged")];
    for f in gone.iter().chain(&stay) {
        std::fs::write(f, b"x").unwrap();
    }
    creds.set_secret(&keyring(), REFRESH_KEY, REFRESH).unwrap();
    let seen = Seen::default();
    let again =
        start_with(store, creds.clone(), emit_into(&seen), r.server.base.clone(), Box::new(AtDesk(r.desk.clone())), Hooks::default()).unwrap();
    again.submit(Job::SignOut { delete_copy: true });
    let until = Instant::now() + WAIT;
    while !said(&teams_of(&seen), &Status::SignedOut) {
        assert!(Instant::now() < until, "never signed out: {:?}", teams_of(&seen));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(creds.get_secret(&keyring(), REFRESH_KEY), None);
    assert!(!file.exists(), "the copy is deleted");
    assert!(gone.iter().all(|f| !f.exists()), "and every copy of it set aside as damaged");
    assert!(stay.iter().all(|f| f.exists()), "what isn't the copy's stays");
    assert_eq!(*creds.there.lock().unwrap(), [true, true], "the token is deleted first, each time: the file was still there");
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn start_goes_on_from_the_kept_sign_in_and_never_opens_the_browser() {
    // No sign-in kept: it says to sign in again, and waits for the user to.
    let r = rig(one_chat(), grant(Some(TENANT)));
    r.handle.submit(Job::Start { settings: settings(ME) });
    r.wait("sign in again", |e| said(e, &Status::SignInAgain));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(r.desk.authorized.load(Ordering::SeqCst), 0);
    assert!(!r.opened_the_browser());
    assert_eq!(r.all_hits(), 0);
    assert!(r.store.snapshot(|_| true).iter().any(|(path, _)| path.ends_with("identity.json")), "the copy is open to read all the same");

    // One kept: renewed, Graph asked who it is, and the list read.
    let r = rig(one_chat(), grant(Some(TENANT)));
    r.creds.set_secret(&keyring(), REFRESH_KEY, REFRESH).unwrap();
    r.handle.submit(Job::Start { settings: settings(ME) });
    let seen = r.wait("the first list", |e| said(e, &Status::UpToDate) || said(e, &Status::SignInAgain));
    assert!(!said(&seen, &Status::SignInAgain), "{seen:?}");
    assert_eq!((r.desk.authorized.load(Ordering::SeqCst), r.desk.renewed.load(Ordering::SeqCst)), (0, 1));
    assert_eq!(*r.desk.presented.lock().unwrap(), [REFRESH]);
    assert!(!r.opened_the_browser());
    assert_eq!((r.hits("me"), r.hits("chats")), (1, 1));
    assert!(r.store.snapshot(|_| true).iter().any(|(path, _)| path.ends_with("identity.json")), "the copy is opened");

    // Switched off in the Options, or nobody signed in there: nothing runs, whatever is kept.
    for off in [TeamsSettings { on: false, ..settings(ME) }, TeamsSettings { signed_in: false, ..settings(ME) }] {
        let r = rig(one_chat(), grant(Some(TENANT)));
        r.creds.set_secret(&keyring(), REFRESH_KEY, REFRESH).unwrap();
        r.handle.submit(Job::Start { settings: off });
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!((r.desk.renewed.load(Ordering::SeqCst), r.all_hits()), (0, 0));
        assert!(teams_of(&r.seen).is_empty() && r.store.snapshot(|_| true).is_empty());
    }

    // Options (read back from a file) that name a directory by its id and another organisation
    // for the account: not gone on with.
    let r = rig(one_chat(), grant(Some(TENANT)));
    r.creds.set_secret(&keyring(), REFRESH_KEY, REFRESH).unwrap();
    r.handle.submit(Job::Start { settings: TeamsSettings { tenant: "aaaaaaaa-1111-2222-3333-bbbbbbbbbbbb".into(), ..settings(ME) } });
    r.wait("sign in again", |e| said(e, &Status::SignInAgain));
    assert_eq!((r.desk.renewed.load(Ordering::SeqCst), r.all_hits()), (0, 0));
    assert!(r.store.snapshot(|_| true).is_empty(), "and no copy is opened for it");
}

#[test]
fn start_again_for_the_account_going_only_takes_the_options() {
    let server = serve_teams(one_chat());
    let (mut w, desk, seen, creds) = by_hand(&server, grant(Some(TENANT)));
    creds.set_secret(&keyring(), REFRESH_KEY, REFRESH).unwrap();
    let t0 = Instant::now();
    w.job(Job::Start { settings: settings(ME) });
    w.turn(t0);
    assert!(said(&teams_of(&seen), &Status::UpToDate), "{:?}", teams_of(&seen));
    let before = (desk.renewed.load(Ordering::SeqCst), hits(&server, "me"), teams_of(&seen).len());
    w.job(Job::Start { settings: TeamsSettings { notify: false, object_id: ME.to_uppercase(), ..settings(ME) } });
    w.turn(t0 + Duration::from_millis(100));
    assert_eq!((desk.renewed.load(Ordering::SeqCst), hits(&server, "me"), teams_of(&seen).len()), before, "nothing is begun again");
    assert!(!w.settings.notify && w.client.is_some());
}

#[test]
fn a_kept_sign_in_of_another_account_is_never_used_for_this_copy() {
    // The Options say one account; the token kept is another's (Graph says whose it is).
    let r = rig(one_chat(), grant(Some(TENANT)));
    r.creds.set_secret(&keyring(), REFRESH_KEY, REFRESH).unwrap();
    r.handle.submit(Job::Start { settings: settings("u-someone-else") });
    let seen = r.wait("sign in again", |e| said(e, &Status::UpToDate) || said(e, &Status::SignInAgain));
    assert!(said(&seen, &Status::SignInAgain) && !said(&seen, &Status::UpToDate), "{seen:?}");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!((r.hits("me"), r.hits("chats"), r.hits("members")), (1, 0, 0), "who it is, and nothing of its chats");
}

#[test]
fn offline_at_start_is_said_and_tried_again_without_haste() {
    let server = serve_teams(one_chat());
    let (mut w, desk, seen, creds) = by_hand(&server, grant(Some(TENANT)));
    creds.set_secret(&keyring(), REFRESH_KEY, REFRESH).unwrap();
    desk.offline.store(true, Ordering::SeqCst);
    let t0 = Instant::now();
    w.job(Job::Start { settings: settings(ME) });
    w.turn(t0);
    let said_so = teams_of(&seen);
    assert!(said(&said_so, &Status::Offline) && !said(&said_so, &Status::SignInAgain), "{said_so:?}");
    assert!(w.copy.is_some(), "the copy is open to read all the same");
    // Back, but not tried at every turn.
    desk.offline.store(false, Ordering::SeqCst);
    for ms in [100, 500, 2000] {
        w.turn(t0 + Duration::from_millis(ms));
    }
    assert_eq!(desk.renewed.load(Ordering::SeqCst), 0, "not tried again at once");
    for secs in [11, 12, 13] {
        w.turn(t0 + Duration::from_secs(secs));
    }
    assert_eq!(desk.renewed.load(Ordering::SeqCst), 1);
    assert!(said(&teams_of(&seen), &Status::UpToDate), "{:?}", teams_of(&seen));
    assert_eq!(creds.get_secret(&keyring(), REFRESH_KEY).as_deref(), Some(REFRESH), "the sign-in is still kept");
}

#[test]
fn a_panic_in_a_step_waits_and_goes_on() {
    let hooks = Hooks { trip_list: Arc::new(AtomicU32::new(1)), panic_wait: Some(Duration::from_secs(3)) };
    let r = rig_with(one_chat(), grant(Some(TENANT)), Store::memory(), Arc::new(MemoryCredentials::default()), hooks);
    r.sign_in("organizations");
    let signed_in = r.wait("the account", |e| account(e).is_some() || failed(e));
    let at = Instant::now();
    // The first list pass panics: nothing is said or asked for the wait.
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(r.hits("chats"), 0, "the step that panicked asked nothing");
    assert_eq!(teams_of(&r.seen), signed_in, "and nothing is said while it waits");
    // Then it lists, as if nothing had happened.
    r.wait("the list after the wait", |e| said(e, &Status::UpToDate));
    assert!(at.elapsed() >= Duration::from_secs(2), "{:?}", at.elapsed());
    assert_eq!(r.hits("chats"), 1);
}

#[test]
fn logs_carry_no_id() {
    // Names, addresses, ids and a topic no other test has.
    const OID: &str = "u-zebra-oid-7731";
    const ORG: &str = "tenant-okapi-5519";
    const OPTIONS_TENANT: &str = "wombat.onmicrosoft.example";
    let zebra = FakeUser::new(OID, "Zebra Quagga");
    let numbat = FakeUser::new("u-numbat-oid-2204", "Numbat Lima");
    // 13 chats, so the lines of this test can be told from another's.
    let chats: Vec<FakeChat> = (0..13)
        .map(|i| {
            let mut c = FakeChat::new(&format!("19:okapi{i}@thread.v2"), "group", Some("Wombat budget"), vec![numbat.clone(), zebra.clone()]);
            c.tenant = Some("tenant-of-its-maker-9".into());
            let at = T0 + i * MINUTE;
            c.messages.push(FakeMessage::new(&at.to_string(), Some(&numbat.id), &rfc3339(at), "<p>Armadillo figures</p>"));
            c
        })
        .collect();
    let mut teams = FakeTeams { chats, ..FakeTeams::new(zebra, ORG) };
    teams.drive = Some("https://wombat-my.sharepoint.com/personal/zebra".into());
    let server = serve_teams(teams);
    let fake = |change: &dyn Fn(&mut FakeTeams)| change(server.data.lock().unwrap().teams.as_mut().unwrap());

    let tap = crate::fakes::logs::tap(|_| true);
    // No `tid`: the organisation is found by reading chats and their members.
    let (mut w, _, seen, _) = by_hand(&server, grant(None));
    let t0 = Instant::now();
    w.job(Job::SignIn { client_id: CLIENT.into(), tenant: OPTIONS_TENANT.into() });
    turn_until(&mut w, t0, "the account", |_| account(&teams_of(&seen)).is_some() || failed(&teams_of(&seen)));
    assert_eq!(tenant_of(&teams_of(&seen)), Some(ORG), "{:?}", teams_of(&seen));
    // The list fails once, is refused for its token the next time, then is read.
    fake(&|t| {
        t.fail_next.insert("chats".into(), 1);
    });
    w.turn(t0 + Duration::from_secs(2));
    fake(&|t| t.refuse_once = Some("chats"));
    w.turn(t0 + Duration::from_secs(13));
    assert!(said(&teams_of(&seen), &Status::UpToDate), "{:?}", teams_of(&seen));
    w.job(Job::SignOut { delete_copy: true });
    assert!(said(&teams_of(&seen), &Status::SignedOut));

    let lines = tap.lines();
    assert!(lines.iter().any(|l| l.starts_with("teams: ") && l.contains("13 conversations")), "the worker's lines are heard: {lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("teams: ") && l.contains("HTTP 500")), "a failure is said by its status: {lines:?}");
    let private = [
        OID,
        ORG,
        OPTIONS_TENANT,
        CLIENT,
        "numbat",
        "zebra",
        "quagga",
        "lima",
        "okapi",
        "wombat",
        "armadillo",
        "tenant-of-its-maker",
        "contoso",
        "@thread.v2",
        "sharepoint",
        REFRESH,
        TOKEN,
        SIGN_IN_PAGE,
    ];
    for line in &lines {
        let lower = line.to_lowercase();
        for p in private {
            assert!(!lower.contains(&p.to_lowercase()), "{p:?} in the log: {line}");
        }
    }
}

#[test]
fn log_hashes_are_keyed_and_short() {
    let server = serve_teams(one_chat());
    let (a, ..) = by_hand(&server, grant(None));
    let (b, ..) = by_hand(&server, grant(None));
    let hash = a.hashed(CHAT);
    assert!(hash.len() == 8 && hash.bytes().all(|c| c.is_ascii_hexdigit()), "{hash}");
    assert_eq!(a.hashed(CHAT), hash, "the same while the worker lives");
    assert_ne!(a.hashed("19:other@thread.v2"), hash);
    assert_ne!(b.hashed(CHAT), hash, "another key at every start: a hash says nothing from one run to the next");
    // Not the plain SHA-256 of the id, which anyone could work out.
    use sha2::Digest;
    let plain: String = sha2::Sha256::digest(CHAT.as_bytes()).iter().take(4).map(|b| format!("{b:02x}")).collect();
    assert_ne!(hash, plain);
}

#[test]
fn a_message_without_an_id_is_never_kept() {
    let msg = |id: &str, unshowable: bool| ChatMessage { id: id.into(), chat: CHAT.into(), unshowable, ..ChatMessage::default() };
    let kept = with_ids(vec![msg("1", false), msg("", true), msg("2", true), msg("", false)]);
    assert_eq!(
        kept.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["1", "2"],
        "one that can't be shown is kept by its id; none under an empty one"
    );
}
