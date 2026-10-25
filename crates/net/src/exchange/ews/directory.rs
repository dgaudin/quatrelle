//! EWS: the certificates the organisation's address book keeps for people, to encrypt to a
//! colleague who never sent signed mail, as Outlook does.

use super::*;

/// The biggest certificate (or bundle of them) taken from the directory.
const MAX_CERT: usize = 64 * 1024;

impl Client {
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
}
