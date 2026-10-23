//! Mail kept as Outlook's Cached Exchange Mode keeps it, over the fake Graph server: what came in
//! the time kept, older mail let go with its source, every message kept downloaded whole.

use crate::sync::{SyncOptions, WordQuery};

use super::*;

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// `days` ago (and an hour), as Graph writes a received time.
fn ago(days: i64) -> String {
    let t = now() - days * 86_400 - 3600;
    let (y, m, d) = quatrelle_model::dates::civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

/// A Graph account on the fake server, its folders listed: the client, its state, a store.
fn connect(server: &Server, name: &str) -> (crate::graph::Client, crate::jmap::State, Store) {
    let c = crate::graph::Client::connect(&server.base, TOKEN).unwrap();
    let mut st = crate::jmap::State::default();
    crate::graph::list_folders(&c, &mut st, "a1", &[]).unwrap();
    (c, st, crate::fake::tests_store(name))
}

/// The Inbox's subjects here, sorted.
fn kept(store: &Store) -> Vec<String> {
    let mut out: Vec<String> = store.load_folder("a1", "INBOX").unwrap().messages.into_iter().map(|m| m.subject).collect();
    out.sort();
    out
}

/// The Inbox's subjects whose source is here, newest first.
fn downloaded(store: &Store) -> Vec<String> {
    store.load_folder("a1", "INBOX").unwrap().messages.into_iter().filter(|m| store.has_body(&m.key)).map(|m| m.subject).collect()
}

/// The folder listings asked for.
fn listings(server: &Server) -> Vec<String> {
    server.data.lock().unwrap().calls.iter().filter(|c| c.contains("/messages?")).cloned().collect()
}

/// Thirty days kept: the first sync brings in only what came in since, asked of the server as
/// a filter on the received time, every page of it. A forward whose own date is years older
/// came in two days ago: it's kept, and stays (not let go and listed again at each sync).
#[test]
fn graph_keeps_only_the_mail_received_in_the_time_kept() {
    let mut d = Data { page_messages: Some(2), ..Default::default() };
    let inbox = d.folder("Inbox", None, Some("inbox"));
    for (n, days) in [1, 2, 3, 4, 5].into_iter().enumerate() {
        d.deliver(&inbox, &mail(&format!("Recent {n}"), &format!("r{n}")), true, &ago(days));
    }
    for (n, days) in [40, 100, 400].into_iter().enumerate() {
        d.deliver(&inbox, &mail(&format!("Old {n}"), &format!("o{n}")), true, &ago(days));
    }
    let forward = b"From: Zoe Park <zoe@contoso.example>\r\nTo: me@contoso.example\r\nSubject: Fwd: the brief\r\n\
Date: Mon, 01 Jan 2018 09:00:00 +0000\r\nMessage-ID: <fwd@x>\r\n\r\nAs promised.\r\n";
    d.deliver(&inbox, forward, false, &ago(2));
    let server = serve(d);
    let (c, mut st, store) = connect(&server, "graph-time-kept");
    let opts = SyncOptions { prefetch: 0, since_days: 30, ..Default::default() };
    let first = crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!(kept(&store), ["Fwd: the brief", "Recent 0", "Recent 1", "Recent 2", "Recent 3", "Recent 4"]);
    assert_eq!((first.new, first.total), (6, 9), "the folder's count is the server's");
    let asked = listings(&server);
    assert!(asked.len() >= 3, "every page: {asked:?}");
    assert!(asked.iter().all(|c| c.contains("$filter=receivedDateTime%20ge%20") && c.contains("$orderby=receivedDateTime%20desc")), "{asked:?}");
    let cutoff = now() - 30 * 86_400;
    let fwd = store.load_folder("a1", "INBOX").unwrap().messages.into_iter().find(|m| m.subject == "Fwd: the brief").unwrap();
    assert!(fwd.date < cutoff && fwd.received_at() >= cutoff, "dated 2018, received this week: {fwd:?}");
    let again = crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!((again.new, again.removed), (0, 0));
    assert_eq!(kept(&store).len(), 6);
    // Kept from before summaries knew when the server received them: it learns it, and stays.
    let mut fs = store.load_folder("a1", "INBOX").unwrap();
    fs.messages.iter_mut().for_each(|m| m.received = 0);
    store.save_folder("a1", "INBOX", &fs).unwrap();
    let after = crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!((after.new, after.removed), (0, 0));
    assert!(store.load_folder("a1", "INBOX").unwrap().messages.iter().all(|m| m.received >= cutoff));
    let _ = std::fs::remove_dir_all(store.root());
}

/// Fewer days kept: the older mail goes at the next sync, its source with it, though nothing
/// changed in the folder; what a search of the server found stays while the folder has it.
#[test]
fn graph_lets_older_mail_go_with_its_source_but_keeps_what_a_search_found() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    for n in 0..3 {
        d.deliver(&inbox, &mail(&format!("Recent {n}"), &format!("r{n}")), true, &ago(n + 1));
    }
    for (subject, days) in [("Storyboard", 60), ("Budget", 90), ("Casting", 120)] {
        d.deliver(&inbox, &mail(subject, subject), true, &ago(days));
    }
    let server = serve(d);
    let (c, mut st, store) = connect(&server, "graph-time-ages");
    crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &SyncOptions::default()).unwrap();
    assert_eq!(downloaded(&store).len(), 6, "all of it kept, and downloaded");
    let old: Vec<quatrelle_model::MessageKey> =
        store.load_folder("a1", "INBOX").unwrap().messages.into_iter().filter(|m| !m.subject.starts_with("Recent")).map(|m| m.key).collect();
    let budget = WordQuery { text: "Budget".into(), ..Default::default() };
    let (found, _) = crate::graph::find_on_server(&c, &mut st, &store, "a1", "INBOX", &budget, 10).unwrap();
    assert_eq!(found.len(), 1);
    let month = SyncOptions { since_days: 30, ..Default::default() };
    let r = crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &month).unwrap();
    assert_eq!((r.new, r.removed), (0, 2));
    assert_eq!(kept(&store), ["Budget", "Recent 0", "Recent 1", "Recent 2"]);
    assert_eq!(old.iter().filter(|k| store.has_body(k)).count(), 1, "the sources of the mail let go are gone");
    assert_eq!(downloaded(&store), ["Recent 0", "Recent 1", "Recent 2", "Budget"]);
    let again = crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &month).unwrap();
    assert_eq!((again.new, again.removed), (0, 0), "what was found stays");
    let _ = std::fs::remove_dir_all(store.root());
}

