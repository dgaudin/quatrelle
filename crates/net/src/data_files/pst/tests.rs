// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Tests of the Outlook data file import.

use super::item::{File, item_of};
use super::props::{decode, one_off_address, val};
use super::rebuild::to_raw;
use super::*;

std::thread_local! {
    /// The folder whose items make the import panic as they're read, in this test's thread.
    static FAULT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// The reader failing (a panic) as the items of the folder named `name` are read.
pub(super) fn fault(name: &str) {
    if FAULT.with(|f| f.borrow().as_deref() == Some(name)) {
        panic!("a test's fault in {name}");
    }
}

/// Reading the folder named `name` panics from now on (`None`: nothing does).
fn fail_on(name: Option<&str>) {
    FAULT.with(|f| *f.borrow_mut() = name.map(str::to_string));
}

/// A one-off entry id holding `name`, `kind` and `address`, in UTF-16 or 8-bit.
fn one_off(name: &str, kind: &str, address: &str, unicode: bool) -> Vec<u8> {
    let mut b = vec![0, 0, 0, 0, 0x81, 0x2B, 0x1F, 0xA4, 0xBE, 0xA3, 0x10, 0x19, 0x9D, 0x6E, 0x00, 0xDD, 0x01, 0x0F, 0x54, 0x02, 0, 0];
    b.extend_from_slice(&(if unicode { 0x8000u16 } else { 0 }).to_le_bytes());
    for s in [name, kind, address] {
        if unicode {
            b.extend(s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        } else {
            b.extend(s.bytes().chain([0]));
        }
    }
    b
}

/// A recipient with no address but in its one-off entry id still comes in, by that address;
/// anything else in that property is no address.
#[test]
fn a_recipient_s_address_from_its_one_off_entry_id() {
    let got = |b: &[u8]| one_off_address(b);
    let pair = |n: &str, a: &str| Some((n.to_string(), a.to_string()));
    assert_eq!(got(&one_off("Cy Lima", "SMTP", "cy@example.com", true)), pair("Cy Lima", "cy@example.com"));
    assert_eq!(got(&one_off("Dee", "SMTP", "dee@example.com", false)), pair("Dee", "dee@example.com"));
    assert_eq!(got(&one_off("Ex", "EX", "/o=Org/cn=Recipients/cn=ex", true)), None, "a directory path is no address");
    let good = one_off("Cy", "SMTP", "cy@example.com", true);
    for n in 0..good.len() {
        let _ = got(&good[..n]);
    }
    // No NUL at the end: the address runs to the end.
    assert_eq!(got(&good[..good.len() - 2]), pair("Cy", "cy@example.com"));
    let mut other = good.clone();
    other[4] = 0;
    assert_eq!(got(&other), None, "another provider");
    let mut big = good.clone();
    big.resize(2000, b'a');
    assert_eq!(got(&big), None, "too long");

    let props: HashMap<u16, Val<'static>> = HashMap::from([(0x001A, Val::Text("IPM.Note".into()))]);
    let entry = one_off("Cy Lima", "SMTP", "cy@example.com", true);
    let cy = HashMap::from([(0x3001, Val::Text("".into())), (0x0FFF, Val::Bin(&entry)), (0x0C15, Val::Int(2))]);
    fn get<'a>(m: &HashMap<u16, Val<'a>>, id: u16) -> Option<Val<'a>> {
        m.get(&id).cloned()
    }
    let it = item_of(&|id| get(&props, id), &[|id| get(&cy, id)], Vec::new(), &crate::pst_pim::Named::default());
    assert_eq!(it.cc.iter().map(|a| (a.name.as_str(), a.email.as_str())).collect::<Vec<_>>(), [("Cy Lima", "cy@example.com")]);
}

