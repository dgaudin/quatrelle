//! Tests of the EWS transport: rules, headers, searching, syncing and Autodiscover, over the fake server.

use super::autodiscover::autodiscover_answer;
use super::rules::{inbox_rule_xml, server_rule};
use super::sync::aqs_of;
use super::*;

#[test]
fn the_inbox_rules_go_to_exchange_and_come_back() {
    use quatrelle_model::organize::{Condition, Rule, RuleAction};
    let mut d = crate::fakes::fake_ews::Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    // A rule made in Outlook: listed with the others, and never touched by Quatrelle.
    d.rules.push(("R-OUTLOOK".into(), "Outlook rule".into()));
    let server = crate::fakes::fake_ews::serve(d);
    let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let rule = |name: &str, id: Option<&str>| Rule {
        name: name.into(),
        enabled: true,
        account: Some("a1".into()),
        conditions: vec![Condition::Subject("invoice".into())],
        match_all: true,
        actions: vec![RuleAction::MoveTo("Invoices".into())],
        stop: false,
        server: true,
        remote_id: id.map(str::to_string),
        priority: None,
        refused: None,
    };
    // Created: the server's id comes back, found by name (creating answers none).
    let ids = c.set_inbox_rules(&[(rule("Invoices", None), Some("F-INVOICES".into()))], &[]).unwrap();
    assert_eq!(ids.len(), 1);
    let (name, id) = ids[0].clone();
    assert_eq!(name, "Invoices");
    assert!(!id.is_empty());
    assert_eq!(
        c.inbox_rule_ids().unwrap(),
        vec![("Outlook rule".to_string(), "R-OUTLOOK".to_string()), ("Invoices".to_string(), id.clone())],
        "the server's rules all come back, whoever made them"
    );
    // Changed by its id: the server keeps the same rule, under its new name.
    let ids = c.set_inbox_rules(&[(rule("Invoices 2026", Some(&id)), Some("F-INVOICES".into()))], &[]).unwrap();
    assert_eq!(ids, vec![("Invoices 2026".to_string(), id.clone())]);
    assert_eq!(c.inbox_rule_ids().unwrap(), vec![("Outlook rule".to_string(), "R-OUTLOOK".to_string()), ("Invoices 2026".to_string(), id.clone())]);
    // Removed.
    assert!(c.set_inbox_rules(&[], std::slice::from_ref(&id)).unwrap().is_empty());
    assert_eq!(
        c.inbox_rule_ids().unwrap(),
        vec![("Outlook rule".to_string(), "R-OUTLOOK".to_string())],
        "ours is gone; the one Outlook made is left alone"
    );
    let calls = server.data.lock().unwrap().calls.clone();
    assert!(calls.iter().any(|c| c == "UpdateInboxRules"), "{calls:?}");
}

#[test]
fn answers_are_read_carefully() {
    let doc = parse(
        "<s:Envelope xmlns:s=\"x\"><s:Body><m:GetItemResponse><m:ResponseMessages>\
             <m:GetItemResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:Items>\
             <t:Message><t:ItemId Id=\"A&amp;1\" ChangeKey=\"c\"/><t:Subject>Caf&#233; &lt;plan&gt;</t:Subject><t:IsRead>true</t:IsRead>\
             <t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty>\
             <t:From><t:Mailbox><t:Name>Zoe \"Z\" Park</t:Name><t:EmailAddress>zoe@contoso.example</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType></t:Mailbox></t:From>\
             <t:ToRecipients><t:Mailbox><t:Name>Me</t:Name><t:EmailAddress>/o=Org/cn=me</t:EmailAddress><t:RoutingType>EX</t:RoutingType></t:Mailbox></t:ToRecipients>\
             <t:DateTimeSent>2026-10-15T09:00:00Z</t:DateTimeSent></t:Message></m:Items></m:GetItemResponseMessage>\
             <m:GetItemResponseMessage ResponseClass=\"Error\"><m:MessageText>The specified object was not found.</m:MessageText><m:ResponseCode>ErrorItemNotFound</m:ResponseCode></m:GetItemResponseMessage>\
             </m:ResponseMessages></m:GetItemResponse></s:Body></s:Envelope>",
    );
    let ms = messages(&doc);
    assert_eq!(ms.len(), 2);
    assert!(check_message(ms[0]).is_ok());
    assert!(matches!(check_message(ms[1]), Err(NetError::No(m)) if m.contains("not found")));
    let it = ms[0].find("Message").unwrap();
    assert_eq!(it.child("ItemId").unwrap().attr("Id"), Some("A&1"));
    assert_eq!(it.text_of("Subject"), Some("Café <plan>"));
    let f = flags(it);
    assert!(f.seen && f.flagged && !f.draft);
    let h = String::from_utf8(header_block(it)).unwrap();
    assert!(h.contains("From: \"Zoe Z Park\" <zoe@contoso.example>\r\n"), "{h}");
    assert!(h.contains("To: \"Me\" <>\r\n"), "an Exchange-internal address keeps the name: {h}");
    assert!(h.contains("Subject: =?UTF-8?B?"), "{h}");
    // Hostile: deep, broken, junk.
    let deep = "<a>".repeat(10_000);
    let _ = parse(&deep);
    let _ = parse("<a><b></a></b>&bogus;&#xFFFFFFFF;");
    // Folders of every kind, in the answer's order (contacts and calendars aren't `Folder`).
    let f = parse(
        "<m:RootFolder><t:Folders><t:Folder><t:DisplayName>A</t:DisplayName></t:Folder>                       <t:ContactsFolder><t:DisplayName>B</t:DisplayName></t:ContactsFolder><t:CalendarFolder><t:DisplayName>C</t:DisplayName></t:CalendarFolder>                       <t:TasksFolder><t:DisplayName>D</t:DisplayName></t:TasksFolder><t:SearchFolder><t:DisplayName>E</t:DisplayName></t:SearchFolder></t:Folders></m:RootFolder>",
    );
    let names: Vec<&str> = f.folders().iter().filter_map(|x| x.text_of("DisplayName")).collect();
    assert_eq!(names, ["A", "B", "C", "D", "E"]);
    // Entities in text and attributes, CDATA and numeric references, read as text.
    let e = parse("<t:a xmlns:t=\"x\" t:Id=\"R&amp;D &quot;1&quot;\">Tom &amp; Jerry &lt;3 &#233;t&#xE9; <![CDATA[<raw> & co]]> ünï</t:a>");
    let a = e.find("a").unwrap();
    assert_eq!(a.attr("Id"), Some("R&D \"1\""));
    assert_eq!(a.text, "Tom & Jerry <3 été <raw> & co ünï");
    assert_eq!(unb64("aGk=\r\n"), Some(b"hi".to_vec()));
    assert_eq!(unb64("**"), None);
    assert_eq!(esc("a<b&\"c\u{1}"), "a&lt;b&amp;&quot;c");
}

