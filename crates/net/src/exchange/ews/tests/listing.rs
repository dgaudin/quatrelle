// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! EWS: a folder listing says what it couldn't list — a shared mailbox, the public folders, the
//! online archive — over the fake server, so that what such a zone held isn't taken for gone;
//! and what finds those zones (Autodiscover, the directory) failing once fails for that listing
//! only.

use super::*;
use crate::exchange::missed::{Listed, Missed, PublicMissed};
use crate::fakes::fake_ews::{self, Data, SharedBox};

const SALES: &str = "sales@contoso.example";
const TEAM: &str = "team@contoso.example";

fn connect(server: &fake_ews::Server) -> Client {
    let user = format!("{}\\{}", fake_ews::DOMAIN, fake_ews::USER);
    Client::connect(&server.url, &user, fake_ews::PASSWORD, "riley@contoso.example").unwrap()
}

fn paths(listed: &Listed) -> Vec<&str> {
    listed.folders.iter().map(|f| f.path.as_str()).collect()
}

/// Riley's Inbox; the public folders (`Team`), listed routed to the hierarchy mailbox Autodiscover
/// names, as Exchange 2013 and later want; Sales, a shared mailbox Autodiscover maps to the
/// account (its Inbox, and `Quotes` in it); and the online archive (`2019`).
fn zones() -> Data {
    let mut d = Data::default();
    d.pf_hierarchy = Some("PFHierarchy@contoso.example".into());
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let public = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    d.folder("Team", Some(&public), None, "IPF.Note");
    let inbox = d.shared_mailbox(SharedBox { address: SALES.into(), name: "Sales".into(), automap: true, legacy: false, granted: true });
    d.shared_folder(SALES, "Quotes", &inbox);
    let archive = d.archive_mailbox("Online Archive - Riley");
    d.archive_folder("2019", &archive);
    d
}

/// A shared mailbox whose list's second page comes back with no response message: its list ends
/// there, what came is listed, and the mailbox isn't failed (it was, all its folders with it).
#[test]
fn a_shared_mailbox_answered_with_no_message_lists_what_came() {
    let mut d = zones();
    d.zones.shared_page = Some(2);
    d.zones.unanswered_pages.push(SALES.into());
    let server = fake_ews::serve(d);
    let listed = list_folders(&connect(&server), &mut State::default(), "a1").unwrap();
    let paths = paths(&listed);
    assert!(paths.contains(&"Sales/Inbox"), "{paths:?}");
    assert!(!paths.iter().any(|p| p.ends_with("Quotes")), "past the page not answered: {paths:?}");
}

/// Each zone a listing couldn't read is said, and only it: a shared mailbox refused, or answered
/// with no folder; Autodiscover not answering (the mailboxes it maps, the archive, and the public
/// folders listed through the mailbox it names can't be asked for); the public folders refused or
/// answered with none; the archive refused or answered with no folder. A listing that read
/// everything misses nothing.
#[test]
fn a_listing_says_which_zones_it_missed() {
    let missed = |script: &dyn Fn(&mut Data)| {
        let mut d = zones();
        script(&mut d);
        let server = fake_ews::serve(d);
        list_folders(&connect(&server), &mut State::default(), "a1").unwrap().missed
    };
    assert_eq!(missed(&|_| {}), Missed::default(), "everything listed");
    let sales = Missed { shared: vec![SALES.into()], ..Default::default() };
    assert_eq!(missed(&|d| d.zones.refused.push(SALES.into())), sales, "refused");
    assert_eq!(missed(&|d| d.zones.emptied.push(SALES.into())), sales, "no folder");
    let untold = Missed { shared_unknown: true, public: PublicMissed::All, archive: true, ..Default::default() };
    assert_eq!(missed(&|d| d.zones.discoveries_failing = 1), untold, "Autodiscover not answering");
    let public = Missed { public: PublicMissed::All, ..Default::default() };
    assert_eq!(missed(&|d| d.zones.public_refused = true), public, "public folders refused");
    assert_eq!(missed(&|d| d.zones.public_emptied = true), public, "no public folder");
    let archive = Missed { archive: true, ..Default::default() };
    assert_eq!(missed(&|d| d.zones.archive_emptied = true), archive, "an archive with no folder");
    assert_eq!(missed(&|d| d.archive.iter_mut().for_each(|a| a.refused = true)), archive, "the archive refused");
}

/// Autodiscover not answering is asked again at the next listing, once: kept for the session, the
/// mailbox it maps to the account, the public folders it routes to and the archive were out of
/// every listing till Quatrelle started again.
#[test]
fn a_discovery_that_failed_is_asked_again_at_the_next_listing() {
    let mut d = zones();
    d.zones.discoveries_failing = 1;
    let server = fake_ews::serve(d);
    let c = connect(&server);
    let mut st = State::default();
    let first = list_folders(&c, &mut st, "a1").unwrap();
    let again = list_folders(&c, &mut st, "a1").unwrap();
    let paths = paths(&again);
    let team = format!("{PUBLIC_FOLDERS}/Team");
    for want in ["Sales/Inbox", team.as_str(), "Online Archive - Riley/2019"] {
        assert!(paths.contains(&want), "{want} in {paths:?}");
    }
    assert_eq!(again.missed, Missed::default());
    assert!(first.missed.shared_unknown && first.missed.archive, "{:?}", first.missed);
    assert_eq!(server.data.lock().unwrap().zones.discoveries, 2, "asked once at each listing");
}

/// A shared mailbox added by hand is named as the directory says. When the directory doesn't
/// answer, it keeps the name it was listed under (it took its address: every path of its folders
/// changed, and their caches with them); with no name known, its address, as before.
#[test]
fn a_mailbox_added_by_hand_keeps_its_name_when_the_directory_is_silent() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.shared_mailbox(SharedBox { address: TEAM.into(), name: "Team".into(), automap: false, legacy: false, granted: true });
    let server = fake_ews::serve(d);
    let c = connect(&server);
    c.set_shared(&[TEAM.to_string()]);
    let mut st = State::default();
    let first = list_folders(&c, &mut st, "a1").unwrap();
    assert!(paths(&first).contains(&"Team/Inbox"), "{:?}", paths(&first));
    // As the worker tells it before a listing: each mailbox's top folder, by its address.
    let known = first.folders.iter().filter(|f| f.depth() == 0).filter_map(|f| Some((f.mailbox.clone()?, utf7::decode(&f.path)))).collect();
    c.set_known_shared(known);
    server.data.lock().unwrap().zones.directory_silent = true;
    let again = list_folders(&c, &mut st, "a1").unwrap();
    assert_eq!(paths(&again), paths(&first), "the directory silent, the names stay");
    // Nothing known of it: its address, as before.
    let fresh = connect(&server);
    fresh.set_shared(&[TEAM.to_string()]);
    let unnamed = list_folders(&fresh, &mut State::default(), "a1").unwrap();
    assert!(paths(&unnamed).contains(&"team@contoso.example/Inbox"), "{:?}", paths(&unnamed));
}
