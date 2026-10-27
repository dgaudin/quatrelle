//! Engine tests: contacts, and the pictures the directory keeps of people.

use super::*;

/// A contact keeps Outlook's work details: the command stores them, and reading the card back
/// gives them, so CardDAV, Exchange and a .pst carry the same fields.
#[test]
fn a_contact_keeps_outlooks_fields() {
    let mut s = sample_session("contact-fields");
    let v = s
        .run(
            "contact.create",
            &json!({
                "name": "Ana Lima",
                "emails": ["ana@contoso.example"],
                "org": "Contoso",
                "department": "Story",
                "office": "Room 4",
                "title": "Producer",
                "profession": "Storyteller",
                "manager": "Riley Kim",
                "assistant": "Sam Lee",
                "web": "https://ana.example",
                "im": "ana@chat.example",
                "nickname": "A",
                "spouse": "Jo Lima",
                "anniversary": "2018-06-02",
                "birthday": "1990-07-12",
            }),
        )
        .unwrap();
    let uid = v["uid"].as_str().unwrap_or_default().to_string();
    assert!(!uid.is_empty(), "{v}");
    let p = s.find_person(&uid).unwrap();
    let c = &p.card;
    assert_eq!((c.org.as_str(), c.department.as_str(), c.office.as_str()), ("Contoso", "Story", "Room 4"));
    assert_eq!((c.title.as_str(), c.profession.as_str()), ("Producer", "Storyteller"));
    assert_eq!((c.manager.as_str(), c.assistant.as_str()), ("Riley Kim", "Sam Lee"));
    assert_eq!((c.web.as_str(), c.im.as_str()), ("https://ana.example", "ana@chat.example"));
    assert_eq!((c.nickname.as_str(), c.spouse.as_str()), ("A", "Jo Lima"));
    assert_eq!((c.birthday.as_str(), c.anniversary.as_str()), ("1990-07-12", "2018-06-02"));
    // Changing one leaves the others; a wrong date is refused.
    s.run("contact.update", &json!({"uid": uid, "office": "Room 5"})).unwrap();
    let c = s.find_person(&uid).unwrap().card;
    assert_eq!((c.office.as_str(), c.department.as_str(), c.anniversary.as_str()), ("Room 5", "Story", "2018-06-02"));
    assert!(s.run("contact.update", &json!({"uid": uid, "anniversary": "June"})).is_err());
}

/// A contact's own picture is looked up by the item's id: Outlook keeps a contact's photo on the
/// contact item, and the server knows it by that id.
#[test]
fn a_contacts_own_picture_is_asked_for_by_its_id() {
    let mut s = sample_session("contact-photo");
    s.accounts[0].config.ews = Some("https://mail.example/EWS/Exchange.asmx".into());
    let account = s.accounts[0].config.id.clone();
    s.pim_mut(&account).cards.push(quatrelle_model::pim::StoredCard {
        book: "contacts".into(),
        href: "contact:C9".into(),
        etag: String::new(),
        card: quatrelle_model::vcard::Card {
            uid: "C9".into(),
            name: "Ana Lima".into(),
            emails: vec![quatrelle_model::vcard::Typed { value: "ana@contoso.example".into(), kind: "work".into() }],
            ..Default::default()
        },
        dirty: false,
    });
    s.run("contact.photo", &json!({"email": "Ana@Contoso.Example"})).unwrap();
    let jobs = s.take_jobs();
    assert_eq!(jobs.len(), 1);
    assert!(matches!(&jobs[0], quatrelle_model::Job::FetchPhoto { contact: Some(id), .. } if id == "C9"), "the item's id goes with the look-up");
}

/// A directory keeps a person's picture, at most once asked for: the People card shows it, and
/// an address without one isn't asked for again.
#[test]
fn a_directory_picture_is_asked_for_once_and_kept() {
    let mut s = sample_session("directory-photo");
    assert!(!s.person_photo_known("zoe@studio.example"));
    // An Exchange account is what can ask; the sample's own account stands in for one.
    s.accounts[0].config.ews = Some("https://mail.example/EWS/Exchange.asmx".into());
    let v = s.run("contact.photo", &json!({"email": "Zoe@Studio.Example"})).unwrap();
    assert_eq!(v["asked"], true, "{v}");
    let jobs = s.take_jobs();
    assert_eq!(jobs.len(), 1, "one look-up");
    assert!(matches!(&jobs[0], quatrelle_model::Job::FetchPhoto { email, .. } if email == "zoe@studio.example"));
    s.run("contact.photo", &json!({"email": "zoe@studio.example"})).unwrap();
    assert!(s.take_jobs().is_empty(), "already asked for");
    // The answer is kept by address, without case; a "none" is kept too.
    s.apply(quatrelle_model::Event::Photo {
        email: "Zoe@Studio.Example".into(),
        media: "image/jpeg".into(),
        data: vec![0xFF, 0xD8, 0xFF],
        error: None,
    });
    assert!(s.person_photo_known("zoe@studio.example"));
    let p = s.person_photo("zoe@studio.example").unwrap();
    assert_eq!(p.media, "image/jpeg");
    assert_eq!(p.bytes(1024).unwrap(), vec![0xFF, 0xD8, 0xFF]);
    s.apply(quatrelle_model::Event::Photo {
        email: "ghost@studio.example".into(),
        media: String::new(),
        data: Vec::new(),
        error: Some("no picture for this person".into()),
    });
    assert!(s.person_photo_known("ghost@studio.example"), "a person without a picture isn't asked for again");
    assert!(s.person_photo("ghost@studio.example").is_none());
}

/// The contact a message's header names is found by its address alone, whatever its case, and
/// not by a name or part of an address (a search found those, reading every field of every card).
#[test]
fn a_contact_is_found_by_its_address() {
    let mut s = sample_session("person-by-email");
    s.run("contact.create", &json!({"name": "Ana Lima", "emails": ["Ana.Lima@Contoso.example"]})).unwrap();
    s.run("contact.create", &json!({"name": "Analyst Desk", "emails": ["desk@contoso.example"]})).unwrap();
    let found = s.person_by_email(" ana.lima@contoso.example ").map(|p| p.card.display());
    assert_eq!(found.as_deref(), Some("Ana Lima"));
    assert!(s.person_by_email("ana").is_none(), "not by a name");
    assert!(s.person_by_email("contoso.example").is_none(), "nor by part of an address");
    assert!(s.person_by_email("").is_none());
    done(s);
}
