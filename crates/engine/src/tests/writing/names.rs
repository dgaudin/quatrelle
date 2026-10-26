//! Engine tests: Check Names (Ctrl+K) and the organisation's directory (Exchange's Global
//! Address List).

use super::*;
use quatrelle_model::{AccountConfig, EmailAddress};

/// The sample with an Exchange account beside it, and that account's id.
fn with_exchange(name: &str) -> (Session, String) {
    let mut s = sample_session(name);
    let config = AccountConfig {
        name: "Riley".into(),
        email: "riley@contoso.example".into(),
        username: "riley".into(),
        ews: Some("https://mail.contoso.example/EWS/Exchange.asmx".into()),
        ..Default::default()
    };
    let id = s.add_account(config, Some("pw".into())).unwrap();
    s.take_jobs();
    (s, id)
}

/// The names the directory is being asked about, in order.
fn asked(s: &mut Session) -> Vec<String> {
    s.take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::SearchDirectory { query, .. } => Some(query),
            _ => None,
        })
        .collect()
}

fn answer(s: &mut Session, account: &str, query: &str, people: &[(&str, &str)]) {
    let people = people.iter().map(|(n, e)| EmailAddress::new(*n, *e)).collect();
    s.apply(Event::DirectoryPeople { account: account.into(), query: query.into(), people, more: false, error: None });
}

/// A name known here is written out at once; the others are asked of the directory one at a
/// time: one person found is written out, several are offered to choose from, none is said.
#[test]
fn check_names_asks_the_directory_for_names_not_known_here() {
    let (mut s, gal) = with_exchange("check-names-gal");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "Zoe Park; reseau; nobody here", "cc": "dupont"})).unwrap();
    let out = s.run("compose.checkNames", &json!({})).unwrap();
    assert_eq!(out["resolved"], 1);
    assert_eq!(out["searching"], json!(["reseau", "nobody here", "dupont"]));
    assert_eq!(asked(&mut s), ["reseau"], "one name at a time");
    answer(&mut s, &gal, "reseau", &[("Proginov Reseau", "reseaux@contoso.example")]);
    let c = s.compose.clone().unwrap();
    assert_eq!(c.to, "Zoe Park <zoe@studio.example>; Proginov Reseau <reseaux@contoso.example>; nobody here; ");
    assert_eq!(asked(&mut s), ["nobody here"]);
    let notices = s.notices.len();
    answer(&mut s, &gal, "nobody here", &[]);
    assert!(s.notices.len() > notices && s.notices.last().is_some_and(|n| n.error && n.text == crate::NAMES_NOT_FOUND));
    assert_eq!(asked(&mut s), ["dupont"]);
    answer(&mut s, &gal, "dupont", &[("Anne Dupont", "anne.dupont@contoso.example"), ("Marc Dupont", "marc.dupont@contoso.example")]);
    let id = s.compose.as_ref().unwrap().id;
    let choice = s.name_choice(id).cloned().expect("two people to choose from");
    assert_eq!(choice.people.len(), 2);
    assert!(s.run("compose.chooseName", &json!({"token": "nobody", "email": "x@y.example"})).is_err(), "no such name to choose for");
    s.run("compose.chooseName", &json!({"token": "dupont", "name": "Marc Dupont", "email": "marc.dupont@contoso.example"})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().cc, "Marc Dupont <marc.dupont@contoso.example>; ");
    assert!(s.name_choice(id).is_none());
    // Asked again: answered from what the directory said, without asking it.
    s.run("compose.update", &json!({"bcc": "Reseau"})).unwrap();
    s.run("compose.checkNames", &json!({})).unwrap();
    assert!(asked(&mut s).is_empty());
    assert_eq!(s.compose.as_ref().unwrap().bcc, "Proginov Reseau <reseaux@contoso.example>; ");
    done(s);
}

/// The address book's Global Address List lists what the directory has for the name typed; a
/// name typed waits its turn in place of the one typed before it. The user's own address is in
/// the address book.
#[test]
fn the_global_address_list_is_searched_as_a_name_is_typed() {
    let (mut s, gal) = with_exchange("gal-book");
    let source = format!("directory:{gal}");
    assert!(s.address_books().iter().any(|(b, name, _)| *b == source && name == "Global Address List"));
    assert!(s.address_book_now(&source, "").is_empty() && asked(&mut s).is_empty(), "nothing typed, nothing asked");
    for typed in ["p", "pr", "pro"] {
        assert!(s.address_book_now(&source, typed).is_empty());
    }
    assert_eq!(asked(&mut s), ["p"], "the first, while the others wait");
    assert!(s.directory_searching(&gal, "pro") && !s.directory_searching(&gal, "pr"), "the last typed waits in place of the one before");
    let revision = s.contacts_revision();
    answer(&mut s, &gal, "p", &[("Proginov Reseau", "reseaux@contoso.example"), ("Paul Petit", "paul@contoso.example")]);
    assert!(s.contacts_revision() > revision, "the window lists it again");
    assert_eq!(asked(&mut s), ["pro"]);
    answer(&mut s, &gal, "pro", &[("Proginov Reseau", "reseaux@contoso.example")]);
    assert_eq!(s.address_book_now(&source, "Pro"), [EmailAddress::new("Proginov Reseau", "reseaux@contoso.example")]);
    // An answer that failed is asked again a while later, not every time it's wanted.
    s.apply(Event::DirectoryPeople { account: gal.clone(), query: "zed".into(), people: Vec::new(), more: false, error: Some("offline".into()) });
    s.search_directory(&gal, "zed", false);
    assert!(asked(&mut s).is_empty());
    // The user's own addresses are in the address book.
    let all = s.address_book_now("all", "riley");
    assert!(all.iter().any(|a| a.email == "riley@studio.example") && all.iter().any(|a| a.email == "riley@contoso.example"), "{all:?}");
    done(s);
}
