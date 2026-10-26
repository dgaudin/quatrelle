//! Exchange telling of changes as they happen, through the worker.

use super::*;
use crate::fakes::fake_ews::server::Server;

fn start(server: &Server, store: &Store) -> crate::worker::Pool {
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    let account = AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(server.url.clone()),
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    pool
}

fn count(server: &Server, op: &str) -> usize {
    server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == op).count()
}

/// As Outlook's Cached Exchange Mode is told of changes: new mail in the Inbox, mail a server
/// rule filed in another folder, a message read elsewhere and a shared mailbox's new mail are
/// read as the server tells of them — the other folders aren't looked at but at the full sync,
/// a quarter of an hour on. The answer the server ends is asked for again (of the server the
/// subscription lives on), and a subscription it forgot is made again: nothing is missed.
#[test]
fn changes_are_read_as_the_server_tells_of_them() {
    let mut d = Data { streaming: true, ..Default::default() };
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let projects = d.folder("Projects", None, None, "IPF.Note");
    let before = d.deliver(&inbox, &mail("Before", "b1"), false, "2026-10-20T08:00:00Z");
    let team =
        d.shared_mailbox(SharedBox { address: "team@contoso.example".into(), name: "Team".into(), automap: true, legacy: false, granted: true });
    let server = serve(d);
    let store = crate::fake::tests_store("ews-listening");
    let pool = start(&server, &store);
    let subjects =
        |path: &str| store.load_folder("a1", path).map(|f| f.messages.iter().map(|m| m.subject.clone()).collect::<Vec<_>>()).unwrap_or_default();
    let has = |path: &str, subject: &str| subjects(path).iter().any(|s| s == subject);
    eventually("listening", || count(&server, "GetStreamingEvents") >= 2);
    let deliver = |folder: &str, subject: &str, id: &str| {
        server.data.lock().unwrap().deliver(folder, &mail(subject, id), false, "2026-10-24T08:00:00Z");
    };
    deliver(&inbox, "Just now", "n1");
    deliver(&projects, "Filed by a rule", "n2");
    deliver(&team, "For the team", "n3");
    eventually("the new mail, wherever it came", || {
        has("INBOX", "Just now") && has("Projects", "Filed by a rule") && has("Team/Inbox", "For the team")
    });
    // Read elsewhere: the mark comes too.
    server.data.lock().unwrap().items.iter_mut().filter(|i| i.id == before).for_each(|i| i.read = true);
    let seen = || store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.subject == "Before" && m.flags.seen);
    eventually("the read mark", seen);
    // The server ends each answer (every couple of seconds here): asked for again.
    let streams = count(&server, "GetStreamingEvents");
    eventually("asked again", || count(&server, "GetStreamingEvents") >= streams + 2);
    deliver(&inbox, "After a while", "n4");
    eventually("still told", || has("INBOX", "After a while"));
    let cookies = server.data.lock().unwrap().stream_cookies.clone();
    assert!(cookies.iter().all(|c| c == crate::fakes::fake_ews::streaming::BACKEND), "asked of the server the subscription lives on: {cookies:?}");
    // The server forgot the subscriptions (it was restarted): made again, and what came
    // meanwhile read.
    let subscribed = count(&server, "Subscribe");
    {
        let mut d = server.data.lock().unwrap();
        d.subscriptions.clear();
        d.deliver(&inbox, &mail("While forgotten", "n5"), false, "2026-10-24T08:00:00Z");
    }
    eventually("subscribed again", || count(&server, "Subscribe") > subscribed);
    eventually("nothing missed", || has("INBOX", "While forgotten"));
    deliver(&projects, "Filed again", "n6");
    eventually("told again", || has("Projects", "Filed again"));
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A server that can't tell of changes (older than Exchange 2010 SP1): asked once, then the
/// Inbox is looked at as before.
#[test]
fn a_server_that_cant_tell_is_looked_at_as_before() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &mail("Before", "b1"), false, "2026-10-20T08:00:00Z");
    let server = serve(d);
    let store = crate::fake::tests_store("ews-not-listening");
    let mut pool = start(&server, &store);
    eventually("asked", || count(&server, "Subscribe") == 1);
    pool.submit(Job::SyncAll { account: "a1".into() });
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!((count(&server, "Subscribe"), count(&server, "GetStreamingEvents")), (1, 0), "asked once, never listened");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// A meeting accepted on the phone, or one a colleague who may moved: the calendar is read as
/// the server tells of it, not at the next full sync a quarter of an hour on.
#[test]
fn the_calendar_is_read_as_the_server_tells_of_its_changes() {
    let mut d = Data { streaming: true, ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let server = serve(d);
    let store = crate::fake::tests_store("ews-listening-calendar");
    let pool = start(&server, &store);
    eventually("listening", || count(&server, "GetStreamingEvents") >= 2);
    until(&pool, "the full sync's own read of the calendar", |e| matches!(e, Event::DavSynced { .. }));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let (start, end) = (crate::ews_pim::utc(now + 86_400), crate::ews_pim::utc(now + 90_000));
    let it = parse(&format!(
        "<t:CalendarItem><t:Subject>Budget review</t:Subject><t:Start>{start}</t:Start><t:End>{end}</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>budget-1</t:UID></t:CalendarItem>"
    ));
    server.data.lock().unwrap().pim.push(("N1".into(), cal, it.kids[0].clone()));
    until(&pool, "the new meeting", |e| matches!(e, Event::DavSynced { sync, .. } if sync.events.iter().any(|x| x.event.summary == "Budget review")));
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// New mail shows as soon as its header is in, as Outlook shows it: the list is told of before
/// the message is downloaded whole (a megabyte or two took as long again).
#[test]
fn new_mail_shows_before_it_is_downloaded_whole() {
    let mut d = Data { streaming: true, ..Default::default() };
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    let store = crate::fake::tests_store("ews-listening-shown");
    let pool = start(&server, &store);
    eventually("listening", || count(&server, "GetStreamingEvents") >= 1);
    server.data.lock().unwrap().slow_sources = Some(Duration::from_secs(3));
    server.data.lock().unwrap().deliver(&inbox, &mail("Just now", "n1"), false, "2026-10-24T08:00:00Z");
    let new = || store.load_folder("a1", "INBOX").ok().and_then(|f| f.messages.iter().find(|m| m.subject == "Just now").map(|m| m.key.clone()));
    until(&pool, "the new mail in the list", |e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX") && new().is_some());
    let key = new().unwrap();
    assert!(!store.has_body(&key), "shown before its source came");
    eventually("then downloaded whole", || store.has_body(&key));
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}

/// The full sync every quarter of an hour reads a hundred folders: what the server tells of
/// meanwhile is read between two of them, not once the pass is done.
#[test]
fn new_mail_is_read_during_a_full_sync() {
    let mut d = Data { streaming: true, ..Default::default() };
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    for n in 1..=5 {
        d.folder(&format!("F{n}"), None, None, "IPF.Note");
    }
    let server = serve(d);
    let store = crate::fake::tests_store("ews-listening-pass");
    let mut pool = start(&server, &store);
    eventually("listening", || count(&server, "GetStreamingEvents") >= 1);
    server.data.lock().unwrap().slow_counts = Some(Duration::from_millis(300));
    pool.submit(Job::SyncAll { account: "a1".into() });
    let syncing = |e: &Event, f: &str| matches!(e, Event::Status { status: SyncStatus::Syncing { folder }, .. } if folder == f);
    until(&pool, "the pass under way", |e| syncing(e, "F2"));
    server.data.lock().unwrap().deliver(&inbox, &mail("During the pass", "n1"), false, "2026-10-24T08:00:00Z");
    let seen = until(&pool, "the pass's last folder", |e| syncing(e, "F5"));
    assert!(seen.iter().any(|e| syncing(e, "INBOX")), "the Inbox read during the pass: {seen:?}");
    let has = || store.load_folder("a1", "INBOX").is_ok_and(|f| f.messages.iter().any(|m| m.subject == "During the pass"));
    eventually("the new mail", has);
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
}
