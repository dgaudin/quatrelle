//! Engine tests: a big message's text ahead of the whole of it.

use std::sync::Arc;

use super::*;
use crate::STILL_COMING;
use crate::mime::{self, Draft, PendingFile};
use crate::session::OUTBOX;

const SUBJECT: &str = "Storyboard review on Thursday";
/// A word in the text the tests build, nowhere in the sample mailbox.
const ONLY_IN_TEXT: &str = "zanzibar";

fn key_for(s: &Session, subject: &str) -> MessageKey {
    s.list.iter().find(|m| m.subject == subject).unwrap().key.clone()
}

/// `key`'s text as the worker builds it, one file to come, with `change` made to it.
fn text_first_with(s: &Session, key: &MessageKey, change: impl FnOnce(&mut Draft, &mut EmailAddress, &mut PendingFile)) -> Vec<u8> {
    let raw = s.store.as_ref().unwrap().read_body(key).unwrap();
    let p = mime::parse(&raw).unwrap();
    let body = format!("{}\n{ONLY_IN_TEXT}", p.text);
    let mut d = Draft { reply_to: p.reply_to, to: p.to, cc: p.cc, subject: p.subject, body, ..Default::default() };
    let mut from = p.from;
    let mut file = PendingFile { name: "colour-key.png".into(), mime: "image/png".into(), size: 2048, content_id: None, inline: false };
    change(&mut d, &mut from, &mut file);
    mime::text_first(&d, &from, p.date, &p.message_id, std::slice::from_ref(&file)).unwrap()
}

fn text_first_of(s: &Session, key: &MessageKey) -> Vec<u8> {
    text_first_with(s, key, |_, _, _| {})
}

/// Takes the message off this computer, as one not downloaded yet; its source comes back.
fn not_here(s: &Session, key: &MessageKey) -> Vec<u8> {
    let st = s.store.as_ref().unwrap();
    let whole = st.read_body(key).unwrap();
    st.remove_body(key);
    whole
}

/// The worker's last step: the source goes on disk, then the event says so.
fn whole_arrives(s: &mut Session, key: &MessageKey, raw: &[u8]) {
    s.store.as_ref().unwrap().write_body(key, raw).unwrap();
    s.apply(Event::Body { key: key.clone(), raw: raw.to_vec() });
}

/// `SUBJECT` opened, its text here and its whole message not; that source comes back.
fn in_its_text_phase(s: &mut Session, change: impl FnOnce(&mut Draft, &mut EmailAddress, &mut PendingFile)) -> (MessageKey, Vec<u8>) {
    let key = key_for(s, SUBJECT);
    let raw = text_first_with(s, &key, change);
    let whole = not_here(s, &key);
    s.open_message(&key);
    s.take_jobs();
    s.apply(Event::TextFirst { key: key.clone(), raw });
    (key, whole)
}

fn results(s: &mut Session, query: &str) -> u64 {
    s.run("search.run", &json!({"query": query})).unwrap()["results"].as_u64().unwrap()
}

#[test]
fn a_message_text_first_then_whole() {
    let mut s = sample_session("text-first-then-whole");
    let (key, whole) = in_its_text_phase(&mut s, |_, _, _| {});
    let r = s.reading.clone().unwrap();
    assert!(r.text_first);
    assert!(s.text_first(&key));
    let p = r.parsed.unwrap();
    assert!(p.text.contains(ONLY_IN_TEXT), "{}", p.text);
    assert_eq!(p.attachments[0].name, "colour-key.png");

    whole_arrives(&mut s, &key, &whole);
    let r = s.reading.clone().unwrap();
    assert!(!r.text_first);
    assert!(!s.text_first(&key));
    assert!(!r.parsed.unwrap().text.contains(ONLY_IN_TEXT), "the whole message replaced the text");
    let out = s.store.as_ref().unwrap().root().join("saved.png");
    s.run("attachment.save", &json!({"index": 0, "path": out.to_string_lossy()})).unwrap();
    assert_eq!(&std::fs::read(&out).unwrap()[..4], b"\x89PNG");
    done(s);
}

