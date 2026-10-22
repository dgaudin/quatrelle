//! EWS contacts and sticky notes: items read as cards and notes, and written back.

use super::*;

/// A person, in the order EWS wants a contact's fields.
pub(super) fn contact_xml(c: &Card) -> String {
    let mut x = format!("<t:Contact><t:Body BodyType=\"Text\">{}</t:Body>", esc(&c.note));
    x.push_str(&format!("<t:DisplayName>{}</t:DisplayName>", esc(&c.display())));
    if !c.given.is_empty() {
        x.push_str(&format!("<t:GivenName>{}</t:GivenName>", esc(&c.given)));
    }
    if !c.org.is_empty() {
        x.push_str(&format!("<t:CompanyName>{}</t:CompanyName>", esc(&c.org)));
    }
    if !c.emails.is_empty() {
        x.push_str("<t:EmailAddresses>");
        for (i, e) in c.emails.iter().take(3).enumerate() {
            x.push_str(&format!("<t:Entry Key=\"EmailAddress{}\">{}</t:Entry>", i + 1, esc(&e.value)));
        }
        x.push_str("</t:EmailAddresses>");
    }
    // Its addresses (Exchange keeps one each of business, home and other).
    let addresses: Vec<(&str, &PostalAddress)> = ["Business", "Home", "Other"]
        .into_iter()
        .filter_map(|key| {
            let kind = match key {
                "Business" => "work",
                "Home" => "home",
                _ => "",
            };
            c.addresses
                .iter()
                .find(|a| !a.is_empty() && (a.kind == kind || (key == "Other" && !matches!(a.kind.as_str(), "work" | "home"))))
                .map(|a| (key, a))
        })
        .collect();
    if !addresses.is_empty() {
        x.push_str("<t:PhysicalAddresses>");
        for (key, a) in addresses {
            x.push_str(&format!("<t:Entry Key=\"{key}\">"));
            for (name, v) in
                [("Street", &a.street), ("City", &a.city), ("State", &a.region), ("CountryOrRegion", &a.country), ("PostalCode", &a.postcode)]
            {
                if !v.trim().is_empty() {
                    x.push_str(&format!("<t:{name}>{}</t:{name}>", esc(v.trim())));
                }
            }
            x.push_str("</t:Entry>");
        }
        x.push_str("</t:PhysicalAddresses>");
    }
    if !c.phones.is_empty() {
        x.push_str("<t:PhoneNumbers>");
        let mut used: Vec<&str> = Vec::new();
        for p in &c.phones {
            let kind = match p.kind.as_str() {
                "cell" | "mobile" => "cell",
                "home" => "home",
                "fax" => "fax",
                "pager" => "pager",
                "work" => "work",
                _ => "",
            };
            // The first free entry for its kind (another number of a kind already full goes to
            // Other).
            let key = PHONE_KEYS
                .iter()
                .find(|(k, kd)| *kd == kind && !used.contains(k))
                .or_else(|| PHONE_KEYS.iter().find(|(k, _)| *k == "OtherTelephone" && !used.contains(k)))
                .map(|(k, _)| *k);
            if let Some(key) = key {
                used.push(key);
                x.push_str(&format!("<t:Entry Key=\"{key}\">{}</t:Entry>", esc(&p.value)));
            }
        }
        x.push_str("</t:PhoneNumbers>");
    }
    // A birthday at midday UTC, so it's the same day everywhere.
    // A birthday with no year (Exchange's placeholder year says so) goes back with it.
    if let Some(b) = quatrelle_model::vcard::birthday_for_exchange(&c.birthday) {
        x.push_str(&format!("<t:Birthday>{b}T12:00:00Z</t:Birthday>"));
    }
    if !c.title.is_empty() {
        x.push_str(&format!("<t:JobTitle>{}</t:JobTitle>", esc(&c.title)));
    }
    // Outlook's work details, which Exchange keeps on the contact.
    for (name, value) in [
        ("Department", &c.department),
        ("OfficeLocation", &c.office),
        ("Manager", &c.manager),
        ("AssistantName", &c.assistant),
        ("Profession", &c.profession),
        ("BusinessHomePage", &c.web),
        ("NickName", &c.nickname),
        ("SpouseName", &c.spouse),
    ] {
        if !value.trim().is_empty() {
            x.push_str(&format!("<t:{name}>{}</t:{name}>", esc(value.trim())));
        }
    }
    if !c.im.is_empty() {
        x.push_str(&format!("<t:IMAddresses><t:String>{}</t:String></t:IMAddresses>", esc(&c.im)));
    }
    if let Some(a) = quatrelle_model::vcard::birthday_of(&c.anniversary).filter(|b| !b.starts_with("--")) {
        x.push_str(&format!("<t:WeddingAnniversary>{a}T12:00:00Z</t:WeddingAnniversary>"));
    }
    if !c.family.is_empty() {
        x.push_str(&format!("<t:Surname>{}</t:Surname>", esc(&c.family)));
    }
    x.push_str("</t:Contact>");
    x
}

