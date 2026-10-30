//! EWS: the organisation's address book (the Global Address List) and the account's contacts:
//! people, lists and contact groups found by name or address, as Outlook's Check Names finds them,
//! each with its kind; a list's members (`ExpandDL`); and the certificates the directory keeps for
//! people, to encrypt to a colleague who never sent signed mail, as Outlook does.

use quatrelle_model::addr::valid_item_id;
use quatrelle_model::{Addressee, AddresseeKind};

use super::*;

/// The biggest certificate (or bundle of them) taken from the directory.
const MAX_CERT: usize = 64 * 1024;

/// The longest name or address looked up.
const MAX_QUERY: usize = 255;

/// The most resolutions read from one `ResolveNames` answer (Exchange gives a hundred at most).
const MAX_ANSWER: usize = 100;

/// The most members read from one `ExpandDL` answer.
const MAX_MEMBERS: usize = 500;

/// The longest name kept (characters; a longer one is cut).
const MAX_NAME: usize = 200;

/// The longest address kept (bytes; an entry with a longer one is dropped).
const MAX_ADDRESS: usize = 320;

/// The longest item id kept (a contact group with a longer one is dropped).
const MAX_ITEM_ID: usize = 512;

impl Client {
    /// The people, lists, contact groups and rooms the directory and the account's contacts have
    /// for `query` (the start of a name, a first or last name, an alias or an address: Exchange's
    /// ambiguous name resolution), each with its kind, as many as it gives (a hundred at most),
    /// and whether it has more.
    pub fn search_directory(&self, query: &str) -> Result<(Vec<Addressee>, bool)> {
        let query: String = query.trim().chars().take(MAX_QUERY).collect();
        if query.is_empty() {
            return Ok((Vec::new(), false));
        }
        let doc = self.call(&format!(
            "<m:ResolveNames ReturnFullContactData=\"false\" SearchScope=\"ActiveDirectoryContacts\"><m:UnresolvedEntry>{}</m:UnresolvedEntry></m:ResolveNames>",
            esc(&query)
        ))?;
        let Some(m) = messages(&doc).first().map(|m| (*m).clone()) else { return Ok((Vec::new(), false)) };
        if m.text_of("ResponseCode") == Some("ErrorNameResolutionNoResults") {
            return Ok((Vec::new(), false));
        }
        // Several matches answer with a warning, and the matches.
        if m.attr("ResponseClass") == Some("Error") {
            check_message(&m)?;
        }
        Ok(people_of(&m))
    }

    /// The members of a list: a distribution list by its `address`, or a contact group by its
    /// `item_id` (which wins when both are given). Each member comes with its kind (a list in it
    /// stays a list, to be expanded in turn); also whether the server left some out, and how many
    /// it says the list has. A list the server doesn't know has none.
    pub fn expand_list(&self, address: &str, item_id: Option<&str>) -> Result<(Vec<Addressee>, bool, usize)> {
        let mailbox = match item_id.map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) if valid_item_id(id) => format!("<t:ItemId Id=\"{}\"/>", esc(id)),
            Some(_) => return Err(NetError::No("that contact group's id isn't one Exchange gives".into())),
            None => {
                let address = address.trim();
                if address.is_empty() {
                    return Ok((Vec::new(), false, 0));
                }
                if address.len() > MAX_ADDRESS {
                    return Err(NetError::No("that list's address is too long".into()));
                }
                format!("<t:EmailAddress>{}</t:EmailAddress>", esc(address))
            }
        };
        let doc = self.call(&format!("<m:ExpandDL><m:Mailbox>{mailbox}</m:Mailbox></m:ExpandDL>"))?;
        let Some(m) = messages(&doc).first().map(|m| (*m).clone()) else { return Ok((Vec::new(), false, 0)) };
        if m.text_of("ResponseCode") == Some("ErrorNameResolutionNoResults") {
            return Ok((Vec::new(), false, 0));
        }
        check_message(&m)?;
        Ok(members_of(&m))
    }

    /// The certificates Exchange's directory keeps for an address, as DER: those Outlook
    /// published (`userCertificate`, EWS's `MSExchangeCertificate`) and the S/MIME bundles
    /// (`userSMIMECertificate`, a PKCS #7 that signs nothing). None when nobody has that
    /// address there, or nothing is kept for them.
    pub fn directory_certificates(&self, email: &str) -> Result<Vec<Vec<u8>>> {
        let doc = self.call(&format!(
            "<m:ResolveNames ReturnFullContactData=\"true\" SearchScope=\"ActiveDirectory\" ContactDataShape=\"AllProperties\">\
             <m:UnresolvedEntry>SMTP:{}</m:UnresolvedEntry></m:ResolveNames>",
            esc(email.trim())
        ))?;
        let Some(m) = messages(&doc).first().map(|m| (*m).clone()) else { return Ok(Vec::new()) };
        if m.text_of("ResponseCode") == Some("ErrorNameResolutionNoResults") {
            return Ok(Vec::new());
        }
        // Several matches answer with a warning, and the matches.
        if m.attr("ResponseClass") == Some("Error") {
            check_message(&m)?;
        }
        Ok(certificates_of(&m, email))
    }
}

