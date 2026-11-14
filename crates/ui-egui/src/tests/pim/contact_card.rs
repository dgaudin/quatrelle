// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! UI tests: a contact's card.

use super::*;

/// The Email button on a card named the Outlook way, "Last, First", starts a message to that one
/// person, whatever the name holds: put in bare, a `;`, or a comma the reading doesn't join
/// ("Park, Ann, Jr."), parted it into several recipients, the first not an address.
#[test]
fn email_on_a_card_named_last_first_writes_to_that_person() {
    use quatrelle_engine::model::addr::parse_list;
    let (mut a, dir) = app("email-last-first");
    for name in ["Park, Ann", "Park, Ann, Jr.", "Lee; Jo"] {
        for act in crate::panes::people::card::write_to(name, "ann@studio.example") {
            a.act(act);
        }
        let to = a.session.compose.as_ref().map(|c| c.to.clone()).unwrap_or_default();
        let people = parse_list(&to).unwrap_or_default();
        assert_eq!(people.len(), 1, "{name}: {to}");
        assert_eq!((people[0].name.as_str(), people[0].email.as_str()), (name, "ann@studio.example"));
        a.run("compose.discard", json!({}));
    }
    let _ = std::fs::remove_dir_all(dir);
}
