//! A message's text first over EWS: what's asked of the server, what it answers, and which
//! requests a real Exchange would refuse.

use quatrelle_mime as mime;
use quatrelle_model::MessageKey;

use super::super::server::Server;
use super::*;
use crate::ews::{Client, fetch_body, fetch_text, sync_folder};
use crate::jmap::{State, VALIDITY};

/// The attached file's size.
const FILE: usize = 30_000;

/// A message with a text, and a file of [`FILE`] bytes.
fn with_a_file(id: &str) -> Vec<u8> {
    let bytes = crate::imap::base64(&vec![7u8; FILE]);
    format!(
        "From: Zoe Park <zoe@contoso.example>\r\nTo: Riley <riley@contoso.example>\r\nSubject: Reel\r\nMessage-ID: <{id}@x>\r\nImportance: high\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
         --b\r\nContent-Type: text/plain\r\n\r\nHere's the reel.\r\n\
         --b\r\nContent-Type: video/quicktime\r\nContent-Disposition: attachment; filename=reel.mov\r\nContent-Transfer-Encoding: base64\r\n\r\n{bytes}\r\n--b--\r\n"
    )
    .into_bytes()
}

fn connect(server: &Server) -> Client {
    Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap()
}

fn one_message(raw: &[u8]) -> (Data, String) {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let id = d.deliver(&inbox, raw, false, "2026-10-30T08:00:00Z");
    (d, id)
}

fn get_item(field: &str, id: &str) -> String {
    format!(
        "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties><t:FieldURI FieldURI=\"{field}\"/></t:AdditionalProperties></m:ItemShape>\
         <m:ItemIds><t:ItemId Id=\"{id}\"/></m:ItemIds></m:GetItem>"
    )
}

/// The name upstream asked for: Exchange checks every field against its schema, and one it
/// doesn't know refuses the whole request.
#[test]
fn exchange_refuses_a_field_it_does_not_know() {
    let (d, id) = one_message(&mail("Reel", "m1"));
    let server = serve(d);
    let c = connect(&server);
    let refused = c.call_one(&get_item("message:InReplyTo", &id)).unwrap_err();
    assert!(refused.to_string().contains("ErrorSchemaValidation"), "{refused}");
    c.call_one(&get_item("item:InReplyTo", &id)).unwrap();
    c.call_one(&get_item("calendar:NotChecked", &id)).unwrap();
}

#[test]
fn a_message_text_from_the_server() {
    let (d, id) = one_message(&with_a_file("m1"));
    let server = serve(d);
    let c = connect(&server);
    let raw = c.download_text(&id).unwrap().unwrap();
    let p = mime::parse_text_first(&raw).unwrap();
    assert_eq!(p.subject, "Reel");
    assert_eq!(p.to, [quatrelle_model::EmailAddress::new("Riley", "riley@contoso.example")]);
    assert_eq!(p.from.email, "zoe@contoso.example");
    assert!(p.text.contains("Here's the reel."), "{}", p.text);
    assert_eq!(p.attachments.len(), 1);
    assert_eq!((p.attachments[0].name.as_str(), p.attachments[0].size), ("reel.mov", FILE));
    assert_eq!(mime::attachment_data(&raw, 0).unwrap().len(), 0, "listed empty");
    assert!(String::from_utf8_lossy(&raw).contains("Importance: high"));
    let shapes = || server.data.lock().unwrap().item_shapes.clone();
    assert_eq!(shapes(), ["IdOnly"]);
    c.download(&id).unwrap();
    assert_eq!(shapes(), ["IdOnly", "MimeContent"], "the source is asked for otherwise");
}

