// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a sync costs the window on a big folder on screen: the folder made up (50,000 messages
//! in the Inbox of a server account, `MC_PER` for another size), shown, and a sync that brings
//! three messages and takes one away, five times. Printed: what the account's thread spends
//! saving the list, and what the window spends taking the change (its time, the messages it read
//! from the disk again), by date and in conversations.
//!
//! ```text
//! cargo test -p quatrelle-engine --release --lib bench::sync::three_in_one_out -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use crate::model::{Event, MessageKey, Summary};

use super::*;

const NOW: i64 = 1_800_000_000;

/// A sync of `folder`, as an account's thread makes it: three messages come (numbered after the
/// others, the newest), one goes (from the middle); the list saved. What the window is told, and
/// how long the save took.
fn sync(store: &Store, account: &str, folder: &str, round: u32) -> (Event, Duration) {
    let t = Instant::now();
    let mut st = store.load_folder(account, folder).unwrap();
    let top = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0);
    let middle = st.messages.len() / 2;
    st.messages.remove(middle);
    for n in 1..=3 {
        let key = MessageKey { account: account.into(), folder: folder.into(), validity: st.validity, uid: top + n };
        let m = Summary {
            key,
            subject: format!("Came in sync {round}, number {n}"),
            from: crate::model::EmailAddress::new("New Person", "new@big.example"),
            date: NOW + i64::from(round * 10 + n),
            preview: "Fresh off the server".into(),
            ..Default::default()
        };
        st.messages.insert(0, m);
    }
    st.uid_next = top + 4;
    store.save_folder(account, folder, &st).unwrap();
    (Event::FolderChanged { account: account.into(), folder: folder.into() }, t.elapsed())
}

/// The marks the window sent kept, as the pool keeps them, and their echo told (or the next
/// change would be taken for it).
fn echo(s: &mut Session, store: &Store) {
    for job in s.take_jobs() {
        let crate::model::Job::Apply { account, op } = job else { continue };
        let changed = store.apply_op(&account, &op).unwrap();
        for folder in changed {
            s.apply(Event::FolderChanged { account: account.clone(), folder });
        }
    }
}

#[test]
#[ignore]
fn three_in_one_out() {
    let per: u32 = std::env::var("MC_PER").ok().and_then(|v| v.parse().ok()).unwrap_or(50_000);
    let dir = std::env::temp_dir().join(format!("mc-sync-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    crate::sample::seed_big_mailbox(&store, 1, per, NOW).unwrap();
    let mut config = store.load_accounts().unwrap().remove(0);
    config.imap.host = "imap.big.example".into();
    config.smtp.host = "smtp.big.example".into();
    let account = config.id.clone();
    store.save_accounts(&[config]).unwrap();
    let mut s = Session::open(Some(store.clone()), Clock::new(NOW, 0));
    s.run("folder.open", &json!({"account": account, "folder": "INBOX"})).unwrap();
    println!("list: {} messages", s.list.len());
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let mut round = 0;
    for (conversations, by) in [(false, "date"), (true, "date")] {
        let _ = s.run("view.conversations", &json!({"on": conversations}));
        let _ = s.run("view.arrange", &json!({"by": by}));
        let Some(first) = s.list.get(10).map(|m| m.key.to_id()) else { continue };
        let _ = s.run("list.open", &json!({"key": first}));
        echo(&mut s, &store);
        let (mut saved, mut window, mut read) = (Duration::ZERO, Duration::ZERO, 0);
        for _ in 0..5 {
            round += 1;
            let (event, save) = sync(&store, &account, "INBOX", round);
            saved += save;
            let before = s.messages_read();
            let t = Instant::now();
            s.apply(event);
            window += t.elapsed();
            read += s.messages_read() - before;
        }
        let want = store.load_folder(&account, "INBOX").unwrap().messages.len();
        assert_eq!(s.list.len(), want, "the list shows the folder");
        println!(
            "a sync, 3 in and 1 out (conversations {conversations}, by {by}): the window {:.1} ms, {} messages read again; \
             the account's thread saving the list {:.1} ms",
            ms(window) / 5.0,
            read / 5,
            ms(saved) / 5.0
        );
    }
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}
