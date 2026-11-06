// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Engine tests: the contacts field reads accents away, as the search does since R14 (R23):
//! `helene` offers « Hélène Dupré » from the book, `eric` the directory's « Éric Lefèvre »,
//! `etoile` the room « Salle Étoile », and Check Names writes out « helene » as « Hélène Dupré »;
//! the order is the one the text typed with its accents gives.

use super::names::{answer, with_exchange};
use super::*;
use crate::{FieldId, Suggestion};
use quatrelle_model::EmailAddress;
use quatrelle_model::addr::Accepts;
use quatrelle_model::scheduling::Room;

fn names(got: &[Suggestion]) -> Vec<String> {
    got.iter().map(|x| x.who.name.clone()).collect()
}

#[test]
fn suggestions_read_accents_away_in_the_same_order() {
    let (mut s, gal) = with_exchange("suggest-accents");
    s.contacts_step(usize::MAX);
    s.contacts_sent(&[
        EmailAddress::new("Hélène Dupré", "h.dupre@studio.example"),
        EmailAddress::new("Hélène Martin", "hmartin@x.example"),
        EmailAddress::new("Helena Ruiz", "helena@studio.example"),
    ]);
    // Written to more: ranked first among those that fit as well.
    s.contacts_sent(&[EmailAddress::new("Hélène Martin", "hmartin@x.example")]);
    answer(&mut s, &gal, "lef", &[("Éric Lefèvre", "eric.lefevre@contoso.example")]);
    let room = Room { email: "salle.etoile@contoso.example".into(), name: "Salle Étoile".into(), list: "Paris".into() };
    s.apply(Event::Rooms { account: gal.clone(), lists: Vec::new(), rooms: vec![room], error: None });
    let f = FieldId { window: 1, name: "to" };
    let mut ask = |text: &str| names(&s.people_suggest(f, Some(&gal), &Accepts::RECIPIENTS, text));
    let typed = ask("Hélène");
    assert_eq!(typed, ["Hélène Martin", "Hélène Dupré"], "with its accents, as before");
    assert_eq!(ask("helene"), typed, "without them: the same, in the same order");
    assert_eq!(ask("HELENE"), typed);
    assert_eq!(ask("dupre"), ["Hélène Dupré"]);
    assert_eq!(ask("Dupré"), ["Hélène Dupré"]);
    // An accent typed where the name has none still finds it, as the search reads both sides.
    assert_eq!(ask("héléna"), ["Helena Ruiz"]);
    // Text without accents keeps its order: an exact first name, then the most written to.
    let hel = ask("hel");
    assert_eq!(hel.first().map(String::as_str), Some("Hélène Martin"), "{hel:?}");
    assert!(hel.contains(&"Helena Ruiz".to_string()) && hel.contains(&"Hélène Dupré".to_string()), "{hel:?}");
    assert_eq!(ask("eric"), ["Éric Lefèvre"], "the directory's answers");
    assert_eq!(ask("lefevre"), ["Éric Lefèvre"]);
    let rooms = Accepts { kinds: &[quatrelle_model::AddresseeKind::Room], one: false, typed: &[], as_typed: false };
    assert_eq!(names(&s.people_suggest(f, Some(&gal), &rooms, "etoile")), ["Salle Étoile"], "the room lists");
    done(s);
}

#[test]
fn check_names_reads_accents_away() {
    let mut s = sample_session("check-names-accents");
    s.contacts_sent(&[EmailAddress::new("Hélène Dupré", "h.dupre@studio.example")]);
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "helene"})).unwrap();
    let out = s.run("compose.checkNames", &json!({})).unwrap();
    assert_eq!(out["resolved"], 1, "{out}");
    assert_eq!(s.compose.as_ref().unwrap().to, "Hélène Dupré <h.dupre@studio.example>; ");
    done(s);
}
