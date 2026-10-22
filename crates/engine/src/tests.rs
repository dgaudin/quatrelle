//! Engine tests: commands against the sample mailbox and scripted network events.

use quatrelle_model::{Event, Flag, FolderRole, Job, Op, SyncStatus};
use quatrelle_store::Store;
use serde_json::{Value, json};

use crate::session::{ListFilter, Row, SearchScope, View};
use crate::{CmdError, Session, sample};
use quatrelle_model::{EmailAddress, MessageKey, Summary};

const NOW: i64 = 1_791_979_200;

fn temp(name: &str) -> Store {
    let dir = std::env::temp_dir().join(format!("quatrelle-engine-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Store::open(dir).unwrap()
}

fn sample_session(name: &str) -> Session {
    let store = temp(name);
    assert!(sample::seed_if_empty(&store, NOW).unwrap());
    assert!(!sample::seed_if_empty(&store, NOW).unwrap());
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    assert!(s.take_jobs().is_empty(), "local accounts start no workers");
    s
}

fn done(s: Session) {
    if let Some(st) = &s.store {
        let _ = std::fs::remove_dir_all(st.root());
    }
}

fn key_of(s: &Session, subject: &str) -> Value {
    let m = s.list.iter().find(|m| m.subject == subject).unwrap_or_else(|| panic!("no {subject}"));
    json!({"key": m.key.to_id()})
}

/// The calendar carried by the newest message in a folder (as `METHOD`, and the text).
fn sent_calendar(s: &Session, folder: &str, subject: &str) -> (String, String) {
    let st = s.store.as_ref().unwrap().load_folder("sample", folder).unwrap();
    let m = st
        .messages
        .iter()
        .find(|m| m.subject == subject)
        .unwrap_or_else(|| panic!("no {subject}: {:?}", st.messages.iter().map(|m| &m.subject).collect::<Vec<_>>()));
    let cal = s.message_calendar(&m.key).unwrap();
    let raw = String::from_utf8_lossy(&s.store.as_ref().unwrap().read_body(&m.key).unwrap()).to_string();
    (cal.method.unwrap_or_default(), raw)
}

/// The start of 2026-10-16, local, as the sample's clock has it.
fn quatrelle_engine_day(s: &Session) -> i64 {
    crate::parse_local("2026-10-16", i64::from(s.clock.offset)).map(|(t, _)| t).unwrap()
}

mod accounts;
mod mail;
mod organize;
mod pim;
mod writing;

#[test]
fn what_the_server_finds_joins_what_the_index_found() {
    let mut s = sample_session("server-search");
    assert_eq!(s.run("search.run", &json!({"query": "deepseek"})).unwrap()["results"], 0);
    // What Exchange answered: a message whose text is on the server only (an old one), so it
    // never reached the index. It's shown with the others, as Outlook merges them.
    let key = MessageKey { account: "sample".into(), folder: "Archive".into(), validity: 1, uid: 9_001 };
    let mut m = Summary {
        key: key.clone(),
        subject: "Deepseek notes".into(),
        from: EmailAddress { name: "Ada".into(), email: "ada@example.org".into() },
        date: 1_107_248_400,
        ..Default::default()
    };
    m.preview = "we tried deepseek in the lab".into();
    s.apply(Event::Found { account: "sample".into(), query: "deepseek".into(), items: vec![m], error: None });
    assert_eq!(s.list.len(), 1);
    assert_eq!(s.list[0].subject, "Deepseek notes");
    assert_eq!(s.summary(&key).map(|m| m.subject.as_str()), Some("Deepseek notes"), "it can be opened from there");
    // Found twice (the index and the server): one row.
    let again = s.list[0].clone();
    s.apply(Event::Found { account: "sample".into(), query: "deepseek".into(), items: vec![again], error: None });
    assert_eq!(s.list.len(), 1);
    // Another search leaves them out.
    s.run("search.run", &json!({"query": "projector"})).unwrap();
    assert!(!s.list.iter().any(|m| m.subject == "Deepseek notes"));
    done(s);
}

/// An IMAP account is asked too: its server's `SEARCH` finds what this computer doesn't keep
/// (the words of a message past the window, or of a body never downloaded), as Exchange's index
/// does for an Exchange account. The sample mailbox, kept here, isn't.
#[test]
fn a_search_asks_an_imap_account_as_well() {
    let mut s = sample_session("imap-search");
    let a = s
        .run("account.add", &json!({"email": "zoe@example.org", "password": "pw", "imapHost": "mail.example.org", "smtpHost": "mail.example.org"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    s.run("search.run", &json!({"query": "needle", "scope": "all"})).unwrap();
    let jobs = s.take_jobs();
    assert!(
        jobs.iter().any(|j| matches!(j, Job::Search { account, query, folder: None } if *account == a && query == "needle")),
        "the account's server is asked for every folder: {jobs:?}"
    );
    assert!(!jobs.iter().any(|j| matches!(j, Job::Search { account, .. } if account == "sample")), "kept here: not asked");
    // A search of the folder on view asks nothing of it while this computer's sample mailbox is
    // the one open; it asks that folder alone once a folder of the account is.
    s.run("search.clear", &json!({})).unwrap();
    s.run("search.run", &json!({"query": "needle", "scope": "current"})).unwrap();
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Search { account, .. } if *account == a)), "the sample is on view");
    done(s);
}

#[test]
fn a_correction_replaces_the_word_and_keeps_its_look() {
    let mut s = sample_session("replace");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plan", "body": "Le **serveur** est lent", "marks": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body, "Le serveur est lent");
    let at = c.body.find("serveur").unwrap();
    assert!(c.styles.style(3).bold, "the word is bold");
    // The suggestion takes the word's place, the look with it.
    s.run("compose.replaceText", &json!({"from": at, "to": at + "serveur".len(), "text": "serveur mail"})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body, "Le serveur mail est lent");
    assert_eq!(c.styles.chars.len(), c.body.chars().count(), "one style per character");
    assert!(c.styles.style(3).bold && c.styles.style(14).bold, "the replacement stays bold");
    assert!(!c.styles.style(15).bold, "what follows it keeps its own look");
    // What isn't there is refused, and the text is left as it was.
    for bad in [json!({"from": 4, "to": 2, "text": "x"}), json!({"from": 0, "to": 9_999, "text": "x"}), json!({"from": -1, "to": 2, "text": "x"})] {
        assert!(s.run("compose.replaceText", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.compose.as_ref().unwrap().body, "Le serveur mail est lent");
    // In the middle of a character: refused, not a panic.
    s.run("compose.update", &json!({"body": "Le café est prêt"})).unwrap();
    let e = s.compose.as_ref().unwrap().body.find('é').unwrap();
    assert!(s.run("compose.replaceText", &json!({"from": e + 1, "to": e + 3, "text": "e"})).is_err());
    assert_eq!(s.compose.as_ref().unwrap().body, "Le café est prêt");
    // Taking a word out.
    let (from, to) = (3, 3 + "café".len());
    s.run("compose.replaceText", &json!({"from": from, "to": to, "text": ""})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body, "Le  est prêt");
    assert_eq!(c.styles.chars.len(), c.body.chars().count());
    // The subject, with offsets into it.
    s.run("compose.update", &json!({"subject": "Le servveur est là"})).unwrap();
    s.run("compose.replaceText", &json!({"from": 3, "to": 11, "text": "serveur", "field": "subject"})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().subject, "Le serveur est là");
    assert!(s.run("compose.replaceText", &json!({"from": 0, "to": 99, "text": "x", "field": "subject"})).is_err());
    assert!(s.run("compose.replaceText", &json!({"from": 0, "to": 1, "text": "x", "field": "nowhere"})).is_err());
    // Underlining as you type, on or off (Outlook's "Check spelling as you type").
    assert!(s.prefs.spell_check);
    assert_eq!(s.run("view.spelling", &json!({"on": false})).unwrap()["on"], false);
    assert!(!s.prefs.spell_check);
    s.run("view.spelling", &json!({})).unwrap();
    assert!(s.prefs.spell_check);
    // The words added to the dictionary, once each, whatever the case.
    assert_eq!(s.run("spelling.add", &json!({"word": "  Quatrelle "})).unwrap()["word"], "quatrelle");
    s.run("spelling.add", &json!({"word": "QUATRELLE"})).unwrap();
    s.run("spelling.add", &json!({"word": "serveur"})).unwrap();
    assert_eq!(s.prefs.spelling, vec!["quatrelle".to_string(), "serveur".to_string()]);
    assert!(s.run("spelling.add", &json!({"word": "   "})).is_err());
    done(s);
}

/// An appointment read from Exchange carries its start in its name (`…#<start>`), and a dialog
/// opened before it moved still holds the old one: it finds the appointment all the same, when
/// only one answers to the name without its start, instead of "no event `…`".
#[test]
fn an_appointment_answers_to_the_name_a_dialog_opened_with() {
    let mut s = sample_session("event-name");
    let uid = s.run("event.create", &json!({"summary": "test dga", "start": "2026-10-23T11:00", "end": "2026-10-23T12:00"})).unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    // What a copy read from Exchange looks like.
    {
        let account = "sample".to_string();
        let p = s.pim_mut(&account);
        if let Some(e) = p.events.iter_mut().find(|e| e.event.uid == uid) {
            e.event.uid = "040000008200E00074C5B7101A82E00800000000A0FB60422C61DD01#1792569600".into();
        }
    }
    let old = "040000008200E00074C5B7101A82E00800000000A0FB60422C61DD01#1792569600";
    let moved = "040000008200E00074C5B7101A82E00800000000A0FB60422C61DD01#1792574700";
    s.run("event.update", &json!({"uid": old, "summary": "test dga déplacé"})).unwrap();
    let r = s.run("event.update", &json!({"uid": moved, "location": "Bureau"})).expect("the name of a moved appointment still finds it");
    assert_eq!(r["uid"], old, "the appointment keeps its own name");
    let (_, e) = s.find_event(old).expect("still there");
    assert_eq!((e.event.summary.as_str(), e.event.location.as_str()), ("test dga déplacé", "Bureau"));
    // Two appointments answering to the same name without its start: nothing is guessed.
    assert!(s.run("event.update", &json!({"uid": "nobody#1", "summary": "x"})).is_err());
    done(s);
}

/// A reminder is minutes before the start; it's kept through a change, and taken off when asked.
#[test]
fn a_reminder_is_set_kept_and_taken_off() {
    let mut s = sample_session("reminder");
    let uid =
        s.run("event.create", &json!({"summary": "Review", "start": "2026-10-23T11:00", "end": "2026-10-23T12:00", "reminder": 15})).unwrap()["uid"]
            .as_str()
            .unwrap()
            .to_string();
    assert_eq!(s.find_event(&uid).unwrap().1.event.reminder, Some(15));
    s.run("event.update", &json!({"uid": uid, "reminder": 1440})).unwrap();
    assert_eq!(s.find_event(&uid).unwrap().1.event.reminder, Some(1440), "a day before");
    // Kept as written, a VALARM in its calendar file.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let mut again = Session::open(Some(Store::open(&root).unwrap()), quatrelle_model::dates::Clock::new(NOW, 0));
    assert_eq!(again.find_event(&uid).unwrap().1.event.reminder, Some(1440));
    again.run("event.update", &json!({"uid": uid, "reminder": null})).unwrap();
    assert_eq!(again.find_event(&uid).unwrap().1.event.reminder, None, "taken off");
    assert!(again.run("event.update", &json!({"uid": uid, "reminder": 99_999})).is_err(), "nonsense is refused, not a panic");
    assert_eq!(again.find_event(&uid).unwrap().1.event.reminder, None);
    done(s);
}

/// A server's rule Quatrelle understands can be changed from here: only that rule goes, with
/// the place it has there; one it doesn't understand is refused rather than truncated.
#[test]
fn a_server_rule_is_changed_where_it_lives() {
    use quatrelle_model::organize::{Condition, ForeignRule, Rule, RuleAction};

    let mut s = Session::open(Some(temp("server-rule-save")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let known = Rule {
        name: "Clients".into(),
        enabled: true,
        account: Some(id.clone()),
        conditions: vec![Condition::From("@client.example".into())],
        match_all: true,
        actions: vec![RuleAction::MoveTo("INBOX/Clients".into())],
        stop: false,
        server: true,
        remote_id: Some("R3".into()),
        priority: Some(7),
    };
    s.foreign_rules.push(ForeignRule {
        account: id.clone(),
        id: "R3".into(),
        name: "Clients".into(),
        enabled: true,
        runnable: true,
        rule: Some(known.clone()),
        summary: String::new(),
    });
    s.foreign_rules.push(ForeignRule {
        account: id.clone(),
        id: "R9".into(),
        name: "Body rule".into(),
        enabled: true,
        runnable: false,
        rule: None,
        summary: String::new(),
    });
    // Changed: its name and what it does, its place kept.
    let edited = Rule {
        name: "Clients (prospects)".into(),
        conditions: vec![Condition::From("@client.example".into()), Condition::HasAttachment],
        actions: vec![RuleAction::MoveTo("INBOX/Clients".into()), RuleAction::MarkRead],
        ..known.clone()
    };
    s.run("rules.server.save", &json!({"account": id, "id": "R3", "rule": edited})).unwrap();
    let job = s.take_jobs().into_iter().find(|j| matches!(j, Job::SetServerRule { .. })).expect("the rule goes to the server");
    assert!(matches!(&job, Job::SetServerRule { .. }), "the rule goes to the server");
    if let Job::SetServerRule { id: rule_id, rule, .. } = job {
        assert_eq!(rule_id, "R3");
        assert_eq!(rule.conditions.len(), 2);
        assert_eq!(rule.actions.len(), 2);
        assert_eq!(rule.priority, Some(7), "the place it has there");
        assert_eq!(rule.remote_id.as_deref(), Some("R3"));
    }
    // A rule Quatrelle can't say entirely isn't half-written.
    assert!(s.run("rules.server.save", &json!({"account": id, "id": "R9", "rule": known})).is_err());
    done(s);
}

/// Two appointments the server keeps under one UID — some Exchange series arrive as separate
/// items that all carry the series' UID — are both kept: Friday's meeting must not vanish
/// behind its siblings, and a re-sync doesn't double them.
#[test]
fn appointments_sharing_a_uid_are_all_kept() {
    use quatrelle_model::dates::Clock;
    use quatrelle_model::ical::CalEvent;
    use quatrelle_model::pim::{CalendarInfo, DavSync, StoredEvent};

    let mut s = Session::open(Some(temp("shared-uid")), Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let cal = "ews:calendar";
    let item = |href: &str, start: i64| StoredEvent {
        calendar: cal.into(),
        href: href.into(),
        event: CalEvent {
            uid: "040000008200E00074C5".into(), summary: "Comité de sécurité".into(), start, end: start + 7_200, ..Default::default()
        },
        ..Default::default()
    };
    let sync = DavSync {
        calendars: Some(vec![CalendarInfo { id: cal.into(), name: "Calendar".into(), ..Default::default() }]),
        synced_calendars: vec![cal.into()],
        events: vec![item("I1", NOW), item("I2", NOW + 7 * 86_400)],
        ..Default::default()
    };
    s.apply_dav(&id, &sync);
    let titles = |s: &Session| s.agenda(NOW - 10, NOW + 8 * 86_400).into_iter().map(|a| a.start).collect::<Vec<_>>();
    assert_eq!(titles(&s), [NOW, NOW + 7 * 86_400], "both meetings, on their own days");
    s.apply_dav(&id, &sync);
    assert_eq!(titles(&s), [NOW, NOW + 7 * 86_400], "and once only after another sync");
    done(s);
}

/// A big mailbox's search is indexed a slice at a time: each call answers with what it has and
/// says whether more is coming, so the window keeps drawing between the calls instead of
/// freezing while the whole mailbox is read.
#[test]
fn a_search_is_indexed_a_slice_at_a_time() {
    let store = temp("slice-search");
    store.save_accounts(&[quatrelle_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    store
        .save_folders(
            "a1",
            &[quatrelle_model::Folder {
                account: "a1".into(),
                path: "INBOX".into(),
                role: FolderRole::Inbox,
                selectable: true,
                ..Default::default()
            }],
        )
        .unwrap();
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    for uid in 1..=60u32 {
        let key = quatrelle_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        st.messages.push(quatrelle_model::Summary { key, subject: format!("Report {uid}"), ..Default::default() });
    }
    st.messages[59].subject = "needle in the last one".into();
    store.save_folder("a1", "INBOX", &st).unwrap();
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    // Ten messages a call: the first answers with what it has indexed and says more is coming.
    let v = s.run("search.run", &json!({"query": "needle", "index": 10})).unwrap();
    assert_eq!(v["pending"], true, "a slice is left: {v}");
    assert_eq!(v["results"], 0, "the matching message isn't indexed yet: {v}");
    // The window keeps asking, a slice a frame, until the index is done.
    let mut calls = 0;
    loop {
        let v = s.run("search.run", &json!({"query": "needle", "index": 10})).unwrap();
        calls += 1;
        if v["pending"] == false {
            assert_eq!(v["results"], 1, "the match is found once the index is done: {v}");
            break;
        }
        assert!(calls < 100, "the scan must finish");
    }
    // A later search of the same scope uses the index it has: nothing left to do.
    let v = s.run("search.run", &json!({"query": "report", "index": 10})).unwrap();
    assert_eq!(v["pending"], false, "the index is built: {v}");
    assert!(v["results"].as_u64().unwrap_or(0) > 0, "the whole folder is searchable: {v}");
    let _ = std::fs::remove_dir_all(store.root());
}

/// A folder's index is kept when the folder is read: the next run searches it again without
/// reading the messages (with the bodies out of the cache, only the kept index finds the words).
#[test]
fn a_search_index_is_kept_for_the_next_run() {
    let store = temp("kept-index");
    store.save_accounts(&[quatrelle_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    store
        .save_folders(
            "a1",
            &[quatrelle_model::Folder {
                account: "a1".into(),
                path: "INBOX".into(),
                role: FolderRole::Inbox,
                selectable: true,
                ..Default::default()
            }],
        )
        .unwrap();
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    st.validity = 1;
    for uid in 1..=40u32 {
        let key = quatrelle_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        st.messages.push(quatrelle_model::Summary { key, subject: format!("Report {uid}"), ..Default::default() });
    }
    // A word only the body holds: the summary alone can't find it.
    let key = st.messages[39].key.clone();
    store.write_body(&key, b"From: a@x.example\r\nSubject: Report 40\r\n\r\nwatermelon in the body\r\n").unwrap();
    store.save_folder("a1", "INBOX", &st).unwrap();
    {
        let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
        s.take_jobs();
        assert_eq!(s.run("search.run", &json!({"query": "watermelon"})).unwrap()["results"], 1);
        assert!(store.load_search_index("a1", "INBOX").is_some(), "the folder's index is kept");
    }
    // The body goes (a cache that let go of it), and the next run still finds the word.
    store.remove_body(&key);
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    let v = s.run("search.run", &json!({"query": "watermelon"})).unwrap();
    assert_eq!(v["results"], 1, "found in the kept index, not by reading the messages: {v}");
    assert_eq!(v["pending"], false);
    assert_eq!(s.run("search.run", &json!({"query": "report"})).unwrap()["results"], 40);
    done(s);
}

/// A body that arrives after the index was kept joins it: the same run finds it, and the next
/// run reads that body only (the folder itself isn't read again).
#[test]
fn a_downloaded_body_joins_a_kept_index() {
    let store = temp("kept-delta");
    store.save_accounts(&[quatrelle_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    store
        .save_folders(
            "a1",
            &[quatrelle_model::Folder {
                account: "a1".into(),
                path: "INBOX".into(),
                role: FolderRole::Inbox,
                selectable: true,
                ..Default::default()
            }],
        )
        .unwrap();
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    st.validity = 1;
    for uid in 1..=20u32 {
        let key = quatrelle_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        st.messages.push(quatrelle_model::Summary { key, subject: format!("Report {uid}"), ..Default::default() });
    }
    let key = st.messages[7].key.clone();
    store.save_folder("a1", "INBOX", &st).unwrap();
    {
        let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
        s.take_jobs();
        assert_eq!(s.run("search.run", &json!({"query": "report"})).unwrap()["results"], 20);
        // The body comes in, as "Download all" brings it.
        let raw = b"From: a@x.example\r\nSubject: Report 8\r\n\r\nwatermelon in the body\r\n".to_vec();
        store.write_body(&key, &raw).unwrap();
        s.apply(Event::Body { key: key.clone(), raw });
        assert_eq!(s.run("search.run", &json!({"query": "watermelon"})).unwrap()["results"], 1);
    }
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    let v = s.run("search.run", &json!({"query": "watermelon"})).unwrap();
    assert_eq!(v["results"], 1, "the body read since is added to the kept index: {v}");
    done(s);
}

/// A folder that changed since its index was kept is read again, on its own: the messages that
/// arrived are searchable, and nothing else is read.
#[test]
fn a_changed_folder_is_indexed_again() {
    let store = temp("kept-change");
    store.save_accounts(&[quatrelle_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    store
        .save_folders(
            "a1",
            &[quatrelle_model::Folder {
                account: "a1".into(),
                path: "INBOX".into(),
                role: FolderRole::Inbox,
                selectable: true,
                ..Default::default()
            }],
        )
        .unwrap();
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    st.validity = 1;
    for uid in 1..=20u32 {
        let key = quatrelle_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        st.messages.push(quatrelle_model::Summary { key, subject: format!("Report {uid}"), ..Default::default() });
    }
    store.save_folder("a1", "INBOX", &st).unwrap();
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    assert_eq!(s.run("search.run", &json!({"query": "report"})).unwrap()["results"], 20);
    // Mail arrives while the window is open: the folder is read again (the index no longer fits
    // it), and the new subject is searchable.
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    let key = quatrelle_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 21 };
    st.messages.push(quatrelle_model::Summary { key, subject: "kitchen sink".into(), ..Default::default() });
    store.save_folder("a1", "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: "a1".into(), folder: "INBOX".into() });
    assert_eq!(s.run("search.run", &json!({"query": "kitchen"})).unwrap()["results"], 1);
    assert_eq!(s.run("search.run", &json!({"query": "report"})).unwrap()["results"], 20);
    done(s);
}

/// Outlook's "Download all" brings every message not on this computer yet, a batch at a time:
/// each answer asks for the next, a message already kept isn't asked for again, and when the
/// last one arrives it says so and stops.
#[test]
fn download_all_brings_every_message_a_batch_at_a_time() {
    let store = temp("download-all");
    store.save_accounts(&[quatrelle_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    store
        .save_folders(
            "a1",
            &[quatrelle_model::Folder {
                account: "a1".into(),
                path: "INBOX".into(),
                role: FolderRole::Inbox,
                selectable: true,
                ..Default::default()
            }],
        )
        .unwrap();
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    // The keys are rebuilt with the folder's validity when it's read back, so set it first.
    st.validity = 1;
    for uid in 1..=50u32 {
        let key = quatrelle_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        st.messages.push(quatrelle_model::Summary { key, subject: format!("Report {uid}"), ..Default::default() });
    }
    // The first one is already here, so it isn't asked for again.
    let kept = st.messages[0].key.clone();
    store.write_body(&kept, b"From: a@x.example\r\nSubject: kept\r\n\r\nbody\r\n").unwrap();
    store.save_folder("a1", "INBOX", &st).unwrap();
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    // The folder's keys get its validity back when it's read (a renamed cache stays consistent).
    assert!(store.has_body(&s.folder_state("a1", "INBOX").messages[0].key), "the kept body is found under the key the engine sees");
    let v = s.run("account.downloadAll", &json!({})).unwrap();
    assert_eq!(v["total"], 49, "one of fifty is already on this computer: {v}");
    let asked = |jobs: Vec<quatrelle_model::Job>| -> Vec<quatrelle_model::MessageKey> {
        jobs.into_iter().filter_map(|j| if let quatrelle_model::Job::FetchBody { key } = j { Some(key) } else { None }).collect()
    };
    let first = asked(s.take_jobs());
    assert!(!first.is_empty() && first.len() < 49, "a batch, not the whole mailbox at once: {}", first.len());
    assert!(!first.contains(&kept), "a kept message isn't downloaded again");
    // Each answer asks for the next; when the last arrives, it says so and stops.
    let mut batch = first;
    let mut rounds = 0;
    loop {
        for key in &batch {
            s.apply(Event::Body { key: key.clone(), raw: b"From: a@x.example\r\nSubject: x\r\n\r\nbody\r\n".to_vec() });
        }
        if s.download_progress().is_none() {
            break;
        }
        batch = asked(s.take_jobs());
        assert!(!batch.is_empty(), "progress says there is more, so more is asked for");
        rounds += 1;
        assert!(rounds < 100, "it must finish");
    }
    assert!(s.notices.iter().any(|n| n.text.contains("49 messages downloaded")), "{:?}", s.notices);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A rule marked "on the server" goes to Exchange as a job, only when what the server would be
/// told changes; the id Exchange gives it comes back and is kept, so the next change finds the
/// same rule. A rule Exchange can't run stays home, and says so.
#[test]
fn server_rules_go_to_the_server_and_keep_their_id() {
    use quatrelle_model::organize::{Condition, Rule, RuleAction};
    let mut s = sample_session("server-rules");
    let account = s.accounts[0].config.id.clone();
    let rule = Rule {
        name: "Invoices".into(),
        enabled: true,
        account: Some(account.clone()),
        conditions: vec![Condition::Subject("invoice".into())],
        match_all: true,
        actions: vec![RuleAction::MoveTo("Inbox".into())],
        stop: false,
        server: true,
        remote_id: None,
        priority: None,
    };
    s.run("rules.set", &json!({ "rules": [rule] })).unwrap();
    let sent: Vec<(String, Vec<Rule>, Vec<String>)> = s
        .take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            quatrelle_model::Job::SetServerRules { account, rules, delete, .. } => Some((account, rules, delete)),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1, "one job for the account");
    assert_eq!(sent[0].1.len(), 1, "the rule marked for the server");
    assert!(sent[0].2.is_empty(), "nothing to remove");
    // The server's answer brings its id back: it's kept, and nothing is sent again.
    s.apply(quatrelle_model::Event::ServerRules {
        account: account.clone(),
        ids: vec![("Invoices".into(), "R1".into())],
        others: Vec::new(),
        cleared: Vec::new(),
        error: None,
    });
    assert_eq!(s.rules[0].remote_id.as_deref(), Some("R1"));
    assert!(s.take_jobs().is_empty(), "the same rules aren't sent twice");
    // The rules the server runs that Quatrelle didn't make are listed as they are (Outlook's
    // own, say), and never changed here.
    s.apply(quatrelle_model::Event::ServerRules {
        account: account.clone(),
        ids: Vec::new(),
        others: vec![quatrelle_model::organize::ForeignRule {
            account: String::new(),
            id: "R9".into(),
            name: "Outlook rule".into(),
            enabled: true,
            runnable: false,
            rule: None,
            summary: "subject \"x\" → delete".into(),
        }],
        cleared: Vec::new(),
        error: None,
    });
    assert_eq!(s.foreign_rules.len(), 1, "{:?}", s.foreign_rules);
    assert_eq!(s.foreign_rules[0].name, "Outlook rule");
    assert_eq!(s.foreign_rules[0].account, account);
    assert_eq!(s.rules[0].remote_id.as_deref(), Some("R1"), "a read leaves our own ids alone");
    // A server that has started syncing is readable: its rules are asked for then, not once
    // the whole mailbox has been walked — and once is enough, however many folders follow.
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Syncing { folder: "INBOX".into() } });
    let asked = |s: &mut Session| s.take_jobs().into_iter().filter(|j| matches!(j, quatrelle_model::Job::GetServerRules { .. })).count();
    assert_eq!(asked(&mut s), 1, "asked as soon as the server answers");
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Syncing { folder: "Sent".into() } });
    assert_eq!(asked(&mut s), 0, "once a session is enough");
    // Connecting asks the server what it keeps: ours (to keep their ids right) and the rest.
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Online { at: NOW } });
    let reads: Vec<(String, Vec<String>)> = s
        .take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            quatrelle_model::Job::GetServerRules { account, ours } => Some((account, ours)),
            _ => None,
        })
        .collect();
    assert_eq!(reads, vec![(account.clone(), vec!["R1".to_string()])]);
    // Online again (after each folder, each look for new mail): not read again — that was over
    // a hundred reads a pass on a big mailbox.
    s.apply(quatrelle_model::Event::Status { account: account.clone(), status: SyncStatus::Online { at: NOW + 60 } });
    assert_eq!(asked(&mut s), 0, "read on coming online, not on every Online");
    // A refusal says nothing of what the server keeps: the list stays.
    s.apply(quatrelle_model::Event::ServerRules {
        account: account.clone(),
        ids: Vec::new(),
        others: Vec::new(),
        cleared: Vec::new(),
        error: Some("the server is busy".into()),
    });
    assert_eq!(s.foreign_rules.len(), 1, "Outlook's rules stay listed after a refusal");
    // Turned off: its server copy is removed, and the id with it.
    let mut off = s.rules[0].clone();
    off.server = false;
    s.run("rules.set", &json!({ "rules": [off] })).unwrap();
    let sent: Vec<(String, Vec<Rule>, Vec<String>)> = s
        .take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            quatrelle_model::Job::SetServerRules { account, rules, delete, .. } => Some((account, rules, delete)),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].1.is_empty());
    assert_eq!(sent[0].2, vec!["R1".to_string()], "the server copy goes");
    assert_eq!(s.rules[0].remote_id, None, "and the id is forgotten");
    // A rule Exchange can't run stays on this computer, and the user is told why.
    let mut good = s.rules[0].clone();
    good.server = true;
    let mut bad = good.clone();
    bad.name = "Newsletters".into();
    bad.conditions = vec![Condition::Bulk];
    s.run("rules.set", &json!({ "rules": [good, bad] })).unwrap();
    assert!(!s.rules[1].server, "Exchange knows no newsletters");
    assert!(s.notices.iter().any(|n| n.text.contains("Newsletters")), "{:?}", s.notices);
}

/// An IMAP account's rules run in its Sieve script, so a rule Exchange couldn't take (looking in
/// every header at once) still goes to the server; what neither can run stays home.
#[test]
fn sieve_rules_go_to_an_imap_account() {
    use quatrelle_model::organize::{Condition, Rule, RuleAction, ServerKind};
    let mut s = sample_session("sieve-rules");
    let account = s.accounts[0].config.id.clone();
    assert_eq!(s.server_kind(&account), Some(ServerKind::Sieve), "the sample account is IMAP");
    let rule = Rule {
        name: "Urgent".into(),
        enabled: true,
        account: Some(account.clone()),
        conditions: vec![Condition::AnyText("urgent".into())],
        match_all: true,
        actions: vec![RuleAction::MarkRead, RuleAction::MoveToRole(FolderRole::Archive)],
        stop: false,
        server: true,
        remote_id: None,
        priority: None,
    };
    assert!(!rule.server_ready() && rule.sieve_ready(), "Exchange can't, Sieve can");
    s.run("rules.set", &json!({ "rules": [rule] })).unwrap();
    let sent: Vec<Vec<Rule>> = s
        .take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            quatrelle_model::Job::SetServerRules { rules, .. } => Some(rules),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1, "the rule marked for the server is sent");
    assert_eq!(sent[0].len(), 1);
    assert!(s.rules[0].server, "it stays marked for the server");
    assert!(!s.notices.iter().any(|n| n.text.contains("Urgent")), "nothing to keep home: {:?}", s.notices);
    // What Sieve can't look at either (attachments) stays on this computer, and says so.
    let mut bad = s.rules[0].clone();
    bad.name = "Attachments".into();
    bad.conditions = vec![Condition::HasAttachment];
    let mut good = s.rules[0].clone();
    good.name = "Urgent".into();
    s.run("rules.set", &json!({ "rules": [good, bad] })).unwrap();
    assert!(!s.rules[1].server, "Sieve can't see attachments");
    assert!(s.notices.iter().any(|n| n.text.contains("Attachments")), "{:?}", s.notices);
}

/// A contact keeps Outlook's work details: the command stores them, and reading the card back
/// gives them, so CardDAV, Exchange and a .pst carry the same fields.
#[test]
fn a_contact_keeps_outlooks_fields() {
    let mut s = sample_session("contact-fields");
    let v = s
        .run(
            "contact.create",
            &json!({
                "name": "Ana Lima",
                "emails": ["ana@contoso.example"],
                "org": "Contoso",
                "department": "Story",
                "office": "Room 4",
                "title": "Producer",
                "profession": "Storyteller",
                "manager": "Riley Kim",
                "assistant": "Sam Lee",
                "web": "https://ana.example",
                "im": "ana@chat.example",
                "nickname": "A",
                "spouse": "Jo Lima",
                "anniversary": "2018-06-02",
                "birthday": "1990-07-12",
            }),
        )
        .unwrap();
    let uid = v["uid"].as_str().unwrap_or_default().to_string();
    assert!(!uid.is_empty(), "{v}");
    let p = s.find_person(&uid).unwrap();
    let c = &p.card;
    assert_eq!((c.org.as_str(), c.department.as_str(), c.office.as_str()), ("Contoso", "Story", "Room 4"));
    assert_eq!((c.title.as_str(), c.profession.as_str()), ("Producer", "Storyteller"));
    assert_eq!((c.manager.as_str(), c.assistant.as_str()), ("Riley Kim", "Sam Lee"));
    assert_eq!((c.web.as_str(), c.im.as_str()), ("https://ana.example", "ana@chat.example"));
    assert_eq!((c.nickname.as_str(), c.spouse.as_str()), ("A", "Jo Lima"));
    assert_eq!((c.birthday.as_str(), c.anniversary.as_str()), ("1990-07-12", "2018-06-02"));
    // Changing one leaves the others; a wrong date is refused.
    s.run("contact.update", &json!({"uid": uid, "office": "Room 5"})).unwrap();
    let c = s.find_person(&uid).unwrap().card;
    assert_eq!((c.office.as_str(), c.department.as_str(), c.anniversary.as_str()), ("Room 5", "Story", "2018-06-02"));
    assert!(s.run("contact.update", &json!({"uid": uid, "anniversary": "June"})).is_err());
}

/// A contact's own picture is looked up by the item's id: Outlook keeps a contact's photo on the
/// contact item, and the server knows it by that id.
#[test]
fn a_contacts_own_picture_is_asked_for_by_its_id() {
    let mut s = sample_session("contact-photo");
    s.accounts[0].config.ews = Some("https://mail.example/EWS/Exchange.asmx".into());
    let account = s.accounts[0].config.id.clone();
    s.pim_mut(&account).cards.push(quatrelle_model::pim::StoredCard {
        book: "contacts".into(),
        href: "contact:C9".into(),
        etag: String::new(),
        card: quatrelle_model::vcard::Card {
            uid: "C9".into(),
            name: "Ana Lima".into(),
            emails: vec![quatrelle_model::vcard::Typed { value: "ana@contoso.example".into(), kind: "work".into() }],
            ..Default::default()
        },
        dirty: false,
    });
    s.run("contact.photo", &json!({"email": "Ana@Contoso.Example"})).unwrap();
    let jobs = s.take_jobs();
    assert_eq!(jobs.len(), 1);
    assert!(matches!(&jobs[0], quatrelle_model::Job::FetchPhoto { contact: Some(id), .. } if id == "C9"), "the item's id goes with the look-up");
}

/// A directory keeps a person's picture, at most once asked for: the People card shows it, and
/// an address without one isn't asked for again.
#[test]
fn a_directory_picture_is_asked_for_once_and_kept() {
    let mut s = sample_session("directory-photo");
    assert!(!s.person_photo_known("zoe@studio.example"));
    // An Exchange account is what can ask; the sample's own account stands in for one.
    s.accounts[0].config.ews = Some("https://mail.example/EWS/Exchange.asmx".into());
    let v = s.run("contact.photo", &json!({"email": "Zoe@Studio.Example"})).unwrap();
    assert_eq!(v["asked"], true, "{v}");
    let jobs = s.take_jobs();
    assert_eq!(jobs.len(), 1, "one look-up");
    assert!(matches!(&jobs[0], quatrelle_model::Job::FetchPhoto { email, .. } if email == "zoe@studio.example"));
    s.run("contact.photo", &json!({"email": "zoe@studio.example"})).unwrap();
    assert!(s.take_jobs().is_empty(), "already asked for");
    // The answer is kept by address, without case; a "none" is kept too.
    s.apply(quatrelle_model::Event::Photo {
        email: "Zoe@Studio.Example".into(),
        media: "image/jpeg".into(),
        data: vec![0xFF, 0xD8, 0xFF],
        error: None,
    });
    assert!(s.person_photo_known("zoe@studio.example"));
    let p = s.person_photo("zoe@studio.example").unwrap();
    assert_eq!(p.media, "image/jpeg");
    assert_eq!(p.bytes(1024).unwrap(), vec![0xFF, 0xD8, 0xFF]);
    s.apply(quatrelle_model::Event::Photo {
        email: "ghost@studio.example".into(),
        media: String::new(),
        data: Vec::new(),
        error: Some("no picture for this person".into()),
    });
    assert!(s.person_photo_known("ghost@studio.example"), "a person without a picture isn't asked for again");
    assert!(s.person_photo("ghost@studio.example").is_none());
}

#[test]
fn voting_buttons_show_and_a_vote_goes_as_a_job() {
    let mut s = sample_session("vote");
    let k = key_of(&s, sample::VOTE_SUBJECT);
    // Not offered on a message without voting buttons.
    s.run("list.open", &key_of(&s, "Sound mix, version 3")).unwrap();
    assert!(s.run("mail.vote", &json!({"choice": "Yes"})).is_err());
    s.run("list.open", &k).unwrap();
    let key = s.reading.as_ref().unwrap().key.clone();
    assert_eq!(s.voting(&key), Some((sample::VOTE_CHOICES.map(String::from).to_vec(), String::new())));
    assert!(matches!(s.run("mail.vote", &json!({"choice": "Maybe"})), Err(CmdError::Params(_))), "only the sender's choices");
    s.take_jobs();
    s.run("mail.vote", &json!({"choice": "Nothing to render"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::Vote { key: k, choice }] if *k == key && choice == "Nothing to render"));
    assert!(s.run("mail.vote", &json!({"choice": "Nothing to render"})).is_err(), "not twice while it's on its way");
    // Refused: nothing is remembered, and the user is told.
    s.apply(Event::Voted { key: key.clone(), choice: "Nothing to render".into(), error: Some("the account is offline".into()) });
    assert!(s.vote_sending.is_none() && s.voting(&key).unwrap().1.is_empty());
    assert!(s.notices.last().unwrap().text.contains("the account is offline"));
    // Sent: remembered with the message.
    s.run("mail.vote", &json!({"choice": "Need it this weekend", "key": key.to_id()})).unwrap();
    s.apply(Event::Voted { key: key.clone(), choice: "Need it this weekend".into(), error: None });
    assert_eq!(s.voting(&key).unwrap().1, "Need it this weekend");
    done(s);
}

#[test]
fn the_list_is_a_table_where_it_has_room() {
    let mut s = sample_session("list-layout");
    assert!(!s.list_as_table(), "cards beside the reading pane");
    s.run("view.readingPane", &json!({"value": "bottom"})).unwrap();
    assert!(s.list_as_table(), "a table over it");
    s.run("view.readingPane", &json!({"value": "off"})).unwrap();
    assert!(s.list_as_table());
    s.run("view.listLayout", &json!({"value": "cards"})).unwrap();
    assert!(!s.list_as_table(), "chosen: cards whatever the pane");
    s.run("view.readingPane", &json!({"value": "right"})).unwrap();
    s.run("view.listLayout", &json!({"value": "table"})).unwrap();
    assert!(s.list_as_table(), "chosen: a table even beside the pane");
    assert!(s.run("view.listLayout", &json!({"value": "grid"})).is_err());
    done(s);
}

#[test]
fn the_todo_bar_is_on_until_turned_off() {
    let mut s = sample_session("todo-bar");
    assert!(s.prefs.todo_bar);
    s.run("view.todoBar", &json!({})).unwrap();
    assert!(!s.prefs.todo_bar);
    s.run("view.todoBar", &json!({"on": true})).unwrap();
    assert!(s.prefs.todo_bar);
    done(s);
}

#[test]
fn another_mailbox_opens_in_an_exchange_account_with_its_own_deleted_items() {
    use quatrelle_model::{AccountConfig, Folder};
    let mut s = Session::open(Some(temp("shared-mailbox")), quatrelle_model::dates::Clock::new(NOW, 0));
    // Not without an Exchange account.
    assert!(s.run("account.openMailbox", &json!({"email": "support@contoso.example"})).is_err());
    let config = AccountConfig {
        name: "Riley".into(),
        email: "riley@contoso.example".into(),
        username: "riley".into(),
        ews: Some("https://mail.contoso.example/EWS/Exchange.asmx".into()),
        ..Default::default()
    };
    let id = s.add_account(config, Some("pw".into())).unwrap();
    s.take_jobs();
    for bad in ["not an address", "riley@contoso.example"] {
        assert!(s.run("account.openMailbox", &json!({"email": bad})).is_err(), "{bad}");
    }
    s.run("account.openMailbox", &json!({"email": "Support@Contoso.example"})).unwrap();
    assert_eq!(s.account(&id).unwrap().config.shared_mailboxes, ["support@contoso.example"]);
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::Start { account, .. } if account.shared_mailboxes == ["support@contoso.example"])), "{jobs:?}");
    assert!(s.pim.get(&id).is_some_and(|p| p.shared.contains(&"support@contoso.example".to_string())), "its calendar opens too");
    // Its folders come under it; deleting there goes to its own Deleted Items.
    let f = |path: &str, role: FolderRole| Folder {
        account: id.clone(),
        path: path.into(),
        delimiter: Some('/'),
        role,
        selectable: true,
        unread: 0,
        total: 0,
        name: None,
        mailbox: None,
        mailbox_role: FolderRole::Other,
    };
    let folders = vec![
        f("INBOX", FolderRole::Inbox),
        f("Deleted Items", FolderRole::Trash),
        f("support@contoso.example", FolderRole::Other),
        f("support@contoso.example/Inbox", FolderRole::Inbox),
        f("support@contoso.example/Deleted Items", FolderRole::Trash),
    ];
    s.apply(Event::Folders { account: id.clone(), folders });
    let a = s.account(&id).unwrap();
    assert_eq!(a.folder(FolderRole::Inbox).unwrap().path, "INBOX", "this account's own Inbox");
    assert_eq!(a.folder(FolderRole::Trash).unwrap().path, "Deleted Items");
    assert_eq!(a.folder_beside("support@contoso.example/Inbox", FolderRole::Trash).unwrap().path, "support@contoso.example/Deleted Items");
    assert_eq!(a.folder_beside("INBOX", FolderRole::Trash).unwrap().path, "Deleted Items");
    // Exchange marks a shared mailbox's folders themselves (`mailbox` and the role it has
    // there): mail deleted in one goes to that mailbox's own Deleted Items, not the account's.
    let marked = |path: &str, mailbox: &str, mailbox_role: FolderRole| Folder {
        account: id.clone(),
        path: path.into(),
        delimiter: Some('/'),
        role: FolderRole::Other,
        selectable: true,
        mailbox: Some(mailbox.into()),
        mailbox_role,
        ..Default::default()
    };
    s.apply(Event::Folders {
        account: id.clone(),
        folders: vec![
            f("INBOX", FolderRole::Inbox),
            f("Deleted Items", FolderRole::Trash),
            marked("Support", "support@contoso.example", FolderRole::Other),
            marked("Support/Inbox", "support@contoso.example", FolderRole::Inbox),
            marked("Support/Deleted Items", "support@contoso.example", FolderRole::Trash),
        ],
    });
    let a = s.account(&id).unwrap();
    assert_eq!(a.folder(FolderRole::Inbox).unwrap().path, "INBOX", "a shared mailbox's Inbox isn't the account's");
    assert_eq!(a.folder_mailbox("Support/Inbox").as_deref(), Some("support@contoso.example"));
    assert_eq!(a.folder_beside("Support/Inbox", FolderRole::Trash).unwrap().path, "Support/Deleted Items");
    assert_eq!(a.folder_beside("INBOX", FolderRole::Trash).unwrap().path, "Deleted Items");
    // Closed: its folders and calendar go.
    s.run("account.closeMailbox", &json!({"email": "support@contoso.example"})).unwrap();
    let a = s.account(&id).unwrap();
    assert!(a.config.shared_mailboxes.is_empty());
    assert!(a.folders.iter().all(|f| a.folder_mailbox(&f.path).is_none()), "its folders go, whichever way they're named");
    assert!(s.pim.get(&id).is_none_or(|p| p.shared.is_empty()));
    assert!(s.run("account.closeMailbox", &json!({"email": "support@contoso.example"})).is_err());
    if let Some(st) = &s.store {
        let _ = std::fs::remove_dir_all(st.root());
    }
}

/// Colleagues' calendars that come with Outlook's calendar groups, and public folders', come
/// unticked the first time (a dozen at once are slow to draw); this account's own, and one
/// opened here by address, come ticked; a tick given later stays.
#[test]
fn calendars_from_groups_come_unticked() {
    use quatrelle_model::pim::{CalendarInfo, DavSync, shared_calendar_id};
    let mut s = Session::open(Some(temp("cal-groups-unticked")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let cal = |id: &str, group: &str| CalendarInfo { id: id.into(), name: id.into(), group: group.into(), ..Default::default() };
    let ana = shared_calendar_id("ana@example.org");
    let tom = shared_calendar_id("tom@example.org");
    let sync = |cals: Vec<CalendarInfo>| DavSync { calendars: Some(cals), checked: NOW, ..Default::default() };
    s.apply_dav(&id, &sync(vec![cal("calendar", ""), cal(&ana, "Équipe : Studio"), cal(&tom, ""), cal("public:F1", "")]));
    assert!(s.calendar_shown(&id, "calendar"));
    assert!(!s.calendar_shown(&id, &ana), "a group's calendar comes unticked");
    assert!(s.calendar_shown(&id, &tom), "one opened by address comes ticked");
    assert!(!s.calendar_shown(&id, "public:F1"));
    // Ticked: it stays so at the next sync.
    s.run("calendar.show", &json!({"account": id, "calendar": ana, "shown": true})).unwrap();
    s.apply_dav(&id, &sync(vec![cal("calendar", ""), cal(&ana, "Équipe : Studio"), cal(&tom, ""), cal("public:F1", "")]));
    assert!(s.calendar_shown(&id, &ana));
}

/// Away from the calendar, an Exchange account's sync reads its own calendar only; opening the
/// calendar reads the calendars ticked at once, and one unticked isn't read.
#[test]
fn calendars_are_read_when_shown() {
    use quatrelle_model::pim::{CalendarInfo, DavSync, shared_calendar_id};
    let mut s = Session::open(Some(temp("cal-read-when-shown")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "riley@contoso.example", "ews": "mail.contoso.example", "username": "CONTOSO\\riley", "password": "pw"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let ana = shared_calendar_id("ana@contoso.example");
    let cal = |id: &str, group: &str| CalendarInfo { id: id.into(), name: id.into(), group: group.into(), ..Default::default() };
    s.apply_dav(&id, &DavSync { calendars: Some(vec![cal("calendar", ""), cal(&ana, "Équipe")]), checked: NOW, ..Default::default() });
    s.take_jobs();
    let pim = |s: &Session| s.pim.get(&id).cloned().unwrap_or_default();
    assert!(pim(&s).own_only, "in Mail");
    assert!(pim(&s).not_read.contains(&ana), "unticked");
    // The calendar opens: the ticked ones are read now.
    s.run("view.module", &json!({"name": "calendar"})).unwrap();
    let jobs = s.take_jobs();
    assert!(!pim(&s).own_only);
    assert!(jobs.iter().any(|j| matches!(j, Job::SyncDav { account } if *account == id)), "{jobs:?}");
    // Ticking Ana's reads it.
    s.run("calendar.show", &json!({"account": id, "calendar": ana, "shown": true})).unwrap();
    let jobs = s.take_jobs();
    assert!(pim(&s).not_read.is_empty());
    assert!(jobs.iter().any(|j| matches!(j, Job::SyncDav { .. })));
    // Back to Mail: nothing more to read until the calendar opens again.
    s.run("view.module", &json!({"name": "mail"})).unwrap();
    let jobs = s.take_jobs();
    assert!(pim(&s).own_only);
    assert!(!jobs.iter().any(|j| matches!(j, Job::SyncDav { .. })));
    // A sync that read this account's own calendar only keeps the others as they were.
    s.apply_dav(&id, &DavSync { calendars: Some(vec![cal("calendar", "")]), partial: true, checked: NOW, ..Default::default() });
    assert!(pim(&s).calendars.iter().any(|c| c.id == ana));
}

/// The signature goes into new messages, replies and forwards, or only some, as the account
/// says (Outlook's New messages and Replies/forwards); the choice is kept with the account.
#[test]
fn a_signature_goes_where_the_account_says() {
    let mut s = sample_session("signature-where");
    let first = s.list.first().map(|m| m.key.clone()).unwrap();
    let body = |s: &Session| s.compose.as_ref().map(|c| c.body.clone()).unwrap_or_default();
    s.run("account.setSignature", &json!({"account": "sample", "signature": "Riley", "newMessages": false})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    assert!(!body(&s).contains("Riley"), "not in new messages");
    s.compose = None;
    s.open_message(&first);
    s.run("mail.reply", &json!({})).unwrap();
    assert!(body(&s).contains("Riley"), "in replies");
    s.compose = None;
    // Changing the text alone keeps the choices.
    s.run("account.setSignature", &json!({"account": "sample", "signature": "R.", "replies": false})).unwrap();
    s.run("mail.forward", &json!({})).unwrap();
    assert!(!body(&s).contains("R."), "not in forwards");
    s.compose = None;
    s.run("mail.new", &json!({})).unwrap();
    assert!(!body(&s).contains("R."), "still not in new messages");
    let config = s.account("sample").unwrap().config.clone();
    assert!(config.signature_not_new && config.signature_not_replies);
}

/// A signature's logo is kept with the account and goes into each message with it, as a
/// picture of the message (never fetched from the web); a picture the HTML no longer names
/// goes.
#[test]
fn a_signature_keeps_its_logo() {
    let mut s = sample_session("signature-logo");
    let logo = json!({"cid": "logo@mc", "name": "logo.gif", "mime": "image/gif", "data": [71, 73, 70, 56, 57, 97]});
    let html = "<p><img src=\"cid:logo@mc\"></p><p><b>Didier GAUDIN</b><br>Tél. 02 51 70 93 93</p>";
    s.run("account.setSignature", &json!({"account": "sample", "signature": "x", "html": html, "pictures": [logo]})).unwrap();
    let config = s.account("sample").unwrap().config.clone();
    assert_eq!(config.signature_pictures.len(), 1);
    assert!(config.signature_html.contains("cid:logo@mc"), "{}", config.signature_html);
    s.run("mail.new", &json!({})).unwrap();
    let (text, html, pictures) = s.compose.as_ref().unwrap().bodies();
    assert!(text.contains("Didier GAUDIN"));
    assert!(html.unwrap_or_default().contains("cid:logo@mc"));
    assert_eq!(pictures.len(), 1);
    // Changed without its picture: it goes.
    s.run("account.setSignature", &json!({"account": "sample", "signature": "Didier", "html": "<p><b>Didier</b></p>"})).unwrap();
    assert!(s.account("sample").unwrap().config.signature_pictures.is_empty());
    // A picture on the web isn't kept (nor fetched).
    s.run("account.setSignature", &json!({"account": "sample", "signature": "x", "html": "<img src=\"https://www.example.com/logo.gif\"><b>D</b>"}))
        .unwrap();
    assert!(!s.account("sample").unwrap().config.signature_html.contains("https://www.example.com"));
}

/// A signature written in HTML goes as it was written (its logo beside the name, cleaned of
/// what mustn't go), where its text stands in the message; the text part has its text.
#[test]
fn an_html_signature_goes_as_written() {
    let mut s = Session::open(Some(temp("signature-html")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "dgaudin@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let html = r#"<table align="left" style="float:left;"><tr><td><a href="https://www.contoso.example"><img src="https://www.contoso.example/images/logo.gif" width="194"></a></td></tr></table>
<table style="float:left;border-left:1px solid #ccc;"><tr><td><p><span style="font-weight:bold">Didier GAUDIN</span><br>T&eacute;l. 02 51 70 93 93</p></td></tr></table><script>x()</script>"#;
    s.run("account.setSignature", &json!({"account": id, "signature": "", "rawHtml": html})).unwrap();
    let config = s.account(&id).unwrap().config.clone();
    assert!(config.signature.contains("Didier GAUDIN"), "{}", config.signature);
    assert!(!config.signature_raw_html.contains("<script"));
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "zoe@example.org", "subject": "Hello", "body": "Hi Zoe"})).unwrap();
    // The body set anew: the signature again, as a new message gets it.
    s.run("compose.insertSignature", &json!({})).unwrap();
    s.take_jobs();
    s.run("compose.send", &json!({})).unwrap();
    let raw = s
        .take_jobs()
        .into_iter()
        .find_map(|j| match j {
            Job::Send { raw, .. } => Some(raw),
            _ => None,
        })
        .unwrap();
    let p = quatrelle_mime::parse(&raw).unwrap();
    // Read back: the logo and the name side by side, the logo on the web, the message's text.
    assert!(p.had_html);
    let side_by_side = p.layout.iter().any(|b| matches!(b, quatrelle_mime::layout::Block::Table(t) if t.rows.iter().any(|r| r.len() == 2)));
    assert!(side_by_side, "{:?}", p.layout);
    assert!(p.images.iter().any(|u| u.contains("www.contoso.example/images/logo.gif")), "{:?}", p.images);
    assert!(p.text.contains("Hi Zoe") && p.text.contains("Didier GAUDIN") && !p.text.contains("x()"), "{}", p.text);
}
