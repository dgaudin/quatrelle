// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Outlook's calendar groups as navigation shortcuts ([MS-OXOCFG] 2.2.9): the folder they live
//! in, the group headers, the account's own calendars, the order, and the paging.

use super::*;

/// The X.500 name of someone in the fake directory.
fn dn(who: &str) -> String {
    format!("/o=Contoso/ou=Exchange Administrative Group (FYDIBOHF23SPDLT)/cn=Recipients/cn={who}")
}

/// A GUID as a shortcut property value (base64 of its 16 bytes).
fn guid(byte: u8) -> String {
    crate::imap::base64(&[byte; 16])
}

/// The Common Views folder is found by the navigation shortcuts it holds, not by the English
/// name Exchange gives it on an English mailbox: [MS-OXOCFG] 2.2.9 says where the shortcuts are
/// kept, and [MS-OXOSFLD] 2.2.1 gives no EWS distinguished id for the folder.
#[test]
fn the_common_views_folder_is_found_even_when_its_name_is_localized() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.common_views_name = "Vues communes".into();
    d.directory = vec![(dn("ana"), "ana@contoso.example".into())];
    d.wlinks = vec![wlink("IPM.Microsoft.WunderBar.Link", 3, 2, "Équipe", "Ana Lima", &dn("ana"))];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let links = crate::ews_pim::calendar_links(&c).unwrap().links;
    assert_eq!(links.len(), 1, "the shortcut in the localized Common Views folder is read");
    assert_eq!(links[0].email, "ana@contoso.example");
}

/// A group header names the group; a member may carry only the header's id
/// (`PidTagWlinkGroupClsid`), not the group's name.
#[test]
fn a_group_header_names_the_group_of_its_members() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.directory = vec![(dn("ana"), "ana@contoso.example".into())];
    let gid = guid(0xAB);
    d.wlinks = vec![
        wlink_of("IPM.Microsoft.WunderBar.Link", 3, 4, "Équipe", &[("0x6842", "Binary", gid.clone())]),
        wlink_of(
            "IPM.Microsoft.WunderBar.Link",
            3,
            2,
            "Ana Lima",
            &[("0x6850", "Binary", gid), ("0x6854", "Binary", crate::imap::base64(&address_book_eid(&dn("ana"))))],
        ),
    ];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let links = crate::ews_pim::calendar_links(&c).unwrap().links;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].group, "Équipe", "the member takes the header's name");
}

/// The account's own calendars are shortcuts too (`PidTagWlinkType` 0): one in a group is kept.
#[test]
fn the_accounts_own_calendar_in_a_group_is_kept() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.wlinks = vec![wlink_of("IPM.Microsoft.WunderBar.Link", 3, 0, "Meetings", &[("0x6851", "String", "Équipe".into())])];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let links = crate::ews_pim::calendar_links(&c).unwrap().links;
    assert_eq!(links.len(), 1, "the account's own calendar in a group is read");
    assert!(links[0].email.is_empty(), "it is the account's own, not someone else's");
    assert_eq!(links[0].group, "Équipe");
}

/// More shortcuts than one `FindItem` page: the calendar ones past the first page are read too.
#[test]
fn calendar_shortcuts_past_one_page_are_read() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let link = "IPM.Microsoft.WunderBar.Link";
    for i in 0..300 {
        d.wlinks.push(wlink(link, 1, 2, "Favorites", &format!("Mail {i}"), &dn(&format!("mail{i}"))));
    }
    for i in 0..5 {
        let who = format!("team{i}");
        d.directory.push((dn(&who), format!("{who}@contoso.example")));
        d.wlinks.push(wlink(link, 3, 2, "Équipe", &format!("Team {i}"), &dn(&who)));
    }
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let links = crate::ews_pim::calendar_links(&c).unwrap().links;
    assert_eq!(links.len(), 5, "the five after the first page are read");
}

