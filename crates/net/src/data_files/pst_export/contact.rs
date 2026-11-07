// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A card as an Outlook contact (`IPM.Contact`, MS-OXOCNTC 2.2.1) or, a group's (`KIND:group`),
//! a contact group (`IPM.DistList`, 2.2.2). A contact has its names, three email addresses, a
//! field for each kind of phone Outlook has, a work, a home and an other address, its company,
//! department, title, profession, office, manager, assistant, nickname, spouse, birthday and
//! anniversary, web page, instant messaging address and note, its picture (an attachment marked
//! as one) and its files; a group its name and members, each a one-off address. What Outlook has
//! no place for (a fourth address, a phone or address of another kind, an address's kind, a
//! birthday without a year, the card's other properties) stays in the item's text as a card of
//! its own (`pst_pim::rest`), which the import reads back.

use quatrelle_model::pim::files::{card_file_data, card_files};
use quatrelle_model::vcard::{Card, PostalAddress, Typed};

use super::item::filetime;
use super::pim::{File, Item, Key, PSETID_ADDRESS, Value, crlf, lid, tag};
use super::*;

/// The phone fields Outlook has, by the kind Quatrelle keeps (the order the import reads them).
const PHONES: [(&str, &[u16]); 6] =
    [("work", &[0x3A08, 0x3A1B]), ("home", &[0x3A09, 0x3A2F]), ("cell", &[0x3A1C]), ("fax", &[0x3A24]), ("pager", &[0x3A21]), ("", &[0x3A1F])];
/// The address fields (street, city, state, postal code, country) by kind; the work address's
/// also as PidLidWorkAddressStreet to PidLidWorkAddressCountry, which Outlook shows.
const ADDRESSES: [(&str, [u16; 5]); 3] = [
    ("work", [0x3A29, 0x3A27, 0x3A28, 0x3A2A, 0x3A26]),
    ("home", [0x3A5D, 0x3A59, 0x3A5C, 0x3A5B, 0x3A5A]),
    ("", [0x3A63, 0x3A5F, 0x3A62, 0x3A61, 0x3A60]),
];
const WORK: [u32; 5] = [0x8045, 0x8046, 0x8047, 0x8048, 0x8049];
/// Email1, Email2 and Email3: each its display name, then address type, address, original
/// display name and original entry id (PSETID_Address, from these).
const EMAILS: [u32; 3] = [0x8080, 0x8090, 0x80A0];
/// The members a group keeps, as the import reads them.
const MAX_MEMBERS: usize = 10_000;
/// The card property holding one of its files (`quatrelle_model::pim::files`).
const FILE_LINE: &str = "X-QUATRELLE-ATTACH";

/// The card as an item.
pub(super) fn item(card: &Card, now: i64) -> Item {
    let group = card.extra.iter().any(|l| {
        let (name, _, value) = parts(l);
        name == "KIND" && value.trim().eq_ignore_ascii_case("group")
    });
    if group { group_item(card, now) } else { contact(card, now) }
}

/// A content line's name (upper case, without its group), its parameters and its value.
pub(super) fn parts(line: &str) -> (String, &str, &str) {
    let mut quoted = false;
    let colon = line.char_indices().find(|(_, c)| {
        if *c == '"' {
            quoted = !quoted;
        }
        *c == ':' && !quoted
    });
    let Some((at, _)) = colon else { return (String::new(), "", line) };
    let (head, value) = (line.get(..at).unwrap_or_default(), line.get(at + 1..).unwrap_or_default());
    let (name, params) = head.split_once(';').unwrap_or((head, ""));
    (name.rsplit('.').next().unwrap_or_default().trim().to_ascii_uppercase(), params, value)
}

