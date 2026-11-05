// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The header of a rebuilt message: the item's own lines kept, the lines its properties give,
//! and nothing from the file outside its own line.

use super::*;

/// The header of a rebuilt message: its lines, each ASCII without control characters (but tab),
/// at most 998 bytes.
fn clean_header(raw: &[u8]) -> Vec<String> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = std::str::from_utf8(&raw[..end]).unwrap();
    let lines: Vec<String> = head.split("\r\n").map(str::to_string).collect();
    for l in &lines {
        assert!(l.len() <= 998 && l.chars().all(|c| c == '\t' || (c.is_ascii() && !c.is_ascii_control())), "{l:?}");
    }
    lines
}

/// Outlook's conversation index and topic and the sensitivity become headers on an item that
/// has none; hostile text in them stays inside its own line, short and encoded.
#[test]
fn a_headerless_item_gets_thread_and_sensitivity_headers() {
    let index: Vec<u8> = (0..22 + 5 * 60 + 3).map(|i: u32| i as u8).collect();
    let topic = "Plan\r\nBcc: evil@example.com\u{0}\u{7}  for the café";
    let props = [
        (0x001A, Val::Text("IPM.Note".into())),
        (0x0037, Val::Text("Plan".into())),
        (0x0071, Val::Bin(&index)),
        (0x0070, Val::Text(topic.into())),
        (0x0036, Val::Int(3)),
        (0x1035, Val::Text("<id@x>\r\nBcc: evil@example.com".into())),
    ];
    let (raw, _) = rebuilt(&props, &[]).unwrap();
    let lines = clean_header(&raw);
    assert!(!lines.iter().any(|l| l.to_ascii_lowercase().starts_with("bcc")), "{lines:#?}");
    let index_line = lines.iter().find(|l| l.starts_with("Thread-Index: ")).unwrap();
    let kept = crate::ews::unb64(&index_line["Thread-Index: ".len()..]).unwrap();
    assert_eq!(kept.len(), 22 + 5 * 50, "whole blocks, at most 50");
    assert_eq!(kept[..], index[..kept.len()]);
    assert!(lines.contains(&"Sensitivity: Company-Confidential".to_string()), "{lines:#?}");
    let m = mailparse_header(&raw, "Thread-Topic");
    assert_eq!(m, "Plan Bcc: evil@example.com for the café");
    // A long topic is cut short, and plain ASCII stays as it is.
    let long = "x".repeat(2000);
    let (raw, _) = rebuilt(&[(0x0070, Val::Text(long.as_str().into()))], &[]).unwrap();
    assert!(clean_header(&raw).contains(&format!("Thread-Topic: {}", "x".repeat(MAX_TOPIC))));
    // Nothing but spaces: no header.
    let (raw, _) = rebuilt(&[(0x0070, Val::Text("\r\n \t".into())), (0x0036, Val::Int(9))], &[]).unwrap();
    assert!(!clean_header(&raw).iter().any(|l| l.starts_with("Thread-Topic") || l.starts_with("Sensitivity")));
}

