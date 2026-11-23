// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A contact changed here, sent to Exchange: its whole name goes with it.

use quatrelle_model::pim::{Pim, StoredCard};
use quatrelle_model::vcard;

use super::*;

/// The contact as the engine records it once its work number is changed from the contact form:
/// the engine's test `a_server_contact_changed_from_the_form_is_kept_with_its_names` ends on this
/// very text (no crate sees both the engine and this server, so the document the store keeps is
/// the joint between the two halves).
const EDITED_CARD: &str = "BEGIN:VCARD\r\nVERSION:4.0\r\nPRODID:-//Quatrelle//Quatrelle//EN\r\nUID:zoe-server\r\nFN:Dr. Zoe Marie Park Jr.\r\n\
N:Park;Zoe;Marie;Dr.;Jr.\r\nEMAIL;TYPE=INTERNET,WORK;PREF=1:zoe@studio.example\r\nTEL;TYPE=work:+1 555 0199\r\n\
TEL;VALUE=uri;TYPE=cell:tel:+49-151-234\r\nitem1.X-ABLabel:Studio fax\r\nEND:VCARD\r\n";

/// The probe of the report, its network half: the account's contacts document carries the card
/// as the engine saved it, marked changed; the sync that sends it leaves the server with the
/// contact's first and family names, its middle name, suffix and prefix, not only the name
/// shown (`GivenName` and `Surname` were empty there).
#[test]
fn a_contact_changed_here_keeps_its_names_on_the_server() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let contacts = d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let there = parse(
        "<t:Contact><t:DisplayName>Dr. Zoe Marie Park Jr.</t:DisplayName><t:GivenName>Zoe</t:GivenName><t:MiddleName>Marie</t:MiddleName>\
         <t:Surname>Park</t:Surname><t:Generation>Jr.</t:Generation></t:Contact>",
    );
    d.pim.push(("C1".into(), contacts, there.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "zoe@contoso.example").unwrap();
    let mut pim = Pim::default();
    let card = vcard::parse(EDITED_CARD).remove(0);
    pim.cards.push(StoredCard { book: crate::ews_pim::CONTACTS.into(), href: "contact:C1".into(), etag: "k".into(), card, dirty: true });
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert_eq!(sync.pushed_cards.len(), 1);
    let data = server.data.lock().unwrap();
    let made: Vec<_> = data.pim.iter().filter(|(id, _, el)| el.name == "Contact" && id != "C1").collect();
    assert_eq!(made.len(), 1, "made again, the old one deleted: {:?}", data.pim.iter().map(|(i, _, _)| i).collect::<Vec<_>>());
    let el = &made[0].2;
    let said = |name: &str| el.text_of(name).unwrap_or_default().to_string();
    assert_eq!(
        (said("DisplayName"), said("GivenName"), said("MiddleName"), said("Surname"), said("Generation")),
        ("Dr. Zoe Marie Park Jr.".into(), "Zoe".into(), "Marie".into(), "Park".into(), "Jr.".into())
    );
    let prefix = el
        .children("ExtendedProperty")
        .find(|p| p.find("ExtendedFieldURI").and_then(|f| f.attr("PropertyTag")).is_some_and(|t| t.eq_ignore_ascii_case("0x3A45")))
        .and_then(|p| p.text_of("Value"));
    assert_eq!(prefix, Some("Dr."), "the prefix has a property of its own");
}