/// No time limit: the folder is listed as before, without a filter, its oldest mail kept.
#[test]
fn graph_keeps_all_its_mail_when_the_time_is_not_limited() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    d.deliver(&inbox, &mail("Recent", "r"), true, &ago(1));
    d.deliver(&inbox, &mail("Ancient", "a"), true, &ago(3000));
    let server = serve(d);
    let (c, mut st, store) = connect(&server, "graph-time-all");
    crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &SyncOptions::default()).unwrap();
    assert_eq!(kept(&store), ["Ancient", "Recent"]);
    let asked = listings(&server);
    assert!(!asked.is_empty() && asked.iter().all(|c| !c.contains("$filter") && c.contains("$orderby=receivedDateTime%20desc")), "{asked:?}");
    let _ = std::fs::remove_dir_all(store.root());
}

fn ahead(c: &crate::graph::Client, st: &crate::jmap::State, store: &Store, opts: &SyncOptions, budget: usize) -> usize {
    crate::graph::download_ahead(c, st, store, "a1", "INBOX", opts, budget).unwrap()
}

/// Every message kept downloaded whole, a few at a time, the newest first: mail with
/// attachments and large mail too. Without that, what the sync downloads ahead: no attachments,
/// nothing over the cap (asked for once, then known by its size).
#[test]
fn graph_downloads_every_kept_message_whole_a_few_at_a_time() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"));
    let attached = b"From: Zoe Park <zoe@contoso.example>\r\nTo: me@contoso.example\r\nSubject: Plans\r\nMessage-ID: <plans@x>\r\n\
MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nSee attached.\r\n\
--b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"plan.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\n\
JVBERi0xLjQK\r\n--b--\r\n";
    let long =
        format!("From: Zoe <zoe@contoso.example>\r\nTo: me@contoso.example\r\nSubject: Long\r\nMessage-ID: <long@x>\r\n\r\n{}\r\n", "x".repeat(4000));
    let ids = [
        d.deliver(&inbox, &mail("First", "d1"), true, &ago(1)),
        d.deliver(&inbox, attached, true, &ago(2)),
        d.deliver(&inbox, long.as_bytes(), true, &ago(3)),
        d.deliver(&inbox, &mail("Fourth", "d4"), true, &ago(4)),
        d.deliver(&inbox, &mail("Fifth", "d5"), true, &ago(5)),
        d.deliver(&inbox, &mail("Sixth", "d6"), true, &ago(6)),
    ];
    d.unreadable.push(ids[4].clone());
    let server = serve(d);
    let downloads = || -> Vec<String> {
        let d = server.data.lock().unwrap();
        d.calls.iter().filter_map(|c| c.strip_prefix("GET /me/messages/")?.strip_suffix("/$value").map(str::to_string)).collect()
    };
    let (c, mut st, store) = connect(&server, "graph-whole");
    let full = SyncOptions { prefetch: 0, prefetch_max_bytes: 1000, full: true, ..Default::default() };
    crate::graph::sync_folder(&c, &mut st, &store, "a1", "INBOX", &full).unwrap();
    assert!(downloads().is_empty());
    assert_eq!(ahead(&c, &st, &store, &full, 2), 4);
    assert_eq!(downloads(), ids[..2], "the newest first, attachments and all");
    assert_eq!(ahead(&c, &st, &store, &full, 2), 2);
    assert_eq!(downloads()[2..], ids[2..4], "the large one too");
    assert_eq!(ahead(&c, &st, &store, &full, 10), 1, "the one the server won't give is still to download");
    assert_eq!(ahead(&c, &st, &store, &full, 10), 1);
    assert_eq!(downloaded(&store), ["First", "Plans", "Long", "Fourth", "Sixth"]);
    let fs = store.load_folder("a1", "INBOX").unwrap();
    assert!(fs.messages.iter().any(|m| m.subject == "Plans" && m.has_attachments && m.size > 0), "filled in from the source");

    // Not all of it: what the sync would download ahead.
    server.data.lock().unwrap().unreadable.clear();
    let part = SyncOptions { full: false, ..full };
    let some = crate::fake::tests_store("graph-whole-part");
    crate::graph::sync_folder(&c, &mut st, &some, "a1", "INBOX", &part).unwrap();
    server.data.lock().unwrap().calls.clear();
    assert_eq!(ahead(&c, &st, &some, &part, 10), 0);
    assert_eq!(
        downloads(),
        [ids[0].clone(), ids[2].clone(), ids[3].clone(), ids[4].clone(), ids[5].clone()],
        "no attachments; the large one asked for once"
    );
    assert_eq!(downloaded(&some), ["First", "Fourth", "Fifth", "Sixth"]);
    let fs = some.load_folder("a1", "INBOX").unwrap();
    assert!(fs.messages.iter().any(|m| m.subject == "Long" && m.size > 1000), "its size kept");
    assert_eq!(ahead(&c, &st, &some, &part, 10), 0);
    assert_eq!(downloads().len(), 5, "not asked for again");

    // All of it: the newest come whole with the folder's list, whatever they hold.
    let with_list = crate::fake::tests_store("graph-whole-sync");
    crate::graph::sync_folder(&c, &mut st, &with_list, "a1", "INBOX", &SyncOptions { prefetch: 3, ..full }).unwrap();
    assert_eq!(downloaded(&with_list), ["First", "Plans", "Long"]);
    for s in [store, some, with_list] {
        let _ = std::fs::remove_dir_all(s.root());
    }
}