/// Names, addresses and ids from a header-less item stay inside their own header lines: a Bcc
/// name with a line break (from the recipient's one-off entry id or its name), an address with
/// one, ids too long for a line.
#[test]
fn names_addresses_and_ids_stay_inside_their_lines() {
    let entry = one_off("x\r\nX-Injected: 1", "SMTP", "a@example.com", true);
    let hidden = HashMap::from([(0x3001, Val::Text("".into())), (0x0FFF, Val::Bin(&entry)), (0x0C15, Val::Int(3))]);
    let named =
        HashMap::from([(0x3001, Val::Text("Bo\r\nX-Injected: 2".into())), (0x39FE, Val::Text("bo@example.com".into())), (0x0C15, Val::Int(3))]);
    let quoted = HashMap::from([(0x3001, Val::Text("Park, Zoe \"Z\"".into())), (0x39FE, Val::Text("zoe@example.com".into())), (0x0C15, Val::Int(3))]);
    let broken = HashMap::from([(0x39FE, Val::Text("c@example.com\r\nX-Injected: 3".into())), (0x0C15, Val::Int(3))]);
    let to = HashMap::from([(0x3001, Val::Text("To\r\nX-Injected: 4".into())), (0x39FE, Val::Text("to@example.com".into()))]);
    let long = format!("<{}>", "a".repeat(2000));
    let references = format!("{long} <r@x>");
    let props: HashMap<u16, Val<'_>> = HashMap::from([
        (0x001A, Val::Text("IPM.Note".into())),
        (0x0037, Val::Text("Hi\u{1}\r\nX-Injected: 5".into())),
        (0x0C1A, Val::Text("Me\r\nX-Injected: 6".into())),
        (0x5D01, Val::Text("me@example.com\r\nX-Injected: 7".into())),
        (0x0065, Val::Text("me@example.com".into())),
        (0x1035, Val::Text(long.as_str().into())),
        (0x1042, Val::Text(long.as_str().into())),
        (0x1039, Val::Text(references.as_str().into())),
    ]);
    fn get<'a>(m: &HashMap<u16, Val<'a>>, id: u16) -> Option<Val<'a>> {
        m.get(&id).cloned()
    }
    let rows = [&hidden, &named, &quoted, &broken, &to].map(|r| move |id| get(r, id));
    let it = item_of(&|id| get(&props, id), &rows, Vec::new(), &crate::pst_pim::Named::default());
    assert_eq!(it.bcc.len(), 3, "an address with a line break is none");
    let raw = to_raw(it, "f@x").unwrap();
    let lines = clean_header(&raw);
    assert!(!lines.iter().any(|l| l.to_ascii_lowercase().starts_with("x-injected")), "{lines:#?}");
    let bcc: Vec<&String> = lines.iter().skip_while(|l| !l.starts_with("Bcc: ")).take(3).collect();
    assert_eq!(bcc, ["Bcc: \"x X-Injected: 1\" <a@example.com>,", " \"Bo X-Injected: 2\" <bo@example.com>,", " \"Park, Zoe 'Z'\" <zoe@example.com>"]);
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.message_id.len(), MAX_ID, "{}", p.message_id);
    assert_eq!((p.from.email.as_str(), p.from.name.as_str()), ("me@example.com", "Me X-Injected: 6"));
    // A blank line in an original header doesn't end the message's header there.
    let it = Item { headers: "From: a@example.com\r\n\r\nX-Later: 1\r\n".into(), thread_topic: "T".into(), ..Default::default() };
    let head = String::from_utf8_lossy(&to_raw(it, "f@x").unwrap()).split("\r\n\r\n").next().unwrap_or_default().to_string();
    assert!(head.contains("X-Later: 1") && head.contains("Thread-Topic: T") && head.contains("Content-Type"), "{head}");
}

/// A header's value as a mail reader decodes it (read as a subject).
fn mailparse_header(raw: &[u8], name: &str) -> String {
    let head = String::from_utf8_lossy(raw);
    let start = head.find(&format!("\r\n{name}: ")).unwrap() + 2 + name.len();
    let field: String = head[start..].split("\r\n").enumerate().take_while(|(i, l)| *i == 0 || l.starts_with(' ')).map(|(_, l)| l).collect();
    mime::parse(format!("Subject{field}\r\n\r\n").as_bytes()).unwrap().subject
}

