//! Tests over the fake EWS server: the directory searched with the account's contacts, each entry
//! with its kind, and a list (or a contact group) expanded into its members.

use quatrelle_model::{Addressee, AddresseeKind};

use super::*;

fn client(server: &super::server::Server) -> crate::ews::Client {
    crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap()
}

fn person(name: &str, address: &str) -> (String, String) {
    (name.into(), address.into())
}

/// A name finds the people of the directory and the account's contact groups, each with its kind;
/// the directory is asked with the contacts; a distribution list and a contact group expand into
/// their members, and a list nobody has into none.
#[test]
fn the_directory_is_searched_with_contacts_and_a_list_expands() {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.people = vec![
        person("Anne Dupont", "anne.dupont@contoso.example"),
        person("Marc Dupont", "marc.dupont@contoso.example"),
        person("Proginov Reseaux", "reseaux@contoso.example"),
    ];
    d.mailbox_types = vec![("reseaux@contoso.example".into(), "PublicDL".into())];
    d.contact_groups = vec![("Dupont family".into(), "AAMk=".into())];
    d.list_members = vec![
        ("reseaux@contoso.example".into(), vec![person("Zoe Park", "zoe@contoso.example"), person("Remi Restout", "remi@contoso.example")]),
        ("AAMk=".into(), vec![person("Anne Dupont", "anne.dupont@contoso.example")]),
    ];
    let server = serve(d);
    let c = client(&server);

    let (people, more) = c.search_directory("dupont").unwrap();
    assert!(!more);
    assert_eq!(
        people,
        [
            Addressee::new("Anne Dupont", "anne.dupont@contoso.example", AddresseeKind::Person),
            Addressee::new("Marc Dupont", "marc.dupont@contoso.example", AddresseeKind::Person),
            Addressee { item_id: "AAMk=".into(), ..Addressee::new("Dupont family", "", AddresseeKind::ContactGroup) },
        ]
    );
    assert_eq!(server.data.lock().unwrap().scopes, ["ActiveDirectoryContacts"]);
    let (list, _) = c.search_directory("reseaux").unwrap();
    assert_eq!(list.iter().map(|p| p.kind).collect::<Vec<_>>(), [AddresseeKind::DistributionList], "a list says it's one");

    let (members, more, total) = c.expand_list("reseaux@contoso.example", None).unwrap();
    assert_eq!(members.iter().map(|m| m.address.as_str()).collect::<Vec<_>>(), ["zoe@contoso.example", "remi@contoso.example"]);
    assert_eq!((more, total), (false, 2));
    assert!(members.iter().all(|m| m.kind == AddresseeKind::Person));
    let (members, _, total) = c.expand_list("", Some("AAMk=")).unwrap();
    assert_eq!((members.len(), total), (1, 1));
    assert_eq!(members[0].address, "anne.dupont@contoso.example");
    assert_eq!(c.expand_list("nobody@contoso.example", None).unwrap(), (vec![], false, 0));
}