#[test]
fn a_query_is_put_in_the_servers_own_words() {
    // The words, and the fields Exchange knows; what only the local index can answer for is
    // left out rather than sent as nonsense.
    assert_eq!(aqs_of("deepseek"), "deepseek");
    assert_eq!(aqs_of("deepseek report"), "deepseek report");
    assert_eq!(aqs_of("from:ada subject:\"shot list\""), "from:ada subject:\"shot list\"");
    assert_eq!(aqs_of("\"shot list\""), "\"shot list\"");
    assert_eq!(aqs_of("is:unread deepseek"), "deepseek");
    assert_eq!(aqs_of("is:unread"), "");
    assert_eq!(aqs_of("-pizza lunch"), "lunch", "a negation stays with the index");
}

#[test]
fn a_search_finds_a_message_whose_text_is_only_on_the_server() {
    let mut d = crate::fakes::fake_ews::Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let archive = d.folder("Archive", None, None, "IPF.Note");
    let inbox = d.by_name("Inbox").map(|f| f.id.clone()).unwrap_or_default();
    // An old one: this computer has neither its headers nor its body.
    d.deliver(
        &archive,
        b"From: Ada <ada@example.org>\r\nTo: riley@contoso.example\r\nSubject: Old notes\r\nDate: Tue, 01 Feb 2005 09:00:00 +0000\r\n\r\nwe tried deepseek in the lab\r\n",
        true,
        "2005-02-01T09:00:00Z",
    );
    d.deliver(
        &inbox,
        b"From: Bob <bob@example.org>\r\nTo: riley@contoso.example\r\nSubject: Lunch\r\nDate: Wed, 21 Oct 2026 09:00:00 +0000\r\n\r\npizza?\r\n",
        false,
        "2026-10-21T09:00:00Z",
    );
    let server = crate::fakes::fake_ews::serve(d);
    let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    let folders: Vec<String> = list_folders(&c, &mut st, "a1").unwrap().into_iter().map(|f| f.path).collect();
    assert!(folders.len() >= 2, "{folders:?}");
    // The word is in a body that was never downloaded: the server answers anyway, with the
    // headers the list shows.
    let found = search(&c, &mut st, "a1", &folders, "deepseek", 50).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].subject, "Old notes");
    assert_eq!(found[0].from.email, "ada@example.org");
    assert_eq!(found[0].key.folder, "Archive", "the folder it came from");
    assert_eq!(found[0].date, 1_107_248_400);
    assert_eq!(found[0].key.account, "a1");
    // Nothing in the inbox has the word.
    assert!(search(&c, &mut st, "a1", &folders, "pizza", 50).unwrap().iter().all(|m| m.subject != "Old notes"));
    assert_eq!(search(&c, &mut st, "a1", &folders, "lunch", 50).unwrap().len(), 1);
    // Two searches, one numbering: the same message keeps its key.
    let again = search(&c, &mut st, "a1", &folders, "deepseek", 50).unwrap();
    assert_eq!(again[0].key, found[0].key);
}