#[test]
fn the_text_is_read_with_its_names_alone_and_the_list_date() {
    let mut s = sample_session("text-first-names");
    let key = key_for(&s, SUBJECT);
    let raw = text_first_with(&s, &key, |d, from, _| {
        *from = EmailAddress::new("Riley Chen", "");
        d.to = vec![EmailAddress::new("Sam Park", "")];
    });
    let raw = String::from_utf8(raw).unwrap();
    let raw: String = raw.split_inclusive('\n').filter(|l| !l.starts_with("Date:")).collect();
    let _ = not_here(&s, &key);
    s.open_message(&key);
    s.apply(Event::TextFirst { key: key.clone(), raw: raw.into_bytes() });
    let p = s.reading.clone().unwrap().parsed.unwrap();
    assert_eq!(p.from.name, "Riley Chen");
    assert_eq!(p.from.email, "");
    assert_eq!(p.to[0].name, "Sam Park");
    assert_eq!(p.date, s.summary(&key).unwrap().date, "without a date, as the list has it");
    done(s);
}

#[test]
fn the_text_never_replaces_a_whole_message() {
    let mut s = sample_session("text-first-never-replaces");
    let key = key_for(&s, SUBJECT);
    let raw = text_first_of(&s, &key);
    s.open_message(&key);
    s.apply(Event::TextFirst { key: key.clone(), raw });
    let r = s.reading.clone().unwrap();
    assert!(!r.text_first);
    assert!(!r.parsed.unwrap().text.contains(ONLY_IN_TEXT));
    done(s);
}

/// Opened again while it downloads: the whole message can beat the second text.
#[test]
fn the_text_after_the_whole_message_changes_nothing() {
    let mut s = sample_session("text-first-after-whole");
    let key = key_for(&s, SUBJECT);
    let raw = text_first_of(&s, &key);
    let whole = not_here(&s, &key);
    s.open_message(&key);
    s.apply(Event::TextFirst { key: key.clone(), raw: raw.clone() });
    s.open_message(&key);
    whole_arrives(&mut s, &key, &whole);
    s.apply(Event::TextFirst { key: key.clone(), raw });
    let r = s.reading.clone().unwrap();
    assert!(!r.text_first);
    assert!(!r.parsed.unwrap().text.contains(ONLY_IN_TEXT));
    done(s);
}

/// The rest thread tells of a body found on disk, so one message can be told of twice.
#[test]
fn the_whole_message_can_come_twice() {
    let mut s = sample_session("text-first-twice");
    let key = key_for(&s, SUBJECT);
    let raw = text_first_of(&s, &key);
    let whole = String::from_utf8(not_here(&s, &key)).unwrap().replace("revised frames", "revised frames kilimanjaro").into_bytes();
    s.open_message(&key);
    s.pop_out(&key).unwrap();
    s.apply(Event::TextFirst { key: key.clone(), raw });
    assert!(s.windows[0].text_first);
    assert_eq!(results(&mut s, "kilimanjaro"), 0);
    whole_arrives(&mut s, &key, &whole);
    s.apply(Event::Body { key: key.clone(), raw: whole.clone() });
    assert_eq!(results(&mut s, "kilimanjaro"), 1, "indexed once");
    assert!(!s.windows[0].text_first);
    assert!(s.windows[0].parsed.as_ref().unwrap().text.contains("kilimanjaro"));
    done(s);
}

#[test]
fn the_text_is_not_kept_nor_indexed() {
    let mut s = sample_session("text-first-not-kept");
    let (key, _) = in_its_text_phase(&mut s, |_, _, _| {});
    assert!(s.store.as_ref().unwrap().read_body(&key).is_none(), "nothing on disk");
    assert_eq!(results(&mut s, ONLY_IN_TEXT), 0);
    let raw = format!("From: a@x.example\r\nSubject: {SUBJECT}\r\n\r\n{ONLY_IN_TEXT}\r\n").into_bytes();
    whole_arrives(&mut s, &key, &raw);
    assert_eq!(results(&mut s, ONLY_IN_TEXT), 1);
    done(s);
}

