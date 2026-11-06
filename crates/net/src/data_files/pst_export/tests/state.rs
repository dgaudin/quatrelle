// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! What the cache keeps of a message beyond its source (read or not, a draft, flagged and when
//! it's due, answered or forwarded, its categories), and what its source says that a data file
//! keeps apart (importance, sensitivity, its header block, Bcc, what it answers), through a data
//! file and back.

use outlook_pst::ltp::prop_context::PropertyValue as V;
use outlook_pst::messaging::named_prop::{NamedPropertyGuid, NamedPropertyId};

use super::*;

/// A data file's named property: its ID, by its property set's data1 (PSETID_Task's is
/// 0x00062003) and LID, as the writer's own reader reads the file's map.
pub(super) fn named_id(path: &Path, set: u32, lid: u32) -> Option<u16> {
    let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(path).ok()?)).ok()?;
    let map = PstStore::named_property_map(store.as_ref()).ok()?;
    let guids = map.properties().stream_guid().ok()?;
    map.properties().stream_entry().ok()?.into_iter().find_map(|e| match (e.guid(), e.id()) {
        (NamedPropertyGuid::GuidIndex(i), NamedPropertyId::Number(n)) if n == lid && guids.get(usize::from(i))?.data1() == set => Some(e.prop_id()),
        _ => None,
    })
}

/// The properties `ids` of the message with `subject` in the folder at `names` of a data file.
pub(super) fn props_of(path: &Path, names: &[&str], subject: &str, ids: &[u16]) -> Vec<Option<V>> {
    let store = UnicodeStore::read(Rc::new(UnicodePstFile::open(path).unwrap())).unwrap();
    let any: Rc<dyn PstStore> = store.clone();
    let mut folder = any.open_folder(&any.properties().ipm_sub_tree_entry_id().unwrap()).unwrap();
    for name in names {
        let table = folder.hierarchy_table().unwrap().clone();
        folder = table
            .rows_matrix()
            .filter_map(|row| any.open_folder(&any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).ok()?).ok())
            .find(|f| f.properties().display_name().is_ok_and(|n| n == *name))
            .unwrap();
    }
    for row in folder.contents_table().unwrap().rows_matrix() {
        let entry = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).unwrap();
        let m = UnicodeMessage::read(store.clone(), &entry, None).unwrap();
        if m.properties().get(0x0037).and_then(crate::pst::text_of).as_deref() == Some(subject) {
            return ids.iter().map(|id| m.properties().get(*id).cloned()).collect();
        }
    }
    panic!("no {subject} in {names:?}")
}

/// A message in `folder` of `store`, with `flags`; `edit` changes its draft first. Its source
/// carries a header of its own, a sensitivity and, for a sent copy, its Bcc.
fn mk(store: &Store, folder: &str, uid: u32, subject: &str, flags: Flags, edit: &dyn Fn(&mut mime::Draft)) -> quatrelle_model::Summary {
    let mut d = mime::Draft {
        to: vec![EmailAddress::new("Ana", "ana@example.com")],
        cc: vec![EmailAddress::new("", "ben@example.com")],
        subject: subject.into(),
        body: "Hello there.".into(),
        html: Some("<p>Hello <b>there</b>.</p>".into()),
        ..Default::default()
    };
    edit(&mut d);
    let built = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
    let mut raw = b"X-Custom: kept\r\nSensitivity: Private\r\n".to_vec();
    if !d.bcc.is_empty() {
        raw.extend_from_slice(b"Bcc: Cleo <cleo@example.com>\r\n");
    }
    raw.extend_from_slice(&built);
    let key = MessageKey { account: "a1".into(), folder: folder.into(), validity: 1, uid };
    store.write_body(&key, &raw).unwrap();
    mime::summary_from_full(&raw, key, flags, 10, 0)
}

/// An account with a flagged, unread message in two categories (one the user named, one only a
/// keyword), one answered, a sent copy forwarded with its Bcc, and a draft.
pub(super) fn account(dir: &Path) -> Store {
    let store = Store::open(dir.join("cache")).unwrap();
    let seen = Flags { seen: true, ..Default::default() };
    let plain = |_: &mut mime::Draft| {};
    let plan_flags = Flags { flagged: true, keywords: vec!["$label4".into(), "project_x".into(), "$Junk".into()], ..Default::default() };
    let mut plan = mk(&store, "INBOX", 1, "Plan — café", plan_flags, &|d| d.importance = 1);
    plan.received = plan.date + 3600;
    let second = mk(&store, "INBOX", 2, "Second", Flags { answered: true, ..seen.clone() }, &|d| {
        d.in_reply_to = "m1@example.com".into();
        d.references = vec!["m0@example.com".into(), "m1@example.com".into()];
    });
    let sent = mk(&store, "Sent", 3, "Sent one", Flags { forwarded: true, ..seen.clone() }, &|d| {
        d.bcc = vec![EmailAddress::new("Cleo", "cleo@example.com")];
    });
    let draft = mk(&store, "Drafts", 4, "Draft one", Flags::default(), &plain);
    let state = |messages| FolderState { validity: 1, uid_next: 10, messages, ..Default::default() };
    store.save_folder("a1", "INBOX", &state(vec![plan, second])).unwrap();
    store.save_folder("a1", "Sent", &state(vec![sent])).unwrap();
    store.save_folder("a1", "Drafts", &state(vec![draft])).unwrap();
    let folder = |path: &str, role| quatrelle_model::Folder { account: "a1".into(), path: path.into(), role, selectable: true, ..Default::default() };
    let folders = [folder("INBOX", FolderRole::Inbox), folder("Sent", FolderRole::Sent), folder("Drafts", FolderRole::Drafts)];
    store.save_folders("a1", &folders).unwrap();
    store
}

