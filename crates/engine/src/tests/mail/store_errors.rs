// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a store that fails to read or write loses no mail (a list that couldn't be read
//! was saved as empty, a source that couldn't be kept left a message without one).

use super::*;

use quatrelle_model::Folder;
use quatrelle_store::FolderState;

const SHORT_FILM: &str = "Projects/Short film";

/// The sample in a cache whose files can be made to fail.
fn failing_session() -> Session {
    let store = Store::failing();
    assert!(sample::seed_if_empty(&store, NOW).unwrap());
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    assert!(s.take_jobs().is_empty());
    s
}

fn store(s: &Session) -> &Store {
    s.store.as_ref().unwrap()
}

fn folder(s: &Session, path: &str) -> FolderState {
    store(s).load_folder(sample::SAMPLE_ACCOUNT, path).unwrap()
}

fn subjects(st: &FolderState) -> Vec<String> {
    let mut v: Vec<String> = st.messages.iter().map(|m| m.subject.clone()).collect();
    v.sort();
    v
}

/// Every message of a folder has its source.
fn all_kept(s: &Session, st: &FolderState) -> bool {
    st.messages.iter().all(|m| store(s).read_body(&m.key).is_some())
}

#[test]
fn a_local_move_that_cant_read_or_write_leaves_the_mail_where_it_was() {
    let mut s = failing_session();
    let inbox = folder(&s, "INBOX");
    let film = folder(&s, SHORT_FILM);
    assert!(!film.messages.is_empty() && all_kept(&s, &inbox) && all_kept(&s, &film));
    // A list, a source that can't be read; a source that can't be written.
    for (reads, writes) in [(Some("state.json"), None), (Some(".eml"), None), (None, Some(".eml"))] {
        let k = key_of(&s, "Character turnarounds");
        s.run("list.select", &k).unwrap();
        let undo = s.undo.len();
        store(&s).fail(reads, writes);
        let r = s.run("mail.move", &json!({"folder": SHORT_FILM}));
        store(&s).fail(None, None);
        // Said by the command (the CLI too), with nothing to undo.
        assert!(matches!(r, Err(CmdError::Failed(_))), "{reads:?} {writes:?}: {r:?}");
        assert_eq!(s.undo.len(), undo, "{reads:?} {writes:?}: no undo for a move that didn't happen");
        assert_eq!(subjects(&folder(&s, "INBOX")), subjects(&inbox), "{reads:?} {writes:?}: still in the Inbox");
        assert_eq!(subjects(&folder(&s, SHORT_FILM)), subjects(&film), "{reads:?} {writes:?}: the destination keeps its mail");
        assert!(all_kept(&s, &folder(&s, "INBOX")) && all_kept(&s, &folder(&s, SHORT_FILM)), "{reads:?} {writes:?}");
        s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
        assert!(s.list.iter().any(|m| m.subject == "Character turnarounds"), "{reads:?} {writes:?}: shown where it is");
    }
    // Working again, it moves.
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.move", &json!({"folder": SHORT_FILM})).unwrap();
    assert_eq!(folder(&s, SHORT_FILM).messages.len(), film.messages.len() + 1);
    assert!(all_kept(&s, &folder(&s, SHORT_FILM)));
}

#[test]
fn filing_locally_that_cant_read_or_write_keeps_the_draft_and_the_message() {
    let mut s = failing_session();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@soundlab.example", "subject": "Mix notes", "body": "First take\n"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    let drafts = folder(&s, "Drafts");
    let first = s.compose.as_ref().unwrap().draft.clone().unwrap();
    assert!(drafts.find(first.uid).is_some() && all_kept(&s, &drafts));
    let sent = folder(&s, "Sent");
    s.run("compose.update", &json!({"body": "Second take\n"})).unwrap();
    for (reads, writes) in [(Some("state.json"), None), (None, Some(".eml"))] {
        store(&s).fail(reads, writes);
        let saved = s.run("compose.saveDraft", &json!({}));
        let kept = s.keep_drafts();
        let sending = s.run("compose.send", &json!({}));
        store(&s).fail(None, None);
        assert!(saved.is_err() && kept == 0 && sending.is_err(), "{reads:?} {writes:?}: {saved:?} {kept} {sending:?}");
        // The earlier draft is still there, whole, and the message is still being written.
        assert_eq!(subjects(&folder(&s, "Drafts")), subjects(&drafts), "{reads:?} {writes:?}");
        assert!(all_kept(&s, &folder(&s, "Drafts")), "{reads:?} {writes:?}");
        assert_eq!(subjects(&folder(&s, "Sent")), subjects(&sent), "{reads:?} {writes:?}: nothing half-sent");
        let c = s.compose.as_ref().unwrap();
        assert_eq!((c.draft.as_ref(), c.body.as_str()), (Some(&first), "Second take\n"), "{reads:?} {writes:?}");
    }
    // Working again, it's saved and sent.
    s.run("compose.saveDraft", &json!({})).unwrap();
    assert!(folder(&s, "Drafts").find(first.uid).is_none(), "the earlier draft goes once the new one is kept");
    s.run("compose.send", &json!({})).unwrap();
    assert!(s.compose.is_none());
    assert_eq!(folder(&s, "Sent").messages.len(), sent.messages.len() + 1);
    assert!(folder(&s, "Drafts").messages.iter().all(|m| m.subject != "Mix notes"));
}