/// Shortcuts are sorted by `PidTagWlinkOrdinal`, the order Outlook shows them in.
#[test]
fn shortcuts_are_sorted_by_their_ordinal() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.directory = vec![(dn("ana"), "ana@contoso.example".into()), (dn("tom"), "tom@contoso.example".into())];
    let link = "IPM.Microsoft.WunderBar.Link";
    let ordinal = |b: u8| crate::imap::base64(&[b]);
    d.wlinks = vec![
        wlink_of(
            link,
            3,
            2,
            "Ana Lima",
            &[
                ("0x6851", "String", "Équipe".into()),
                ("0x6854", "Binary", crate::imap::base64(&address_book_eid(&dn("ana")))),
                ("0x684B", "Binary", ordinal(0x90)),
            ],
        ),
        wlink_of(
            link,
            3,
            2,
            "Tom Diaz",
            &[
                ("0x6851", "String", "Équipe".into()),
                ("0x6854", "Binary", crate::imap::base64(&address_book_eid(&dn("tom")))),
                ("0x684B", "Binary", ordinal(0x10)),
            ],
        ),
    ];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let links = crate::ews_pim::calendar_links(&c).unwrap().links;
    assert_eq!(
        links.iter().map(|l| l.email.as_str()).collect::<Vec<_>>(),
        ["tom@contoso.example", "ana@contoso.example"],
        "the smaller ordinal comes first"
    );
}

/// Another section's shortcuts, a malformed one and one that isn't a shortcut are left alone,
/// and none of them stops the calendar ones.
#[test]
fn other_sections_and_malformed_shortcuts_are_left_alone() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.directory = vec![(dn("ana"), "ana@contoso.example".into())];
    let link = "IPM.Microsoft.WunderBar.Link";
    d.wlinks = vec![
        wlink(link, 1, 2, "Favorites", "Ana's Inbox", &dn("ana")),
        wlink(link, 3, 2, "Équipe", "Ana Lima", &dn("ana")),
        wlink_of(link, 3, 2, "Broken", &[("0x6854", "Binary", "not base64!".into())]),
        wlink("IPM.Note", 3, 2, "Équipe", "Not a shortcut", &dn("ana")),
    ];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let links = crate::ews_pim::calendar_links(&c).unwrap().links;
    assert_eq!(links.iter().map(|l| l.email.as_str()).collect::<Vec<_>>(), ["ana@contoso.example"], "only the calendar shortcut is read");
}

/// A colleague whose address the directory doesn't know doesn't take the group down with them:
/// the others stay, under the same group.
#[test]
fn a_group_keeps_the_calendars_whose_addresses_resolve() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.directory = vec![(dn("ana"), "ana@contoso.example".into()), (dn("tom"), "tom@contoso.example".into())];
    let link = "IPM.Microsoft.WunderBar.Link";
    d.wlinks = vec![
        wlink(link, 3, 2, "Équipe", "Ana Lima", &dn("ana")),
        wlink(link, 3, 2, "Équipe", "Ghost", &dn("ghost")),
        wlink(link, 3, 2, "Équipe", "Tom Diaz", &dn("tom")),
    ];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let links = crate::ews_pim::calendar_links(&c).unwrap().links;
    assert_eq!(
        links.iter().map(|l| (l.email.as_str(), l.group.as_str())).collect::<Vec<_>>(),
        [("ana@contoso.example", "Équipe"), ("tom@contoso.example", "Équipe")],
        "the group's known colleagues stay; the unknown one is left out"
    );
}

/// A group's header names it; the order of the groups is the ordinals' ([MS-OXOCFG] 2.2.9.7).
#[test]
fn groups_come_in_the_order_their_headers_give() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.directory = vec![(dn("ana"), "ana@contoso.example".into())];
    let link = "IPM.Microsoft.WunderBar.Link";
    let ordinal = |b: u8| crate::imap::base64(&[b]);
    let season = guid(0x20);
    let rooms = guid(0x10);
    d.wlinks = vec![
        wlink_of(link, 3, 4, "Équipe", &[("0x6842", "CLSID", season.clone()), ("0x684B", "Binary", ordinal(0x20))]),
        wlink_of(link, 3, 4, "Salles", &[("0x6842", "CLSID", rooms.clone()), ("0x684B", "Binary", ordinal(0x10))]),
        wlink_of(link, 3, 2, "Ana Lima", &[("0x6850", "CLSID", season), ("0x6854", "Binary", crate::imap::base64(&address_book_eid(&dn("ana"))))]),
    ];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let shortcuts = crate::ews_pim::calendar_links(&c).unwrap();
    assert_eq!(shortcuts.groups, ["Salles", "Équipe"], "the smaller ordinal comes first");
    assert_eq!(shortcuts.links.len(), 1);
}