/// An item with its own header gets only the lines that header lacks.
#[test]
fn an_original_header_gets_only_what_it_lacks() {
    let it = || Item {
        subject: "Hello".into(),
        from: EmailAddress::new("Ana", "ana@example.com"),
        date: 1_704_067_200,
        body: "Hi".into(),
        headers: "From: ana@example.com\r\nSubject: Hello\r\nthread-topic: Theirs\r\n".into(),
        thread_index: vec![1; 22],
        thread_topic: "Ours".into(),
        sensitivity: 2,
        ..Default::default()
    };
    let raw = to_raw(it(), "f@x").unwrap();
    let lines = clean_header(&raw);
    let named = |n: &str| lines.iter().filter(|l| l.to_ascii_lowercase().starts_with(&n.to_ascii_lowercase())).count();
    assert_eq!((named("thread-topic:"), named("thread-index:"), named("sensitivity:")), (1, 1, 1), "{lines:#?}");
    assert!(lines.contains(&"thread-topic: Theirs".to_string()));
    assert!(lines.contains(&"Sensitivity: Private".to_string()));
    // A signature's folded list of names is no line of those names.
    let folded = "DKIM-Signature: v=1; h=From:To:Subject:\r\n Thread-Index:Thread-Topic:Date; b=x\r\nSubject: s\r\n";
    let raw = to_raw(Item { headers: folded.into(), ..it() }, "f@x").unwrap();
    let lines = clean_header(&raw);
    assert!(lines.contains(&"Thread-Topic: Ours".to_string()) && lines.iter().any(|l| l.starts_with("Thread-Index: ")), "{lines:#?}");
}

/// An unusual address from the file that is one still comes in as the file has it: a quoted
/// local part with a space, a domain ending in a dot.
#[test]
fn an_unusual_address_still_comes_in() {
    let quoted = HashMap::from([(0x39FE, Val::Text("\"a b\"@x.example".into()))]);
    let dotted = HashMap::from([(0x3003, Val::Text("first.last+tag@x.example.".into())), (0x0C15, Val::Int(2))]);
    let props: HashMap<u16, Val<'static>> = HashMap::from([(0x001A, Val::Text("IPM.Note".into()))]);
    fn get<'a>(m: &HashMap<u16, Val<'a>>, id: u16) -> Option<Val<'a>> {
        m.get(&id).cloned()
    }
    let rows = [&quoted, &dotted].map(|r| move |id| get(r, id));
    let it = item_of(&|id| get(&props, id), &rows, Vec::new(), &crate::pst_pim::Named::default());
    assert_eq!(it.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["\"a b\"@x.example"]);
    assert_eq!(it.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["first.last+tag@x.example."]);
    let raw = to_raw(Item { subject: "s".into(), body: "b".into(), date: 1_704_067_200, ..it }, "f@x").unwrap();
    let lines = clean_header(&raw);
    assert!(lines.iter().any(|l| l.contains("\"a b\"@x.example")) && lines.iter().any(|l| l.contains("first.last+tag@x.example.")), "{lines:#?}");
}

#[test]
fn original_headers_are_kept_over_the_rebuilt_body() {
    let it = Item {
        subject: "Hello".into(),
        from: EmailAddress::new("Ana", "ana@example.com"),
        to: vec![EmailAddress::new("", "me@example.com")],
        date: 1_704_067_200,
        body: "Hi there".into(),
        message_id: "<orig@example.com>".into(),
        headers: "Received: from x\r\n by y\r\nFrom: Ana <ana@example.com>\r\nTo: me@example.com\r\nSubject: Hello\r\nList-Id: <l.example.com>\r\nContent-Type: multipart/mixed;\r\n boundary=zzz\r\nMIME-Version: 1.0\r\n"
            .into(),
        ..Default::default()
    };
    let raw = to_raw(it, "fallback@x").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("List-Id: <l.example.com>"));
    assert!(text.contains("Received: from x\r\n by y"));
    assert!(!text.contains("boundary=zzz"), "the original body's description goes");
    assert_eq!(text.matches("MIME-Version").count(), 1);
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.subject, "Hello");
    assert!(p.text.contains("Hi there"));
}

#[test]
fn a_sent_item_keeps_its_bcc_and_gets_an_id() {
    let it = Item {
        subject: "Plans".into(),
        from: EmailAddress::new("Me", "me@example.com"),
        bcc: vec![EmailAddress::new("", "secret@example.com")],
        date: 1_704_067_200,
        body: "Hi".into(),
        ..Default::default()
    };
    let raw = to_raw(it, "fallback@x").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.starts_with("Bcc: secret@example.com\r\n"));
    assert!(text.contains("<fallback@x>"));
}