#[test]
fn calendars_and_contacts_that_cant_be_read_arent_written_over() {
    let cache = Store::failing();
    assert!(sample::seed_if_empty(&cache, NOW).unwrap());
    let before = cache.load_account_doc::<serde_json::Value>(sample::SAMPLE_ACCOUNT, quatrelle_model::pim::PIM_DOC).unwrap();
    cache.fail(Some(quatrelle_model::pim::PIM_DOC), None);
    let mut s = Session::open(Some(cache), quatrelle_model::dates::Clock::new(NOW, 0));
    store(&s).fail(None, None);
    assert!(s.notices.iter().any(|n| n.error), "said");
    s.run("contact.create", &json!({"name": "Mira Chen", "emails": "mira@colour.example"})).unwrap();
    let after = store(&s).load_account_doc::<serde_json::Value>(sample::SAMPLE_ACCOUNT, quatrelle_model::pim::PIM_DOC).unwrap();
    assert_eq!(after, before, "what couldn't be read isn't saved over");
    // Gone, the account is forgotten: one added later under its id saves its own.
    s.run("account.remove", &json!({"account": sample::SAMPLE_ACCOUNT})).unwrap();
    assert!(s.pim_unread.is_empty());
}

#[test]
fn a_damaged_source_moves_without_it() {
    let mut s = failing_session();
    let film = folder(&s, SHORT_FILM);
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    store(&s).fail(Some("!.eml"), None);
    s.run("mail.move", &json!({"folder": SHORT_FILM})).unwrap();
    store(&s).fail(None, None);
    assert_eq!(folder(&s, SHORT_FILM).messages.len(), film.messages.len() + 1, "moved, its unreadable source left behind");
    assert!(folder(&s, "INBOX").messages.iter().all(|m| m.subject != "Character turnarounds"));
}

#[test]
fn a_destination_out_of_numbers_takes_nothing_rather_than_two_under_one() {
    let mut s = failing_session();
    let mut film = folder(&s, SHORT_FILM);
    film.uid_next = u32::MAX;
    store(&s).save_folder(sample::SAMPLE_ACCOUNT, SHORT_FILM, &film).unwrap();
    let inbox = folder(&s, "INBOX");
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    assert!(s.run("mail.move", &json!({"folder": SHORT_FILM})).is_err());
    assert_eq!(subjects(&folder(&s, "INBOX")), subjects(&inbox));
    assert_eq!(subjects(&folder(&s, SHORT_FILM)), subjects(&film));
    // Nor is a draft filed there.
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    let mut drafts = folder(&s, "Drafts");
    drafts.uid_next = u32::MAX;
    store(&s).save_folder(sample::SAMPLE_ACCOUNT, "Drafts", &drafts).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@soundlab.example", "subject": "Mix notes", "body": "First take\n"})).unwrap();
    assert!(s.run("compose.saveDraft", &json!({})).is_err());
    assert_eq!(subjects(&folder(&s, "Drafts")), subjects(&drafts));
}