/// Outlook's notes come in as notes: their text, colour and time; an empty one doesn't.
#[test]
fn notes_come_in_as_notes() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-notes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d1", &mut progress);
    let note = |body: &str, color: Option<u8>| Item {
        class: "IPM.StickyNote".into(),
        subject: body.lines().next().unwrap_or_default().into(),
        body: body.into(),
        date: 1_700_000_000,
        extra: Some(crate::pst_pim::Extra { note_color: color, modified: Some(1_700_000_500), ..Default::default() }),
        ..Default::default()
    };
    out.add_pim(note("Poster\r\nAsk about 170 gsm", Some(1)));
    out.add_pim(note("Plain", None));
    out.add_pim(note("  ", Some(2)));
    let done = out.finish();
    assert_eq!((done.notes, done.skipped), (2, 1));
    let pim: Pim = store.load_account_doc("d1", PIM_DOC).unwrap();
    let n = &pim.notes[0].note;
    assert_eq!((n.text.as_str(), n.color, n.modified), ("Poster\nAsk about 170 gsm", 1, 1_700_000_500));
    assert_eq!(pim.notes[1].note.color, NOTE_YELLOW, "no colour: Outlook's yellow");
    assert!(pim.notes.iter().all(|s| !s.dirty && s.href.is_empty()), "kept on this computer");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn filetime() {
    // 2024-01-01T00:00:00Z
    assert_eq!(val(&PropertyValue::Time(133_485_408_000_000_000), 0).time(), Some(1_704_067_200));
    assert_eq!(val(&PropertyValue::Time(0), 0).time(), None);
}

/// Only a 32-bit integer reads as a number, and a value of a type the import does not read is
/// still there, as it was before items were read through `Val`.
#[test]
fn only_a_32_bit_integer_is_a_number() {
    assert_eq!(val(&PropertyValue::Integer32(2), 0), Val::Int(2));
    for v in [PropertyValue::Integer16(2), PropertyValue::Integer64(2), PropertyValue::ErrorCode(2), PropertyValue::Null] {
        assert_eq!(val(&v, 0), Val::Other, "{v:?}");
    }
}

/// An item is read from its values alone: the subject's prefix marker goes, flags, time and
/// recipients come through, and an HTML body in bytes is read in its code page.
#[test]
fn an_item_reads_from_its_values() {
    let props: HashMap<u16, Val<'static>> = HashMap::from([
        (0x001A, Val::Text("IPM.Note".into())),
        (0x0037, Val::Text("\u{1}\u{5}FW: Hello".into())),
        (0x0C1A, Val::Text("Ana".into())),
        (0x0C1F, Val::Text("ana@example.com".into())),
        (0x0E06, Val::Time(133_485_408_000_000_000)),
        (0x0E07, Val::Int(0x1)),
        (0x1090, Val::Int(2)),
        (0x1081, Val::Int(0x0066)),
        (0x1013, Val::Bin(b"caf\xe9")),
        (0x3FDE, Val::Int(65001)),
    ]);
    let ben = HashMap::from([(0x3001, Val::Text("Ben".into())), (0x39FE, Val::Text("ben@example.com".into())), (0x0C15, Val::Int(2))]);
    let nobody = HashMap::from([(0x3001, Val::Text("No address".into()))]);
    let get = |m: &HashMap<u16, Val<'static>>, id: u16| m.get(&id).cloned();
    let recipients = [&ben, &nobody].map(|r| move |id| get(r, id));
    let it = item_of(&|id| get(&props, id), &recipients, Vec::new(), &crate::pst_pim::Named::default());
    assert_eq!(it.subject, "FW: Hello");
    assert_eq!((it.from.name.as_str(), it.from.email.as_str(), it.date), ("Ana", "ana@example.com", 1_704_067_200));
    assert!(it.flags.seen && !it.flags.draft && it.flags.flagged && it.flags.answered);
    assert_eq!(it.html.as_deref(), Some("caf\u{FFFD}"), "the HTML is read in its code page");
    assert_eq!(it.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ben@example.com"]);
    assert!(it.to.is_empty() && it.bcc.is_empty() && it.extra.is_none());
}

