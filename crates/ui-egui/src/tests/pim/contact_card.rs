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

/// A contact with more postal addresses than the form has places for (two at home), and a number
/// and an address of kinds it has no place for (a pager, a postal address), saved from the form
/// as it opened keeps them all, each with its kind. The second home address was dropped, and the
/// pager number and the postal address, put in the form's Other places, lost their kinds.
#[test]
fn a_contact_saved_from_the_form_keeps_its_extra_addresses_and_kinds() {
    let (mut a, dir) = app("contact-extras");
    let address = |kind: &str, city: &str| json!({"kind": kind, "street": "1 Quay", "city": city});
    let made = a.session.run(
        "contact.create",
        &json!({
            "name": "Ana Exemple",
            "phones": [{"value": "+1 555 0100", "kind": "pager"}, {"value": "+1 555 0101", "kind": "home"}],
            "addresses": [address("home", "Gent"), address("home", "Brugge"), address("postal", "Antwerpen")],
        }),
    );
    let uid = made.unwrap()["uid"].as_str().unwrap().to_string();
    let before = a.session.find_person(&uid).unwrap().card.clone();
    let form = crate::pim_dialogs::ContactForm::edit(&before);
    a.act(crate::Action::Submit("contact.update", form.params()));
    let after = a.session.find_person(&uid).unwrap().card.clone();
    let kinds = |c: &quatrelle_engine::model::vcard::Card| {
        let mut phones: Vec<(String, String)> = c.phones.iter().map(|p| (p.kind.clone(), p.value.clone())).collect();
        let mut places: Vec<(String, String)> = c.addresses.iter().map(|x| (x.kind.clone(), x.city.clone())).collect();
        phones.sort();
        places.sort();
        (phones, places)
    };
    assert_eq!(kinds(&after), kinds(&before));
    assert_eq!(after.addresses.len(), 3);
    let _ = std::fs::remove_dir_all(dir);
}

/// The kind kept for the Other places (a pager, a postal address) is the one of the value that
/// was there: a number typed in the box once the old one is gone, or an address put in its
/// place, takes the box's own kind rather than the pager's.
#[test]
fn a_number_typed_in_the_other_box_takes_that_boxs_kind() {
    let (mut a, dir) = app("contact-other-box");
    let made = a.session.run(
        "contact.create",
        &json!({
            "name": "Ana Exemple",
            "phones": [{"value": "+1 555 0100", "kind": "pager"}],
            "addresses": [{"kind": "postal", "street": "1 Quay", "city": "Antwerpen", "postcode": "2000"}],
        }),
    );
    let uid = made.unwrap()["uid"].as_str().unwrap().to_string();
    let card = a.session.find_person(&uid).unwrap().card.clone();
    let opened = crate::pim_dialogs::ContactForm::edit(&card);
    assert_eq!(opened.phones[4], "+1 555 0100", "the pager number is in the Other box");
    let kinds = |f: &crate::pim_dialogs::ContactForm| {
        let p = f.params();
        let phone = p["phones"][0]["kind"].as_str().unwrap_or("?").to_string();
        let place = p["addresses"][0]["kind"].as_str().unwrap_or("?").to_string();
        (phone, place)
    };
    // As it opened: both keep their kinds.
    assert_eq!(kinds(&opened), ("pager".to_string(), "postal".to_string()));
    // The pager number erased and another typed: the box's kind, which is none.
    let mut changed = opened.clone();
    changed.phones[4] = "+1 555 0123".into();
    changed.addresses[2] = quatrelle_engine::model::vcard::PostalAddress { street: "9 Mill".into(), city: "Leuven".into(), ..Default::default() };
    assert_eq!(kinds(&changed), (String::new(), String::new()));
    a.act(crate::Action::Submit("contact.update", changed.params()));
    let saved = a.session.find_person(&uid).unwrap().card.clone();
    assert_eq!((saved.phones[0].value.as_str(), saved.phones[0].kind.as_str()), ("+1 555 0123", ""));
    assert_eq!(saved.addresses[0].kind, "");
    let _ = std::fs::remove_dir_all(dir);
}