#[test]
fn rules_and_snoozes_that_cant_be_read_arent_written_over() {
    let cache = Store::failing();
    assert!(sample::seed_if_empty(&cache, NOW).unwrap());
    let rule = json!({"name": "Print", "conditions": [{"if": "from", "value": "printshop"}], "actions": [{"do": "markRead"}]});
    let mut s = Session::open(Some(cache.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.run("rules.add", &json!({"rule": rule})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.snooze", &json!({"until": NOW + 3600})).unwrap();
    let docs = || (cache.load_doc::<serde_json::Value>("rules.json").unwrap(), cache.load_doc::<serde_json::Value>("snoozes.json").unwrap());
    drop(s);
    for name in ["rules.json", "snoozes.json"] {
        let before = docs();
        cache.fail(Some(name), None);
        let mut s = Session::open(Some(cache.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
        cache.fail(None, None);
        assert!(s.notices.iter().any(|n| n.error), "{name}: said");
        let _ = s.run("rules.add", &json!({"rule": {"name": "Other", "conditions": [{"if": "subject", "value": "x"}], "actions": [{"do": "flag"}]}}));
        let k = json!({"key": s.list[0].key.to_id()});
        let _ = s.run("list.select", &k);
        let _ = s.run("mail.snooze", &json!({"until": NOW + 7200}));
        s.clock = quatrelle_model::dates::Clock::new(NOW + 4000, 0);
        s.tick();
        let after = docs();
        if name == "rules.json" {
            assert_eq!(after.0, before.0, "the rules aren't saved over");
        } else {
            assert_eq!(after.1, before.1, "the snoozes aren't saved over");
        }
    }
}

/// The Replied and Forwarded flags owed by replies waiting in the Outbox (`pending-marks.json`),
/// when they can't be read at start, aren't saved over by the next reply queued.
#[test]
fn owed_flags_that_cant_be_read_arent_written_over() {
    let cache = Store::failing();
    let mut s = Session::open(Some(cache.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let inbox =
        quatrelle_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    let key = quatrelle_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid: 1 };
    let hello = quatrelle_model::Summary { key: key.clone(), subject: "Hello".into(), date: NOW - 60, ..Default::default() };
    cache.save_folder(&id, "INBOX", &FolderState { validity: 9, uid_next: 2, messages: vec![hello], ..Default::default() }).unwrap();
    // A reply the network layer couldn't send: in the Outbox, its Replied flag owed (the
    // account's folders as its sync finds them).
    let queue_reply = |s: &mut Session| {
        s.apply(Event::Folders { account: id.clone(), folders: vec![inbox.clone()] });
        s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
        s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
        s.run("list.open", &json!({"key": key.to_id()})).unwrap();
        s.apply(Event::Body { key: key.clone(), raw: b"From: a@b.example\r\nSubject: Hello\r\nMessage-ID: <h@b>\r\n\r\nHi\r\n".to_vec() });
        s.run("mail.reply", &json!({})).unwrap();
        s.run("compose.send", &json!({})).unwrap();
        let Some(Job::Send { id: send_id, .. }) = s.take_jobs().into_iter().find(|j| matches!(j, Job::Send { .. })) else { panic!("not sent") };
        s.apply(Event::Queued { account: id.clone(), id: send_id, error: "offline".into() });
    };
    queue_reply(&mut s);
    let owed = || cache.load_doc::<Value>("pending-marks.json").unwrap();
    let before = owed();
    assert_eq!(before.as_array().map(Vec::len), Some(1), "{before}");
    drop(s);
    cache.fail(Some("pending-marks.json"), None);
    // (Later, so the reply has an id of its own.)
    let mut s = Session::open(Some(cache.clone()), quatrelle_model::dates::Clock::new(NOW + 60, 0));
    cache.fail(None, None);
    // Said, as rules and snoozed mail that couldn't be read are (it was only logged).
    assert!(s.notices.iter().any(|n| n.error && n.text.contains("Outbox")), "{:?}", s.notices);
    s.take_jobs();
    queue_reply(&mut s);
    assert_eq!(owed(), before, "the owed flags aren't saved over");
}

/// A damaged `pending-marks.json` isn't read as none: it's set aside whole (kept beside, never
/// written over) and that's said, and the owed flags start again from an empty list that is saved.
/// When it can't be set aside it stays where it is, said, and nothing is saved over it.
#[test]
fn damaged_owed_flags_are_set_aside_and_said() {
    let said = |s: &Session| s.notices.iter().any(|n| n.error && n.text.contains("Outbox"));
    let cache = Store::failing();
    cache.save_doc("pending-marks.json", "not a list").unwrap();
    let s = Session::open(Some(cache.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    assert!(said(&s), "{:?}", s.notices);
    assert_eq!(cache.load_doc_strict::<Value>("pending-marks.json.bad").unwrap(), json!("not a list"), "not set aside as it was");
    assert!(cache.load_doc_strict::<Vec<Value>>("pending-marks.json").unwrap().is_empty(), "left in its place");
    assert!(!s.docs_unread.contains("pending-marks.json"), "saved over only once set aside");
    // A copy that can't be written: the file stays, whole, and isn't saved over.
    let cache = Store::failing();
    cache.save_doc("pending-marks.json", "not a list").unwrap();
    cache.fail(None, Some("pending-marks.json.bad"));
    let s = Session::open(Some(cache.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    cache.fail(None, None);
    assert!(said(&s), "{:?}", s.notices);
    assert!(s.docs_unread.contains("pending-marks.json"), "saved over");
    assert_eq!(cache.load_doc_strict::<Value>("pending-marks.json").unwrap(), json!("not a list"), "the file changed");
}

/// Two accounts on this computer, kept in a cache whose files can be made to fail.
fn two_accounts_kept() -> (Store, Vec<serde_json::Value>) {
    let cache = Store::failing();
    let account = |id: &str, email: &str| quatrelle_model::AccountConfig { id: id.into(), email: email.into(), ..Default::default() };
    cache.save_accounts(&[account("a1", "zoe@studio.example"), account("a2", "kim@contoso.example")]).unwrap();
    let kept = cache.load_accounts().unwrap().iter().map(|a| serde_json::to_value(a).unwrap()).collect();
    (cache, kept)
}

fn kept_accounts(cache: &Store) -> Vec<serde_json::Value> {
    cache.load_accounts().unwrap().iter().map(|a| serde_json::to_value(a).unwrap()).collect()
}

/// A list of accounts that can't be read is never taken for none: the session opens with no
/// account, says so, and refuses to change the accounts, instead of writing one account over the
/// others (whose data files were then nobody's).
#[test]
fn accounts_that_cant_be_read_are_never_saved_over() {
    for damaged in [false, true] {
        let (cache, before) = two_accounts_kept();
        if damaged {
            // Not a list of accounts: kept as it is.
            cache.save_doc("accounts.json", "not a list").unwrap();
        } else {
            cache.fail(Some("accounts.json"), None);
        }
        let raw = cache.load_doc::<serde_json::Value>("accounts.json").ok();
        let mut s = Session::open(Some(cache.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
        cache.fail(None, None);
        assert!(s.accounts_unread.is_some(), "damaged {damaged}: the list wasn't read, and the session says so");
        assert!(s.accounts.is_empty() && s.notices.iter().any(|n| n.error), "damaged {damaged}: no account shown, an error said");
        let r = s.run("account.add", &json!({"email": "new@studio.example", "local": true}));
        let said = match &r {
            Err(CmdError::Failed(m)) => m.clone(),
            other => panic!("damaged {damaged}: the command must fail, not {other:?}"),
        };
        assert!(said.starts_with("the list of accounts couldn't be read, so it isn't saved: "), "{said}");
        assert!(s.accounts.is_empty(), "damaged {damaged}: nothing added in memory");
        if damaged {
            assert_eq!(cache.load_doc::<serde_json::Value>("accounts.json").ok(), raw, "left as it is");
            assert!(cache.load_accounts().is_err());
        } else {
            assert_eq!(kept_accounts(&cache), before, "the two accounts are still kept");
        }
    }
}

/// A list read again (a new session) works as before: accounts are added and kept.
#[test]
fn accounts_that_could_be_read_are_changed_and_saved_as_before() {
    let (cache, before) = two_accounts_kept();
    let mut s = Session::open(Some(cache.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    assert!(s.accounts_unread.is_none() && s.accounts.len() == 2);
    s.run("account.add", &json!({"email": "new@studio.example", "local": true})).unwrap();
    assert_eq!(kept_accounts(&cache).len(), before.len() + 1);
}

/// A refused change to the accounts leaves them as they were in memory: nothing to undo, and no
/// account shown that isn't kept.
#[test]
fn a_refused_change_to_the_accounts_changes_nothing_in_memory() {
    let cache = Store::failing();
    let ews = quatrelle_model::AccountConfig {
        id: "a2".into(),
        email: "kim@contoso.example".into(),
        ews: Some("https://mail.contoso.example/EWS/Exchange.asmx".into()),
        ..Default::default()
    };
    let local = quatrelle_model::AccountConfig { id: "a1".into(), email: "zoe@studio.example".into(), ..Default::default() };
    cache.save_accounts(&[local, ews]).unwrap();
    let mut s = Session::open(Some(cache.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let _ = s.take_jobs();
    assert_eq!(s.accounts.len(), 2);
    s.accounts_unread = Some("test".into());
    let shown = |s: &Session| s.accounts.iter().map(|a| serde_json::to_value(&a.config).unwrap()).collect::<Vec<_>>();
    let before = shown(&s);
    let kept = kept_accounts(&cache);
    for (id, params) in [
        ("account.add", json!({"email": "new@studio.example", "local": true})),
        ("account.setSignature", json!({"account": "a1", "signature": "Zoe", "newMessages": false})),
        ("account.openMailbox", json!({"account": "a2", "email": "shared@contoso.example"})),
        ("account.remove", json!({"account": "a1"})),
    ] {
        let r = s.run(id, &params);
        assert!(matches!(&r, Err(CmdError::Failed(m)) if m.contains("isn't saved")), "{id}: {r:?}");
        assert_eq!(shown(&s), before, "{id}: the accounts in memory are as they were");
        assert_eq!(kept_accounts(&cache), kept, "{id}: and the ones kept");
    }
    assert!(s.take_jobs().is_empty(), "nothing started, nothing removed");
}

/// Changes waiting that couldn't be read, kept aside by the worker, are said: how many, and that
/// what they changed may still show here; the account's folders held in memory are read again (what the
/// session made of them is the cache's, not the server's).
#[test]
fn changes_kept_aside_are_said_and_their_folders_read_again() {
    let mut s = failing_session();
    let account = sample::SAMPLE_ACCOUNT;
    let held = s.folder_state(account, "INBOX").messages.first().map(|m| (m.key.clone(), m.flags.flagged)).unwrap();
    // The cache changed behind the session's back, as the worker keeps it.
    let mut st = folder(&s, "INBOX");
    if let Some(m) = st.messages.iter_mut().find(|m| m.key == held.0) {
        m.flags.flagged = !held.1;
    }
    store(&s).save_folder(account, "INBOX", &st).unwrap();
    s.apply(Event::OpsSetAside { account: account.into(), count: 2 });
    let said = s.notices.last().cloned();
    assert!(said.as_ref().is_some_and(|n| n.error), "not said: {said:?}");
    assert_eq!(
        said.as_ref().map(|n| n.template),
        Some(
            "{count} changes made on this computer couldn't be read and weren't sent; they are kept aside, and what they changed may still show here"
        )
    );
    assert_eq!(said.map(|n| n.values), Some(vec![("count", "2".to_string())]));
    let now = s.folder_state(account, "INBOX").messages.iter().find(|m| m.key == held.0).map(|m| m.flags.flagged);
    assert_eq!(now, Some(!held.1), "the folder in memory wasn't read again");
}

/// What a damaged folder list's file holds.
const DAMAGED_LIST: &[u8] = b"[{\"account\":\"a1\",\"path\":\"Pla";

/// An account's file as text (the damaged list is plain ASCII: the same text is the same bytes).
fn file_text(store: &Store, account: &str, name: &str) -> Option<String> {
    store.account_file(account, name).unwrap().map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn damaged_text() -> Option<String> {
    Some(String::from_utf8_lossy(DAMAGED_LIST).into_owned())
}

/// An Exchange account, `a2`, opening these other mailboxes.
fn exchange(shared: &[&str]) -> quatrelle_model::AccountConfig {
    quatrelle_model::AccountConfig {
        id: "a2".into(),
        email: "kim@contoso.example".into(),
        ews: Some("https://mail.contoso.example/EWS/Exchange.asmx".into()),
        shared_mailboxes: shared.iter().map(|m| m.to_string()).collect(),
        ..Default::default()
    }
}

/// Found in review: an account's folder list whose file was damaged was renamed `.json.bad` and
/// read as none, without a word: the account opened with no folders. It's said (its server lists
/// them again; with no server, nothing is filed in it), the account opens with none shown, and the
/// file stays as it is.
#[test]
fn a_damaged_folder_list_is_said_when_quatrelle_opens() {
    let cache = Store::failing();
    let local = quatrelle_model::AccountConfig { id: "a1".into(), email: "zoe@studio.example".into(), ..Default::default() };
    cache.save_accounts(&[local, exchange(&[])]).unwrap();
    for id in ["a1", "a2"] {
        cache.save_folders(id, &[Folder { account: id.into(), path: "INBOX".into(), ..Default::default() }]).unwrap();
        cache.write_account_files(id, &[("folders.json".into(), DAMAGED_LIST.to_vec())]).unwrap();
    }
    let s = Session::open(Some(cache.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let said: Vec<&str> = s.notices.iter().filter(|n| n.error).map(|n| n.text.as_str()).collect();
    assert!(said.contains(&"kim@contoso.example: this account's list of folders is damaged and is read again from the server"), "{said:?}");
    let here =
        "zoe@studio.example: this account's list of folders is damaged, so nothing is filed in it for now: a file Quatrelle keeps is damaged: ";
    assert!(said.iter().any(|t| t.starts_with(here) && t.ends_with("folders.json")), "{said:?}");
    for id in ["a1", "a2"] {
        assert!(s.account(id).is_some_and(|a| a.folders.is_empty()), "{id}: folders shown");
        assert_eq!(file_text(&cache, id, "folders.json"), damaged_text(), "{id}: not left as it is");
    }
}

/// The sample mailbox whose folder list is damaged isn't made again over it: taken for empty, it
/// was, the damaged list written over.
#[test]
fn a_damaged_sample_isnt_made_again_over_its_folder_list() {
    let store = Store::failing();
    assert!(sample::seed_if_empty(&store, NOW).unwrap());
    store.write_account_files(sample::SAMPLE_ACCOUNT, &[("folders.json".into(), DAMAGED_LIST.to_vec())]).unwrap();
    assert!(!sample::seed_if_empty(&store, NOW).unwrap(), "made again");
    assert_eq!(file_text(&store, sample::SAMPLE_ACCOUNT, "folders.json"), damaged_text());
}

/// Closing another mailbox while the account's list on disk is damaged writes nothing over it: the
/// list shown is a listing's, which may have missed a zone kept here; the account's next listing
/// that misses none writes it.
#[test]
fn closing_a_mailbox_writes_no_list_over_a_damaged_one() {
    let cache = Store::failing();
    cache.save_accounts(&[exchange(&["support@contoso.example"])]).unwrap();
    cache.write_account_files("a2", &[("folders.json".into(), DAMAGED_LIST.to_vec())]).unwrap();
    let mut s = Session::open(Some(cache.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let _ = s.take_jobs();
    let f = |path: &str| Folder { account: "a2".into(), path: path.into(), delimiter: Some('/'), selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: "a2".into(), folders: vec![f("INBOX"), f("support@contoso.example"), f("support@contoso.example/Inbox")] });
    s.run("account.closeMailbox", &json!({"email": "support@contoso.example"})).unwrap();
    assert!(s.account("a2").is_some_and(|a| a.folders.iter().all(|f| f.path == "INBOX")), "its folders still shown");
    assert_eq!(file_text(&cache, "a2", "folders.json"), damaged_text(), "written over");
    assert_eq!(cache.account_file("a2", "folders.json.bad").unwrap(), None);
}

/// An Outbox message whose files can't be taken out stays, and that's said: deleted, it went all
/// the same; taken back to be changed, it went twice (as it was, and as changed).
#[test]
fn an_outbox_message_that_cant_be_taken_out_says_so() {
    let mut s = failing_session();
    let item = quatrelle_store::OutboxItem {
        id: 71,
        from: "riley@studio.example".into(),
        recipients: vec!["ana@contoso.example".into()],
        queued: NOW,
        ..Default::default()
    };
    let raw = b"From: riley@studio.example\r\nTo: ana@contoso.example\r\nSubject: Waiting\r\n\r\nhi\r\n";
    store(&s).outbox_add(sample::SAMPLE_ACCOUNT, &item, raw).unwrap();
    s.run("folder.outbox", &json!({})).unwrap();
    let key = s.list[0].key.clone();
    store(&s).fail(None, Some("outbox/71.json|outbox/71.eml|outbox/71.gone"));
    let r = s.run("outbox.edit", &json!({"key": key.to_id()}));
    assert!(matches!(r, Err(CmdError::Failed(_))), "{r:?}");
    assert!(s.compose.is_none(), "not opened to be sent again");
    s.run("list.select", &json!({"key": key.to_id()})).unwrap();
    let r = s.run("mail.delete", &json!({}));
    assert!(matches!(r, Err(CmdError::Failed(_))), "{r:?}");
    assert_eq!(store(&s).outbox(sample::SAMPLE_ACCOUNT).len(), 1);
    assert_eq!(s.outbox.len(), 1, "still shown");
    // Its files held, but a mark beside them can be written: out of the Outbox.
    store(&s).fail(None, Some("outbox/71.json|outbox/71.eml"));
    s.run("list.select", &json!({"key": key.to_id()})).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    store(&s).fail(None, None);
    assert!(store(&s).outbox(sample::SAMPLE_ACCOUNT).is_empty() && s.outbox.is_empty());
}

/// Two Outbox messages deleted together, one of which can't be taken out (its files held, or
/// their removal answering as if done and leaving them, and no mark beside them): the other goes
/// with the flag it owed, and the one held is said, still shown, its flag still owed. Taken back
/// to be changed, it isn't opened while it can't be taken out; once it can, it opens with what
/// it owes, out of the Outbox.
#[test]
fn deleting_and_editing_in_the_outbox_say_what_failed() {
    for quietly in [false, true] {
        let cache = Store::failing();
        assert!(sample::seed_if_empty(&cache, NOW).unwrap());
        let original = cache.load_folder(sample::SAMPLE_ACCOUNT, "INBOX").unwrap().messages[0].key.clone();
        for (id, subject) in [(71u64, "Held"), (72, "Free")] {
            let item = quatrelle_store::OutboxItem {
                id,
                from: "riley@studio.example".into(),
                recipients: vec!["ana@contoso.example".into()],
                queued: NOW,
                ..Default::default()
            };
            let raw = format!("From: riley@studio.example\r\nTo: ana@contoso.example\r\nSubject: {subject}\r\n\r\nhi\r\n");
            cache.outbox_add(sample::SAMPLE_ACCOUNT, &item, raw.as_bytes()).unwrap();
        }
        // Both answer the same message, and owe it its Replied flag.
        let owed = vec![(71u64, original.clone(), Flag::Answered), (72, original.clone(), Flag::Answered)];
        cache.save_doc("pending-marks.json", &owed).unwrap();
        let mut s = Session::open(Some(cache.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
        let _ = s.take_jobs();
        s.run("folder.outbox", &json!({})).unwrap();
        if quietly {
            cache.fail_quietly(Some("outbox/71.json|outbox/71.eml"));
            cache.fail(None, Some("outbox/71.gone"));
        } else {
            cache.fail(None, Some("outbox/71.json|outbox/71.eml|outbox/71.gone"));
        }
        s.run("list.selectAll", &json!({})).unwrap();
        let r = s.run("mail.delete", &json!({}));
        let deleting = "a message couldn't be taken out of the Outbox; it's still there, and may still be sent: ";
        assert!(matches!(&r, Err(CmdError::Failed(m)) if m.starts_with(deleting)), "quietly {quietly}: {r:?}");
        let left: Vec<u64> = cache.outbox(sample::SAMPLE_ACCOUNT).iter().map(|i| i.id).collect();
        assert_eq!(left, [71], "quietly {quietly}");
        assert_eq!(s.list.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Held"], "quietly {quietly}: still shown");
        let still: Vec<(u64, MessageKey, Flag)> = cache.load_doc("pending-marks.json").unwrap();
        assert_eq!(still.iter().map(|(id, ..)| *id).collect::<Vec<_>>(), [71], "quietly {quietly}: the flags owed");
        let key = s.list[0].key.clone();
        let r = s.run("outbox.edit", &json!({"key": key.to_id()}));
        let changing = "the message couldn't be taken out of the Outbox to be changed: ";
        assert!(matches!(&r, Err(CmdError::Failed(m)) if m.starts_with(changing)), "quietly {quietly}: {r:?}");
        assert!(s.compose.is_none(), "quietly {quietly}: opened, and it would go twice");
        assert_eq!(cache.outbox(sample::SAMPLE_ACCOUNT).len(), 1, "quietly {quietly}");
        cache.fail(None, None);
        cache.fail_quietly(None);
        s.run("outbox.edit", &json!({"key": key.to_id()})).unwrap();
        assert!(s.compose.as_ref().is_some_and(|c| c.source.as_ref() == Some(&original)), "quietly {quietly}: what it owes");
        assert!(cache.outbox(sample::SAMPLE_ACCOUNT).is_empty() && s.outbox.is_empty(), "quietly {quietly}");
    }
}