#[test]
fn a_window_follows_the_same_way() {
    let mut s = sample_session("text-first-window");
    let key = key_for(&s, SUBJECT);
    let raw = text_first_of(&s, &key);
    let whole = not_here(&s, &key);
    s.pop_out(&key).unwrap();
    s.apply(Event::TextFirst { key: key.clone(), raw });
    assert!(s.windows[0].text_first);
    assert!(s.text_first(&key));
    assert!(s.reading.is_none(), "the reading pane is left alone");
    assert!(s.windows[0].parsed.as_ref().unwrap().text.contains(ONLY_IN_TEXT));

    s.apply(Event::BodyFailed { key: key.clone(), error: "no way through".into() });
    assert_eq!(s.windows[0].error.as_deref(), Some("no way through"));
    assert!(s.windows[0].parsed.as_ref().unwrap().text.contains(ONLY_IN_TEXT), "the text stays");
    assert!(s.windows[0].text_first, "so what needs the rest still says it's not here");

    whole_arrives(&mut s, &key, &whole);
    let w = &s.windows[0];
    assert!(!w.text_first);
    assert_eq!(w.error, None);
    assert!(w.parsed.as_ref().unwrap().text.contains("revised frames"));
    assert!(!w.parsed.as_ref().unwrap().text.contains(ONLY_IN_TEXT));
    done(s);
}

#[test]
fn the_reading_pane_keeps_the_text_when_the_rest_fails() {
    let mut s = sample_session("text-first-failed");
    let (key, _) = in_its_text_phase(&mut s, |_, _, _| {});
    s.apply(Event::BodyFailed { key: key.clone(), error: "no way through".into() });
    let r = s.reading.clone().unwrap();
    assert_eq!(r.error.as_deref(), Some("no way through"));
    assert!(r.text_first);
    assert!(r.parsed.is_some());
    assert!(matches!(s.run("mail.forward", &json!({})), Err(CmdError::Failed(m)) if m == STILL_COMING));
    done(s);
}

#[test]
fn the_whole_message_arriving_leaves_the_open_one_alone() {
    let mut s = sample_session("text-first-other-open");
    let (a, whole) = in_its_text_phase(&mut s, |_, _, _| {});
    let whole = String::from_utf8(whole).unwrap().replace("revised frames", &format!("revised frames {ONLY_IN_TEXT}"));
    let b = s.list.iter().find(|m| m.key != a).unwrap().key.clone();
    s.open_message(&b);
    let before = s.reading.clone().unwrap();
    assert!(!s.text_first(&a), "nothing shows it now");
    whole_arrives(&mut s, &a, whole.as_bytes());
    assert_eq!(s.reading.clone().unwrap(), before);
    assert_eq!(results(&mut s, ONLY_IN_TEXT), 1, "indexed all the same");
    done(s);
}

#[test]
fn opening_it_again_asks_for_the_text_again() {
    let mut s = sample_session("text-first-again");
    let key = key_for(&s, SUBJECT);
    exchange(&mut s, |c| c.ews = Some("https://mail.studio.example/EWS/Exchange.asmx".into()));
    shape(&mut s, &key, 2 << 20, true);
    let raw = text_first_of(&s, &key);
    let _ = not_here(&s, &key);
    s.open_message(&key);
    let asked = |s: &mut Session| s.take_jobs().iter().filter(|j| matches!(j, Job::FetchTextFirst { key: k } if *k == key)).count();
    assert_eq!(asked(&mut s), 1);
    s.apply(Event::TextFirst { key: key.clone(), raw: raw.clone() });
    let other = s.list.iter().find(|m| m.key != key).unwrap().key.clone();
    s.open_message(&other);
    s.take_jobs();
    shape(&mut s, &key, 2 << 20, true); // Marking the other read rebuilt the list from the cache.
    s.open_message(&key);
    assert_eq!(asked(&mut s), 1, "its text wasn't kept, so it's asked for again");
    assert!(!s.reading.as_ref().unwrap().text_first, "nothing shown yet");
    // The first download's failure, then the second's text: only what's new is said.
    s.apply(Event::BodyFailed { key: key.clone(), error: "no way through".into() });
    s.apply(Event::TextFirst { key: key.clone(), raw });
    let r = s.reading.clone().unwrap();
    assert!(r.text_first);
    assert_eq!(r.error, None);
    done(s);
}