/// A group is made, renamed, moved, has a calendar put in it and is deleted, all read back from
/// the fake's Common Views folder; another section's shortcut is never touched.
#[test]
fn a_group_is_made_renamed_moved_and_deleted() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.directory = vec![(dn("ana"), "ana@contoso.example".into())];
    let link = "IPM.Microsoft.WunderBar.Link";
    let ordinal = |b: u8| crate::imap::base64(&[b]);
    let team = guid(0x10);
    d.wlinks = vec![
        wlink(link, 1, 2, "Favorites", "Ana's Inbox", &dn("ana")),
        wlink_of(link, 3, 4, "Équipe", &[("0x6842", "CLSID", team.clone()), ("0x684B", "Binary", ordinal(0x10))]),
        wlink_of(
            link,
            3,
            2,
            "Ana Lima",
            &[
                ("0x6850", "CLSID", team),
                ("0x6851", "String", "Équipe".into()),
                ("0x6854", "Binary", crate::imap::base64(&address_book_eid(&dn("ana")))),
                ("0x684B", "Binary", ordinal(0x11)),
            ],
        ),
    ];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let names = |c: &crate::ews::Client| crate::ews_pim::calendar_links(c).unwrap().groups;
    let ana = |c: &crate::ews::Client| {
        crate::ews_pim::calendar_links(c).unwrap().links.into_iter().find(|l| l.email == "ana@contoso.example").map(|l| l.group)
    };

    crate::ews_pim::create_calendar_group(&c, "Salles").unwrap();
    assert_eq!(names(&c), ["Équipe", "Salles"]);

    crate::ews_pim::rename_calendar_group(&c, "Équipe", "Équipe Studio").unwrap();
    assert_eq!(names(&c), ["Équipe Studio", "Salles"]);
    assert_eq!(ana(&c).as_deref(), Some("Équipe Studio"));

    crate::ews_pim::move_calendar_group(&c, "Salles", true).unwrap();
    assert_eq!(names(&c), ["Salles", "Équipe Studio"]);

    crate::ews_pim::move_calendar_to_group(&c, "shared:ana@contoso.example", "Ana Lima", "Salles").unwrap();
    assert_eq!(ana(&c).as_deref(), Some("Salles"));

    crate::ews_pim::delete_calendar_group(&c, "Salles").unwrap();
    assert_eq!(names(&c), ["Équipe Studio"]);
    assert_eq!(ana(&c), None, "the group's shared calendar left the list");
    let kept = server.data.lock().unwrap();
    assert!(kept.wlinks.iter().any(|w| w.contains("Ana's Inbox")), "another section's shortcut is untouched");
}

/// A group of the account's own calendars: the main one stays in My Calendars, the others are
/// filed under the group's name as Outlook shows them.
#[test]
fn the_accounts_own_calendars_are_filed_under_their_group() {
    use quatrelle_model::pim::Pim;
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Meetings", Some(&cal), None, "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let link = "IPM.Microsoft.WunderBar.Link";
    let mine = guid(0x10);
    let team = guid(0x20);
    let ordinal = |b: u8| crate::imap::base64(&[b]);
    d.directory = vec![(dn("ana"), "ana@contoso.example".into())];
    d.wlinks = vec![
        wlink_of(link, 3, 4, "My Calendars", &[("0x6842", "CLSID", mine.clone()), ("0x684B", "Binary", ordinal(0x10))]),
        wlink_of(link, 3, 4, "Équipe", &[("0x6842", "CLSID", team.clone()), ("0x684B", "Binary", ordinal(0x20))]),
        wlink_of(link, 3, 0, "Calendar", &[("0x6850", "CLSID", mine), ("0x684B", "Binary", ordinal(0x11))]),
        wlink_of(link, 3, 0, "Meetings", &[("0x6850", "CLSID", team), ("0x684B", "Binary", ordinal(0x21))]),
        wlink_of(
            link,
            3,
            2,
            "Ana Lima",
            &[
                ("0x6850", "CLSID", guid(0x20)),
                ("0x6854", "Binary", crate::imap::base64(&address_book_eid(&dn("ana")))),
                ("0x684B", "Binary", ordinal(0x22)),
            ],
        ),
    ];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    assert_eq!(sync.calendar_groups, Some(vec!["Équipe".to_string()]), "My Calendars isn't a group of its own");
    let cals = sync.calendars.clone().unwrap_or_default();
    let main = cals.iter().find(|c| c.main).unwrap_or_else(|| panic!("the main calendar is listed"));
    assert_eq!(main.group, "", "My Calendars is the account's own, not a group");
    assert_eq!(cals.iter().find(|c| c.name == "Meetings").map(|c| c.group.as_str()), Some("Équipe"));
    assert_eq!(cals.iter().find(|c| c.name == "Ana Lima").map(|c| c.group.as_str()), Some("Équipe"));
}