/// The card's files: its picture (`photo`: when it can have one, a group can't) and those it
/// carries (`X-QUATRELLE-ATTACH`), and its other lines, kept as they are.
fn files(card: &Card, photo: bool) -> (Vec<File>, Vec<String>) {
    let mut files = Vec::new();
    if let Some(p) = card.photo.as_ref().filter(|_| photo)
        && let Some(data) = quatrelle_model::vcard::base64_decode(&p.base64).filter(|d| !d.is_empty())
    {
        let mime = if p.media.trim().is_empty() { "image/jpeg".to_string() } else { p.media.trim().to_ascii_lowercase() };
        files.push(File { name: "ContactPicture.jpg".into(), mime, data, picture: true });
    }
    for (i, f) in card_files(card).into_iter().enumerate() {
        if let Some(data) = card_file_data(card, i) {
            let name = if f.name.trim().is_empty() { "attachment".to_string() } else { f.name };
            let mime = if f.mime.is_empty() { mime::mime_from_name(&name).to_string() } else { f.mime };
            files.push(File { name, mime, data, picture: false });
        }
    }
    let rest = card.extra.iter().filter(|l| parts(l).0 != FILE_LINE).cloned().collect();
    (files, rest)
}

/// What a contact and a group both have: names, company, department, title and the rest of the
/// card's own text fields, each that it has.
fn names(card: &Card, name: &str) -> Vec<(Key, Value)> {
    let mut props = vec![tag(0x3001, Value::Text(name.to_string()))];
    let fields = [
        (0x3A06, &card.given),
        (0x3A11, &card.family),
        (0x3A16, &card.org),
        (0x3A18, &card.department),
        (0x3A17, &card.title),
        (0x3A46, &card.profession),
        (0x3A19, &card.office),
        (0x3A4E, &card.manager),
        (0x3A30, &card.assistant),
        (0x3A4F, &card.nickname),
        (0x3A48, &card.spouse),
    ];
    for (id, v) in fields {
        if !v.trim().is_empty() {
            props.push(tag(id, Value::Text(v.clone())));
        }
    }
    // PidLidHtml (the web page Outlook shows) and PidLidInstantMessagingAddress.
    for (id, v) in [(0x802B, &card.web), (0x8062, &card.im)] {
        if !v.trim().is_empty() {
            props.push(lid(PSETID_ADDRESS, id, Value::Text(v.trim().to_string())));
        }
    }
    props
}

/// A day Quatrelle keeps (`YYYY-MM-DD`) as its midnight in UTC; `None` for one without a year
/// (`--MM-DD`) or that isn't a day.
fn day(b: &str) -> Option<i64> {
    let mut it = b.split('-');
    let (y, m, d) = (it.next()?.parse::<i64>().ok()?, it.next()?.parse::<u32>().ok()?, it.next()?.parse::<u32>().ok()?);
    ((1601..=9999).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d) && it.next().is_none())
        .then(|| quatrelle_model::dates::days_from_civil(y, m, d).saturating_mul(86_400))
}

/// The lines of a card past its uid and names, as the export keeps them in an item's text: its
/// vCard, unfolded, without the lines every card has.
fn rest_lines(rest: &Card) -> Vec<String> {
    let text = quatrelle_model::vcard::write(rest);
    let mut lines: Vec<String> = Vec::new();
    for l in text.split("\r\n") {
        match (l.strip_prefix(' '), lines.last_mut()) {
            (Some(more), Some(last)) => last.push_str(more),
            _ => lines.push(l.to_string()),
        }
    }
    let every = ["BEGIN", "END", "VERSION", "PRODID", "UID", "FN", "N"];
    lines.retain(|l| !l.is_empty() && !every.contains(&parts(l).0.as_str()));
    lines
}