/// One entry of a `ResolveNames` or `ExpandDL` answer (a `Mailbox`), read with care: its name cut
/// to a length; kept only with an internet address of a sane length, or as a contact group with a
/// usable item id; its kind from its `MailboxType` (one nobody documented reads as unknown).
fn addressee_of(b: &El) -> Option<Addressee> {
    let name: String = b.text_of("Name").unwrap_or("").trim().chars().take(MAX_NAME).collect();
    let kind = AddresseeKind::from_mailbox_type(b.text_of("MailboxType").unwrap_or("").trim());
    let address = b.text_of("EmailAddress").unwrap_or("").trim();
    let item_id = b.child("ItemId").and_then(|i| i.attr("Id")).unwrap_or("").trim();
    let item_id = if kind == AddresseeKind::ContactGroup && item_id.len() <= MAX_ITEM_ID && valid_item_id(item_id) { item_id } else { "" };
    if address.contains('@') && address.len() <= MAX_ADDRESS {
        return Some(Addressee { item_id: item_id.to_string(), ..Addressee::new(name, address, kind) });
    }
    // No address: only a contact group (the account's own), named by its item.
    (kind == AddresseeKind::ContactGroup && !item_id.is_empty()).then(|| Addressee { item_id: item_id.to_string(), ..Addressee::new(name, "", kind) })
}

/// Whether `a` is already among `out`: the same address, or (a group without one) the same item.
fn known(out: &[Addressee], a: &Addressee) -> bool {
    out.iter().any(|x| if a.address.is_empty() { x.address.is_empty() && x.item_id == a.item_id } else { x.address.eq_ignore_ascii_case(&a.address) })
}

/// The entries in a `ResolveNames` answer (people with an internet address, lists, the account's
/// contact groups), each once, from the first hundred resolutions; and whether the directory has
/// more (it says so, or it gave more than were read).
pub(super) fn people_of(m: &El) -> (Vec<Addressee>, bool) {
    let mut resolutions = Vec::new();
    m.all("Resolution", &mut resolutions);
    let mut out: Vec<Addressee> = Vec::new();
    for r in resolutions.iter().take(MAX_ANSWER) {
        let Some(a) = r.child("Mailbox").and_then(addressee_of) else { continue };
        if !known(&out, &a) {
            out.push(a);
        }
    }
    let more = resolutions.len() > MAX_ANSWER || m.find("ResolutionSet").and_then(|s| s.attr("IncludesLastItemInRange")) == Some("false");
    (out, more)
}

