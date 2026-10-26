//! Engine tests: a reply in the reading pane set aside for another message, and the copies kept
//! in Drafts.

use super::*;

fn drafts(s: &mut Session) -> Vec<String> {
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    let subjects = s.list.iter().map(|m| m.subject.clone()).collect();
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    subjects
}

/// As Outlook's reply in the reading pane: reading another message sets it aside — kept in
/// Drafts — and that message shows; back on the first, the reply comes back as it was left.
/// Discard takes its copy out of Drafts too.
#[test]
fn a_reply_in_the_reading_pane_is_set_aside_for_another_message() {
    let mut s = sample_session("set-aside");
    let poster = key_of(&s, "Your proof for the festival poster is ready");
    let mix = key_of(&s, "Sound mix, version 3");
    s.run("list.open", &poster).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    s.run("compose.update", &json!({"body": "Approved!\n"})).unwrap();
    // The message replied to, read again: the reply stays.
    s.run("list.open", &poster).unwrap();
    assert!(s.compose.is_some());
    s.run("list.open", &mix).unwrap();
    assert!(s.compose.is_none(), "the pane shows the message chosen");
    assert_eq!(s.reading.as_ref().map(|r| r.key.to_id()).as_deref(), mix["key"].as_str());
    assert_eq!(s.set_aside.len(), 1);
    assert!(s.notices.last().is_some_and(|n| n.text == "Draft saved"));
    assert!(drafts(&mut s).contains(&"Re: Your proof for the festival poster is ready".to_string()), "kept in Drafts");
    s.run("list.open", &poster).unwrap();
    let c = s.compose.clone().expect("the reply comes back");
    assert!(c.body.starts_with("Approved!") && !c.window, "{:?}", c.body);
    assert!(s.set_aside.is_empty());
    // Another folder sets it aside too.
    s.run("folder.open", &json!({"account": "sample", "folder": "Archive"})).unwrap();
    assert!(s.compose.is_none() && s.set_aside.len() == 1);
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &poster).unwrap();
    assert!(s.compose.is_some());
    s.run("compose.discard", &json!({})).unwrap();
    assert!(s.compose.is_none());
    assert!(!drafts(&mut s).iter().any(|d| d.starts_with("Re: Your proof")), "discarded with its copy in Drafts");
    // A reply left as it came simply goes.
    s.run("list.open", &poster).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    s.run("list.open", &mix).unwrap();
    assert!(s.compose.is_none() && s.set_aside.is_empty());
    assert!(drafts(&mut s).is_empty(), "nothing kept");
    done(s);
}

/// A draft from Drafts is carried on (Quatrelle closed meanwhile, or the reply's message gone):
/// sent, its copy goes.
#[test]
fn a_draft_from_drafts_is_carried_on_and_goes_once_sent() {
    let mut s = sample_session("edit-draft");
    let poster = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &poster).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    s.run("compose.update", &json!({"body": "Approved, *thanks*!\n"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.compose = None;
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    let draft = key_of(&s, "Re: Your proof for the festival poster is ready");
    s.run("draft.edit", &draft).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.to, "Print Shop <orders@printshop.example>; ");
    assert!(c.body.starts_with("Approved,") && !c.saved_id.is_empty());
    s.run("compose.send", &json!({})).unwrap();
    assert!(s.compose.is_none());
    assert!(drafts(&mut s).is_empty(), "sent: the draft goes");
    done(s);
}

/// A server account's copy is numbered by the server once it's listed: found in Drafts by its
/// `Message-ID`, it's replaced at the next save and goes when the message is discarded (it used
/// to stay there for good).
#[test]
fn a_server_accounts_draft_is_found_by_its_message_id() {
    let store = temp("server-draft");
    let mut s = Session::open(Some(store), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let f = |p: &str, role| quatrelle_model::Folder { account: id.clone(), path: p.into(), role, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![f("INBOX", FolderRole::Inbox), f("Drafts", FolderRole::Drafts)] });
    s.take_jobs();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@soundlab.example", "subject": "Plans"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::SaveDraft { replaces: None, .. }]));
    let mid = s.compose.as_ref().unwrap().saved_id.clone();
    assert!(!mid.is_empty());
    // The server lists the copy.
    let listed = |s: &mut Session, uid: u32| {
        let key = MessageKey { account: id.clone(), folder: "Drafts".into(), validity: 1, uid };
        let m = Summary { key: key.clone(), message_id: mid.clone(), subject: "Plans".into(), ..Default::default() };
        let st = quatrelle_store::FolderState { validity: 1, uid_next: uid + 1, messages: vec![m], ..Default::default() };
        s.store.as_ref().unwrap().save_folder(&id, "Drafts", &st).unwrap();
        s.apply(Event::FolderChanged { account: id.clone(), folder: "Drafts".into() });
        key
    };
    let first = listed(&mut s, 7);
    s.run("compose.saveDraft", &json!({})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::SaveDraft { replaces: Some(k), .. }] if *k == first), "saved again in its place");
    assert_eq!(s.compose.as_ref().unwrap().saved_id, mid, "under the same Message-ID");
    let second = listed(&mut s, 8);
    s.run("compose.discard", &json!({})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::Apply { op: Op::Expunge { keys }, .. }] if *keys == vec![second]), "discarded: its copy goes");
    let _ = std::fs::remove_dir_all(s.store.as_ref().unwrap().root());
}
