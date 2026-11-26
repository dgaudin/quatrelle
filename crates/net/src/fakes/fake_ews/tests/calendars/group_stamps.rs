// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Which calendar group headers Quatrelle made, and so may change: that holds across
//! connections, and never extends to a header Outlook made.

use super::*;

fn client(url: &str) -> crate::ews::Client {
    crate::ews::Client::connect(url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap()
}

/// A group Quatrelle made stays its own after the connection is gone and a new one is made (a
/// dropped connection, the next launch): it can still be renamed, moved and deleted. The group
/// Outlook made next to it, with the same name pattern, stays protected.
#[test]
fn a_group_made_here_stays_ours_on_a_new_connection() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let link = "IPM.Microsoft.WunderBar.Link";
    d.wlinks = vec![wlink_of(
        link,
        3,
        4,
        "Autres calendriers",
        &[
            ("0x6842", "Binary", crate::imap::base64(&[0x20; 16])),
            ("0x6847", "Integer", "305419896".into()),
            ("0x684B", "Binary", crate::imap::base64(&[0x20])),
        ],
    )];
    let server = serve(d);
    let first = client(&server.url);
    crate::ews_pim::create_calendar_group(&first, "Salles").unwrap();
    drop(first);
    let second = client(&server.url);
    let groups = crate::ews_pim::calendar_links(&second).unwrap().groups;
    let owned: Vec<(&str, bool)> = groups.iter().map(|g| (g.name.as_str(), g.ours)).collect();
    assert_eq!(owned, [("Autres calendriers", false), ("Salles", true)], "{owned:?}");
    crate::ews_pim::rename_calendar_group(&second, "Salles", "Rooms").unwrap();
    crate::ews_pim::move_calendar_group(&second, "Rooms", true).unwrap();
    assert!(crate::ews_pim::rename_calendar_group(&second, "Autres calendriers", "Mine").is_err(), "Outlook's group stays protected");
    crate::ews_pim::delete_calendar_group(&second, "Rooms").unwrap();
}
