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
        wlink_of(link, 3, 4, "Équipe", &[("0x6842", "Binary", season.clone()), ("0x684B", "Binary", ordinal(0x20))]),
        wlink_of(link, 3, 4, "Salles", &[("0x6842", "Binary", rooms.clone()), ("0x684B", "Binary", ordinal(0x10))]),
        wlink_of(link, 3, 2, "Ana Lima", &[("0x6850", "Binary", season), ("0x6854", "Binary", crate::imap::base64(&address_book_eid(&dn("ana"))))]),
    ];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let shortcuts = crate::ews_pim::calendar_links(&c).unwrap();
    assert_eq!(shortcuts.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Salles", "Équipe"], "the smaller ordinal comes first");
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
    let favorites = wlink(link, 1, 2, "Favorites", "Ana's Inbox", &dn("ana"));
    d.wlinks = vec![
        favorites.clone(),
        wlink_of(link, 3, 4, "Équipe", &[("0x6842", "Binary", team.clone()), ("0x684B", "Binary", ordinal(0x10))]),
        wlink_of(
            link,
            3,
            2,
            "Ana Lima",
            &[
                ("0x6850", "Binary", team),
                ("0x6851", "String", "Équipe".into()),
                ("0x6854", "Binary", crate::imap::base64(&address_book_eid(&dn("ana")))),
                ("0x684B", "Binary", ordinal(0x11)),
            ],
        ),
    ];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let names = |c: &crate::ews::Client| crate::ews_pim::calendar_links(c).unwrap().groups.into_iter().map(|g| g.name).collect::<Vec<_>>();
    let ana = |c: &crate::ews::Client| {
        crate::ews_pim::calendar_links(c).unwrap().links.into_iter().find(|l| l.email == "ana@contoso.example").map(|l| l.group)
    };

    crate::ews_pim::create_calendar_group(&c, "Salles").unwrap();
    assert_eq!(names(&c), ["Équipe", "Salles"]);

    crate::ews_pim::rename_calendar_group(&c, "Salles", "Salles Studio").unwrap();
    assert_eq!(names(&c), ["Équipe", "Salles Studio"]);

    crate::ews_pim::move_calendar_group(&c, "Salles Studio", true).unwrap();
    assert_eq!(names(&c), ["Salles Studio", "Équipe"]);

    crate::ews_pim::move_calendar_to_group(&c, "shared:ana@contoso.example", "Ana Lima", "Salles Studio").unwrap();
    assert_eq!(ana(&c).as_deref(), Some("Salles Studio"));

    crate::ews_pim::delete_calendar_group(&c, "Salles Studio").unwrap();
    assert_eq!(names(&c), ["Équipe"]);
    assert_eq!(ana(&c), None, "the group's shared calendar left the list");
    let kept = server.data.lock().unwrap();
    assert!(kept.wlinks.contains(&favorites), "another section's shortcut is untouched, byte for byte");
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
        wlink_of(link, 3, 4, "My Calendars", &[("0x6842", "Binary", mine.clone()), ("0x684B", "Binary", ordinal(0x10))]),
        wlink_of(link, 3, 4, "Équipe", &[("0x6842", "Binary", team.clone()), ("0x684B", "Binary", ordinal(0x20))]),
        wlink_of(link, 3, 0, "Calendar", &[("0x6850", "Binary", mine), ("0x684B", "Binary", ordinal(0x11))]),
        wlink_of(link, 3, 0, "Meetings", &[("0x6850", "Binary", team), ("0x684B", "Binary", ordinal(0x21))]),
        wlink_of(
            link,
            3,
            2,
            "Ana Lima",
            &[
                ("0x6850", "Binary", guid(0x20)),
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

/// A shortcut of the Calendar section that doesn't read whole — no address book entry to say
/// whose calendar it is — is left alone: renaming the group it names must not rewrite it, and
/// deleting the group must not delete it.
#[test]
fn a_malformed_shortcut_is_never_renamed_or_deleted() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let link = "IPM.Microsoft.WunderBar.Link";
    d.wlinks = Vec::new();
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    crate::ews_pim::create_calendar_group(&c, "Studio").unwrap();
    let malformed = wlink_of(link, 3, 2, "Half a shortcut", &[("0x6851", "String", "Studio".into())]);
    server.data.lock().unwrap().wlinks.push(malformed);
    crate::ews_pim::rename_calendar_group(&c, "Studio", "Studio 2").unwrap();
    let kept = server.data.lock().unwrap();
    let item = |id: &str| kept.wlinks.iter().find(|w| w.contains(&format!("Id=\"{id}\""))).cloned().unwrap_or_default();
    assert!(item("WHalf a shortcut").contains(">Studio<"), "the malformed shortcut keeps its group name: {}", item("WHalf a shortcut"));
    assert!(!item("WHalf a shortcut").contains("Studio 2"), "the malformed shortcut isn't renamed");
    drop(kept);
    crate::ews_pim::delete_calendar_group(&c, "Studio 2").unwrap();
    let kept = server.data.lock().unwrap();
    assert!(kept.wlinks.iter().any(|w| w.contains("Half a shortcut")), "the malformed shortcut isn't deleted with the group");
}

/// A server that never says its page is the last one is not written on: the shortcuts weren't
/// read whole.
#[test]
fn a_page_that_never_says_it_is_the_last_one_is_not_written_on() {
    let mut d = Data { wlinks_more: true, ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.wlinks = vec![wlink_of(
        "IPM.Microsoft.WunderBar.Link",
        3,
        4,
        "Équipe",
        &[("0x6842", "Binary", guid(0x10)), ("0x684B", "Binary", crate::imap::base64(&[0x10]))],
    )];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let e = crate::ews_pim::create_calendar_group(&c, "Salles").unwrap_err();
    assert!(e.to_string().contains("read whole"), "{e}");
}

/// A group header carries the save stamp and the flags Outlook keeps ([MS-OXOCFG] 2.2.9.4,
/// 2.2.9.6); renaming it sets the one-off name flag.
#[test]
fn a_group_header_has_a_save_stamp_and_flags() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    crate::ews_pim::create_calendar_group(&c, "Salles").unwrap();
    {
        let kept = server.data.lock().unwrap();
        let header = kept.wlinks.iter().find(|w| w.contains(">Salles<")).expect("the new group header");
        assert!(header.contains("PropertyTag=\"0x6847\""), "the save stamp is written: {header}");
        assert!(header.contains("PropertyTag=\"0x684A\""), "the flags are written: {header}");
        assert!(header.contains("PropertyTag=\"0x684F\" PropertyType=\"CLSID\""), "the folder type is a CLSID: {header}");
    }
    crate::ews_pim::rename_calendar_group(&c, "Salles", "Rooms").unwrap();
    let kept = server.data.lock().unwrap();
    let header = kept.wlinks.iter().find(|w| w.contains(">Rooms<")).expect("the renamed group header");
    assert!(header.contains(">8192<"), "renaming sets sipOneOffName (0x2000): {header}");
}

/// A make whose read-back doesn't show the group takes the made shortcut out again: a failed
/// make leaves nothing on the server.
#[test]
fn a_group_make_that_doesnt_read_back_is_taken_out_again() {
    let mut d = Data { hide_created_group: true, ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.wlinks = vec![wlink_of(
        "IPM.Microsoft.WunderBar.Link",
        3,
        4,
        "Salles",
        &[("0x6842", "Binary", guid(0x10)), ("0x684B", "Binary", crate::imap::base64(&[0x10]))],
    )];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    assert!(crate::ews_pim::create_calendar_group(&c, "Équipe").is_err(), "the read-back doesn't show it");
    let kept = server.data.lock().unwrap();
    assert_eq!(kept.wlinks.len(), 1, "only the group that was there before is left: {:?}", kept.wlinks);
    assert!(kept.hidden_wlinks.is_empty(), "the made shortcut was deleted");
}

/// Two colleagues' shortcuts can show the same name; the one moved is the one the calendar's
/// address names, not the first with that name in the list.
#[test]
fn a_shared_calendar_is_moved_by_its_address_not_by_a_homonyms_name() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.directory = vec![(dn("ana"), "ana@contoso.example".into()), (dn("tom"), "tom@contoso.example".into())];
    let link = "IPM.Microsoft.WunderBar.Link";
    let ana = wlink_of(
        link,
        3,
        2,
        "Ana Lima",
        &[("0x6854", "Binary", crate::imap::base64(&address_book_eid(&dn("ana")))), ("0x684B", "Binary", crate::imap::base64(&[0x11]))],
    )
    .replace("Id=\"WAna Lima\"", "Id=\"W1\"");
    let tom = wlink_of(
        link,
        3,
        2,
        "Ana Lima",
        &[("0x6854", "Binary", crate::imap::base64(&address_book_eid(&dn("tom")))), ("0x684B", "Binary", crate::imap::base64(&[0x12]))],
    )
    .replace("Id=\"WAna Lima\"", "Id=\"W2\"");
    d.wlinks =
        vec![wlink_of(link, 3, 4, "Salles", &[("0x6842", "Binary", guid(0x20)), ("0x684B", "Binary", crate::imap::base64(&[0x10]))]), ana, tom];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    crate::ews_pim::move_calendar_to_group(&c, "shared:tom@contoso.example", "Ana Lima", "Salles").unwrap();
    let kept = server.data.lock().unwrap();
    let item = |id: &str| kept.wlinks.iter().find(|w| w.contains(&format!("Id=\"{id}\""))).cloned().unwrap_or_default();
    assert!(item("W2").contains(">Salles<"), "Tom's shortcut moved: {}", item("W2"));
    assert!(!item("W1").contains(">Salles<"), "Ana's shortcut with the same name stayed: {}", item("W1"));
}

/// A mailbox with more headers than the pane keeps is bounded.
#[test]
fn the_groups_are_capped() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let link = "IPM.Microsoft.WunderBar.Link";
    d.wlinks = (0..301)
        .map(|n| {
            let name = format!("G{n}");
            wlink_of(
                link,
                3,
                4,
                &name,
                &[
                    ("0x6842", "Binary", guid(u8::try_from(n % 255).unwrap_or(0))),
                    ("0x684B", "Binary", crate::imap::base64(&[u8::try_from(n % 255).unwrap_or(0)])),
                ],
            )
        })
        .collect();
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    assert_eq!(crate::ews_pim::calendar_links(&c).unwrap().groups.len(), 300);
}

/// A group whose calendars all fail to resolve keeps its header: the group is the server's, even
/// when none of its calendars can be listed.
#[test]
fn a_group_whose_calendars_all_fail_to_resolve_is_kept() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.wlinks = vec![
        wlink_of(
            "IPM.Microsoft.WunderBar.Link",
            3,
            4,
            "Équipe",
            &[("0x6842", "Binary", guid(0x10)), ("0x684B", "Binary", crate::imap::base64(&[0x10]))],
        ),
        wlink_of(
            "IPM.Microsoft.WunderBar.Link",
            3,
            2,
            "Nobody",
            &[("0x6854", "Binary", crate::imap::base64(&address_book_eid(&dn("nobody")))), ("0x684B", "Binary", crate::imap::base64(&[0x11]))],
        ),
    ];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let shortcuts = crate::ews_pim::calendar_links(&c).unwrap();
    assert_eq!(shortcuts.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Équipe"], "the header stays");
    assert!(shortcuts.links.is_empty(), "no address resolved");
}

/// A group Outlook made (its header has Outlook's save stamp, not ours) is protected: its
/// rename and move are refused, and it can only be deleted while it holds no calendar.
#[test]
fn an_outlook_group_is_protected() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.directory = vec![(dn("ana"), "ana@contoso.example".into())];
    let link = "IPM.Microsoft.WunderBar.Link";
    let team = guid(0x10);
    d.wlinks = vec![
        wlink_of(
            link,
            3,
            4,
            "Équipe",
            &[("0x6842", "Binary", team.clone()), ("0x6847", "Integer", "305419896".into()), ("0x684B", "Binary", crate::imap::base64(&[0x10]))],
        ),
        wlink_of(
            link,
            3,
            4,
            "Autres calendriers",
            &[("0x6842", "Binary", guid(0x20)), ("0x6847", "Integer", "305419896".into()), ("0x684B", "Binary", crate::imap::base64(&[0x20]))],
        ),
        wlink_of(
            link,
            3,
            2,
            "Ana Lima",
            &[
                ("0x6850", "Binary", team),
                ("0x6851", "String", "Équipe".into()),
                ("0x6854", "Binary", crate::imap::base64(&address_book_eid(&dn("ana")))),
                ("0x684B", "Binary", crate::imap::base64(&[0x11])),
            ],
        ),
    ];
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    assert!(crate::ews_pim::rename_calendar_group(&c, "Équipe", "Équipe 2").is_err(), "Outlook's group isn't renamed");
    assert!(crate::ews_pim::delete_calendar_group(&c, "Équipe").is_err(), "a group that holds calendars isn't deleted");
    assert!(crate::ews_pim::move_calendar_group(&c, "Équipe", true).is_err(), "Outlook's group isn't moved");
    crate::ews_pim::delete_calendar_group(&c, "Autres calendriers").unwrap();
    assert!(!crate::ews_pim::calendar_links(&c).unwrap().groups.iter().any(|g| g.name == "Autres calendriers"), "an empty group may go");
}
