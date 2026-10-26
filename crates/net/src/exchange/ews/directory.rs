//! EWS: the organisation's address book (the Global Address List): people found by name or
//! address, as Outlook's Check Names finds them, and the certificates it keeps for them, to
//! encrypt to a colleague who never sent signed mail, as Outlook does.

use quatrelle_model::EmailAddress;

use super::*;

/// The biggest certificate (or bundle of them) taken from the directory.
const MAX_CERT: usize = 64 * 1024;

/// The longest name or address looked up.
const MAX_QUERY: usize = 255;

impl Client {
    /// The people, groups and rooms the directory has for `query` (the start of a name, a first
    /// or last name, an alias or an address: Exchange's ambiguous name resolution), as many as
    /// it gives (a hundred at most), and whether it has more.
    pub fn search_directory(&self, query: &str) -> Result<(Vec<EmailAddress>, bool)> {
        let query: String = query.trim().chars().take(MAX_QUERY).collect();
        if query.is_empty() {
            return Ok((Vec::new(), false));
        }
        let doc = self.call(&format!(
            "<m:ResolveNames ReturnFullContactData=\"false\" SearchScope=\"ActiveDirectory\"><m:UnresolvedEntry>{}</m:UnresolvedEntry></m:ResolveNames>",
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

/// The people in a `ResolveNames` answer (those with an internet address), and whether the
/// directory has more.
pub(super) fn people_of(m: &El) -> (Vec<EmailAddress>, bool) {
    let mut resolutions = Vec::new();
    m.all("Resolution", &mut resolutions);
    let mut out: Vec<EmailAddress> = Vec::new();
    for r in resolutions {
        let Some(b) = r.child("Mailbox") else { continue };
        let email = b.text_of("EmailAddress").unwrap_or("").trim();
        if !email.contains('@') || out.iter().any(|a| a.email.eq_ignore_ascii_case(email)) {
            continue;
        }
        out.push(EmailAddress::new(b.text_of("Name").unwrap_or("").trim(), email));
    }
    let more = m.find("ResolutionSet").and_then(|s| s.attr("IncludesLastItemInRange")) == Some("false");
    (out, more)
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
        assert_eq!(people, [EmailAddress::new("Proginov R\u{e9}seau", "reseaux@proginov.example")], "an X.500 name and a second copy left out");
        assert!(more);
    }
}
