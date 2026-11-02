//! A message's text first: from a server that gives its body and the list of its files apart from
//! its source (Exchange), the message as MIME with its files named and sized but empty, to read
//! while the whole source (a 20 MB attachment) comes.

use super::*;

/// The most files listed: the rest is not shown.
pub const MOST_PENDING_FILES: usize = 500;

/// A file of a message whose text came first: what it is, not its bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingFile {
    pub name: String,
    pub mime: String,
    pub size: u64,
    /// `Content-ID`, for a picture the HTML shows.
    pub content_id: Option<String>,
    pub inline: bool,
}

/// Whether a file says it is signed or encrypted (S/MIME): its bytes are needed to read the
/// message at all.
pub fn is_protected_file(mime: &str, name: &str) -> bool {
    let kind = mime.trim().to_ascii_lowercase();
    ["application/pkcs7-mime", "application/x-pkcs7-mime", "multipart/signed"].iter().any(|k| kind.starts_with(k))
        || name.trim().eq_ignore_ascii_case("smime.p7m")
}

/// Text for a header: no line break can end it.
fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// The addresses that say something: a name alone stays (the engine reads an empty address as
/// "not known yet"), neither is dropped.
fn kept(v: &[EmailAddress]) -> Vec<EmailAddress> {
    v.iter()
        .map(|a| EmailAddress::new(one_line(a.name.trim()), one_line(a.email.trim())))
        .filter(|a| !a.name.trim().is_empty() || !a.email.trim().is_empty())
        .collect()
}

/// A media type only if it is plain: one `/`, under 128 bytes, no control character, `;` or space.
fn plain_type(m: &str) -> String {
    let m = m.trim();
    let ok = m.len() < 128 && m.matches('/').count() == 1 && !m.chars().any(|c| c.is_control() || c == ';' || c == ' ');
    if ok { m.to_ascii_lowercase() } else { "application/octet-stream".into() }
}

/// [`parse`] for a message built by [`text_first`], and only one: a sender or a recipient with a
/// name alone stays (an empty `email`: not known yet), as `text_first` writes it. Ordinary mail
/// goes through [`parse`], which keeps addresses only.
pub fn parse_text_first(raw: &[u8]) -> Option<Parsed> {
    crate::parse_as(raw, crate::addrs_with_names)
}