/// `PidTagRtfCompressed` stored as it is (`MELA`), as Outlook may keep a short body.
fn stored_rtf(rtf: &[u8]) -> Vec<u8> {
    let len = u32::try_from(rtf.len()).unwrap();
    let mut out = (len + 12).to_le_bytes().to_vec();
    out.extend(len.to_le_bytes());
    out.extend(0x414C_454Du32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(rtf);
    out
}

/// An item from its properties alone, with no recipients.
fn item_with<'a>(props: &HashMap<u16, Val<'a>>, files: Vec<File>) -> Item {
    let none: [fn(u16) -> Option<Val<'a>>; 0] = [];
    item_of(&|id| props.get(&id).cloned(), &none, files, &crate::pst_pim::Named::default())
}

/// A body Outlook kept only as RTF comes in as the HTML it wraps, and the picture that HTML
/// names by `cid:` goes inline instead of being a file.
#[test]
fn an_rtf_only_body_comes_in_with_its_pictures_inline() {
    let rtf = stored_rtf(
        br#"{\rtf1\ansi\ansicpg1252\fromhtml1 \deff0{\fonttbl{\f0\fswiss Calibri;}}{\*\htmltag19 <html>}{\*\htmltag34 <body>}{\*\htmltag <p>}\htmlrtf {\htmlrtf0 Hi Riley, the {\*\htmltag <b>}\htmlrtf\b\htmlrtf0 storyboard{\*\htmltag </b>}\htmlrtf\b0\htmlrtf0  is attached.{\*\htmltag <img src="cid:logo@studio.example">}\htmlrtf\par\htmlrtf0}{\*\htmltag </p></body></html>}}"#,
    );
    let props = HashMap::from([(0x001A, Val::Text("IPM.Note".into())), (0x0037, Val::Text("Storyboard".into())), (0x1009, Val::Bin(&rtf))]);
    let logo =
        File::Data { name: "logo.png".into(), mime: "image/png".into(), cid: "logo@studio.example".into(), data: b"\x89PNG\r\n\x1a\nfake".to_vec() };
    let it = item_with(&props, vec![logo]);
    let html = it.html.as_deref().unwrap_or_default();
    assert!(html.contains("<b>storyboard</b>") && html.contains("cid:logo@studio.example"), "{html}");
    let raw = to_raw(it, "rtf@x").unwrap();
    let p = mime::parse(&raw).unwrap();
    assert!(p.had_html && p.text.contains("storyboard"), "{}", p.text);
    let inline: Vec<_> = p.attachments.iter().map(|a| (a.inline, a.content_id.as_deref())).collect();
    assert_eq!(inline, [(true, Some("logo@studio.example"))], "the picture is inline");

    // Compressed (LZFu, MS-OXRTFCP 3.1.1): RTF of its own reads as simple HTML.
    let compressed = [
        0x2d, 0x00, 0x00, 0x00, 0x2b, 0x00, 0x00, 0x00, 0x4c, 0x5a, 0x46, 0x75, 0xf1, 0xc5, 0xc7, 0xa7, 0x03, 0x00, 0x0a, 0x00, 0x72, 0x63, 0x70,
        0x67, 0x31, 0x32, 0x35, 0x42, 0x32, 0x0a, 0xf3, 0x20, 0x68, 0x65, 0x6c, 0x09, 0x00, 0x20, 0x62, 0x77, 0x05, 0xb0, 0x6c, 0x64, 0x7d, 0x0a,
        0x80, 0x0f, 0xa0,
    ];
    let it = item_with(&HashMap::from([(0x1009, Val::Bin(&compressed[..]))]), Vec::new());
    assert!(it.html.as_deref().is_some_and(|h| h.contains("hello world")), "{:?}", it.html);
    let p = mime::parse(&to_raw(it, "lzfu@x").unwrap()).unwrap();
    assert!(p.text.contains("hello world"), "{}", p.text);
}

/// RTF fills in only what the item lacks: plain text keeps its place and gains the HTML the RTF
/// wraps, an HTML body wins over the RTF, wrapped text fills an empty body, and RTF that can't be
/// read leaves the item as it was.
#[test]
fn rtf_fills_in_only_what_is_missing() {
    let wrapped = stored_rtf(br#"{\rtf1\ansi\fromhtml1 {\*\htmltag <p>}\htmlrtf {\htmlrtf0 From the RTF}{\*\htmltag </p>}}"#);
    let with = |extra: &[(u16, Val<'static>)], rtf: &[u8]| {
        let mut props: HashMap<u16, Val<'_>> = extra.iter().cloned().collect();
        props.insert(0x1009, Val::Bin(rtf));
        item_with(&props, Vec::new())
    };
    let it = with(&[(0x1000, Val::Text("Plain text".into()))], &wrapped);
    assert_eq!(it.body, "Plain text");
    assert!(it.html.as_deref().is_some_and(|h| h.contains("From the RTF")), "{:?}", it.html);
    let it = with(&[(0x1000, Val::Text("Plain text".into())), (0x0E1F, Val::Bool(true))], &wrapped);
    assert!(it.html.as_deref().is_some_and(|h| h.contains("From the RTF")), "{:?}", it.html);
    // The plain text was changed after the RTF: the HTML the RTF wraps is older, and left out.
    let it = with(&[(0x1000, Val::Text("Plain text".into())), (0x0E1F, Val::Bool(false))], &wrapped);
    assert_eq!((it.body.as_str(), it.html.as_deref()), ("Plain text", None));
    let it = with(&[(0x0E1F, Val::Bool(false))], &wrapped);
    assert!(it.html.as_deref().is_some_and(|h| h.contains("From the RTF")), "the only body: {:?}", it.html);

    let it = with(&[(0x1013, Val::Text("<p>From the HTML</p>".into()))], &wrapped);
    assert_eq!(it.html.as_deref(), Some("<p>From the HTML</p>"));

    let text = stored_rtf(b"{\\rtf1\\ansi\\fromtext \\deff0{\\fonttbl{\\f0 Courier;}}Line one\\par\nLine\\tab two\\par}");
    let it = with(&[], &text);
    assert_eq!((it.body.as_str(), it.html.as_deref()), ("Line one\r\nLine\ttwo\r\n", None));
    let it = with(&[(0x1000, Val::Text("Kept".into()))], &text);
    assert_eq!(it.body, "Kept");

    // RTF of its own (an RTF-format mail, a meeting request) doesn't stand in for the plain text:
    // its simple HTML would run a table's cells together and drop its pictures and objects.
    let own = stored_rtf(br"{\rtf1\ansi\deff0 Name\cell Value\cell\row Alpha\cell Beta\cell\row Before{\pict\pngblip 89504e47}After\par}");
    let plain = "Name\tValue\r\nAlpha\tBeta\r\nBefore [picture] After";
    let it = with(&[(0x1000, Val::Text(plain.into()))], &own);
    assert_eq!((it.body.as_str(), it.html.as_deref()), (plain, None));
    let it = with(&[], &own);
    assert!(it.body.is_empty() && it.html.as_deref().is_some_and(|h| h.contains("Before")), "{:?}", it.html);

    for junk in [&b""[..], b"0123456789abcdefghij", &stored_rtf(b"not rtf at all"), &wrapped[..20]] {
        let it = with(&[], junk);
        assert!(it.body.is_empty() && it.html.is_none(), "{junk:?}");
        assert!(it.body_not_converted, "{junk:?}");
        assert!(to_raw(it, "junk@x").is_ok());
        let it = with(&[(0x1000, Val::Text("Plain text".into()))], junk);
        assert!(it.body == "Plain text" && !it.body_not_converted, "{junk:?}");
    }
    assert!(!with(&[], &wrapped).body_not_converted && !with(&[], &text).body_not_converted);
}

/// An item with no headers of its own keeps that it was forwarded, its importance and the ids
/// it answers, however the file wrote them.
#[test]
fn a_headerless_item_keeps_forwarded_importance_and_references() {
    let read = |props: &[(u16, Val<'static>)]| {
        let props: HashMap<u16, Val<'static>> = props.iter().cloned().collect();
        let none: [fn(u16) -> Option<Val<'static>>; 0] = [];
        item_of(&|id| props.get(&id).cloned(), &none, Vec::new(), &crate::pst_pim::Named::default())
    };
    // Forwarded by its last verb, or by its icon when it has none; a reply stays a reply.
    let it = read(&[(0x1081, Val::Int(0x0068))]);
    assert!(it.flags.forwarded && !it.flags.answered);
    assert!(read(&[(0x1080, Val::Int(0x0106))]).flags.forwarded);
    let reply = read(&[(0x1081, Val::Int(0x0066)), (0x1080, Val::Int(0x0106))]);
    assert!(reply.flags.answered && !reply.flags.forwarded);
    // Importance: low, normal, high.
    for (n, want) in [(0, -1), (1, 0), (2, 1)] {
        assert_eq!(read(&[(0x0017, Val::Int(n))]).importance, want, "{n}");
    }
    // References however they're separated.
    for refs in ["<a@x><b@x>", "<a@x>,<b@x>", "<a@x> , <b@x>", "a@x b@x", "a@x,b@x", " <a@x>\r\n\t<b@x> "] {
        let it = read(&[(0x1039, Val::Text(refs.into())), (0x1042, Val::Text(refs.into()))]);
        assert_eq!(it.references, ["a@x", "b@x"], "{refs:?}");
        assert_eq!(it.in_reply_to, "a@x", "{refs:?}");
    }
    let many = (0..300).map(|i| format!("<{i}@x>")).collect::<String>();
    let it = read(&[(0x1039, Val::Text(many.into()))]);
    assert_eq!((it.references.len(), it.references.first().map(String::as_str)), (256, Some("44@x")), "the latest are kept");

    // …and they reach the message and its summary.
    let it = Item { subject: "Fwd".into(), body: "x".into(), date: 1_704_067_200, ..read(&[(0x1081, Val::Int(0x0068)), (0x0017, Val::Int(2))]) };
    let raw = to_raw(Item { references: vec!["a@x".into(), "b@x".into()], ..it }, "id@x").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("References: <a@x> <b@x>"), "{text}");
    assert!(text.contains("Importance: high"), "{text}");
}

/// An item's categories, a property named by a string, come in as its keywords: once each,
/// none that is a system keyword, and only from a list of texts.
#[test]
fn categories_named_by_a_string_become_keywords() {
    let read = |v: Val<'static>| {
        let props: HashMap<u16, Val<'static>> = HashMap::from([(0x8010, v)]);
        let none: [fn(u16) -> Option<Val<'static>>; 0] = [];
        item_of(&|id| props.get(&id).cloned(), &none, Vec::new(), &crate::pst_pim::Named::with_keywords(0x8010)).flags.keywords
    };
    let names = ["Work", "To do", "work", " ", "$Junk"].map(String::from).to_vec();
    assert_eq!(read(Val::Texts(names)), ["work", "to_do", "cat_$junk"]);
    assert!(read(Val::Text("Work".into())).is_empty());
    assert_eq!(read(Val::Texts((0..100).map(|i| format!("c{i}")).collect())).len(), 64);
    // A file without the name knows no categories.
    let props: HashMap<u16, Val<'static>> = HashMap::from([(0x8010, Val::Texts(vec!["Work".into()]))]);
    let none: [fn(u16) -> Option<Val<'static>>; 0] = [];
    assert!(item_of(&|id| props.get(&id).cloned(), &none, Vec::new(), &crate::pst_pim::Named::default()).flags.keywords.is_empty());
}

#[test]
fn an_offline_copy_is_explained() {
    let dir = std::env::temp_dir().join(format!("quatrelle-ost-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("me@example.com.ost");
    std::fs::write(&file, b"!BDN\0\0\0\0SO$\0rest").unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    let e = import(&file, &store, "a1", &mut |_| {}).unwrap_err();
    assert!(e.contains(".ost") && e.contains("add the account"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn junk_is_refused_and_nothing_is_written() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-junk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("junk.pst");
    std::fs::write(&file, b"!BDN this is not really a data file").unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    assert!(import(&file, &store, "a1", &mut |_| {}).is_err());
    assert!(import(&dir.join("missing.pst"), &store, "a1", &mut |_| {}).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A contact's picture is one of its hidden attachments: taken when it is a picture of a
/// sane size, and never a visible file (an attachment the mail shows) or something else.
#[test]
fn a_contact_picture_is_a_hidden_picture() {
    let jpeg = [0xFFu8, 0xD8, 0xFF, 0xE0, 1, 2, 3];
    assert_eq!(
        crate::pst_pim::contact_photo(true, "", Some(&jpeg)),
        Some(("image/jpeg".to_string(), jpeg.to_vec())),
        "the bytes say it's a JPEG, as Outlook's ContactPicture.jpg is"
    );
    assert_eq!(
        crate::pst_pim::contact_photo(true, "image/png", Some(&jpeg)),
        Some(("image/png".to_string(), jpeg.to_vec())),
        "the item's own type wins"
    );
    assert_eq!(crate::pst_pim::contact_photo(false, "image/jpeg", Some(&jpeg)), None, "a visible attachment isn't the picture");
    assert_eq!(crate::pst_pim::contact_photo(true, "text/plain", Some(b"hello")), None, "not a picture");
    assert_eq!(crate::pst_pim::contact_photo(true, "", Some(b"")), None);
    assert_eq!(crate::pst_pim::contact_photo(true, "", None), None);
    // A picture bigger than a contact photo (512 KiB) is left out.
    let mut big = vec![0xFFu8, 0xD8, 0xFF, 0xE0];
    big.resize(quatrelle_model::vcard::MAX_PHOTO + 1, 0);
    assert_eq!(crate::pst_pim::contact_photo(true, "image/jpeg", Some(&big)), None);
}

#[test]
fn imports_a_data_file() {
    use outlook_pst::{UnicodePstAttachment, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Archive 2024.pst");
    let recipients = [
        UnicodePstRecipient { name: "Ana Ruiz", email: "ana@example.com", recipient_type: UnicodePstRecipientType::To, ..Default::default() },
        UnicodePstRecipient { name: "Ben", email: "ben@example.com", recipient_type: UnicodePstRecipientType::Cc, ..Default::default() },
    ];
    let message = UnicodePstMessage {
        subject: "Quarterly plan — café",
        sender_name: "Riley Moreno",
        sender_email: "riley@example.com",
        recipients: &recipients,
        body: "Here is the plan.",
        html_body: Some("<p>Here is <b>the plan</b>.</p><img src=\"cid:chart@example.com\">"),
        message_id: "<plan@example.com>",
        // 2024-01-01T00:00:00Z
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    let files = [
        UnicodePstAttachment {
            filename: "chart.png",
            mime_type: "image/png",
            content_id: Some("<chart@example.com>"),
            data: b"\x89PNG\r\n\x1a\nfake",
            ..Default::default()
        },
        UnicodePstAttachment { filename: "notes.txt", mime_type: "text/plain", content_id: None, data: b"remember the milk", ..Default::default() },
    ];
    drop(Writer::create_with_attachments(&file, &message, &files).unwrap());
    let nested = UnicodePstMessage { subject: "Old thread", message_id: "<old@example.com>", html_body: None, ..message };
    drop(Writer::append_in_folder(&file, &["Projects", "2024"], &nested).unwrap());

    let store = Store::open(dir.join("cache")).unwrap();
    let mut ticks = 0;
    let done = import(&file, &store, "a1", &mut |_| ticks += 1).unwrap();
    assert_eq!(done.messages, 2, "{done:?}");
    assert_eq!(done.skipped, 0);
    let inbox = done.folders.iter().find(|f| f.role == FolderRole::Inbox).unwrap();
    assert_eq!(inbox.path, "INBOX");
    let all = store.load_folders("a1").unwrap();
    assert_eq!(all, done.folders);
    let nested_folder = all.iter().find(|f| f.path == "Projects/2024").unwrap();
    assert_eq!(nested_folder.total, 1);
    assert!(all.iter().any(|f| f.path == "Projects"));

    // The message where the writer put it, with its addresses, body, picture and file.
    let (path, st) = all
        .iter()
        .filter(|f| f.path != "Projects/2024")
        .find_map(|f| store.load_folder("a1", &f.path).ok().filter(|s| !s.messages.is_empty()).map(|s| (f.path.clone(), s)))
        .unwrap();
    let s = &st.messages[0];
    assert_eq!(s.subject, "Quarterly plan — café");
    assert_eq!(s.date, 1_704_067_200);
    let key = MessageKey { account: "a1".into(), folder: path, validity: 1, uid: s.key.uid };
    let raw = store.read_body(&key).unwrap();
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.from.email, "riley@example.com");
    assert_eq!(p.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ana@example.com"]);
    assert_eq!(p.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ben@example.com"]);
    assert_eq!(p.message_id, "plan@example.com");
    assert!(p.had_html && p.text.contains("the plan"), "{}", p.text);
    let names: Vec<&str> = p.attachments.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"notes.txt"), "{names:?}");
    // The picture goes inline, under its own name.
    assert!(p.attachments.iter().any(|a| a.name == "chart.png" && a.inline), "{names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Outlook's tasks come in as tasks: their due day (local midnight), done or not, and those
/// without a due day too.
#[test]
fn tasks_come_in_as_tasks() {
    let dir = std::env::temp_dir().join(format!("quatrelle-pst-tasks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d1", &mut progress);
    // 27 October 2026 at local midnight in Paris (22:00 UTC the day before, summer time).
    let due = quatrelle_model::dates::days_from_civil(2026, 10, 27) * 86_400 - 7200;
    let task = |subject: &str, due: Option<i64>, done: bool| Item {
        class: "IPM.Task".into(),
        subject: subject.into(),
        body: "Relancer\r\n".into(),
        date: 1_700_000_000,
        extra: Some(crate::pst_pim::Extra { due, done, ..Default::default() }),
        ..Default::default()
    };
    out.add_pim(task("Devis Martin", Some(due), false));
    out.add_pim(task("Payer l'imprimeur", Some(due), true));
    out.add_pim(task("Sans date", None, false));
    let done = out.finish();
    assert_eq!((done.tasks, done.events, done.skipped), (3, 0, 0), "tasks, not calendar items");
    let pim: Pim = store.load_account_doc("d1", PIM_DOC).unwrap();
    let t = |i: usize| &pim.tasks[i].task;
    assert_eq!((t(0).subject.as_str(), t(0).due, t(0).notes.as_str()), ("Devis Martin", Some(due.div_euclid(86_400) + 1), "Relancer"));
    assert!(!t(0).is_complete() && t(1).is_complete() && t(1).percent == 100);
    assert_eq!(t(2).due, None);
    assert!(pim.tasks.iter().all(|s| !s.dirty && s.href.is_empty()), "kept on this computer");
    let _ = std::fs::remove_dir_all(&dir);
}

mod chunked;
mod code_pages;
mod embedded;
mod fidelity;
mod files;
mod headers;
mod losses;
mod open;
mod protected;
mod reader;
mod real_files;
mod rebuilt;
mod report;
mod seeds;
