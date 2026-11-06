// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Any message through a data file and back: its subject, text, addresses and what the cache keeps
//! of it come back as they went, in a Unicode file and in an ANSI one.

use outlook_pst::AnsiPstFile;
use proptest::prelude::*;

use super::*;

/// What a message is made of here.
#[derive(Clone, Debug)]
struct Mail {
    subject: String,
    body: String,
    seen: bool,
    flagged: bool,
    complete: bool,
    pinned: bool,
    /// 0 none, 1 replied, 2 forwarded.
    acted: u8,
    categories: Vec<String>,
}

fn mail() -> impl Strategy<Value = Mail> {
    (
        "[A-Za-zÀ-ÿΑ-ω0-9 ,.!?'-]{1,40}",
        "[A-Za-zÀ-ÿ0-9 ,.\n]{0,200}",
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        0u8..3,
        proptest::collection::btree_set("[a-z][a-z0-9_]{0,10}", 0..3),
    )
        .prop_map(|(subject, body, seen, flagged, complete, pinned, acted, categories)| Mail {
            subject: subject.trim().to_string(),
            body,
            seen,
            flagged,
            complete: complete && !flagged,
            pinned,
            acted,
            categories: categories.into_iter().collect(),
        })
        .prop_filter("a subject", |m| !m.subject.is_empty())
}

fn flags(m: &Mail) -> Flags {
    let mut f = Flags { seen: m.seen, flagged: m.flagged, answered: m.acted == 1, forwarded: m.acted == 2, ..Default::default() };
    for k in &m.categories {
        f.set_keyword(k, true);
    }
    f.set_keyword(quatrelle_model::PINNED, m.pinned);
    f.set_keyword(quatrelle_model::FOLLOW_UP_COMPLETE, m.complete);
    f.keywords.sort_by_key(|k| k.to_ascii_lowercase());
    f
}

/// `mails` exported into a new file (`ansi`: an ANSI one) and imported back, by subject.
fn through(mails: &[Mail], ansi: bool) -> Vec<(String, String, Flags)> {
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-export-round-trip-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    let mut summaries = Vec::new();
    for (uid, m) in (1..).zip(mails) {
        let d = mime::Draft {
            to: vec![EmailAddress::new("Ana Lima", "ana@example.com")],
            subject: m.subject.clone(),
            body: m.body.clone(),
            ..Default::default()
        };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        summaries.push(mime::summary_from_full(&raw, key, flags(m), 10, 0));
    }
    store.save_folder("a1", "INBOX", &FolderState { validity: 1, uid_next: 100, messages: summaries, ..Default::default() }).unwrap();
    let inbox =
        quatrelle_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[inbox]).unwrap();
    let file = dir.join("Out.pst");
    if ansi {
        drop(AnsiPstFile::create_empty(&file).unwrap());
    }
    assert_eq!(export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap().messages, mails.len());
    let back = Store::open(dir.join("back")).unwrap();
    crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    let inbox = back.load_folders("b1").unwrap().into_iter().find(|f| f.role == FolderRole::Inbox).unwrap();
    let mut out = Vec::new();
    for s in back.load_folder("b1", &inbox.path).unwrap().messages.iter() {
        let p = mime::parse(&back.read_body(&s.key).unwrap()).unwrap();
        assert_eq!((p.from.email.as_str(), p.to.first().map(|a| a.email.as_str())), ("riley@example.com", Some("ana@example.com")));
        // Keywords in no set order.
        let mut f = s.flags.clone();
        f.keywords.sort_by_key(|k| k.to_ascii_lowercase());
        out.push((s.subject.clone(), p.text, f));
    }
    let _ = std::fs::remove_dir_all(&dir);
    out
}

proptest! {
    // Each case writes and reads two data files.
    #![proptest_config(ProptestConfig::with_cases(12))]

    #[test]
    fn mail_comes_back_as_it_went(mails in proptest::collection::vec(mail(), 1..5), ansi in any::<bool>()) {
        let got = through(&mails, ansi);
        prop_assert_eq!(got.len(), mails.len());
        for m in &mails {
            let found = got.iter().find(|(subject, _, f)| *subject == m.subject && *f == flags(m));
            let (_, body, _) = found.ok_or_else(|| TestCaseError::fail(format!("{m:?} not in {got:?}")))?;
            // The words as they were (line ends and trailing spaces are the transfer's).
            let words = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
            prop_assert_eq!(words(body), words(&m.body), "{:?}", m);
        }
    }
}
