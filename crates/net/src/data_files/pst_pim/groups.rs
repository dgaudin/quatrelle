// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A contact group (`IPM.DistList`, MS-OXOCNTC 2.2.2): its members that carry their address, and
//! the card it comes in as (vCard 4's `KIND:group`, a `MEMBER` for each), in the address book.

use super::*;

/// A contact group's members kept at most.
const MAX_MEMBERS: usize = 10_000;
/// The provider of a wrapped entry id (a contact group's member, MS-OXOCNTC 2.2.2.2.1).
pub(super) const WRAPPED: [u8; 16] = [0xC0, 0x91, 0xAD, 0xD3, 0x51, 0x9D, 0xCF, 0x11, 0xA4, 0xA9, 0x00, 0xAA, 0x00, 0x47, 0xFA, 0xA4];

/// A contact group's members that carry their address: each of its one-off members
/// (PidLidDistributionListOneOffMembers), and those of its members (PidLidDistributionListMembers)
/// whose wrapped entry id holds one; once each. A member that is only a contact of the file is
/// left out (its address is in that contact).
pub(super) fn members(one_offs: Option<Val<'_>>, wrapped: Option<Val<'_>>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |found: Option<(String, String)>| {
        if let Some((name, address)) = found
            && out.len() < MAX_MEMBERS
            && !out.iter().any(|(_, a)| a.eq_ignore_ascii_case(&address))
        {
            out.push((name, address));
        }
    };
    if let Some(Val::Bins(list)) = one_offs {
        list.iter().for_each(|b| add(crate::pst::one_off_address(b)));
    }
    if let Some(Val::Bins(list)) = wrapped {
        // Flags, the provider, then a type whose low bits say what's wrapped (0: a one-off).
        for b in list {
            if b.get(4..20) == Some(&WRAPPED[..]) && b.get(20).is_some_and(|t| t & 0x0F == 0) {
                add(b.get(21..).and_then(crate::pst::one_off_address));
            }
        }
    }
    out
}

/// A contact group's member as [`members`] reads one from PidLidDistributionListMembers: the
/// one-off entry id `one_off` wrapped (MS-OXCDATA 2.2.5.2: no flags, the wrapping provider, a type
/// of 0, a one-off).
pub(crate) fn wrapped_entry_id(one_off: &[u8]) -> Vec<u8> {
    [&[0u8; 4][..], &WRAPPED, &[0], one_off].concat()
}

/// The contact group as a card (vCard 4's `KIND:group`, a `MEMBER` for each member) whose note
/// lists its members, above its own text.
pub(crate) fn group_of(x: &Extra, subject: &str, body: &str, uid: String) -> Card {
    let mut card = card_of(x, body, uid);
    if card.name.is_empty() {
        card.name = subject.trim().to_string();
    }
    let list: Vec<String> = x.members.iter().map(|(n, a)| if n.is_empty() { a.clone() } else { format!("{n} <{a}>") }).collect();
    card.note = [list.join("\n"), card.note].into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n\n");
    card.extra.push("KIND:group".into());
    card.extra.extend(x.members.iter().map(|(_, a)| format!("MEMBER:mailto:{a}")));
    card
}