#[test]
fn what_needs_the_whole_message_says_so() {
    let mut s = sample_session("text-first-refused");
    let (key, _) = in_its_text_phase(&mut s, |_, _, _| {});
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let file = root.join("out.bin").to_string_lossy().to_string();
    for (id, params) in [
        ("mail.forward", json!({})),
        ("mail.forwardAsAttachment", json!({})),
        ("attachment.save", json!({"index": 0, "path": file})),
        ("attachment.saveAll", json!({"dir": root.to_string_lossy()})),
        ("attachment.preview", json!({"index": 0})),
        ("message.saveAs", json!({"path": file})),
        ("message.exportPdf", json!({"path": file})),
        ("message.headers", json!({})),
    ] {
        match s.run(id, &params) {
            Err(CmdError::Failed(m)) => assert_eq!(m, STILL_COMING, "{id}"),
            other => panic!("{id}: {other:?}"),
        }
    }
    assert!(s.compose.is_none());
    assert!(s.message_source().is_err());
    assert!(s.message_pdf().is_err(), "printing goes through this");
    assert!(s.attachment_bytes(0).is_err());
    assert!(s.reading.as_ref().is_some_and(|r| r.key == key));
    done(s);
}

/// `cmd` on a message in its text phase, `change` made to that text.
fn answer_in_text_phase(name: &str, cmd: &str, change: impl FnOnce(&mut Draft, &mut EmailAddress, &mut PendingFile)) -> Result<(), String> {
    let mut s = sample_session(name);
    in_its_text_phase(&mut s, change);
    let out = s.run(cmd, &json!({}));
    let wrote = s.compose.as_ref().map(|c| (c.to.clone(), c.body.contains(ONLY_IN_TEXT)));
    done(s);
    match out {
        Ok(_) => {
            let (to, quoted) = wrote.ok_or("no message opened")?;
            assert!(!to.is_empty());
            assert!(quoted, "the text is quoted");
            Ok(())
        }
        Err(CmdError::Failed(m)) => Err(m),
        Err(e) => panic!("{cmd}: {e:?}"),
    }
}

#[test]
fn a_reply_waits_only_for_what_it_needs() {
    let refused = Err(STILL_COMING.to_string());
    assert_eq!(answer_in_text_phase("text-first-reply", "mail.reply", |_, _, _| {}), Ok(()));
    assert_eq!(answer_in_text_phase("text-first-reply-all", "mail.replyAll", |_, _, _| {}), Ok(()));
    // An inline file.
    let inline = |_: &mut Draft, _: &mut EmailAddress, f: &mut PendingFile| {
        f.inline = true;
        f.content_id = Some("still@x".into());
    };
    assert_eq!(answer_in_text_phase("text-first-inline", "mail.reply", inline), refused);
    // A picture the text names, its file not known as inline yet (Graph's).
    let named = |d: &mut Draft, _: &mut EmailAddress, _: &mut PendingFile| d.html = Some("<p>See:</p><img src=\"cid:still@x\">".into());
    assert_eq!(answer_in_text_phase("text-first-cid", "mail.reply", named), refused);
    let shouted = |d: &mut Draft, _: &mut EmailAddress, _: &mut PendingFile| d.html = Some("<p>See:</p><img src=\"CID:still@x\">".into());
    assert_eq!(answer_in_text_phase("text-first-cid-upper", "mail.reply", shouted), refused);
    // Whoever is written to has no address yet.
    let sender = |_: &mut Draft, from: &mut EmailAddress, _: &mut PendingFile| *from = EmailAddress::new("Riley Chen", "");
    assert_eq!(answer_in_text_phase("text-first-sender", "mail.reply", sender), refused);
    let replies = |d: &mut Draft, _: &mut EmailAddress, _: &mut PendingFile| d.reply_to = vec![EmailAddress::new("Team", "")];
    assert_eq!(answer_in_text_phase("text-first-replies", "mail.reply", replies), refused);
    let elsewhere = |d: &mut Draft, from: &mut EmailAddress, _: &mut PendingFile| {
        *from = EmailAddress::new("Riley Chen", "");
        d.reply_to = vec![EmailAddress::new("Team", "team@studio.example")];
    };
    assert_eq!(answer_in_text_phase("text-first-elsewhere", "mail.reply", elsewhere), Ok(()), "the sender isn't written to");
    // A name alone in Cc matters to Reply all only.
    let cc = |d: &mut Draft, _: &mut EmailAddress, _: &mut PendingFile| d.cc = vec![EmailAddress::new("Sam Park", "")];
    assert_eq!(answer_in_text_phase("text-first-cc", "mail.replyAll", cc), refused);
    assert_eq!(answer_in_text_phase("text-first-cc-reply", "mail.reply", cc), Ok(()));
    let to = |d: &mut Draft, _: &mut EmailAddress, _: &mut PendingFile| d.to = vec![EmailAddress::new("Sam Park", "")];
    assert_eq!(answer_in_text_phase("text-first-to", "mail.replyAll", to), refused);
}

