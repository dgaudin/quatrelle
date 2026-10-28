//! Engine tests: downloading a whole mailbox, other mailboxes opened in an account, and the
//! envelope that stands for new mail.

use super::*;

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

/// Exchange's public folders and shared mailboxes as Outlook's Cached Exchange Mode keeps
/// them. A public folder opened is read from the server then; its Favorites are kept in sync
/// only when the account keeps them ("Download Public Folder Favorites", off unless chosen),
/// and the sync is told which (one followed syncs at once). Favorites never set say nothing:
/// the list kept stays. A shared mailbox's folders are kept unless the account doesn't keep
/// them ("Download shared folders"), then read on the server when opened.
#[test]
fn public_folders_are_read_when_opened_unless_favorites() {
    use quatrelle_model::{AccountConfig, FOLLOWED_PUBLIC_DOC, Folder};
    let store = temp("public-favorites");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let config = AccountConfig {
        name: "Riley".into(),
        email: "riley@contoso.example".into(),
        username: "riley".into(),
        ews: Some("https://mail.contoso.example/EWS/Exchange.asmx".into()),
        public_favorites_kept: true,
        ..Default::default()
    };
    let id = s.add_account(config, Some("pw".into())).unwrap();
    // What an earlier session told the sync to follow.
    store.save_account_doc(&id, FOLLOWED_PUBLIC_DOC, &vec!["Public Folders/Reports"]).unwrap();
    let f = |path: &str, role: FolderRole| Folder {
        account: id.clone(),
        path: path.into(),
        delimiter: Some('/'),
        role,
        selectable: true,
        ..Default::default()
    };
    let shared = Folder { mailbox: Some("support@contoso.example".into()), ..f("support@contoso.example/Inbox", FolderRole::Other) };
    let folders = vec![
        f("INBOX", FolderRole::Inbox),
        f("Public Folders", FolderRole::Other),
        f("Public Folders/Team", FolderRole::Other),
        f("Public Folders/Reports", FolderRole::Other),
        shared,
    ];
    s.apply(Event::Folders { account: id.clone(), folders });
    s.take_jobs();
    let followed = || store.load_account_doc::<Vec<String>>(&id, FOLLOWED_PUBLIC_DOC).ok();
    assert_eq!(followed(), Some(vec!["Public Folders/Reports".to_string()]), "Favorites never set (not read yet) say nothing");
    let synced = |jobs: Vec<Job>| -> Vec<String> {
        jobs.into_iter()
            .filter_map(|j| match j {
                Job::SyncFolder { folder, .. } => Some(folder),
                _ => None,
            })
            .collect()
    };
    // Opened: read from the server then. The user's own folders and shared mailboxes aren't.
    s.run("folder.open", &json!({"account": id, "folder": "Public Folders/Team"})).unwrap();
    assert_eq!(synced(s.take_jobs()), ["Public Folders/Team"]);
    for kept in ["INBOX", "support@contoso.example/Inbox"] {
        s.run("folder.open", &json!({"account": id, "folder": kept})).unwrap();
        assert!(synced(s.take_jobs()).is_empty(), "{kept} is kept here");
    }
    // A Favorite: kept in sync by the account's sync, at once; opened, nothing more is asked.
    s.run("folder.favorite", &json!({"account": id, "folder": "Public Folders/Team", "on": true})).unwrap();
    assert_eq!(followed(), Some(vec!["Public Folders/Team".to_string()]));
    assert_eq!(synced(s.take_jobs()), ["Public Folders/Team"], "synced as soon as it's followed");
    s.run("folder.open", &json!({"account": id, "folder": "Public Folders/Team"})).unwrap();
    assert!(synced(s.take_jobs()).is_empty(), "a followed folder is in sync already");
    // Public Favorites no longer kept (Outlook's default): none followed, each read when opened.
    s.run("account.update", &json!({"account": id, "publicFavoritesKept": false})).unwrap();
    assert_eq!(followed(), Some(Vec::new()));
    s.take_jobs();
    s.run("folder.open", &json!({"account": id, "folder": "Public Folders/Team"})).unwrap();
    assert_eq!(synced(s.take_jobs()), ["Public Folders/Team"], "a Favorite not kept is read when opened");
    // Shared mailboxes not kept: read on the server when opened, and the sync starts again.
    s.run("account.update", &json!({"account": id, "sharedOnline": true})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Start { account, .. } if account.shared_online)));
    s.run("folder.open", &json!({"account": id, "folder": "support@contoso.example/Inbox"})).unwrap();
    assert_eq!(synced(s.take_jobs()), ["support@contoso.example/Inbox"]);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Exchange mail older than what's kept, as Outlook's Cached Exchange Mode shows it: Show more
