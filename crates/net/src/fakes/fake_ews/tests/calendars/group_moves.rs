// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Moving a calendar group writes the ordinal of the group moved, and nothing Outlook made
//! ([MS-OXOCFG] 2.2.9.7: a shortcut sorts by its ordinal, so one that fits between two others
//! is all a move needs).

use super::*;

const LINK: &str = "IPM.Microsoft.WunderBar.Link";

fn ordinal(bytes: &[u8]) -> String {
    crate::imap::base64(bytes)
}

/// A header of a group: Quatrelle's when `ours`, else with a stamp of someone else's.
fn header(name: &str, id: u8, ord: &[u8], ours: bool) -> String {
    let stamp = if ours { crate::ews::group_stamp(&[id; 16]) } else { 305_419_896 };
    wlink_of(
        LINK,
        3,
        4,
        name,
        &[("0x6842", "Binary", crate::imap::base64(&[id; 16])), ("0x6847", "Integer", stamp.to_string()), ("0x684B", "Binary", ordinal(ord))],
    )
}

fn names(c: &crate::ews::Client) -> Vec<String> {
    crate::ews_pim::calendar_links(c).unwrap().groups.into_iter().map(|g| g.name).collect()
}

fn setup(wlinks: Vec<String>) -> (crate::fakes::fake_ews::Server, crate::ews::Client) {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.wlinks = wlinks;
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    (server, c)
}

/// Moving a group of ours between two Outlook groups writes only ours: Outlook's two shortcuts
/// are exactly as they were.
#[test]
fn moving_our_group_leaves_outlooks_groups_as_they_were() {
    let outlook = [header("Autres", 0x10, &[0x10], false), header("Salles", 0x30, &[0x30], false)];
    let (server, c) = setup(vec![outlook[0].clone(), header("Équipe", 0x20, &[0x20], true), outlook[1].clone()]);
    assert_eq!(names(&c), ["Autres", "Équipe", "Salles"]);
    crate::ews_pim::move_calendar_group(&c, "Équipe", true).unwrap();
    assert_eq!(names(&c), ["Équipe", "Autres", "Salles"]);
    let kept = server.data.lock().unwrap();
    assert!(kept.wlinks.contains(&outlook[0]) && kept.wlinks.contains(&outlook[1]), "Outlook's headers were written: {:?}", kept.wlinks);
    drop(kept);
    crate::ews_pim::move_calendar_group(&c, "Équipe", false).unwrap();
    crate::ews_pim::move_calendar_group(&c, "Équipe", false).unwrap();
    assert_eq!(names(&c), ["Autres", "Salles", "Équipe"]);
    let kept = server.data.lock().unwrap();
    assert!(kept.wlinks.contains(&outlook[0]) && kept.wlinks.contains(&outlook[1]), "Outlook's headers were written: {:?}", kept.wlinks);
}

/// Two Outlook groups sharing an ordinal leave no ordinal to put ours between: the move is
/// refused, and nothing is written (the old way gave every group a new ordinal).
#[test]
fn a_move_with_no_room_between_the_neighbours_is_refused() {
    let wlinks = vec![header("Autres", 0x10, &[0x10], false), header("Salles", 0x11, &[0x10], false), header("Équipe", 0x20, &[0x20], true)];
    let (server, c) = setup(wlinks.clone());
    let e = crate::ews_pim::move_calendar_group(&c, "Équipe", true).unwrap_err();
    assert!(e.to_string().contains("room"), "{e}");
    assert_eq!(server.data.lock().unwrap().wlinks, wlinks, "nothing was written");
}

/// A header with no ordinal can't be told from the others, so nothing is put next to it.
#[test]
fn a_move_next_to_a_header_with_no_ordinal_is_refused() {
    let wlinks = vec![header("Autres", 0x10, &[], false), header("Équipe", 0x20, &[0x20], true)];
    let (server, c) = setup(wlinks.clone());
    assert!(crate::ews_pim::move_calendar_group(&c, "Équipe", true).is_err());
    assert_eq!(server.data.lock().unwrap().wlinks, wlinks, "nothing was written");
}
