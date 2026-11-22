// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: a contact kept on a server, edited here.

use quatrelle_model::pim::{PIM_DOC, Pim, StoredCard};
use quatrelle_model::vcard;

use super::*;

/// A contact as Exchange's address book, or a CardDAV server, keeps it: its name whole (a middle
/// name, a prefix and a suffix), and its numbers with a group, several types and a preference.
const SERVER_CARD: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:zoe-server\r\nFN:Dr. Zoe Marie Park Jr.\r\nN:Park;Zoe;Marie;Dr.;Jr.\r\n\
item1.TEL;TYPE=WORK,FAX;PREF=1:+1 555 0100\r\nitem1.X-ABLabel:Studio fax\r\n\
EMAIL;TYPE=INTERNET,WORK;PREF=1:zoe@studio.example\r\nTEL;VALUE=uri;TYPE=cell:tel:+49-151-234\r\nEND:VCARD\r\n";

/// [`SERVER_CARD`] as this engine records it once its work number is changed from the form: the
/// network side's test of the same name starts from this very text (no crate sees both the engine
/// and the fake server, so the document the store keeps is the joint).
const EDITED_CARD: &str = "BEGIN:VCARD\r\nVERSION:4.0\r\nPRODID:-//Quatrelle//Quatrelle//EN\r\nUID:zoe-server\r\nFN:Dr. Zoe Marie Park Jr.\r\n\
N:Park;Zoe;Marie;Dr.;Jr.\r\nEMAIL;TYPE=INTERNET,WORK;PREF=1:zoe@studio.example\r\nTEL;TYPE=work:+1 555 0199\r\n\
TEL;VALUE=uri;TYPE=cell:tel:+49-151-234\r\nitem1.X-ABLabel:Studio fax\r\nEND:VCARD\r\n";

/// What the contact form sends when it saves the card with its first number changed: every field,
/// the name as shown, the addresses as text and the numbers with the kind of the place they are in.
fn form(number: &str) -> Value {
    json!({
        "uid": "zoe-server",
        "name": "Dr. Zoe Marie Park Jr.",
        "emails": ["zoe@studio.example"],
        "phones": [{"value": number, "kind": "work"}, {"value": "+49-151-234", "kind": "cell"}],
        "addresses": [],
        "birthday": "", "org": "", "department": "", "office": "", "title": "", "profession": "", "manager": "", "assistant": "",
        "web": "", "im": "", "nickname": "", "spouse": "", "anniversary": "", "note": "",
    })
}

/// An Exchange account holding the contact its server kept ([`SERVER_CARD`]).
fn account_with_server_card(name: &str) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let card = vcard::parse(SERVER_CARD).remove(0);
    s.pim_mut(&id).cards.push(StoredCard { book: "ews:contacts".into(), href: "contact:C1".into(), etag: "k".into(), card, dirty: false });
    (s, id)
}

/// The probe of the report, its engine half: a server's contact saved from the contact form with
/// a number changed keeps its first and family names, its middle name, prefix and suffix, and the
/// lines of what was not changed. The form sends the name shown, and that cleared the names it
/// is made of (the card then went to the server with `Surname` and `GivenName` empty).
#[test]
fn a_server_contact_changed_from_the_form_is_kept_with_its_names() {
    let (mut s, id) = account_with_server_card("server-card-form");
    s.run("contact.update", &form("+1 555 0199")).unwrap();
    // What the store keeps for the worker: the account's contacts and calendars document.
    let saved: Pim = s.store.as_ref().unwrap().load_account_doc(&id, PIM_DOC).unwrap();
    let kept = saved.cards.iter().find(|k| k.card.uid == "zoe-server").unwrap();
    assert!(kept.dirty, "to go to the server");
    assert_eq!(kept.href, "contact:C1", "the same item there");
    assert_eq!((kept.card.given.as_str(), kept.card.family.as_str()), ("Zoe", "Park"));
    assert_eq!(kept.card.name_rest, "Marie;Dr.;Jr.");
    assert_eq!(vcard::write(&kept.card), EDITED_CARD);
    done(s);
}

/// The same card saved from the form with nothing changed is written as it was read, except
/// that vCard 4 is what Quatrelle writes: its lines keep their group, types and preference.
#[test]
fn a_server_contact_saved_from_the_form_keeps_what_it_had() {
    let (mut s, id) = account_with_server_card("server-card-same");
    s.run("contact.update", &form("+1 555 0100")).unwrap();
    let c = &s.pim[&id].cards.iter().find(|k| k.card.uid == "zoe-server").unwrap().card;
    let out = vcard::write(c);
    for line in [
        "FN:Dr. Zoe Marie Park Jr.",
        "N:Park;Zoe;Marie;Dr.;Jr.",
        "item1.TEL;TYPE=WORK,FAX;PREF=1:+1 555 0100",
        "item1.X-ABLabel:Studio fax",
        "EMAIL;TYPE=INTERNET,WORK;PREF=1:zoe@studio.example",
        "TEL;VALUE=uri;TYPE=cell:tel:+49-151-234",
    ] {
        assert!(out.lines().any(|l| l == line), "{line}:\n{out}");
    }
    // Renamed, the old name's parts go with it (the contact is not Zoe Park any more).
    s.run("contact.update", &json!({"uid": "zoe-server", "name": "Zoe Park"})).unwrap();
    let c = &s.pim[&id].cards.iter().find(|k| k.card.uid == "zoe-server").unwrap().card;
    assert!(c.given.is_empty() && c.family.is_empty() && c.name_rest.is_empty(), "{c:?}");
    assert!(vcard::write(c).lines().any(|l| l == "N:;;;;"), "{}", vcard::write(c));
    done(s);
}