/// The members in an `ExpandDL` answer, each once, five hundred at most; whether the server has
/// more (it says so, or it gave more than were read); and how many it says the list has (as many
/// as it listed, when it doesn't say).
pub(super) fn members_of(m: &El) -> (Vec<Addressee>, bool, usize) {
    let Some(x) = m.find("DLExpansion") else { return (Vec::new(), false, 0) };
    let listed = x.children("Mailbox").count();
    let mut out: Vec<Addressee> = Vec::new();
    for b in x.children("Mailbox").take(MAX_MEMBERS) {
        let Some(a) = addressee_of(b) else { continue };
        if !known(&out, &a) {
            out.push(a);
        }
    }
    let more = listed > MAX_MEMBERS || x.attr("IncludesLastItemInRange") == Some("false");
    let total = x.attr("TotalItemsInView").and_then(|t| t.trim().parse::<usize>().ok()).unwrap_or(listed).max(out.len());
    (out, more, total)
}

/// The certificates in a `ResolveNames` answer for the entry with this address.
pub(super) fn certificates_of(m: &El, email: &str) -> Vec<Vec<u8>> {
    let mut resolutions = Vec::new();
    m.all("Resolution", &mut resolutions);
    let mut out: Vec<Vec<u8>> = Vec::new();
    for r in resolutions {
        let mail = r.child("Mailbox").and_then(|b| b.text_of("EmailAddress")).unwrap_or("");
        let Some(contact) = r.child("Contact").filter(|_| mail.trim().eq_ignore_ascii_case(email.trim())) else { continue };
        for field in ["MSExchangeCertificate", "UserSMIMECertificate"] {
            for b in contact.children(field).flat_map(|f| f.children("Base64Binary")) {
                let Some(der) = unb64(b.text.trim()).filter(|d| !d.is_empty() && d.len() <= MAX_CERT) else { continue };
                // A certificate itself, or a bundle of them.
                let certs =
                    if quatrelle_smime::cert::info(&der).is_ok() { vec![der] } else { quatrelle_smime::certificates(&der).unwrap_or_default() };
                for c in certs {
                    if !out.contains(&c) {
                        out.push(c);
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use quatrelle_model::EmailAddress;

    use super::*;

    fn testdata(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../smime/testdata").join(name)).unwrap()
    }

    #[test]
    fn the_address_book_gives_a_persons_certificates_and_bundles() {
        let pem = String::from_utf8(testdata("ana.pem")).unwrap();
        let ana = der::pem::decode_vec(pem.as_bytes()).unwrap().1;
        // A signature carries its signer's certificate: as a bundle, it gives it too.
        let bundle = testdata("sig-rsa.der");
        let from_bundle = quatrelle_smime::certificates(&bundle).unwrap();
        assert!(!from_bundle.is_empty());
        let answer = format!(
            "<m:ResolveNamesResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:ResolutionSet>\
             <t:Resolution><t:Mailbox><t:EmailAddress>Ana@Posteo.example</t:EmailAddress></t:Mailbox><t:Contact>\
             <t:MSExchangeCertificate><t:Base64Binary>{}</t:Base64Binary><t:Base64Binary>not a certificate</t:Base64Binary></t:MSExchangeCertificate>\
             <t:UserSMIMECertificate><t:Base64Binary>{}</t:Base64Binary></t:UserSMIMECertificate></t:Contact></t:Resolution>\
             <t:Resolution><t:Mailbox><t:EmailAddress>other@posteo.example</t:EmailAddress></t:Mailbox><t:Contact>\
             <t:MSExchangeCertificate><t:Base64Binary>{}</t:Base64Binary></t:MSExchangeCertificate></t:Contact></t:Resolution>\
             </m:ResolutionSet></m:ResolveNamesResponseMessage>",
            b64(&ana),
            b64(&bundle),
            b64(&ana)
        );
        let certs = certificates_of(&parse(&answer), "ana@posteo.example");
        assert_eq!(certs.first(), Some(&ana), "the published certificate first");
        assert!(from_bundle.iter().all(|c| certs.contains(c)), "and those of the bundle");
        assert_eq!(certs.iter().filter(|c| **c == ana).count(), 1, "each once");
        assert!(certificates_of(&parse(&answer), "nobody@posteo.example").is_empty());
    }

    #[test]
    fn the_directory_gives_people_with_an_internet_address() {
        let answer = "<m:ResolveNamesResponseMessage ResponseClass=\"Warning\"><m:ResponseCode>ErrorNameResolutionMultipleResults</m:ResponseCode>\
             <m:ResolutionSet TotalItemsInView=\"100\" IncludesLastItemInRange=\"false\">\
             <t:Resolution><t:Mailbox><t:Name>Proginov R\u{e9}seau</t:Name><t:EmailAddress>reseaux@proginov.example</t:EmailAddress>\
             <t:RoutingType>SMTP</t:RoutingType><t:MailboxType>Mailbox</t:MailboxType></t:Mailbox></t:Resolution>\
             <t:Resolution><t:Mailbox><t:Name>Old entry</t:Name><t:EmailAddress>/o=Org/cn=old</t:EmailAddress><t:RoutingType>EX</t:RoutingType></t:Mailbox></t:Resolution>\
             <t:Resolution><t:Mailbox><t:Name>Again</t:Name><t:EmailAddress>Reseaux@proginov.example</t:EmailAddress></t:Mailbox></t:Resolution>\
             </m:ResolutionSet></m:ResolveNamesResponseMessage>";
        let (people, more) = people_of(&parse(answer));
        assert_eq!(
            people.iter().map(Addressee::email).collect::<Vec<_>>(),
            [EmailAddress::new("Proginov R\u{e9}seau", "reseaux@proginov.example")],
            "an X.500 name and a second copy left out"
        );
        assert_eq!(people[0].kind, AddresseeKind::Person);
        assert!(more);
    }

    fn resolution(inner: &str) -> String {
        format!("<t:Resolution><t:Mailbox>{inner}</t:Mailbox></t:Resolution>")
    }

    fn resolve_answer(resolutions: &str) -> String {
        format!(
            "<m:ResolveNamesResponseMessage ResponseClass=\"Warning\"><m:ResponseCode>ErrorNameResolutionMultipleResults</m:ResponseCode>\
             <m:ResolutionSet IncludesLastItemInRange=\"true\">{resolutions}</m:ResolutionSet></m:ResolveNamesResponseMessage>"
        )
    }

    #[test]
    fn kinds_and_contact_groups_come_from_resolve_names() {
        use AddresseeKind::*;
        let one = |name: &str, email: &str, kind: &str| {
            resolution(&format!(
                "<t:Name>{name}</t:Name><t:EmailAddress>{email}</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType><t:MailboxType>{kind}</t:MailboxType>"
            ))
        };
        let answer = resolve_answer(
            &[
                one("Anne Dupont", "anne@contoso.example", "Mailbox"),
                one("R\u{e9}seaux", "reseaux@contoso.example", "PublicDL"),
                resolution("<t:Name>Dupont family</t:Name><t:RoutingType>MAPIPDL</t:RoutingType><t:MailboxType>PrivateDL</t:MailboxType><t:ItemId Id=\"AAMk=\" ChangeKey=\"EQ\"/>"),
                one("Archives", "archives@contoso.example", "PublicFolder"),
                one("Projet Lyon", "projet.lyon@contoso.example", "GroupMailbox"),
                one("Marc Dupont", "marc@posteo.example", "Contact"),
                one("Odd", "odd@contoso.example", "Something"),
            ]
            .concat(),
        );
        let (people, _) = people_of(&parse(&answer));
        assert_eq!(
            people.iter().map(|p| p.kind).collect::<Vec<_>>(),
            [Person, DistributionList, ContactGroup, PublicFolder, Microsoft365Group, Person, Unknown]
        );
        assert_eq!((people[2].address.as_str(), people[2].item_id.as_str()), ("", "AAMk="));
        assert_eq!(people[2].name, "Dupont family");
    }

    #[test]
    fn a_hostile_directory_answer_is_capped_and_unknown_kinds_read_as_unknown() {
        let mut set = String::new();
        set.push_str(&resolution(&format!(
            "<t:Name>{}</t:Name><t:EmailAddress>long@contoso.example</t:EmailAddress><t:MailboxType>Mailbox</t:MailboxType>",
            "n".repeat(5000)
        )));
        set.push_str(&resolution("<t:Name>No id</t:Name><t:MailboxType>PrivateDL</t:MailboxType>"));
        set.push_str(&resolution(&format!(
            "<t:Name>Huge id</t:Name><t:MailboxType>PrivateDL</t:MailboxType><t:ItemId Id=\"{}\"/>",
            "A".repeat(10_000)
        )));
        set.push_str(&resolution(&format!("<t:Name>Far</t:Name><t:EmailAddress>{}@contoso.example</t:EmailAddress>", "a".repeat(400))));
        for i in 0..10_000 {
            set.push_str(&resolution(&format!(
                "<t:Name>P{i}</t:Name><t:EmailAddress>p{i}@contoso.example</t:EmailAddress><t:MailboxType>Weird{i}</t:MailboxType>"
            )));
        }
        let answer = resolve_answer(&set);
        let (people, more) = people_of(&parse(&answer));
        assert!(people.len() <= 100 && more, "{} people, more {more}", people.len());
        assert!(people.iter().all(|p| p.name.chars().count() <= 200 && p.address.len() <= 320 && p.item_id.len() <= 512));
        assert!(!people.iter().any(|p| p.kind == AddresseeKind::ContactGroup && p.item_id.is_empty()), "a group with no id is dropped");
        assert!(people.iter().skip(1).all(|p| p.kind == AddresseeKind::Unknown), "a type nobody documented reads as unknown");
        assert_eq!(people[0].address, "long@contoso.example", "a long name is cut, the entry kept");
    }

    #[test]
    fn a_list_is_expanded_with_its_members_and_how_many_more() {
        let answer = "<m:ExpandDLResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>\
             <m:DLExpansion TotalItemsInView=\"3\" IncludesLastItemInRange=\"false\">\
             <t:Mailbox><t:Name>Lyon team</t:Name><t:EmailAddress>lyon@contoso.example</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType>\
             <t:MailboxType>PublicDL</t:MailboxType></t:Mailbox>\
             <t:Mailbox><t:Name>Anne Dupont</t:Name><t:EmailAddress>anne@contoso.example</t:EmailAddress><t:MailboxType>Mailbox</t:MailboxType></t:Mailbox>\
             <t:Mailbox><t:Name>Gone</t:Name><t:MailboxType>Mailbox</t:MailboxType></t:Mailbox>\
             </m:DLExpansion></m:ExpandDLResponseMessage>";
        let (members, more, total) = members_of(&parse(answer));
        assert_eq!((members.len(), more, total), (2, true, 3));
        assert_eq!(members[0].kind, AddresseeKind::DistributionList, "a nested list stays a list");
        assert_eq!(members[1].email(), EmailAddress::new("Anne Dupont", "anne@contoso.example"));
    }

    #[test]
    fn a_hostile_expansion_is_capped() {
        let mut set = String::new();
        for i in 0..2000 {
            set.push_str(&format!("<t:Mailbox><t:Name>M{i}</t:Name><t:EmailAddress>m{i}@contoso.example</t:EmailAddress></t:Mailbox>"));
        }
        let answer = format!(
            "<m:ExpandDLResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>\
             <m:DLExpansion TotalItemsInView=\"not a number\" IncludesLastItemInRange=\"true\">{set}</m:DLExpansion></m:ExpandDLResponseMessage>"
        );
        let (members, more, total) = members_of(&parse(&answer));
        assert_eq!(members.len(), 500);
        assert!(more, "members left out are more");
        assert_eq!(total, 2000, "an unreadable total is how many were listed");
    }
}
