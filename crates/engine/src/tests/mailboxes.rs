//! Engine tests: downloading a whole mailbox, and other mailboxes opened in an account.

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
