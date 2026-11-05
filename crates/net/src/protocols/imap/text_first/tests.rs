// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The text first over IMAP against canned servers: what's asked, what's built, what's refused.

use super::super::tests::Script;
use super::*;

/// The client after a greeting, the server then answering `answers`.
fn client(answers: &str) -> Client<Script> {
    Client::from_stream(Script::new(format!("* OK ready\r\n{answers}").as_bytes())).unwrap()
}

fn sent(c: Client<Script>) -> String {
    String::from_utf8_lossy(&c.conn.into_inner().written).into_owned()
}

/// A signed or encrypted message never comes as its text first (only its whole source shows it
/// and checks it); nor one of a single part. Neither asks for more than its structure.
#[test]
fn signed_and_single_part_mail_comes_whole() {
    for structure in [
        r#"(("TEXT" "PLAIN" NIL NIL NIL "7BIT" 10 1)("APPLICATION" "PKCS7-SIGNATURE" NIL NIL NIL "BASE64" 900000) "SIGNED" NIL NIL NIL)"#,
        r#"("APPLICATION" "PKCS7-MIME" ("SMIME-TYPE" "enveloped-data") NIL NIL "BASE64" 900000)"#,
        r#"(("TEXT" "PLAIN" NIL NIL NIL "7BIT" 10 1)("APPLICATION" "PKCS7-MIME" NIL NIL NIL "BASE64" 900000) "MIXED" NIL NIL NIL)"#,
        r#"("TEXT" "PLAIN" NIL NIL NIL "7BIT" 900000 1)"#,
    ] {
        let mut c = client(&format!("* 1 FETCH (UID 1 BODYSTRUCTURE {structure})\r\nM1 OK done\r\n"));
        assert_eq!(c.text_first(1).unwrap(), None, "{structure}");
        let sent = sent(c);
        assert!(!sent.contains("BODY.PEEK"), "{sent}");
    }
}

const HEADER: &str = "From: Sam <sam@studio.example>\r\nTo: me@studio.example\r\nSubject: Reel\r\nMessage-ID: <r@x>\r\nImportance: high\r\n\r\n";

/// The structure of a text in quoted-printable Latin-1 and a file named the RFC 2231 way.
const STRUCTURE: &str = r#"(("TEXT" "PLAIN" ("CHARSET" "iso-8859-1") NIL NIL "QUOTED-PRINTABLE" 22 1 NIL NIL NIL NIL)("VIDEO" "QUICKTIME" ("NAME*" "utf-8''r%C3%A9el.mov") NIL NIL "BASE64" 4000000 NIL ("ATTACHMENT" ("FILENAME*0*" "utf-8''r%C3%A9" "FILENAME*1" "el.mov")) NIL NIL) "MIXED" ("BOUNDARY" "b") NIL NIL NIL)"#;

/// Its text decoded as its structure says (the server's own part header isn't asked for), its
/// file named and sized from the structure; the header kept.
#[test]
fn the_text_and_the_files_from_the_structure() {
    let text = "Caf=E9 at n=F6on.\r\n";
    let answers = format!(
        "* 1 FETCH (UID 1 BODYSTRUCTURE {STRUCTURE})\r\nM1 OK done\r\n\
         * 1 FETCH (UID 1 BODY[HEADER] {{{}}}\r\n{HEADER} BODY[1] {{{}}}\r\n{text})\r\nM2 OK done\r\n",
        HEADER.len(),
        text.len()
    );
    let mut c = client(&answers);
    let raw = c.text_first(1).unwrap().expect("its text first");
    let sent = sent(c);
    assert!(sent.contains("UID FETCH 1 (UID BODY.PEEK[HEADER] BODY.PEEK[1])") && !sent.contains(".MIME"), "{sent}");
    let p = mime::parse_text_first(&raw).unwrap();
    assert_eq!((p.subject.as_str(), p.from.email.as_str(), p.message_id.as_str()), ("Reel", "sam@studio.example", "r@x"));
    assert!(p.text.contains("Café at nöon."), "{}", p.text);
    let files: Vec<(&str, usize)> = p.attachments.iter().map(|a| (a.name.as_str(), a.size)).collect();
    assert_eq!(files, [("réel.mov", 3_000_000)]);
    assert!(String::from_utf8_lossy(&raw).contains("Importance: high"));
}

/// The text asked for doesn't come (a server that skips a section): the message comes whole.
#[test]
fn a_text_that_does_not_come_leaves_it_whole() {
    let answers = format!(
        "* 1 FETCH (UID 1 BODYSTRUCTURE {STRUCTURE})\r\nM1 OK done\r\n* 1 FETCH (UID 1 BODY[HEADER] {{{}}}\r\n{HEADER})\r\nM2 OK done\r\n",
        HEADER.len()
    );
    assert_eq!(client(&answers).text_first(1).unwrap(), None);
}

/// A part whose `BODYSTRUCTURE` nests too deep, or a message of too many parts: whole.
#[test]
fn too_deep_or_too_many_parts_come_whole() {
    let leaf = r#"("TEXT" "PLAIN" NIL NIL NIL "7BIT" 10 1)("APPLICATION" "PDF" NIL NIL NIL "BASE64" 9000)"#;
    let mut deep = leaf.to_string();
    for _ in 0..(MAX_DEPTH + 2) {
        deep = format!("({deep} \"MIXED\")");
    }
    let many = format!("({} \"MIXED\")", leaf.repeat(MAX_PARTS));
    for structure in [deep, many] {
        let mut c = client(&format!("* 1 FETCH (UID 1 BODYSTRUCTURE {structure})\r\nM1 OK done\r\n"));
        assert_eq!(c.text_first(1).unwrap(), None);
        assert!(!sent(c).contains("BODY.PEEK"));
    }
}

/// What the structure names can't add a header to the text part's: its type, character set and
/// encoding are kept to plain tokens, the header two lines whatever the server says.
#[test]
fn a_hostile_structure_adds_no_header() {
    use std::borrow::Cow;
    let evil = "utf-8\"\r\nX-Evil: 2";
    let ty = imap_proto::ContentType {
        ty: Cow::Borrowed("TEXT"),
        subtype: Cow::Borrowed("PLAIN\r\nX-Evil: 1"),
        params: Some(vec![(Cow::Borrowed("CHARSET"), Cow::Borrowed(evil))]),
    };
    let head = String::from_utf8(text_header("plain\r\nx-evil: 1", &ty, &imap_proto::ContentEncoding::Other(Cow::Borrowed(evil)))).unwrap();
    assert_eq!(head.matches("\r\n").count(), 2, "{head}");
    assert!(!head.contains("\nX-Evil") && !head.contains('"') || head.matches('"').count() == 2, "{head}");
    assert_eq!(token("plain\r\nX-Evil: 1"), "plainX-Evil:1");
    assert_eq!(token(&"a".repeat(200)).len(), 64);
}