/// `HasAttachments` says so on the list, which is how a message gets here at all.
#[test]
fn the_list_says_a_message_has_files() {
    let (mut d, _) = one_message(&with_a_file("m1"));
    let inbox = d.by_name("Inbox").unwrap().id.clone();
    d.deliver(&inbox, &mail("Plain", "m2"), false, "2026-10-29T08:00:00Z");
    let server = serve(d);
    let (store, ..) = synced(&connect(&server), "ews-text-first-list");
    let list = store.load_folder("a1", "INBOX").unwrap().messages;
    let has = |subject: &str| list.iter().find(|m| m.subject == subject).unwrap().has_attachments;
    assert!(has("Reel") && !has("Plain"));
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn the_class_the_server_gives_decides() {
    let (mut d, id) = one_message(&with_a_file("m1"));
    d.items[0].class = "IPM.Note.SMIME".into();
    let server = serve(d);
    assert!(connect(&server).download_text(&id).unwrap().is_none());
}

/// The two ways the whole message and the text can part: each fails on its own.
#[test]
fn the_text_and_the_source_fail_apart() {
    let (mut d, id) = one_message(&with_a_file("m1"));
    d.refuse_text = true;
    let server = serve(d);
    let c = connect(&server);
    let refused = c.download_text(&id).unwrap_err();
    assert!(refused.to_string().contains("ErrorSchemaValidation"), "{refused}");
    assert!(!c.download(&id).unwrap().is_empty(), "refused for the body it names, not for the source");
    {
        let mut d = server.data.lock().unwrap();
        d.refuse_text = false;
        d.unreadable.push(id.clone());
    }
    assert!(c.download(&id).is_err());
    assert!(c.download_text(&id).unwrap().is_some(), "a source it can't give isn't the text");
}

/// A store and a numbering that know the Inbox, and the key of its newest message.
fn synced(c: &Client, name: &str) -> (Store, State, MessageKey) {
    let mut st = State::default();
    crate::ews::list_folders(c, &mut st, "a1").unwrap();
    let store = crate::fake::tests_store(name);
    sync_folder(c, &mut st, &store, "a1", "INBOX", &crate::sync::SyncOptions { prefetch: 0, full: true, ..Default::default() }).unwrap();
    let key = store.load_folder("a1", "INBOX").unwrap().messages[0].key.clone();
    (store, st, key)
}

#[test]
fn the_text_of_a_listed_message_and_its_id() {
    let (d, id) = one_message(&with_a_file("m1"));
    let server = serve(d);
    let c = connect(&server);
    let (store, mut st, key) = synced(&c, "ews-text-first-id");
    let (raw, found) = fetch_text(&c, &mut st, &store, &key).unwrap().unwrap();
    assert_eq!(found, id);
    assert_eq!(mime::parse_text_first(&raw).unwrap().attachments.len(), 1);
    assert!(!store.has_body(&key), "the text isn't the message: nothing kept");
    let _ = std::fs::remove_dir_all(store.root());
}

/// As [`fetch_body`]: a message the numbering lost is found again by its Message-ID.
#[test]
fn the_text_of_a_message_the_numbering_lost() {
    let (d, id) = one_message(&with_a_file("m1"));
    let server = serve(d);
    let c = connect(&server);
    let (store, st, key) = synced(&c, "ews-text-first-lost");
    let mut lost = State::default();
    lost.mailboxes = st.mailboxes.clone();
    let (_, found) = fetch_text(&c, &mut lost, &store, &key).unwrap().unwrap();
    assert_eq!(found, id);
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn no_text_for_what_is_here_or_out_of_reach() {
    let (d, _) = one_message(&with_a_file("m1"));
    let server = serve(d);
    let c = connect(&server);
    let (store, mut st, key) = synced(&c, "ews-text-first-none");
    let asked = || server.data.lock().unwrap().item_shapes.len();
    let before = asked();
    let other = MessageKey { validity: VALIDITY + 1, ..key.clone() };
    assert!(fetch_text(&c, &mut st, &store, &other).unwrap().is_none(), "another numbering");
    fetch_body(&c, &mut st, &store, &key).unwrap();
    let whole = asked();
    assert_eq!(whole, before + 1);
    assert!(fetch_text(&c, &mut st, &store, &key).unwrap().is_none(), "it's here");
    assert_eq!(asked(), whole, "nothing asked of the server");
    let mut gone = State::default();
    gone.mailboxes = st.mailboxes.clone();
    let unknown = MessageKey { uid: 999, ..key.clone() };
    assert!(fetch_text(&c, &mut gone, &store, &unknown).unwrap().is_none(), "not found: left to the whole path to say so");
    let _ = std::fs::remove_dir_all(store.root());
}
