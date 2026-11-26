// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A calendar group moved here keeps its place while the move is on its way to the server: a
//! read that left before it landed doesn't put the old order back.

use quatrelle_model::pim::DavSync;

use super::*;

/// Moved down here, a group stays down when a read still shows the old order; the read that
/// shows the move ends the wait, and a later change made elsewhere is taken as it comes. Past
/// the wait, the server's order wins (the move may have failed).
#[test]
fn a_group_moved_here_keeps_its_place_until_the_server_shows_it() {
    let mut s = Session::open(Some(temp("group-order")), quatrelle_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let read = |s: &mut Session, groups: &[&str], at: i64| {
        let sync = DavSync { calendar_groups: Some(groups.iter().map(|g| g.to_string()).collect()), checked: at, ..Default::default() };
        s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    };
    let groups = |s: &Session| s.pim.get(&id).unwrap().calendar_groups.clone();
    read(&mut s, &["Équipe", "Salles", "Projets"], NOW);
    s.run("calendar.group.move", &json!({"account": id, "group": "Équipe", "up": false})).unwrap();
    assert_eq!(groups(&s), ["Salles", "Équipe", "Projets"], "moved here at once");
    read(&mut s, &["Équipe", "Salles", "Projets"], NOW + 1);
    assert_eq!(groups(&s), ["Salles", "Équipe", "Projets"], "a read from before the move doesn't put it back");
    read(&mut s, &["Salles", "Équipe", "Projets"], NOW + 2);
    read(&mut s, &["Projets", "Salles", "Équipe"], NOW + 3);
    assert_eq!(groups(&s), ["Projets", "Salles", "Équipe"], "once the move is there, the server's order is taken");
    // A move whose write never lands: the server's order comes back after the wait.
    s.run("calendar.group.move", &json!({"account": id, "group": "Équipe", "up": true})).unwrap();
    s.clock.now += 600;
    read(&mut s, &["Projets", "Salles", "Équipe"], NOW + 4);
    assert_eq!(groups(&s), ["Projets", "Salles", "Équipe"], "past the wait, the server's order");
    done(s);
}
