// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a flag or category changed on mail in the list, and what a sync changed in its
//! folder, are changed in the list as it stands, which must be the list built again.

use std::collections::BTreeMap;

use super::*;
use crate::model::{Event, ListChange, PINNED};

/// What a step can do: change a flag or a category on some messages, open one, or a sync's
/// change to the folder (messages gone, messages changed each a way, messages come).
#[derive(Clone, Debug)]
enum Step {
    Flag(Vec<u32>, Flag, bool),
    Keyword(Vec<u32>, &'static str, bool),
    Open(u32),
    Sync(Vec<u32>, Vec<(u32, u8)>, Vec<(u8, u8)>),
}

fn step(n: u32) -> impl Strategy<Value = Step> {
    let uids = prop::collection::vec(0..n.max(1), 1..4);
    let flag = prop::sample::select(vec![Flag::Seen, Flag::Seen, Flag::Flagged, Flag::Answered, Flag::Deleted]);
    let keyword = prop::sample::select(vec![PINNED, "work", "$Junk"]);
    prop_oneof![
        3 => (uids.clone(), flag, any::<bool>()).prop_map(|(u, f, on)| Step::Flag(u, f, on)),
        2 => (uids, keyword, any::<bool>()).prop_map(|(u, k, on)| Step::Keyword(u, k, on)),
        3 => (0..n.max(1)).prop_map(Step::Open),
        3 => (
            prop::collection::vec(0..n.max(1), 0..3),
            prop::collection::vec((0..n.max(1), any::<u8>()), 0..4),
            prop::collection::vec(any::<(u8, u8)>(), 0..4),
        )
            .prop_map(|(gone, changed, come)| Step::Sync(gone, changed, come)),
    ]
}

/// A message as `folder` makes it, numbered `uid`.
fn made(uid: u32, (b, parent): (u8, u8)) -> Summary {
    let mut m = message(uid);
    m.key.folder = "INBOX".into();
    m.key.validity = 1;
    m.date = NOW - i64::from(b % 5) * 3_600;
    m.message_id = format!("<m{uid}@x>");
    if b & 0x20 != 0 && u32::from(parent) < uid {
        m.in_reply_to = format!("<m{parent}@x>");
    }
    m.from = EmailAddress::new(format!("Person {}", b % 3), format!("p{}@x", b % 3));
    m.size = u32::from(b) * 1_000;
    m.flags.seen = b & 1 != 0;
    m.flags.flagged = b & 2 != 0;
    m.has_attachments = b & 4 != 0;
    m.flags.set_keyword(PINNED, b & 8 != 0);
    m.flags.set_keyword("work", b & 0x10 != 0);
    m
}

/// A message changed the way `how` says: its read mark, replied, a flag, a pin, the date, the
/// sender, the subject, the preview, deleted.
fn changed(m: &mut Summary, how: u8) {
    match how % 9 {
        0 => m.flags.seen = !m.flags.seen,
        1 => m.flags.answered = !m.flags.answered,
        2 => m.flags.flagged = !m.flags.flagged,
        3 => {
            let on = !m.flags.has(PINNED);
            m.flags.set_keyword(PINNED, on);
        }
        4 => m.date -= 7_200,
        5 => m.from = EmailAddress::new(format!("Person {}", how % 4), format!("p{}@x", how % 4)),
        6 => m.subject = format!("Subject {how}"),
        7 => m.preview = format!("Preview {how}"),
        _ => m.flags.deleted = !m.flags.deleted,
    }
}

/// The sync's change, as the store notes it, taken by the session: what the folder holds after.
fn sync(s: &mut Session, gone: &[u32], how: &[(u32, u8)], come: &[(u8, u8)], next: &mut u32) -> BTreeMap<u32, Summary> {
    let mut now: BTreeMap<u32, Summary> = s
        .cache
        .get(&("a".to_string(), "INBOX".to_string()))
        .map(|st| st.messages.iter().map(|m| (m.key.uid, Summary::clone(m))).collect())
        .unwrap_or_default();
    let mut change = ListChange { validity: 1, ..ListChange::default() };
    let mut touched = BTreeMap::new();
    for uid in gone {
        if now.remove(uid).is_some() {
            touched.insert(*uid, None);
        }
    }
    for (uid, b) in how {
        if let Some(m) = now.get_mut(uid) {
            changed(m, *b);
            touched.insert(*uid, Some(m.clone()));
        }
    }
    for bits in come {
        let m = made(*next, *bits);
        now.insert(*next, m.clone());
        touched.insert(*next, Some(m));
        *next += 1;
    }
    for (uid, m) in touched {
        match m {
            Some(m) => change.upserts.push(m),
            None => change.removed.push(uid),
        }
    }
    change.count = now.len();
    change.print = ListChange::print(now.keys().copied());
    s.apply(Event::FolderSynced { account: "a".into(), folder: "INBOX".into(), change: Box::new(change) });
    now
}

/// A folder of `n` messages: dates that tie, conversations that form, flags and categories
/// at random (from `bits`).
fn folder(bits: &[(u8, u8)]) -> Vec<Summary> {
    bits.iter()
        .enumerate()
        .map(|(i, &(b, parent))| {
            let uid = u32::try_from(i).unwrap();
            let mut m = message(uid);
            m.key.folder = "INBOX".into();
            m.date = NOW - i64::from(b % 5) * 3_600;
            m.message_id = format!("<m{uid}@x>");
            if b & 0x20 != 0 && u32::from(parent) < uid {
                m.in_reply_to = format!("<m{parent}@x>");
            }
            m.from = EmailAddress::new(format!("Person {}", b % 3), format!("p{}@x", b % 3));
            m.size = u32::from(b) * 1_000;
            m.flags.seen = b & 1 != 0;
            m.flags.flagged = b & 2 != 0;
            m.has_attachments = b & 4 != 0;
            m.flags.set_keyword(PINNED, b & 8 != 0);
            m.flags.set_keyword("work", b & 0x10 != 0);
            m
        })
        .collect()
}

/// What the list shows, to compare with it built again.
type Shown = (Vec<Arc<Summary>>, Vec<Row>, Vec<MessageKey>, Option<MessageKey>, Option<MessageKey>);

fn shown(s: &Session) -> Shown {
    (s.list.clone(), s.rows().to_vec(), s.selection.clone(), s.focus.clone(), s.reading.as_ref().map(|r| r.key.clone()))
}

fn key_in_inbox(uid: u32) -> MessageKey {
    MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Each change, opened message, sync and step taken into the list as it stands leaves it as
    /// building it again would: the messages, their rows, the selection, the focus and the
    /// message read; filtered, arranged and grouped every way. A sync's change leaves the folder
    /// in memory as the list on the disk after it, nothing read from the disk again.
    #[test]
    fn patched_list_equals_refresh(
        bits in prop::collection::vec(any::<(u8, u8)>(), 1..300),
        conversations in any::<bool>(),
        arrange in prop::sample::select(Arrange::ALL.to_vec()),
        reverse in any::<bool>(),
        filter in prop::sample::select(vec![ListFilter::All, ListFilter::Unread, ListFilter::Flagged, ListFilter::Categorized, ListFilter::Attachments]),
        steps in prop::collection::vec(step(300), 1..40),
    ) {
        let messages = folder(&bits);
        let n = u32::try_from(messages.len()).unwrap();
        let mut s = Session::open(Some(quatrelle_store::Store::memory()), quatrelle_model::dates::Clock::new(NOW, 0));
        s.cache.insert(("a".into(), "INBOX".into()), quatrelle_store::FolderState { validity: 1, messages: messages.into_iter().map(Arc::new).collect(), ..Default::default() });
        s.prefs.conversations = conversations;
        s.prefs.arrange = arrange;
        s.prefs.reverse_sort = reverse;
        s.filter = filter;
        s.open_view(View::Folder { account: "a".into(), folder: "INBOX".into() });
        let mut next = n;
        for (at, st) in steps.iter().enumerate() {
            let keys = |u: &[u32]| u.iter().map(|&u| key_in_inbox(u % n)).collect::<Vec<_>>();
            match st {
                Step::Flag(u, flag, on) => s.apply_op("a", Op::SetFlag { keys: keys(u), flag: *flag, on: *on }),
                Step::Keyword(u, k, on) => s.apply_op("a", Op::SetKeyword { keys: keys(u), keyword: (*k).to_string(), on: *on, name: None }),
                Step::Open(u) => {
                    let k = key_in_inbox(u % n);
                    if s.index_of(&k).is_some() {
                        s.open_message(&k);
                    }
                }
                Step::Sync(gone, how, come) => {
                    let read = s.messages_read();
                    let now = sync(&mut s, gone, how, come, &mut next);
                    let held: BTreeMap<u32, Summary> = s.cache.get(&("a".to_string(), "INBOX".to_string())).map(|st| st.messages.iter().map(|m| (m.key.uid, Summary::clone(m))).collect()).unwrap_or_default();
                    prop_assert_eq!(&held, &now, "the folder in memory after step {}", at);
                    prop_assert_eq!(s.messages_read(), read, "nothing read again");
                }
            }
            let patched = shown(&s);
            s.refresh_list();
            prop_assert_eq!(&patched, &shown(&s), "after step {} ({:?})", at, st);
        }
    }
}

/// `n` unread messages in conversations, as `big_folder` makes them.
fn unread_folder(n: u32) -> Session {
    let mut s = big_folder(n);
    if let Some(st) = s.cache.get_mut(&("a".to_string(), "INBOX".to_string())) {
        st.validity = 1;
        for m in &mut st.messages {
            Arc::make_mut(m).flags.seen = false;
        }
    }
    s.refresh_list();
    s
}

/// Opening message after message in a big folder marks each read without building the list
/// again (each read mark built every row of it, worked its conversations out and copied every
/// message out of the folder); what the rows count unread is as built afresh.
#[test]
fn marking_read_does_not_rebuild() {
    let mut s = unread_folder(10_941);
    for conversations in [false, true] {
        s.run("view.conversations", &json!({"on": conversations})).unwrap();
        let when = format!("conversations {conversations}");
        let keys: Vec<MessageKey> = s.row_keys().into_iter().filter(|k| s.summary(k).is_some_and(|m| !m.flags.seen)).take(100).collect();
        assert_eq!(keys.len(), 100, "{when}");
        let (built, threadings, revision) = (s.nav_structure(), s.nav.threadings(), s.revision);
        let started = std::time::Instant::now();
        for k in &keys {
            s.open_message(k);
        }
        eprintln!("{when}: 100 opened and marked read in {:?}", started.elapsed());
        assert!(keys.iter().all(|k| s.summary(k).is_some_and(|m| m.flags.seen)), "{when}: marked read");
        assert_eq!(s.nav_structure(), built, "{when}: no rows built");
        assert_eq!(s.nav.threadings(), threadings, "{when}: no conversations worked out");
        assert_ne!(s.revision, revision, "{when}: the interface draws the change");
        let (rows, _, _) = s.build_rows();
        assert_eq!(s.rows(), &rows[..], "{when}: unread counts as built afresh");
    }
}

/// Shown Unread only, the message read stays while it's read and goes when the next is opened,
/// one row at a time; the list is the one built again.
#[test]
fn unread_filter_drops_the_message_read_before() {
    let mut s = unread_folder(2_000);
    s.run("list.filter", &json!({"value": "unread"})).unwrap();
    let keys: Vec<MessageKey> = s.row_keys().into_iter().take(20).collect();
    let total = s.list.len();
    for (i, k) in keys.iter().enumerate() {
        s.open_message(k);
        assert!(s.index_of(k).is_some(), "the one read stays");
        assert_eq!(s.list.len(), total - i, "the one read before goes");
    }
    let patched = shown(&s);
    s.refresh_list();
    assert_eq!(patched, shown(&s));
}

/// Arranged by flag, a flag moves the message to its group; a pin to the top.
#[test]
fn flag_and_pin_move_their_rows() {
    let mut s = unread_folder(500);
    s.run("view.arrange", &json!({"by": "flag"})).unwrap();
    let k = s.list.iter().rev().find(|m| !m.flags.flagged).map(|m| m.key.clone()).unwrap();
    s.apply_op("a", Op::SetFlag { keys: vec![k.clone()], flag: Flag::Flagged, on: true });
    assert!(s.index_of(&k).unwrap() < s.list.iter().filter(|m| m.flags.flagged).count(), "among the flagged");
    let patched = shown(&s);
    s.refresh_list();
    assert_eq!(patched, shown(&s));
    s.apply_op("a", Op::SetKeyword { keys: vec![k.clone()], keyword: PINNED.to_string(), on: true, name: None });
    assert_eq!(s.rows()[0], Row::Header("Pinned"));
    assert_eq!(s.row_index_of(&k), Some(1), "pinned at the top");
    let patched = shown(&s);
    s.refresh_list();
    assert_eq!(patched, shown(&s));
}