/// The message as MIME, its text and `files` (named and sized, no bytes): `d`'s headers and
/// body (HTML in `d.html` when there is some), from `from` at `date`, under `message_id`
/// (empty: none).
pub fn text_first(d: &Draft, from: &EmailAddress, date: i64, message_id: &str, files: &[PendingFile]) -> Result<Vec<u8>, String> {
    let mut b = MessageBuilder::new().subject(one_line(&d.subject)).date(date.max(0));
    if let Some(f) = kept(std::slice::from_ref(from)).first() {
        b = b.from(b_addr(f));
    }
    if !message_id.trim().is_empty() {
        b = b.message_id(quatrelle_model::thread::norm_id(&one_line(message_id)));
    }
    for (list, set) in [(kept(&d.to), 0), (kept(&d.cc), 1), (kept(&d.reply_to), 2)] {
        if list.is_empty() {
            continue;
        }
        b = match set {
            0 => b.to(b_list(&list)),
            1 => b.cc(b_list(&list)),
            _ => b.reply_to(b_list(&list)),
        };
    }
    let raw = mail_builder::headers::raw::Raw::new;
    match d.importance.signum() {
        1 => b = b.header("Importance", raw("high")),
        -1 => b = b.header("Importance", raw("low")),
        _ => {}
    }
    let text = match &d.html {
        Some(h) => MimePart::new(ContentType::new("text/html").attribute("charset", "utf-8"), h.clone()),
        None => MimePart::new(ContentType::new("text/plain").attribute("charset", "utf-8"), d.body.clone()),
    };
    let mut parts = vec![text];
    for f in files.iter().take(MOST_PENDING_FILES) {
        let name = one_line(&f.name);
        let disposition = ContentType::new(if f.inline { "inline" } else { "attachment" })
            .attribute("filename", name.clone())
            .attribute("size", f.size.to_string());
        let mut p =
            MimePart::new(ContentType::new(plain_type(&f.mime)).attribute("name", name), Vec::<u8>::new()).header("Content-Disposition", disposition);
        let cid = f.content_id.as_deref().map(|c| one_line(c).trim().trim_matches(['<', '>']).to_string()).filter(|c| !c.is_empty());
        if let Some(cid) = cid {
            p = p.header("Content-ID", mail_builder::headers::raw::Raw::new(format!("<{cid}>")));
        }
        parts.push(p);
    }
    let mut out = Vec::new();
    b.body(MimePart::new("multipart/mixed", parts)).write_to(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn its_text_now_its_files_named_and_sized() {
        let d = Draft {
            to: vec![EmailAddress::new("Riley", "riley@studio.example")],
            subject: "Rushes reel 2 — räder".into(),
            html: Some("<p>Here's reel 2.</p><img src=\"cid:still@x\">".into()),
            importance: 1,
            ..Default::default()
        };
        let files = [
            PendingFile { name: "reel2.mov".into(), mime: "video/quicktime".into(), size: 20_971_586, ..Default::default() },
            PendingFile { name: "still.png".into(), mime: "image/png".into(), size: 2048, content_id: Some("<still@x>".into()), inline: true },
            PendingFile { name: "bad\r\nX: y".into(), mime: "x\r\nEvil: 1".into(), size: 1, ..Default::default() },
        ];
        let raw = text_first(&d, &EmailAddress::new("Sam", "sam@studio.example"), 1_793_300_000, "<m1@studio.example>", &files).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("\nEvil:") && !String::from_utf8_lossy(&raw).contains("\nX: y"));
        let p = parse(&raw).unwrap();
        assert_eq!(p.subject, "Rushes reel 2 — räder");
        assert_eq!(p.message_id, "m1@studio.example");
        let sizes: Vec<(String, usize, bool)> = p.attachments.iter().map(|a| (a.name.clone(), a.size, a.inline)).collect();
        assert_eq!(sizes[0], ("reel2.mov".to_string(), 20_971_586, false));
        assert_eq!((sizes[1].1, sizes[1].2), (2048, true));
        assert!(p.text.contains("reel 2"));
    }

    #[test]
    fn a_name_alone_stays_a_name() {
        let d = Draft { to: vec![EmailAddress::new("Alex Martin", "")], subject: "s".into(), body: "b".into(), ..Default::default() };
        let raw = text_first(&d, &EmailAddress::new("Sam", "sam@x.example"), 1, "", &[]).unwrap();
        let p = parse_text_first(&raw).unwrap();
        assert_eq!((p.to[0].name.as_str(), p.to[0].email.as_str()), ("Alex Martin", ""));
    }

    #[test]
    fn ordinary_mail_keeps_only_addresses() {
        let raw = b"To: Alex Martin, b@x.example\r\nFrom: Nobody\r\nSender: s@x.example\r\nSubject: s\r\n\r\nb";
        assert_eq!(parse(raw).unwrap().to, vec![EmailAddress::new("", "b@x.example")]);
        assert_eq!(parse(raw).unwrap().from, EmailAddress::default(), "a name-only From stays out too");
        // The list falls back to the sender when From has no address.
        let sum = summary_from_header(raw, MessageKey::default(), Flags::default(), 0, 0);
        assert_eq!(sum.from.email, "s@x.example");
    }

    #[test]
    fn an_address_with_nothing_is_dropped() {
        let d = Draft { to: vec![EmailAddress::new(" ", ""), EmailAddress::new("", "a@b.example")], body: "b".into(), ..Default::default() };
        let raw = text_first(&d, &EmailAddress::default(), 1, "", &[]).unwrap();
        assert_eq!(parse_text_first(&raw).unwrap().to.len(), 1);
    }

    #[test]
    fn files_without_a_name_or_a_size_and_too_many() {
        let many = vec![PendingFile { mime: "application/pdf".into(), ..Default::default() }; MOST_PENDING_FILES + 20];
        let raw = text_first(&Draft::default(), &EmailAddress::default(), 0, "", &many).unwrap();
        assert_eq!(parse(&raw).unwrap().attachments.len(), MOST_PENDING_FILES);
    }

    #[test]
    fn an_odd_media_type_becomes_a_plain_one() {
        for odd in ["", "pdf", "a/b/c", "text/html; charset=x", "a /b", &format!("a/{}", "b".repeat(200))] {
            assert_eq!(plain_type(odd), "application/octet-stream", "{odd}");
        }
        assert_eq!(plain_type("Image/PNG"), "image/png");
    }

    #[test]
    fn what_says_it_is_signed_or_encrypted() {
        assert!(is_protected_file("application/pkcs7-mime; smime-type=enveloped-data", "x.bin"));
        assert!(is_protected_file("Multipart/Signed", ""));
        assert!(is_protected_file("application/octet-stream", "SMIME.P7M"));
        assert!(!is_protected_file("application/pdf", "report.pdf"));
    }

    proptest! {
        /// Nothing the server says adds a header: every header name is one the builder writes.
        #[test]
        fn hostile_text_adds_no_header(subject in ".{0,80}", name in ".{0,80}", kind in ".{0,40}", cid in ".{0,40}", who in ".{0,40}") {
            let to = vec![EmailAddress::new(format!("{who}\r\nEvil: 1"), "a@b.example")];
            let d = Draft { to, subject: format!("{subject}\r\nEvil: 1"), body: "x".into(), ..Default::default() };
            let file = PendingFile { name: format!("{name}\r\nEvil: 1"), mime: format!("{kind}\r\nEvil: 1"), size: 1, content_id: Some(format!("{cid}\r\nEvil: 1")), inline: true };
            let raw = text_first(&d, &EmailAddress::default(), 0, "", &[file]).unwrap();
            let text = String::from_utf8_lossy(&raw);
            prop_assert!(!text.contains("\nEvil:"));
        }
    }
}