/// A contact as a card.
pub fn card_of(it: &El) -> Option<Card> {
    let id = it.child("ItemId").and_then(|i| i.attr("Id"))?.to_string();
    let entries = |list: &str| -> Vec<(String, String)> {
        it.child(list)
            .map(|l| l.children("Entry").map(|e| (e.attr("Key").unwrap_or("").to_string(), e.text.trim().to_string())).collect())
            .unwrap_or_default()
    };
    let emails = entries("EmailAddresses")
        .into_iter()
        .filter(|(_, v)| v.contains('@'))
        .map(|(_, v)| Typed { value: v.trim_start_matches("SMTP:").trim_start_matches("smtp:").to_string(), kind: String::new() })
        .collect();
    let mut phones: Vec<Typed> = entries("PhoneNumbers")
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| {
            let kind = match k.as_str() {
                "MobilePhone" => "cell",
                "HomePhone" | "HomePhone2" => "home",
                "BusinessFax" | "HomeFax" | "OtherFax" => "fax",
                "BusinessPhone" | "BusinessPhone2" | "CompanyMainPhone" => "work",
                "Pager" => "pager",
                _ => "",
            };
            Typed { value: v, kind: kind.into() }
        })
        .collect();
    // As Outlook lists them: business, home, mobile, then the rest.
    phones.sort_by_key(|p| match p.kind.as_str() {
        "work" => 0,
        "home" => 1,
        "cell" => 2,
        _ => 3,
    });
    let addresses = it
        .child("PhysicalAddresses")
        .map(|l| {
            l.children("Entry")
                .map(|e| PostalAddress {
                    kind: match e.attr("Key").unwrap_or("") {
                        "Business" => "work",
                        "Home" => "home",
                        _ => "",
                    }
                    .into(),
                    street: e.text_of("Street").unwrap_or("").trim().replace("\r\n", "\n"),
                    city: e.text_of("City").unwrap_or("").trim().to_string(),
                    region: e.text_of("State").unwrap_or("").trim().to_string(),
                    postcode: e.text_of("PostalCode").unwrap_or("").trim().to_string(),
                    country: e.text_of("CountryOrRegion").unwrap_or("").trim().to_string(),
                })
                .filter(|a| !a.is_empty())
                .collect()
        })
        .unwrap_or_default();
    // Exchange keeps a birthday (and a wedding anniversary) as the start of that day where it
    // was set, in UTC (the evening before, east of Greenwich); Quatrelle writes noon. Either
    // way, the day from just after noon the day before to noon is the one.
    let day_of = |name: &str| -> String {
        it.text_of(name)
            .and_then(|b| quatrelle_jmap::utc_date(b.trim()))
            .map(|t| {
                let (y, m, d) = quatrelle_model::dates::civil_from_days((t + 12 * 3600 - 1).div_euclid(86_400));
                // The placeholder year Exchange stores for a day and month only isn't a year.
                quatrelle_model::vcard::birthday_of(&format!("{y:04}-{m:02}-{d:02}")).unwrap_or_else(|| format!("{y:04}-{m:02}-{d:02}"))
            })
            .unwrap_or_default()
    };
    let body = it.child("Body").map(|b| b.text.trim().to_string()).unwrap_or_default();
    Some(Card {
        uid: id,
        name: text(it, "DisplayName"),
        given: text(it, "GivenName"),
        family: text(it, "Surname"),
        emails,
        phones,
        org: text(it, "CompanyName"),
        department: text(it, "Department"),
        title: text(it, "JobTitle"),
        profession: text(it, "Profession"),
        office: text(it, "OfficeLocation"),
        manager: text(it, "Manager"),
        assistant: text(it, "AssistantName"),
        web: text(it, "BusinessHomePage"),
        im: it.child("IMAddresses").and_then(|x| x.text_of("String")).map(str::trim).unwrap_or_default().to_string(),
        nickname: text(it, "NickName"),
        spouse: text(it, "SpouseName"),
        anniversary: day_of("WeddingAnniversary"),
        note: body,
        addresses,
        birthday: day_of("Birthday"),
        photo: None,
        extra: Vec::new(),
    })
}