#[test]
fn what_the_cache_keeps_of_a_message_goes_through_a_data_file() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-export-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = account(&dir);
    let file = dir.join("Out.pst");
    let marks = ExportMarks {
        due: std::collections::BTreeMap::from([("m1@example.com".to_string(), 20_000)]),
        categories: quatrelle_model::organize::default_categories(),
    };
    let done = export(&store, "a1", "Riley's mail", &file, &marks, &mut |_| {}).unwrap();
    assert_eq!(done.messages, 4);
    // When it's due, at midnight of its day, as the task's start and due dates.
    let day = (20_000 * 86_400 + 11_644_473_600) * 10_000_000;
    let (start, due) = (named_id(&file, 0x0006_2003, 0x8104).unwrap(), named_id(&file, 0x0006_2003, 0x8105).unwrap());
    let got = props_of(&file, &["Inbox"], "Plan — café", &[start, due]);
    assert!(got.iter().all(|v| matches!(v, Some(V::Time(t)) if *t == day)), "{got:?}");

    // What the import doesn't read back: the follow-up's text and icon, that the addresses are
    // SMTP ones, the sensitivity, and when it was sent beside when it came.
    let got = props_of(&file, &["Inbox"], "Plan — café", &[0x1095, 0x0E2B, 0x0C1E, 0x0065, 0x0036, 0x0E06, 0x0039]);
    assert!(matches!(got[0], Some(V::Integer32(6))) && matches!(got[1], Some(V::Integer32(1))), "{got:?}");
    assert_eq!(got[2].as_ref().and_then(crate::pst::text_of).as_deref(), Some("SMTP"));
    assert_eq!(got[3].as_ref().and_then(crate::pst::text_of).as_deref(), Some("riley@example.com"));
    assert!(matches!(got[4], Some(V::Integer32(2))));
    let (Some(V::Time(delivered)), Some(V::Time(sent))) = (&got[5], &got[6]) else { panic!("{got:?}") };
    assert_eq!(delivered - sent, 3600 * 10_000_000);
    let flag_request = named_id(&file, 0x0006_2008, 0x8530).unwrap();
    let got = props_of(&file, &["Inbox"], "Plan — café", &[flag_request]);
    assert_eq!(got[0].as_ref().and_then(crate::pst::text_of).as_deref(), Some("Follow up"));

    // Read back with the import.
    let back = Store::open(dir.join("back")).unwrap();
    assert_eq!(crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap().messages, 4);
    let inbox = back.load_folder("b1", "INBOX").unwrap();
    let plan = inbox.messages.iter().find(|m| m.subject == "Plan — café").unwrap();
    assert!(!plan.flags.seen && plan.flags.flagged && !plan.flags.draft, "{:?}", plan.flags);
    // Its categories by name, which the import keeps as their names' keywords: the user's
    // category, found again by its name, and the other keyword as it was; no junk mark.
    assert_eq!(plan.flags.keywords, ["to_do", "project_x"]);
    let to_do = quatrelle_model::organize::default_categories().into_iter().find(|c| c.name == "To do").unwrap();
    assert!(to_do.marks(&plan.flags));
    assert_eq!(plan.importance, 1);
    assert_eq!(plan.received, plan.date + 3600);
    let raw = back.read_body(&plan.key).unwrap();
    let head = String::from_utf8_lossy(&raw);
    assert!(head.contains("X-Custom: kept") && head.contains("Sensitivity: Private"), "{head}");
    let second = inbox.messages.iter().find(|m| m.subject == "Second").unwrap();
    assert!(second.flags.seen && second.flags.answered && !second.flags.flagged && second.flags.keywords.is_empty(), "{:?}", second.flags);
    assert_eq!(second.in_reply_to, "m1@example.com");
    assert_eq!(second.references, ["m0@example.com", "m1@example.com"]);
    let sent = &back.load_folder("b1", "Sent Items").unwrap().messages[0];
    assert!(sent.flags.seen && sent.flags.forwarded, "{:?}", sent.flags);
    let raw = back.read_body(&sent.key).unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("cleo@example.com"), "the Bcc is lost");
    let draft = &back.load_folder("b1", "Drafts").unwrap().messages[0];
    assert!(draft.flags.draft && !draft.flags.seen, "{:?}", draft.flags);
    let _ = std::fs::remove_dir_all(&dir);
}