fn contact(card: &Card, now: i64) -> Item {
    let name = card.display();
    let mut props = names(card, &name);
    let file_under = if card.family.trim().is_empty() || card.given.trim().is_empty() {
        name.clone()
    } else {
        format!("{}, {}", card.family.trim(), card.given.trim())
    };
    props.push(lid(PSETID_ADDRESS, 0x8005, Value::Text(file_under)));
    let mut rest = Card::default();
    // Three email addresses (PidLidAddressBookProviderArrayType says which); each one's kind, and
    // any after the third, in the rest.
    let mut slot = 0;
    for e in &card.emails {
        let Some(base) = EMAILS.get(slot).filter(|_| e.value.contains('@')) else {
            rest.emails.push(e.clone());
            continue;
        };
        let shown = if name.is_empty() || name == e.value { e.value.clone() } else { format!("{name} ({})", e.value) };
        props.extend([
            lid(PSETID_ADDRESS, *base, Value::Text(shown)),
            lid(PSETID_ADDRESS, base + 2, Value::Text("SMTP".into())),
            lid(PSETID_ADDRESS, base + 3, Value::Text(e.value.clone())),
            lid(PSETID_ADDRESS, base + 4, Value::Text(e.value.clone())),
            lid(PSETID_ADDRESS, base + 5, Value::Bin(crate::pst::one_off_entry_id(&name, &e.value))),
        ]);
        if !e.kind.is_empty() {
            rest.emails.push(e.clone());
        }
        slot += 1;
    }
    if slot > 0 {
        props.push(lid(PSETID_ADDRESS, 0x8029, Value::Int((1 << slot) - 1)));
    }
    let mut used: Vec<u16> = Vec::new();
    for p in &card.phones {
        let field = PHONES.iter().find(|(k, _)| k.eq_ignore_ascii_case(p.kind.trim())).and_then(|(_, ids)| ids.iter().find(|id| !used.contains(id)));
        match field {
            Some(id) if !p.value.trim().is_empty() => {
                used.push(*id);
                props.push(tag(*id, Value::Text(p.value.trim().to_string())));
            }
            _ => rest.phones.push(Typed { value: p.value.clone(), kind: p.kind.clone() }),
        }
    }
    let mut kinds: Vec<&str> = Vec::new();
    for a in &card.addresses {
        match ADDRESSES.iter().find(|(k, _)| k.eq_ignore_ascii_case(a.kind.trim()) && !kinds.contains(k)) {
            Some((kind, ids)) => {
                kinds.push(kind);
                props.extend(address(a, ids, *kind == "work"));
            }
            None => rest.addresses.push(a.clone()),
        }
    }
    // The birthday and anniversary, also as PidLidBirthdayLocal and PidLidWeddingAnniversaryLocal.
    match day(&card.birthday) {
        Some(d) => props.extend([tag(0x3A42, Value::Time(filetime(d))), lid(PSETID_ADDRESS, 0x80DE, Value::Time(filetime(d)))]),
        None => rest.birthday = card.birthday.clone(),
    }
    match day(&card.anniversary) {
        Some(d) => props.extend([tag(0x3A41, Value::Time(filetime(d))), lid(PSETID_ADDRESS, 0x80DF, Value::Time(filetime(d)))]),
        None => rest.anniversary = card.anniversary.clone(),
    }
    let (files, extra) = files(card, true);
    if files.iter().any(|f| f.picture) {
        props.push(lid(PSETID_ADDRESS, 0x8015, Value::Bool(true)));
    }
    rest.extra = extra;
    Item {
        class: "IPM.Contact",
        subject: name,
        body: crlf(&crate::pst_pim::with_rest(&card.note, "VCARD", &rest_lines(&rest))),
        modified: filetime(now),
        props,
        files,
        ..Default::default()
    }
}

/// An address's fields (`ids`: street, city, state, postal code, country), and the work one's
/// named ones too.
fn address(a: &PostalAddress, ids: &[u16; 5], work: bool) -> Vec<(Key, Value)> {
    let values =
        [crlf(a.street.trim()), a.city.trim().to_string(), a.region.trim().to_string(), a.postcode.trim().to_string(), a.country.trim().to_string()];
    let mut out = Vec::new();
    for (i, v) in values.iter().enumerate().filter(|(_, v)| !v.is_empty()) {
        if let Some(id) = ids.get(i) {
            out.push(tag(*id, Value::Text(v.clone())));
        }
        if let Some(l) = WORK.get(i).filter(|_| work) {
            out.push(lid(PSETID_ADDRESS, *l, Value::Text(v.clone())));
        }
    }
    out
}