/// Exchange's phone entries, by the kind each takes (two of business and home).
const PHONE_KEYS: [(&str, &str); 9] = [
    ("BusinessPhone", "work"),
    ("BusinessPhone2", "work"),
    ("HomePhone", "home"),
    ("HomePhone2", "home"),
    ("MobilePhone", "cell"),
    ("BusinessFax", "fax"),
    ("HomeFax", "fax"),
    ("Pager", "pager"),
    ("OtherTelephone", ""),
];

/// A sticky note's colour (MS-OXONOTE PidLidNoteColor), as EWS asks for it.
pub(super) const NOTE_COLOR: &str =
    r#"<t:ExtendedFieldURI PropertySetId="0006200E-0000-0000-C000-000000000046" PropertyId="35584" PropertyType="Integer"/>"#;

/// A note in Exchange's Notes folder (`IPM.StickyNote`).
pub fn note_of(it: &El) -> Option<StoredNote> {
    if !it.text_of("ItemClass").is_some_and(|c| c.starts_with("IPM.StickyNote")) {
        return None;
    }
    let id = it.child("ItemId")?;
    let href = format!("note:{}", id.attr("Id")?);
    let text = it.text_of("Body").map(str::to_string).filter(|b| !b.trim().is_empty()).unwrap_or_else(|| text(it, "Subject"));
    let color = it
        .children("ExtendedProperty")
        .find(|p| p.find("ExtendedFieldURI").and_then(|f| f.attr("PropertyId")) == Some("35584"))
        .and_then(|p| p.text_of("Value"))
        .and_then(|v| v.trim().parse::<u8>().ok())
        .filter(|c| *c < quatrelle_model::pim::NOTE_COLORS)
        .unwrap_or(quatrelle_model::pim::NOTE_YELLOW);
    let modified = it.text_of("LastModifiedTime").and_then(quatrelle_jmap::utc_date).unwrap_or(0);
    Some(StoredNote {
        href: href.clone(),
        etag: id.attr("ChangeKey").unwrap_or("").to_string(),
        note: Note { uid: href, text: text.replace("\r\n", "\n"), color, modified },
        dirty: false,
    })
}

/// A note as EWS creates it: a message of class `IPM.StickyNote`, its first line the subject.
pub(super) fn note_xml(n: &Note) -> String {
    let subject: String = n.title().chars().take(255).collect();
    format!(
        "<t:Message><t:ItemClass>IPM.StickyNote</t:ItemClass><t:Subject>{}</t:Subject><t:Body BodyType=\"Text\">{}</t:Body>\
         <t:ExtendedProperty>{NOTE_COLOR}<t:Value>{}</t:Value></t:ExtendedProperty></t:Message>",
        esc(&subject),
        esc(&n.text),
        n.color
    )
}
