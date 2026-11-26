// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The calendar groups Outlook keeps with shortcut types [MS-OXOCFG] doesn't document: a header
//! of type 5 and its members of type 6, as a mailbox's "My Team"-like group shows them. They're
//! read as a group and its people's calendars, and never written.

use super::*;

/// The X.500 name of someone in the fake directory.
fn dn(who: &str) -> String {
    format!("/o=Contoso/ou=Exchange Administrative Group (FYDIBOHF23SPDLT)/cn=Recipients/cn={who}")
}

/// A header of type 5 with a member of type 6 (an address book entry, the header's id as its
/// group): the group is listed with the member's calendar in it; renaming, moving or deleting
/// it is refused, nothing written.
#[test]
fn an_undocumented_group_is_read_and_left_alone() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.directory = vec![(dn("ana"), "ana@contoso.example".into())];
    let link = "IPM.Microsoft.WunderBar.Link";
    let team = crate::imap::base64(&[0x30; 16]);
    d.wlinks = vec![
        wlink_of(link, 3, 5, "Mon équipe", &[("0x6842", "Binary", team.clone()), ("0x684B", "Binary", crate::imap::base64(&[0x30]))]),
        wlink_of(
            link,
            3,
            6,
            "Ana Exemple",
            &[
                ("0x6850", "Binary", team),
                ("0x6854", "Binary", crate::imap::base64(&address_book_eid(&dn("ana")))),
                ("0x684B", "Binary", crate::imap::base64(&[0x31])),
            ],
        ),
    ];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let shortcuts = crate::ews_pim::calendar_links(&c).unwrap();
    assert_eq!(shortcuts.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Mon équipe"], "the group is listed");
    assert!(
        shortcuts.links.iter().any(|l| l.email == "ana@contoso.example" && l.name == "Ana Exemple" && l.group == "Mon équipe"),
        "its member's calendar is in it: {:?}",
        shortcuts.links.iter().map(|l| (&l.email, &l.group)).collect::<Vec<_>>()
    );
    let before = server.data.lock().unwrap().wlinks.clone();
    assert!(crate::ews_pim::rename_calendar_group(&c, "Mon équipe", "Équipe").is_err(), "not renamed");
    assert!(crate::ews_pim::move_calendar_group(&c, "Mon équipe", true).is_err(), "not moved");
    assert!(crate::ews_pim::delete_calendar_group(&c, "Mon équipe").is_err(), "not deleted");
    assert_eq!(server.data.lock().unwrap().wlinks, before, "nothing written");
}