/// A contact group: its name, and its members (`MEMBER:mailto:`), each a one-off address
/// (PidLidDistributionListOneOffMembers) and the same wrapped (PidLidDistributionListMembers,
/// with its checksum). A note that starts with the list of its members, as the import writes
/// one, gives them their names, and the list isn't written again.
fn group_item(card: &Card, now: i64) -> Item {
    let name = card.display();
    let mut props = names(card, &name);
    props.push(lid(PSETID_ADDRESS, 0x8053, Value::Text(name.clone())));
    let (files, extra) = files(card, false);
    let mut addresses = Vec::new();
    let mut rest = Card {
        emails: card.emails.clone(),
        phones: card.phones.clone(),
        addresses: card.addresses.clone(),
        birthday: card.birthday.clone(),
        anniversary: card.anniversary.clone(),
        ..Default::default()
    };
    for line in extra {
        let (prop, _, value) = parts(&line);
        let mailto = value.trim().get(..7).filter(|h| h.eq_ignore_ascii_case("mailto:")).and_then(|_| value.trim().get(7..));
        match (prop.as_str(), mailto) {
            ("KIND", _) if value.trim().eq_ignore_ascii_case("group") => {}
            ("MEMBER", Some(a)) if a.contains('@') && addresses.len() < MAX_MEMBERS => addresses.push(a.to_string()),
            _ => rest.extra.push(line),
        }
    }
    let (named, note) = listed(&card.note, &addresses);
    let one_offs: Vec<Vec<u8>> = addresses.iter().zip(&named).map(|(a, n)| crate::pst::one_off_entry_id(n, a)).collect();
    let wrapped: Vec<Vec<u8>> = one_offs.iter().map(|o| crate::pst_pim::wrapped_entry_id(o)).collect();
    if !addresses.is_empty() {
        let sum = crc(&wrapped.concat());
        props.extend([
            lid(PSETID_ADDRESS, 0x8054, Value::Bins(one_offs)),
            lid(PSETID_ADDRESS, 0x8055, Value::Bins(wrapped)),
            lid(PSETID_ADDRESS, 0x804C, Value::Int(i32::from_le_bytes(sum.to_le_bytes()))),
        ]);
    }
    Item {
        class: "IPM.DistList",
        subject: name,
        body: crlf(&crate::pst_pim::with_rest(&note, "VCARD", &rest_lines(&rest))),
        modified: filetime(now),
        props,
        files,
        ..Default::default()
    }
}

/// The members' names from a note that starts with their list as the import writes it (a line
/// each, `Name <address>` or the address), and the note after it; no names, and the whole
/// note, when it doesn't.
fn listed(note: &str, addresses: &[String]) -> (Vec<String>, String) {
    let note = note.replace("\r\n", "\n");
    let (head, tail) = note.split_once("\n\n").unwrap_or((&note, ""));
    let lines: Vec<&str> = head.lines().collect();
    let names: Option<Vec<String>> = (!addresses.is_empty() && lines.len() == addresses.len())
        .then(|| {
            lines
                .iter()
                .zip(addresses)
                .map(|(l, a)| match l.strip_suffix(&format!("<{a}>")) {
                    _ if *l == a => Some(String::new()),
                    Some(n) if !n.trim().is_empty() => Some(n.trim_end().to_string()),
                    _ => None,
                })
                .collect()
        })
        .flatten();
    match names {
        Some(n) => (n, tail.trim().to_string()),
        None => (vec![String::new(); addresses.len()], note.trim().to_string()),
    }
}

/// PidLidDistributionListChecksum: the CRC-32 of MS-OXRTFCP 2.1.3.2 (no inversion), over the
/// members' entry ids one after another (MS-OXOCNTC 2.2.2.2.3).
fn crc(data: &[u8]) -> u32 {
    let mut c = 0u32;
    for b in data {
        c ^= u32::from(*b);
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    c
}