#[test]
fn a_message_the_search_found_can_be_read() {
    // Three recent ones and an old one outside the window this computer keeps: the search is
    // the only way to it, and it has to be readable from what the search answered.
    let mut d = crate::fakes::fake_ews::Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    for n in 1..=3 {
        let raw = format!(
            "From: Bob <bob@example.org>\r\nTo: riley@contoso.example\r\nSubject: Note {n}\r\nDate: Tue, 20 Oct 2026 09:0{n}:00 +0000\r\n\r\nhello {n}\r\n"
        );
        d.deliver(&inbox, raw.as_bytes(), false, &format!("2026-10-20T09:0{n}:00Z"));
    }
    let old = d.deliver(
        &inbox,
        b"From: Ada <ada@example.org>\r\nTo: riley@contoso.example\r\nSubject: Old notes\r\nDate: Tue, 01 Feb 2005 09:00:00 +0000\r\n\r\nwe tried deepseek in the lab\r\n",
        true,
        "2005-02-01T09:00:00Z",
    );
    let _ = old;
    let server = crate::fakes::fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-search-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { window: 3, prefetch: 0, ..Default::default() };
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 3, "the old one is out of the window");
    let hits = search(&c, &mut st, "a1", &["INBOX".to_string()], "deepseek", 50).unwrap();
    assert_eq!(hits.len(), 1);
    let raw = fetch_body(&c, &mut st, &store, &hits[0].key).unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("deepseek"), "read from the id the search numbered");
    assert!(store.read_body(&hits[0].key).is_some(), "kept like any other message");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A folder as it was when last read in full (the server's stamp of its last change, its
/// counts) is left alone: one look at it rather than a listing. New mail, a message read
/// elsewhere, an old mark, or a server that doesn't stamp folders: listed again.
#[test]
fn a_folder_that_hasnt_changed_isnt_listed_again() {
    let mut d = crate::fakes::fake_ews::Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    for i in 1..=2 {
        d.deliver(&inbox, format!("From: a@x.example\r\nSubject: m{i}\r\n\r\nx\r\n").as_bytes(), false, &format!("2024-05-0{i}T09:00:00Z"));
    }
    let server = crate::fakes::fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-marks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    // Anything that reads the folder's messages: a listing, or its changes.
    let listings = || server.data.lock().unwrap().calls.iter().filter(|c| matches!(c.as_str(), "FindItem" | "SyncFolderItems")).count();
    let sync = |st: &mut State| sync_folder(&c, st, &store, "a1", "INBOX", &opts).unwrap();
    assert_eq!(sync(&mut st).new, 2);
    let n = listings();
    let r = sync(&mut st);
    assert_eq!(listings(), n, "unchanged: not listed again");
    assert_eq!((r.total, r.unread, r.new), (2, 2, 0), "its counts all the same");
    // New mail: listed, and it's there.
    let id = server.data.lock().unwrap().deliver(&inbox, b"From: b@x.example\r\nSubject: m3\r\n\r\nx\r\n", false, "2024-05-03T09:00:00Z");
    assert_eq!(sync(&mut st).new, 1);
    assert!(listings() > n);
    // Read elsewhere: the read mark comes in.
    let n = listings();
    server.data.lock().unwrap().items.iter_mut().filter(|i| i.id == id).for_each(|i| i.read = true);
    sync(&mut st);
    assert!(listings() > n);
    assert!(store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.subject == "m3" && m.flags.seen));
    // A mark hours old isn't trusted.
    let n = listings();
    if let Some(m) = st.marks.get_mut("INBOX") {
        m.at -= 7 * 3600;
    }
    sync(&mut st);
    assert!(listings() > n, "an old mark: listed again");
    // A server that doesn't stamp its folders: read every time.
    server.data.lock().unwrap().no_commit_time = true;
    let n = listings();
    sync(&mut st);
    sync(&mut st);
    assert!(listings() >= n + 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Once a folder has been read, only what changed since is asked for (`SyncFolderItems`): new
/// mail, read marks, flags and deletions come in without listing the folder, a page at a time;
/// a state the server refuses has the folder listed in full again.
#[test]
fn a_folder_read_once_asks_only_what_changed() {
    let mut d = crate::fakes::fake_ews::Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let mut ids = Vec::new();
    for i in 1..=3 {
        ids.push(d.deliver(&inbox, format!("From: a@x.example\r\nSubject: m{i}\r\n\r\nx\r\n").as_bytes(), false, &format!("2024-05-0{i}T09:00:00Z")));
    }
    let server = crate::fakes::fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-changes-only-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    let count = |op: &str| server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == op).count();
    let sync = |st: &mut State| sync_folder(&c, st, &store, "a1", "INBOX", &opts).unwrap();
    let subjects = || {
        let mut v: Vec<(String, bool, bool)> =
            store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| (m.subject.clone(), m.flags.seen, m.flags.flagged)).collect();
        v.sort();
        v
    };
    assert_eq!(sync(&mut st).new, 3);
    assert!(st.marks.get("INBOX").is_some_and(|m| !m.sync.is_empty()), "its state kept");
    // New mail, one read and one flagged elsewhere, one deleted: only the changes are asked.
    let listed = count("FindItem");
    {
        let mut d = server.data.lock().unwrap();
        d.deliver(&inbox, b"From: b@x.example\r\nSubject: m4\r\n\r\nx\r\n", false, "2024-05-04T09:00:00Z");
        d.items.iter_mut().filter(|i| i.id == ids[0]).for_each(|i| i.read = true);
        d.items.iter_mut().filter(|i| i.id == ids[1]).for_each(|i| i.flagged = true);
        d.items.retain(|i| i.id != ids[2]);
    }
    let r = sync(&mut st);
    assert_eq!(count("FindItem"), listed, "not listed again");
    assert_eq!((r.new, r.changed, r.removed), (1, 2, 1), "{r:?}");
    assert_eq!(subjects(), [("m1".into(), true, false), ("m2".into(), false, true), ("m4".into(), false, false)]);
    // A page at a time.
    server.data.lock().unwrap().sync_page = Some(1);
    {
        let mut d = server.data.lock().unwrap();
        for i in 5..=7 {
            d.deliver(&inbox, format!("From: c@x.example\r\nSubject: m{i}\r\n\r\nx\r\n").as_bytes(), false, &format!("2024-05-0{i}T09:00:00Z"));
        }
    }
    let pages = count("SyncFolderItems");
    assert_eq!(sync(&mut st).new, 3);
    assert!(count("SyncFolderItems") >= pages + 3, "three pages");
    server.data.lock().unwrap().sync_page = None;
    // A state the server no longer takes: listed in full, and right.
    if let Some(m) = st.marks.get_mut("INBOX") {
        m.sync = "garbage".into();
    }
    server.data.lock().unwrap().deliver(&inbox, b"From: d@x.example\r\nSubject: m8\r\n\r\nx\r\n", false, "2024-05-08T09:00:00Z");
    let listed = count("FindItem");
    assert_eq!(sync(&mut st).new, 1);
    assert!(count("FindItem") > listed, "listed in full");
    assert!(st.marks.get("INBOX").is_some_and(|m| m.sync.starts_with("v2")), "a good state again");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Messages are downloaded ahead a few at a time in one request, not one request each; one the
/// server won't give is left for when it's opened, the others still come.
#[test]
fn messages_are_downloaded_ahead_a_few_at_a_time() {
    let mut d = crate::fakes::fake_ews::Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let mut ids = Vec::new();
    for i in 1..=25 {
        ids.push(d.deliver(&inbox, format!("From: a@x.example\r\nSubject: m{i}\r\n\r\nbody {i}\r\n").as_bytes(), false, "2024-05-01T09:00:00Z"));
    }
    d.unreadable.push(ids[3].clone());
    let server = crate::fakes::fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-ahead-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let gets = || server.data.lock().unwrap().calls.iter().filter(|c| c.as_str() == "GetItem").count();
    let before = gets();
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &SyncOptions { prefetch: 25, ..Default::default() }).unwrap();
    // One GetItem for the details, three for the sources (10, 10 and 5).
    assert_eq!(gets() - before, 4);
    let fs = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!(fs.messages.iter().filter(|m| store.has_body(&m.key)).count(), 24, "all but the one the server won't give");
    // Asked to keep more downloaded: read again though nothing changed.
    let before = gets();
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &SyncOptions { prefetch: 30, ..Default::default() }).unwrap();
    assert!(gets() > before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_message_the_numbering_lost_is_found_again_by_its_message_id() {
    let mut d = crate::fakes::fake_ews::Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(
        &inbox,
        b"From: Ada <ada@example.org>\r\nTo: riley@contoso.example\r\nSubject: Old notes\r\nMessage-ID: <old-1@example.org>\r\nDate: Tue, 01 Feb 2005 09:00:00 +0000\r\n\r\nwe tried deepseek\r\n",
        true,
        "2005-02-01T09:00:00Z",
    );
    let server = crate::fakes::fake_ews::serve(d);
    let dir = std::env::temp_dir().join(format!("quatrelle-ews-renumber-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    list_folders(&c, &mut st, "a1").unwrap();
    let opts = SyncOptions { prefetch: 0, ..Default::default() };
    sync_folder(&c, &mut st, &store, "a1", "INBOX", &opts).unwrap();
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    assert!(store.read_body(&key).is_none(), "nothing kept yet: the click asks the server");
    // A state that lost the numbering but still knows the folders: what an old state file
    // (or one written by another connection) leaves behind. The old message must still open.
    let mut lost = State::default();
    lost.mailboxes = st.mailboxes.clone();
    let raw = fetch_body(&c, &mut lost, &store, &key).unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("deepseek"));
    assert!(lost.id(key.uid).is_some(), "and it is numbered again");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_connection_whose_sign_in_was_forgotten_signs_in_again() {
    let mut d = crate::fakes::fake_ews::Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = crate::fakes::fake_ews::serve(d);
    let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let before = server.data.lock().unwrap().handshakes;
    // The server forgot the session (a connection it recycled): the next request is refused
    // once, and the account must sign in again rather than hear its password was refused.
    server.data.lock().unwrap().forget_sign_in = true;
    let mut st = State::default();
    let folders = list_folders(&c, &mut st, "a1").unwrap();
    assert!(!folders.is_empty());
    assert!(server.data.lock().unwrap().handshakes > before, "signed in again");
}

/// A kept connection the server closed (idle too long) isn't a refused password: the
/// request goes again on a new connection, signed in there. And a connection lost to an
/// error leaves no sign-in behind: the next request signs in on its new connection rather
/// than going unsigned and hearing 401 (which asked for a password that was right).
#[test]
fn a_connection_the_server_closed_signs_in_again_on_a_new_one() {
    let mut d = crate::fakes::fake_ews::Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = crate::fakes::fake_ews::serve(d);
    let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    // Closed by the server between two requests: sent again, unnoticed.
    server.data.lock().unwrap().drop_kept = 1;
    assert!(!list_folders(&c, &mut st, "a1").unwrap().is_empty(), "a closed kept connection is no failure");
    // Closed twice in a row: the second time it's said, as a network failure, not a password.
    server.data.lock().unwrap().drop_kept = 2;
    let e = list_folders(&c, &mut st, "a1").unwrap_err();
    assert!(!matches!(e, NetError::Auth(_)), "a dropped connection isn't a refused password: {e}");
    assert!(e.is_fatal(), "{e}");
    // The next request signs in on its new connection.
    let before = server.data.lock().unwrap().handshakes;
    assert!(!list_folders(&c, &mut st, "a1").unwrap().is_empty(), "the next request works");
    assert!(server.data.lock().unwrap().handshakes > before, "signed in on the new connection");
}

/// Exchange busy (`ErrorServerBusy`) is waited out as it asks, then asked again; a refusal
/// of one request leaves the connection alone (it isn't fatal).
#[test]
fn a_busy_server_is_asked_again_after_its_back_off() {
    let mut d = crate::fakes::fake_ews::Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = crate::fakes::fake_ews::serve(d);
    let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let mut st = State::default();
    server.data.lock().unwrap().busy = 1;
    assert!(!list_folders(&c, &mut st, "a1").unwrap().is_empty(), "asked again once the back-off passed");
    server.data.lock().unwrap().busy = 5;
    let e = list_folders(&c, &mut st, "a1").unwrap_err();
    assert!(matches!(e, NetError::Later(_)), "still busy: for later, {e}");
    assert!(!e.is_fatal(), "the connection is fine: {e}");
    server.data.lock().unwrap().busy = 0;
    assert!(!list_folders(&c, &mut st, "a1").unwrap().is_empty());
}

/// A rule goes to Exchange as its schema has it: the yes-or-no actions saying `true`, each
/// test and action once and in the schema's order. `<t:Delete/>` got every rule refused.
#[test]
fn a_rule_is_written_as_exchange_reads_it() {
    let r = Rule {
        name: "Tri".into(),
        conditions: vec![
            Condition::Subject("facture".into()),
            Condition::From("postmaster@".into()),
            Condition::FromAny(vec!["a@x.example".into(), "postmaster@".into()]),
            Condition::HasAttachment,
        ],
        actions: vec![RuleAction::MarkRead, RuleAction::Delete, RuleAction::Categorize("Work".into()), RuleAction::MoveTo("Archive".into())],
        stop: true,
        enabled: true,
        account: None,
        match_all: true,
        server: true,
        remote_id: None,
        priority: None,
        refused: None,
    };
    let x = inbox_rule_xml(&r, Some("F1"), 3, None);
    for empty in ["<t:Delete/>", "<t:MarkAsRead/>", "<t:StopProcessingRules/>"] {
        assert!(!x.contains(empty), "{x}");
    }
    let order = [
        "<t:DisplayName>",
        "<t:Priority>3</t:Priority>",
        "<t:IsEnabled>",
        "<t:Conditions>",
        "<t:ContainsSenderStrings><t:String>postmaster@</t:String><t:String>a@x.example</t:String></t:ContainsSenderStrings>",
        "<t:ContainsSubjectStrings><t:String>facture</t:String></t:ContainsSubjectStrings>",
        "<t:HasAttachments>true</t:HasAttachments>",
        "<t:Actions>",
        "<t:AssignCategories><t:String>Work</t:String></t:AssignCategories>",
        "<t:Delete>true</t:Delete>",
        "<t:MarkAsRead>true</t:MarkAsRead>",
        "<t:MoveToFolder><t:FolderId Id=\"F1\"/></t:MoveToFolder>",
        "<t:StopProcessingRules>true</t:StopProcessingRules>",
    ];
    let mut at = 0;
    for part in order {
        let found = x.get(at..).and_then(|rest| rest.find(part)).unwrap_or_else(|| panic!("{part} after {at} in {x}"));
        at += found + part.len();
    }
    assert_eq!(x.matches("ContainsSenderStrings>").count(), 2, "one sender test: {x}");
}

#[test]
fn the_directory_picture_is_fetched() {
    let mut d = crate::fakes::fake_ews::Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = crate::fakes::fake_ews::serve(d);
    let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let (media, data) = c.user_photo("zoe@contoso.example").unwrap();
    assert_eq!(media, "image/jpeg");
    assert_eq!(data, vec![0xFF, 0xD8, 0xFF], "the bytes, not the base64 it came as");
    assert!(c.user_photo("ghost@nophoto.example").is_err(), "no picture is an answer, not a broken account");
    // Said with a bare HTTP 404 (a shared mailbox's, on Exchange 2016): a refusal, not a broken
    // connection (that set the account in error at every start); the next call goes through.
    let none = c.user_photo("support@http404.example");
    assert!(matches!(none, Err(NetError::No(_))), "{none:?}");
    assert!(none.is_err_and(|e| !e.is_fatal()));
    assert!(c.user_photo("zoe@contoso.example").is_ok());
}

/// A server rule is described as far as Quatrelle can say it, and the rest is kept as text:
/// a body condition (which Quatrelle's rules don't have) makes it one it can't run here.
#[test]
fn a_server_rule_is_read_as_far_as_quatrelle_can_say_it() {
    use quatrelle_model::organize::{Condition, RuleAction};
    let doc = parse(
        "<t:Rule><t:RuleId>AgAABUdELrU=</t:RuleId><t:DisplayName>GRC:</t:DisplayName><t:Priority>1</t:Priority>\
         <t:IsEnabled>true</t:IsEnabled><t:Conditions><t:ContainsBodyStrings><t:String>DGA</t:String></t:ContainsBodyStrings>\
         <t:ContainsSubjectStrings><t:String>GRC:</t:String></t:ContainsSubjectStrings></t:Conditions>\
         <t:Actions><t:Delete>true</t:Delete><t:StopProcessingRules>true</t:StopProcessingRules></t:Actions></t:Rule>",
    );
    let r = server_rule(&doc.kids[0]);
    assert_eq!((r.id.as_str(), r.name.as_str(), r.enabled), ("AgAABUdELrU=", "GRC:", true));
    assert!(!r.runnable, "a body condition isn't one Quatrelle can run");
    assert_eq!(r.rule, None);
    assert!(r.summary.contains("body \"DGA\"") && r.summary.contains("subject \"GRC:\"") && r.summary.contains("delete"), "{}", r.summary);

    // One Quatrelle can say entirely is kept whole: what it does, and on/off.
    let doc = parse(
        "<t:Rule><t:RuleId Id=\"R7\"/><t:DisplayName>Invoices</t:DisplayName><t:IsEnabled>false</t:IsEnabled>\
         <t:Conditions><t:ContainsSubjectStrings><t:String>Invoice</t:String></t:ContainsSubjectStrings></t:Conditions>\
         <t:Actions><t:Delete>true</t:Delete><t:StopProcessingRules>true</t:StopProcessingRules></t:Actions></t:Rule>",
    );
    let r = server_rule(&doc.kids[0]);
    assert!(r.runnable && !r.enabled);
    let rule = r.rule.unwrap();
    assert_eq!(rule.conditions, vec![Condition::Subject("Invoice".into())]);
    assert_eq!(rule.actions, vec![RuleAction::Delete]);
    assert_eq!((rule.stop, rule.server, rule.remote_id.as_deref()), (true, true, Some("R7")));
}

/// A rule that names several people (Outlook's "from these people") is read as one
/// Quatrelle can run and change here, and written back the same way.
#[test]
fn a_rule_from_several_people_can_be_changed_here() {
    use quatrelle_model::organize::{Condition, RuleAction};
    let doc = parse(
        "<t:Rule><t:RuleId Id=\"R5\"/><t:DisplayName>Alertes</t:DisplayName><t:Priority>3</t:Priority><t:IsEnabled>true</t:IsEnabled>\
         <t:Conditions><t:FromAddresses>\
         <t:Address><t:Name>Nagios</t:Name><t:EmailAddress>nagios@asp.example</t:EmailAddress></t:Address>\
         <t:Address><t:Name>Zabbix</t:Name><t:EmailAddress>zabbix@asp.example</t:EmailAddress></t:Address>\
         </t:FromAddresses></t:Conditions>\
         <t:Actions><t:Delete>true</t:Delete></t:Actions></t:Rule>",
    );
    let r = server_rule(&doc.kids[0]);
    assert!(r.runnable, "{}", r.summary);
    let rule = r.rule.unwrap();
    assert_eq!(rule.conditions, vec![Condition::FromAny(vec!["nagios@asp.example".into(), "zabbix@asp.example".into()])]);
    assert_eq!(rule.actions, vec![RuleAction::Delete]);
    let xml = inbox_rule_xml(&rule, None, 1, Some("R5"));
    assert!(
        xml.contains(
            "<t:ContainsSenderStrings><t:String>nagios@asp.example</t:String><t:String>zabbix@asp.example</t:String></t:ContainsSenderStrings>"
        ),
        "{xml}"
    );
    assert!(xml.contains("<t:Priority>3</t:Priority>"), "{xml}");
    // Several subjects too.
    let doc = parse(
        "<t:Rule><t:RuleId Id=\"R6\"/><t:DisplayName>Rien</t:DisplayName><t:IsEnabled>true</t:IsEnabled>\
         <t:Conditions><t:ContainsSubjectStrings><t:String>spam</t:String><t:String>virus</t:String></t:ContainsSubjectStrings></t:Conditions>\
         <t:Actions><t:Delete>true</t:Delete></t:Actions></t:Rule>",
    );
    let r = server_rule(&doc.kids[0]);
    assert_eq!(r.rule.as_ref().map(|x| x.conditions.clone()), Some(vec![Condition::SubjectAny(vec!["spam".into(), "virus".into()])]));
    assert!(r.runnable);
}

/// comes as the id the server knows, and the rule keeps its place among the server's rules.
#[test]
fn a_rule_that_moves_mail_can_be_changed_here() {
    use quatrelle_model::organize::{Condition, RuleAction};
    let doc = parse(
        "<t:Rule><t:RuleId Id=\"R3\"/><t:DisplayName>Clients</t:DisplayName><t:Priority>7</t:Priority><t:IsEnabled>true</t:IsEnabled>\
         <t:Conditions><t:ContainsSenderStrings><t:String>@client.example</t:String></t:ContainsSenderStrings></t:Conditions>\
         <t:Actions><t:MoveToFolder><t:FolderId Id=\"AAMkAD-folder\"/></t:MoveToFolder><t:MarkAsRead>true</t:MarkAsRead></t:Actions></t:Rule>",
    );
    let r = server_rule(&doc.kids[0]);
    assert!(r.runnable, "a sender test and a move are both known");
    let rule = r.rule.unwrap();
    assert_eq!(rule.conditions, vec![Condition::From("@client.example".into())]);
    assert_eq!(rule.actions, vec![RuleAction::MoveTo("id:AAMkAD-folder".into()), RuleAction::MarkRead]);
    assert_eq!(rule.priority, Some(7), "its place among the server's rules");
    // Changed here, it goes back with that place, so the rules around it keep theirs.
    let xml = inbox_rule_xml(&rule, Some("AAMkAD-folder"), 1, Some("R3"));
    assert!(xml.contains("<t:Priority>7</t:Priority>"), "{xml}");
    assert!(xml.contains("<t:RuleId Id=\"R3\">R3</t:RuleId>"), "{xml}");
    assert!(xml.contains("<t:MoveToFolder><t:FolderId Id=\"AAMkAD-folder\"/></t:MoveToFolder>"), "{xml}");
    // A new one goes where the caller puts it.
    let mut fresh = rule.clone();
    fresh.priority = None;
    assert!(inbox_rule_xml(&fresh, None, 4, None).contains("<t:Priority>4</t:Priority>"));
}

/// A server's own rule is turned off and removed where it lives, everything else in it
/// going back as the server keeps it.
#[test]
fn a_server_rule_is_changed_where_it_lives() {
    let mut d = crate::fakes::fake_ews::Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.rules.push(("R-OUTLOOK".into(), "Outlook rule".into()));
    let server = crate::fakes::fake_ews::serve(d);
    let user = format!("{}{}{}", crate::fakes::fake_ews::DOMAIN, '\\', crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    c.set_rule_enabled("R-OUTLOOK", false).unwrap();
    assert_eq!(c.inbox_rule_ids().unwrap(), vec![("Outlook rule".to_string(), "R-OUTLOOK".to_string())], "still there, under its name");
    let calls = server.data.lock().unwrap().calls.clone();
    assert!(calls.iter().any(|c| c == "UpdateInboxRules"), "{calls:?}");
    c.delete_rule("R-OUTLOOK").unwrap();
    assert!(c.inbox_rule_ids().unwrap().is_empty(), "removed");
}

#[test]
fn inbox_rules_are_read_in_either_shape() {
    // As Exchange 2016 answers: the response element itself, the ids as the element's text.
    let direct = parse(
        "<s:Envelope><s:Body><GetInboxRulesResponse ResponseClass=\"Success\"><ResponseCode>NoError</ResponseCode><InboxRules>\
         <Rule><RuleId>AgAABUdELrU=</RuleId><DisplayName>GRC:</DisplayName><Priority>1</Priority><IsEnabled>true</IsEnabled></Rule>\
         <Rule><RuleId Id=\"R2\"/><DisplayName>Invoices</DisplayName><Priority>2</Priority><IsEnabled>true</IsEnabled></Rule>\
         </InboxRules></GetInboxRulesResponse></s:Body></s:Envelope>",
    );
    let m = operation(&direct, "GetInboxRulesResponse").unwrap();
    let rules: Vec<(String, String)> = m.find("InboxRules").unwrap().children("Rule").map(server_rule).map(|r| (r.name, r.id)).collect();
    assert_eq!(
        rules,
        vec![("GRC:".to_string(), "AgAABUdELrU=".to_string()), ("Invoices".to_string(), "R2".to_string())],
        "the id read from the text and from the attribute"
    );
    let wrapped = parse(
        "<s:Envelope><s:Body><m:GetInboxRulesResponse><m:ResponseMessages><m:GetInboxRulesResponseMessage ResponseClass=\"Success\">\
         <m:ResponseCode>NoError</m:ResponseCode></m:GetInboxRulesResponseMessage></m:ResponseMessages></m:GetInboxRulesResponse></s:Body></s:Envelope>",
    );
    assert!(operation(&wrapped, "GetInboxRulesResponse").is_ok());
    let none = parse("<s:Envelope><s:Body></s:Body></s:Envelope>");
    assert!(matches!(operation(&none, "GetInboxRulesResponse"), Err(NetError::Protocol(_))));
}

#[test]
fn a_picture_answer_is_read_in_either_shape() {
    // As Exchange 2016 answers: the response element itself, no ResponseMessages list.
    let direct = parse(
        "<s:Envelope><s:Body><GetUserPhotoResponse ResponseClass=\"Success\">\
         <ResponseCode>NoError</ResponseCode><HasChanged>true</HasChanged><PictureData>/9j/</PictureData>\
         </GetUserPhotoResponse></s:Body></s:Envelope>",
    );
    assert_eq!(photo_of(&direct).unwrap(), ("image/jpeg".to_string(), vec![0xFF, 0xD8, 0xFF]));
    // As a server that wraps it does.
    let wrapped = parse(
        "<s:Envelope><s:Body><m:GetUserPhotoResponse><m:ResponseMessages><m:GetUserPhotoResponseMessage ResponseClass=\"Success\">\
         <m:ResponseCode>NoError</m:ResponseCode><m:PictureData>/9j/</m:PictureData>\
         </m:GetUserPhotoResponseMessage></m:ResponseMessages></m:GetUserPhotoResponse></s:Body></s:Envelope>",
    );
    assert_eq!(photo_of(&wrapped).unwrap().0, "image/jpeg");
    // No picture is an answer; an error is said as such.
    let none = parse(
        "<s:Envelope><s:Body><GetUserPhotoResponse ResponseClass=\"Success\"><PictureData></PictureData></GetUserPhotoResponse></s:Body></s:Envelope>",
    );
    assert!(matches!(photo_of(&none), Err(NetError::No(_))));
    let gone = parse(
        "<s:Envelope><s:Body><GetUserPhotoResponse ResponseClass=\"Error\"><ResponseCode>ErrorItemNotFound</ResponseCode></GetUserPhotoResponse></s:Body></s:Envelope>",
    );
    assert!(matches!(photo_of(&gone), Err(NetError::No(_))));
}

#[test]
fn a_contacts_picture_comes_with_the_contact() {
    let mut d = crate::fakes::fake_ews::Data::default();
    let contacts = d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    d.pim.push((
        "P1".into(),
        contacts,
        parse(
            "<t:Contact><t:GivenName>Ana</t:GivenName><t:Attachments>\
             <t:FileAttachment IsContactPhoto=\"true\"><t:AttachmentId Id=\"A1\"/><t:Name>ContactPicture.jpg</t:Name></t:FileAttachment>\
             </t:Attachments></t:Contact>",
        )
        .kids[0]
            .clone(),
    ));
    d.pim.push(("P2".into(), "contacts".into(), parse("<t:Contact><t:GivenName>Ben</t:GivenName></t:Contact>").kids[0].clone()));
    let server = crate::fakes::fake_ews::serve(d);
    let user = format!("{}{}{}", crate::fakes::fake_ews::DOMAIN, '\\', crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let (media, data) = c.contact_photo("P1").unwrap();
    assert_eq!(media, "image/jpeg");
    assert_eq!(data, vec![0xFF, 0xD8, 0xFF], "the bytes, not the base64 it came as");
    assert!(matches!(c.contact_photo("P2"), Err(NetError::No(_))), "a contact with no picture is an answer, not a broken account");
    assert!(matches!(c.contact_photo("ghost"), Err(NetError::No(_) | NetError::Protocol(_))), "an unknown contact is no picture either");
}

#[test]
fn autodiscover_answers() {
    let ok = "<Autodiscover><Response><Account><AccountType>email</AccountType><Action>settings</Action>\
                  <Protocol><Type>EXCH</Type><EwsUrl>https://mail.corp.local/EWS/Exchange.asmx</EwsUrl></Protocol>\
                  <Protocol><Type>EXPR</Type><EwsUrl>https://mail.contoso.example/EWS/Exchange.asmx</EwsUrl></Protocol></Account></Response></Autodiscover>";
    assert_eq!(autodiscover_answer(ok).0.as_deref(), Some("https://mail.contoso.example/EWS/Exchange.asmx"), "the outside address first");
    let redirect = "<Autodiscover><Response><Account><Action>redirectAddr</Action><RedirectAddr>riley@corp.example</RedirectAddr></Account></Response></Autodiscover>";
    assert_eq!(autodiscover_answer(redirect).1.as_deref(), Some("riley@corp.example"));
    assert_eq!(autodiscover_answer("<x/>"), (None, None));
    let plain = "<Autodiscover><Response><Account><Protocol><Type>EXPR</Type><EwsUrl>http://mail.contoso.example/EWS</EwsUrl></Protocol></Account></Response></Autodiscover>";
    assert_eq!(autodiscover_answer(plain).0, None, "never plain http");
    assert_eq!(autodiscover_urls("riley@Contoso.example")[0], "https://autodiscover.contoso.example/autodiscover/autodiscover.xml");
    assert!(autodiscover_urls("riley@bad/host").is_empty());
    // A public folder's content mailbox: PR_REPLICA_LIST as Exchange sends it.
    assert_eq!(replica_guid("MWVjMmEyMzYtZWQ5My00Zjg4LWI5YzYtMzNlNjNmYTRhYTQ0AA==").as_deref(), Some("1ec2a236-ed93-4f88-b9c6-33e63fa4aa44"));
    assert_eq!(replica_guid("bm90IGEgZ3VpZA=="), None);
    assert_eq!(replica_guid("%%%"), None);
    let pf =
        "<Autodiscover><Response><User><AutoDiscoverSMTPAddress>PFContent@contoso.example</AutoDiscoverSMTPAddress></User></Response></Autodiscover>";
    assert_eq!(autodiscover_smtp(pf).as_deref(), Some("PFContent@contoso.example"));
    assert_eq!(autodiscover_smtp("<x/>"), None);
}

#[test]
fn only_https_unless_local() {
    assert!(Client::new("http://mail.contoso.example/EWS/Exchange.asmx", "u", "p", "e").is_err());
    assert!(Client::new("https://mail.contoso.example/EWS/Exchange.asmx", "u", "p", "e").is_ok());
    assert!(Client::new("http://127.0.0.1:9/EWS/Exchange.asmx", "u", "p", "e").is_ok());
}

#[test]
fn outlooks_copy_of_the_rules_is_replaced_only_when_asked() {
    use quatrelle_model::organize::{Condition, Rule, RuleAction};
    let mut d = crate::fakes::fake_ews::Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.outlook_rules = true;
    let server = crate::fakes::fake_ews::serve(d);
    let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
    let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
    let rule = Rule {
        name: "Invoices".into(),
        enabled: true,
        conditions: vec![Condition::Subject("invoice".into())],
        match_all: true,
        actions: vec![RuleAction::MarkRead],
        stop: false,
        server: true,
        account: Some("a1".into()),
        remote_id: None,
        priority: None,
        refused: None,
    };
    let refused = c.set_inbox_rules(&[(rule.clone(), None)], &[]).unwrap_err().to_string();
    assert!(refused.contains("Outlook keeps its own copy"), "{refused}");
    c.replace_outlook_rules();
    let ids = c.set_inbox_rules(&[(rule.clone(), None)], &[]).unwrap();
    assert_eq!(ids.len(), 1, "Outlook's copy removed, the rule made");
    // Once: the next change goes as usual (and the copy is gone, so it passes).
    assert!(!c.update_request("").contains("RemoveOutlookRuleBlob"));
}