/// reads it on the server a page at a time, shown after the folder's own list and held in
/// memory only; one opened is read on the server, not kept; another folder opened lets it go.
#[test]
fn older_mail_is_read_on_the_server_not_kept() {
    use quatrelle_model::{AccountConfig, Folder, MessageKey, Summary};
    let store = temp("older-online");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let config = AccountConfig {
        name: "Riley".into(),
        email: "riley@contoso.example".into(),
        username: "riley".into(),
        ews: Some("https://mail.contoso.example/EWS/Exchange.asmx".into()),
        ..Default::default()
    };
    let id = s.add_account(config, Some("pw".into())).unwrap();
    let msg = |uid: u32, subject: &str, days_ago: i64| Summary {
        key: MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 1, uid },
        subject: subject.into(),
        date: NOW - days_ago * 86_400,
        ..Default::default()
    };
    store
        .save_folder(&id, "INBOX", &quatrelle_store::FolderState { validity: 1, messages: vec![msg(1, "This week", 3)], ..Default::default() })
        .unwrap();
    let f = |path: &str, role: FolderRole, total: u32| Folder {
        account: id.clone(),
        path: path.into(),
        delimiter: Some('/'),
        role,
        selectable: true,
        total,
        ..Default::default()
    };
    s.apply(Event::Folders { account: id.clone(), folders: vec![f("INBOX", FolderRole::Inbox, 3), f("Archive", FolderRole::Archive, 0)] });
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    s.take_jobs();
    assert_eq!(s.list.len(), 1, "{:?}", s.list.iter().map(|m| &m.subject).collect::<Vec<_>>());
    assert_eq!(s.more_on_server().map(|m| m.2), Some(2));
    let asked = |s: &mut Session| {
        s.take_jobs().into_iter().find_map(|j| match j {
            Job::Older { offset, .. } => Some(offset),
            _ => None,
        })
    };
    s.run("folder.more", &json!({})).unwrap();
    assert_eq!(asked(&mut s), Some(0), "read on the server, not kept by more of the folder");
    s.apply(Event::Older { account: id.clone(), folder: "INBOX".into(), items: vec![msg(7, "Three years ago", 1100)], more: true, error: None });
    assert_eq!(s.list.len(), 2);
    assert_eq!(s.more_on_server().map(|m| m.2), Some(1));
    s.run("folder.more", &json!({})).unwrap();
    assert_eq!(asked(&mut s), Some(1), "the next page");
    s.apply(Event::Older { account: id.clone(), folder: "INBOX".into(), items: vec![msg(8, "Four years ago", 1500)], more: false, error: None });
    assert_eq!(s.list.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["This week", "Three years ago", "Four years ago"]);
    assert_eq!(s.more_on_server(), None, "the server has no more");
    // Opened: read on the server, not kept.
    let old = msg(7, "", 0).key;
    s.open_message(&old);
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::ReadOnline { key } if *key == old)));
    s.apply(Event::Body { key: old.clone(), raw: b"Subject: Three years ago\r\n\r\nHello.\r\n".to_vec() });
    assert!(s.reading.as_ref().is_some_and(|r| r.parsed.is_some()), "shown");
    assert!(!store.has_body(&old), "not kept");
    // Another folder: let go.
    s.run("folder.open", &json!({"account": id, "folder": "Archive"})).unwrap();
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    assert_eq!(s.list.len(), 1);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Mail downloaded ahead comes into the list without a preview (a big folder's list isn't
/// written again for it): the rows drawn get theirs from their sources, a few at a time, and a
/// row whose source isn't here yet is looked at again a minute later.
#[test]
fn rows_downloaded_ahead_get_their_preview_from_the_source() {
    let store = temp("previews");
    store.save_accounts(&[quatrelle_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    let inbox =
        quatrelle_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[inbox]).unwrap();
    let key = |uid| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    let mut st = quatrelle_store::FolderState { validity: 1, uid_next: 12, ..Default::default() };
    for uid in 1..=11u32 {
        let preview = if uid == 11 { "From its listing".to_string() } else { String::new() };
        st.messages.push(Summary { key: key(uid), subject: format!("Report {uid}"), date: NOW - i64::from(uid), preview, ..Default::default() });
        if uid <= 8 {
            let raw = format!("From: a@x.example\r\nSubject: Report {uid}\r\n\r\nThe figures for week {uid}.\r\n\r\n-- \r\nA signature\r\n");
            store.write_body(&key(uid), raw.as_bytes()).unwrap();
        }
    }
    store.save_folder("a1", "INBOX", &st).unwrap();
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    s.run("folder.open", &json!({"account": "a1", "folder": "INBOX"})).unwrap();
    let wanted = |s: &Session| s.list.iter().filter(|m| s.preview_wanted(m)).map(|m| m.key.clone()).collect::<Vec<_>>();
    let rows = wanted(&s);
    assert_eq!(rows.len(), 10, "the one listed with a preview keeps it");
    assert!(s.fill_previews(&rows), "a few at a time: some are left");
    assert!(!s.fill_previews(&wanted(&s)), "the rest in the next go");
    assert!(wanted(&s).is_empty(), "the two without a source wait");
    let preview = |s: &Session, uid| s.list.iter().find(|m| m.key == key(uid)).map(|m| s.preview_of(m).to_string()).unwrap_or_default();
    assert_eq!(preview(&s, 3), "The figures for week 3.", "the signature isn't part of it");
    assert_eq!(preview(&s, 9), "");
    assert_eq!(preview(&s, 11), "From its listing");
    // Its source downloaded meanwhile: a minute later the row is looked at again.
    store.write_body(&key(9), b"From: a@x.example\r\nSubject: Report 9\r\n\r\nLate figures.\r\n").unwrap();
    s.set_clock(NOW + 61, 0);
    assert_eq!(wanted(&s), vec![key(9), key(10)]);
    s.fill_previews(&wanted(&s));
    assert_eq!(preview(&s, 9), "Late figures.");
    done(s);
}

/// Outlook's Folder Size: each side is asked for where it can be answered (Exchange tells each
/// folder's size, an IMAP server doesn't), and each answer fills its own side; one for an
/// account no longer looked at is dropped.
#[test]
fn folder_size_asks_each_side_and_fills_it_as_it_answers() {
    use crate::Sizes;
    use quatrelle_model::{FolderSize, MailboxQuota};
    let mut s = Session::open(Some(temp("folder-sizes")), quatrelle_model::dates::Clock::new(NOW, 0));
    let ews = s
        .run("account.add", &json!({"email": "riley@contoso.example", "ews": "mail.contoso.example", "username": "CONTOSO\\riley", "password": "pw"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let imap = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();

    let out = s.run("account.folderSizes", &json!({"account": ews})).unwrap();
    assert_eq!((&out["local"]["loading"], &out["server"]["loading"]), (&json!(true), &json!(true)));
    let jobs = s.take_jobs();
    assert!(
        jobs.contains(&Job::LocalFolderSizes { account: ews.clone() }) && jobs.contains(&Job::ServerFolderSizes { account: ews.clone() }),
        "{jobs:?}"
    );
    let inbox = FolderSize { path: vec!["Boîte de réception".into()], size: 4 << 20, items: 12, folder: None };
    let quota = MailboxQuota { warning: 1 << 30, send: 3 << 29, receive: 2 << 30 };
    s.apply(Event::FolderSizes { account: ews.clone(), server: true, folders: vec![inbox.clone()], quota: Some(quota), error: None });
    let v = s.sizes.clone().unwrap();
    assert_eq!((v.server, v.local, v.quota), (Sizes::Loaded(vec![inbox]), Sizes::Loading, Some(quota)));
    s.apply(Event::FolderSizes { account: ews.clone(), server: false, folders: Vec::new(), quota: None, error: Some("damaged".into()) });
    assert_eq!(s.sizes.as_ref().unwrap().local, Sizes::Failed("damaged".into()));

    // An IMAP account: its server isn't asked, and Exchange's late answer is dropped.
    let out = s.run("account.folderSizes", &json!({"account": imap})).unwrap();
    assert!(out["server"]["error"].is_string());
    assert_eq!(s.take_jobs(), vec![Job::LocalFolderSizes { account: imap.clone() }]);
    s.apply(Event::FolderSizes { account: ews, server: true, folders: Vec::new(), quota: None, error: None });
    let v = s.sizes.clone().unwrap();
    assert_eq!((v.account, v.server, v.quota), (imap, Sizes::NotTold, None));
    assert!(s.run("account.folderSizes", &json!({"account": "nobody"})).is_err());
}

/// Outlook's envelope in the notification area: it stands for mail that arrived in an Inbox and
/// is still unread, goes when that mail is read (here or elsewhere), and clicking it opens the
/// Inbox. Mail already there at the first sync, or read on arrival, doesn't bring it.
#[test]
fn the_envelope_shows_while_new_mail_is_unread() {
    let store = temp("envelope");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let inbox =
        quatrelle_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    let m = |uid: u32, seen: bool| quatrelle_model::Summary {
        key: quatrelle_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid },
        subject: format!("m{uid}"),
        date: NOW - 60,
        flags: quatrelle_model::Flags { seen, ..Default::default() },
        ..Default::default()
    };
    let synced = |s: &mut Session, msgs: Vec<quatrelle_model::Summary>| {
        store
            .save_folder(&id, "INBOX", &quatrelle_store::FolderState { validity: 9, uid_next: 99, highest_modseq: 0, messages: msgs, window: 0 })
            .unwrap();
        s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    };
    synced(&mut s, vec![m(2, false), m(1, false)]);
    assert_eq!(s.new_unread(), 0, "the first sync's backlog isn't new");
    synced(&mut s, vec![m(4, false), m(3, true), m(2, false), m(1, false)]);
    assert_eq!(s.new_unread(), 1);
    // Read in Outlook: the next sync says so.
    synced(&mut s, vec![m(4, true), m(3, true), m(2, false), m(1, false)]);
    assert_eq!(s.new_unread(), 0);

    // Clicking it opens that Inbox, from wherever Quatrelle was.
    synced(&mut s, vec![m(5, false), m(4, true), m(3, true), m(2, false), m(1, false)]);
    assert_eq!(s.new_unread(), 1);
    s.run("view.calendar", &json!({})).unwrap();
    s.run("folder.outbox", &json!({})).unwrap();
    s.run("mail.showNew", &json!({})).unwrap();
    assert_eq!(s.module, crate::Module::Mail);
    assert_eq!(s.view, View::Folder { account: id.clone(), folder: "INBOX".into() });
    // Read here: gone.
    let key = quatrelle_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid: 5 };
    s.run("message.read", &json!({"key": key.to_id()})).unwrap();
    s.run("mail.markRead", &json!({})).unwrap();
    assert_eq!(s.new_unread(), 0);

    // Switched off: nothing is followed.
    s.run("view.envelope", &json!({"on": false})).unwrap();
    synced(&mut s, vec![m(6, false), m(5, true), m(4, true)]);
    assert_eq!(s.new_unread(), 0);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Quatrelle closed and opened again with the new mail still unread: the envelope shows again;
/// read elsewhere meanwhile, it doesn't.
#[test]
fn the_envelope_outlives_a_restart() {
    let store = temp("envelope-restart");
    let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let inbox =
        quatrelle_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    let m = |uid: u32, seen: bool| quatrelle_model::Summary {
        key: quatrelle_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid },
        subject: format!("m{uid}"),
        date: NOW - 60,
        flags: quatrelle_model::Flags { seen, ..Default::default() },
        ..Default::default()
    };
    let synced = |s: &mut Session, msgs: Vec<quatrelle_model::Summary>| {
        store
            .save_folder(&id, "INBOX", &quatrelle_store::FolderState { validity: 9, uid_next: 99, highest_modseq: 0, messages: msgs, window: 0 })
            .unwrap();
        s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    };
    synced(&mut s, vec![m(1, false)]);
    synced(&mut s, vec![m(3, false), m(2, false), m(1, false)]);
    assert_eq!(s.new_unread(), 2);
    let kept = s.envelope_keys().to_vec();
    drop(s);

    let reopen = |kept: &[quatrelle_model::MessageKey]| {
        let mut s = Session::open(Some(store.clone()), quatrelle_model::dates::Clock::new(NOW, 0));
        s.restore_envelope(kept.to_vec());
        s
    };
    let mut s = reopen(&kept);
    assert_eq!(s.new_unread(), 2, "still unread, still new");
    // One read in Outlook while Quatrelle was closed.
    drop(s);
    store
        .save_folder(
            &id,
            "INBOX",
            &quatrelle_store::FolderState {
                validity: 9,
                uid_next: 99,
                highest_modseq: 0,
                messages: vec![m(3, true), m(2, false), m(1, false)],
                window: 0,
            },
        )
        .unwrap();
    let mut s = reopen(&kept);
    assert_eq!(s.new_unread(), 1);
    // Switched off: nothing comes back.
    s.run("view.envelope", &json!({"on": false})).unwrap();
    s.restore_envelope(kept);
    assert_eq!(s.new_unread(), 0);
    let _ = std::fs::remove_dir_all(store.root());
}
