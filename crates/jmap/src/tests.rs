use super::*;

#[test]
fn an_empty_numbering_never_overwrites_the_one_on_disk() {
    let dir = std::env::temp_dir().join(format!("quatrelle-jmap-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut st = State::default();
    assert_eq!(st.number("A"), 1);
    st.save(&store, "a1").unwrap();
    // A state that numbered nothing (a connection refused halfway, a mistake) must not take
    // the numbering with it: the next sync would renumber every folder and delete the cache.
    State::default().save(&store, "a1").unwrap();
    let mut back = State::load(&store, "a1");
    assert_eq!(back.id(1).as_deref(), Some("A"));
    assert_eq!(back.number("A"), 1, "and the same number");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The numbering is written again only once it changed: a big mailbox's is tens of megabytes,
/// and it went to disk after every folder read, changed or not. A reader connection's marks
/// come back newer than this one's copy of them, and win.
#[test]
fn the_numbering_is_written_only_when_it_changed() {
    let dir = std::env::temp_dir().join(format!("quatrelle-jmap-saved-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut st = State::default();
    st.number("m1");
    st.save(&store, "a1").unwrap();
    // Something else on disk: an unchanged state doesn't write over it...
    let mut other = State::default();
    other.number("x");
    other.number("y");
    store.save_account_doc("a1", "jmap.json", &other).unwrap();
    st.save(&store, "a1").unwrap();
    assert_eq!(State::load(&store, "a1").len(), 2, "unchanged: not written");
    // ...a changed one does.
    st.number("m2");
    st.save(&store, "a1").unwrap();
    assert!(State::load(&store, "a1").id(st.number("m2")).is_some());
    // Marks: the newer of each.
    let mark = |at| FolderMark { stamp: "s".into(), at, ..Default::default() };
    st.marks.insert("INBOX".into(), mark(10));
    st.marks.insert("Sent".into(), mark(30));
    let mut reader = st.reserve(100, &st.numbering());
    reader.marks.insert("INBOX".into(), mark(20));
    st.absorb(reader);
    assert_eq!((st.marks["INBOX"].at, st.marks["Sent"].at), (20, 30));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn session_urls_dates_and_keywords() {
    assert_eq!(session_url("jmap.example.com"), "https://jmap.example.com/.well-known/jmap");
    assert_eq!(session_url("https://api.example.com/"), "https://api.example.com/.well-known/jmap");
    assert_eq!(session_url("https://api.example.com/jmap/session"), "https://api.example.com/jmap/session");
    assert_eq!(utc_date("1970-01-02T00:00:00Z"), Some(86_400));
    assert_eq!(utc_date("2024-05-01T09:30:00.123Z"), utc_date("2024-05-01T09:30:00Z"));
    assert_eq!(utc_date("2024-05-01T11:30:00+02:00"), utc_date("2024-05-01T09:30:00Z"));
    for bad in ["", "2024-13-01T00:00:00Z", "yesterday", "2024-05-01"] {
        assert_eq!(utc_date(bad), None, "{bad}");
    }
    // Written back as asked (`after`), within the years a UTCDate has.
    assert_eq!(utc_date_string(utc_date("2024-02-29T23:59:58Z").unwrap()), "2024-02-29T23:59:58Z");
    assert_eq!((utc_date_string(-5), utc_date_string(i64::MAX)), ("1970-01-01T00:00:00Z".to_string(), "9999-12-31T23:59:59Z".to_string()));
    let two_years = SyncOptions { since_days: 730, ..Default::default() };
    assert_eq!(two_years.since(1_000_000_000), Some(1_000_000_000 - 730 * 86_400));
    assert_eq!(SyncOptions::default().since(1_000_000_000), None, "all of it");
    let f = keywords(Some(&json!({"$seen": true, "$Flagged": true, "$label1": true, "$draft": false, "bad word": true})));
    assert!(f.seen && f.flagged && !f.draft && f.has("$label1"));
    assert_eq!(flag_keyword(Flag::Deleted), None);
    let base = Url::parse("https://h.example/.well-known/jmap").unwrap();
    assert_eq!(
        template_url(&base, "/download/{accountId}/{blobId}/{name}?type={type}").unwrap(),
        "https://h.example/download/{accountId}/{blobId}/{name}?type={type}"
    );
    assert_eq!(expand("/d/{blobId}/{name}", &[("blobId", "b 1"), ("name", "m.eml")]), "/d/b%201/m.eml");
    assert!(template_url(&base, "http://h.example/upload/{accountId}/").is_err(), "no downgrade");
}

/// Two connections numbering messages at once never give the same number to two of them,
/// and the counter comes back to what's really in use once they're done.
#[test]
fn numbers_reserved_for_connections_of_their_own() {
    let mut main = State::default();
    main.mailboxes.insert("INBOX".into(), "mb1".into());
    let numbering = main.numbering();
    let mut a = main.reserve(1_000_000, &numbering);
    let mut b = main.reserve(1_000_000, &numbering);
    let na = a.number("id-a");
    let nb = b.number("id-b");
    assert_ne!(na, nb, "two connections, two numbers");
    assert!(main.known("id-a").is_none() && main.known("id-b").is_none(), "the main one saw neither yet");
    assert_eq!(a.mailbox("INBOX").unwrap(), "mb1", "the folders come with the copy");
    main.absorb(a);
    main.absorb(b);
    assert_eq!(main.id(na).as_deref(), Some("id-a"));
    assert_eq!(main.id(nb).as_deref(), Some("id-b"));
    assert_eq!(main.number("id-a"), na, "absorbed ids keep their number");
    main.compact();
    assert_eq!(main.next, nb.max(na) + 1, "the counter is back to what's used");
    // A number it had already: the same one.
    assert_eq!(main.number("id-a"), na);
    let base = main.next;
    let mut c = main.reserve(10, &main.numbering());
    assert_eq!(c.number("id-c"), base + 1, "a fresh block, above the numbers in use");
    assert_eq!(main.next, base + 10, "and the main one keeps clear of it");
    // A reader knows the numbers given before it: a message it finds again keeps its number
    // (a reader that didn't numbered a whole folder anew at every full read).
    assert_eq!(c.number("id-a"), na, "an old message, its old number");
    assert_eq!(c.known("id-b"), Some(nb));
    assert_eq!(c.id(na).as_deref(), Some("id-a"));
    // What it gives is told as it goes, for the main one to know before the reader is done.
    let nd = c.number("id-d");
    assert_eq!(c.take_given(), vec![("id-c".to_string(), base + 1), ("id-d".to_string(), nd)], "only the new ones");
    assert!(c.take_given().is_empty(), "told once");
    main.learn(vec![("id-d".to_string(), nd)]);
    assert_eq!(main.id(nd).as_deref(), Some("id-d"));
    assert_eq!(main.number("id-d"), nd);
}

#[test]
fn mailbox_paths() {
    let mb = |id: &str, name: &str, parent: Option<&str>, role: FolderRole| Mailbox {
        id: id.into(),
        name: name.into(),
        parent: parent.map(str::to_string),
        role,
        total: 0,
        unread: 0,
    };
    let boxes = vec![
        mb("1", "Inbox", None, FolderRole::Inbox),
        mb("2", "Projects", None, FolderRole::Other),
        mb("3", "Café", Some("2"), FolderRole::Other),
        mb("4", "Lists", Some("1"), FolderRole::Other),
        mb("5", "a/b", None, FolderRole::Other),
        mb("6", "Loop", Some("7"), FolderRole::Other),
        mb("7", "Back", Some("6"), FolderRole::Other),
        mb("8", "Projects", None, FolderRole::Other),
    ];
    let f = folders("a1", &boxes);
    let paths: Vec<&str> = f.iter().map(|(f, _)| f.path.as_str()).collect();
    assert_eq!(paths[..4], ["INBOX", "Projects", "Projects/Caf&AOk-", "INBOX/Lists"]);
    assert_eq!(utf7::decode(paths[4]), "a∕b", "a slash in a name isn't a level");
    assert!(paths[5].ends_with("Loop") && paths[6].ends_with("Back"), "a parent loop ends: {paths:?}");
    assert_eq!(paths[7], "Projects~8", "duplicate names stay apart");
    assert_eq!(f[2].0.display_name(), "Café");
    let mut st = State::default();
    let (a, b) = (st.number("M1"), st.number("M2"));
    assert_eq!((a, b, st.number("M1"), st.id(b).as_deref()), (1, 2, 1, Some("M2")));
    st.prune(&[b].into_iter().collect());
    assert_eq!(st.id(a), None);
    assert_eq!(st.number("M3"), 3, "numbers aren't reused");
}

/// A canned server: the session, then whatever the test hands back per API call.
struct Canned(std::cell::RefCell<Vec<Response>>);

impl Http for Canned {
    async fn send(&self, _req: Request) -> Result<Response> {
        let mut q = self.0.borrow_mut();
        if q.is_empty() { Err(Error::Io("no more answers".into())) } else { Ok(q.remove(0)) }
    }
}

#[test]
fn connect_checks_the_session() {
    let session = json!({
        "capabilities": {"urn:ietf:params:jmap:core": {"maxObjectsInGet": 50}, "urn:ietf:params:jmap:mail": {}},
        "primaryAccounts": {"urn:ietf:params:jmap:mail": "A1"},
        "apiUrl": "/api/", "downloadUrl": "/d/{accountId}/{blobId}", "uploadUrl": "/u/{accountId}/",
    });
    let ok = |v: &Value| Response { status: 200, body: v.to_string().into_bytes() };
    let c = block_on(Client::connect(Canned(vec![ok(&session)].into()), "jmap.example.com", "me", "pw", false)).unwrap();
    assert_eq!(c.api, "https://jmap.example.com/api/");
    assert_eq!(c.authorization(), "Basic bWU6cHc=");
    assert_eq!(c.events_url(), None);
    let refused = block_on(Client::connect(Canned(vec![Response { status: 401, body: Vec::new() }].into()), "h", "me", "x", false));
    assert!(matches!(refused, Err(Error::Auth(_))));
    let no_mail = json!({"capabilities": {}, "apiUrl": "/a"});
    assert!(matches!(block_on(Client::connect(Canned(vec![ok(&no_mail)].into()), "h", "me", "x", true)), Err(Error::Protocol(_))));
    let downgrade = json!({"capabilities": {"urn:ietf:params:jmap:mail": {}}, "primaryAccounts": {"urn:ietf:params:jmap:mail": "A"}, "apiUrl": "http://h/api", "downloadUrl": "/d", "uploadUrl": "/u"});
    assert!(matches!(block_on(Client::connect(Canned(vec![ok(&downgrade)].into()), "h", "me", "x", true)), Err(Error::Tls(_))));
}