fn exchange(s: &mut Session, change: impl Fn(&mut quatrelle_model::AccountConfig)) {
    change(&mut s.accounts.iter_mut().find(|a| a.config.id == sample::SAMPLE_ACCOUNT).unwrap().config);
}

fn shape(s: &mut Session, key: &MessageKey, size: u32, files: bool) {
    let m = Arc::make_mut(s.list.iter_mut().find(|m| m.key == *key).unwrap());
    m.size = size;
    m.has_attachments = files;
}

/// What opening `key` asks for.
fn asked_for(s: &mut Session, key: &MessageKey) -> &'static str {
    s.take_jobs();
    s.open_message(key);
    s.take_jobs()
        .iter()
        .find_map(|j| match j {
            Job::FetchTextFirst { key: k } if k == key => Some("text first"),
            Job::FetchBody { key: k } if k == key => Some("whole"),
            Job::ReadOnline { key: k } if k == key => Some("online"),
            _ => None,
        })
        .unwrap_or("nothing")
}

#[test]
fn which_messages_come_text_first() {
    const EWS: &str = "https://mail.studio.example/EWS/Exchange.asmx";
    const BIG: u32 = 2 << 20;
    let mut s = sample_session("text-first-which");
    let key = key_for(&s, SUBJECT);
    let _ = not_here(&s, &key);
    s.open_message(&key); // Read now: nothing later rebuilds the list under the shapes below.

    // IMAP.
    shape(&mut s, &key, BIG, true);
    assert_eq!(asked_for(&mut s, &key), "whole");

    exchange(&mut s, |c| c.ews = Some(EWS.into()));
    assert_eq!(asked_for(&mut s, &key), "text first");
    s.reading = None;
    s.pop_out(&key).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::FetchTextFirst { key: k } if *k == key)), "a window as well");
    shape(&mut s, &key, quatrelle_model::job::TEXT_FIRST_OVER, true);
    assert_eq!(asked_for(&mut s, &key), "text first", "from the size on");
    shape(&mut s, &key, quatrelle_model::job::TEXT_FIRST_OVER - 1, true);
    assert_eq!(asked_for(&mut s, &key), "whole");
    shape(&mut s, &key, BIG, false);
    assert_eq!(asked_for(&mut s, &key), "whole", "nothing to wait for");
    shape(&mut s, &key, 100 << 10, true);
    assert_eq!(asked_for(&mut s, &key), "whole");

    // Graph gives no size.
    exchange(&mut s, |c| {
        c.ews = None;
        c.graph = Some(quatrelle_model::GRAPH_URL.to_string());
    });
    shape(&mut s, &key, 0, true);
    assert_eq!(asked_for(&mut s, &key), "text first");
    shape(&mut s, &key, 0, false);
    assert_eq!(asked_for(&mut s, &key), "whole");

    // Older mail, read on the server.
    exchange(&mut s, |c| {
        c.graph = None;
        c.ews = Some(EWS.into());
    });
    shape(&mut s, &key, BIG, true);
    let summary = (*s.list.iter().find(|m| m.key == key).unwrap().clone()).clone();
    s.older = Some((key.account.clone(), key.folder.clone(), vec![summary.clone()], false));
    assert_eq!(asked_for(&mut s, &key), "online");
    s.older = None;

    // The Outbox, and a message the list doesn't have.
    let waiting = MessageKey { folder: OUTBOX.into(), ..key.clone() };
    s.list.push(Arc::new(Summary { key: waiting.clone(), ..summary }));
    assert_eq!(asked_for(&mut s, &waiting), "whole");
    s.list.retain(|m| m.key != key);
    assert_eq!(asked_for(&mut s, &key), "whole");

    // "Download all" stays whole.
    s.take_jobs();
    s.run("account.downloadAll", &json!({})).unwrap();
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::FetchBody { .. })));
    assert!(!jobs.iter().any(|j| matches!(j, Job::FetchTextFirst { .. })));
    done(s);
}
