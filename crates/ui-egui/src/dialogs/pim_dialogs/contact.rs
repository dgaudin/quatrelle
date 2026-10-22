//! The contact form: Outlook's, with its emails, phones by kind, addresses and birthday.

use super::*;

/// Outlook's phone fields, and the kind each keeps.
const PHONES: [(&str, &str); 5] = [("Business", "work"), ("Home", "home"), ("Mobile", "cell"), ("Fax", "fax"), ("Other", "")];
/// Outlook's addresses, and the kind each keeps.
const ADDRESSES: [(&str, &str); 3] = [("Business", "work"), ("Home", "home"), ("Other", "")];

/// A person being added or edited: Outlook's contact form (its emails, phones by kind, business,
/// home and other address, birthday).
#[derive(Clone, Debug, Default)]
pub struct ContactForm {
    pub uid: Option<String>,
    pub name: String,
    /// Email, Email 2 and Email 3.
    pub emails: [String; 3],
    /// More than three, kept as they were.
    pub more_emails: Vec<String>,
    /// Business, home, mobile, fax and other (as [`PHONES`]).
    pub phones: [String; 5],
    /// Numbers beyond one of each kind, kept as they were.
    pub more_phones: Vec<quatrelle_engine::model::vcard::Typed>,
    /// Business, home and other (as [`ADDRESSES`]).
    pub addresses: [quatrelle_engine::model::vcard::PostalAddress; 3],
    /// The address shown in the form.
    pub address_shown: usize,
    /// `YYYY-MM-DD`.
    pub birthday: String,
    pub org: String,
    pub department: String,
    pub office: String,
    pub title: String,
    pub profession: String,
    pub manager: String,
    pub assistant: String,
    pub web: String,
    pub im: String,
    pub nickname: String,
    pub spouse: String,
    pub anniversary: String,
    pub note: String,
    pub error: Option<String>,
}

impl ContactForm {
    /// A new contact with a name and address filled in (Add to contacts on someone in a message).
    pub fn from_address(a: &quatrelle_engine::model::EmailAddress) -> ContactForm {
        let mut f = ContactForm { uid: None, name: a.name.trim().to_string(), ..Default::default() };
        f.emails[0] = a.email.trim().to_string();
        f
    }

    pub fn edit(c: &quatrelle_engine::model::vcard::Card) -> ContactForm {
        let mut f = ContactForm {
            uid: Some(c.uid.clone()),
            name: c.display(),
            org: c.org.clone(),
            department: c.department.clone(),
            office: c.office.clone(),
            title: c.title.clone(),
            profession: c.profession.clone(),
            manager: c.manager.clone(),
            assistant: c.assistant.clone(),
            web: c.web.clone(),
            im: c.im.clone(),
            nickname: c.nickname.clone(),
            spouse: c.spouse.clone(),
            anniversary: c.anniversary.clone(),
            note: c.note.clone(),
            birthday: c.birthday.clone(),
            ..Default::default()
        };
        for (i, e) in c.emails.iter().enumerate() {
            match f.emails.get_mut(i) {
                Some(slot) => *slot = e.value.clone(),
                None => f.more_emails.push(e.value.clone()),
            }
        }
        for ph in &c.phones {
            let kind = match ph.kind.as_str() {
                "mobile" => "cell",
                k if PHONES.iter().any(|(_, x)| *x == k) => k,
                _ => "",
            };
            let slot = PHONES.iter().position(|(_, k)| *k == kind).and_then(|i| f.phones.get_mut(i)).filter(|s| s.is_empty());
            match slot {
                Some(slot) => *slot = ph.value.clone(),
                None => f.more_phones.push(ph.clone()),
            }
        }
        for a in &c.addresses {
            let i = ADDRESSES.iter().position(|(_, k)| *k == a.kind).unwrap_or(2);
            if let Some(slot) = f.addresses.get_mut(i).filter(|s| s.is_empty()) {
                *slot = a.clone();
            }
        }
        f.address_shown = f.addresses.iter().position(|a| !a.is_empty()).unwrap_or(0);
        f
    }

    pub(crate) fn params(&self) -> Value {
        let emails: Vec<&str> =
            self.emails.iter().map(|e| e.trim()).chain(self.more_emails.iter().map(String::as_str)).filter(|e| !e.is_empty()).collect();
        let mut phones: Vec<Value> = PHONES
            .iter()
            .zip(&self.phones)
            .filter(|(_, v)| !v.trim().is_empty())
            .map(|((_, kind), v)| json!({"value": v.trim(), "kind": kind}))
            .collect();
        phones.extend(self.more_phones.iter().map(|x| json!({"value": x.value, "kind": x.kind})));
        let addresses: Vec<Value> = ADDRESSES
            .iter()
            .zip(&self.addresses)
            .filter(|(_, a)| !a.is_empty())
            .map(|((_, kind), a)| {
                json!({"kind": kind, "street": a.street, "city": a.city, "region": a.region, "postcode": a.postcode, "country": a.country})
            })
            .collect();
        let mut p = json!({
            "name": self.name.trim(),
            "emails": emails,
            "phones": phones,
            "addresses": addresses,
            "birthday": self.birthday.trim(),
            "org": self.org.trim(),
            "department": self.department.trim(),
            "office": self.office.trim(),
            "title": self.title.trim(),
            "profession": self.profession.trim(),
            "manager": self.manager.trim(),
            "assistant": self.assistant.trim(),
            "web": self.web.trim(),
            "im": self.im.trim(),
            "nickname": self.nickname.trim(),
            "spouse": self.spouse.trim(),
            "anniversary": self.anniversary.trim(),
            "note": self.note,
        });
        if let Some(u) = &self.uid {
            p["uid"] = json!(u);
        }
        p
    }
}

pub fn contact(ui: &mut Ui, f: &mut ContactForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(560.0);
    // The person's initials once a name is typed, as the people list shows them.
    let initials: String = f.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
    let badge = if initials.is_empty() { Ok(Icon::Person) } else { Err(initials) };
    form_header(ui, t, badge, if f.uid.is_some() { "Edit contact" } else { "New contact" });
    let tall = ui.ctx().content_rect().height() * 0.62;
    egui::ScrollArea::vertical().max_height(tall.max(260.0)).auto_shrink([false, true]).show(ui, |ui| {
        // Room on the right for the scroll bar, so it doesn't run over the fields.
        ui.set_max_width((ui.available_width() - 14.0).max(120.0));
        form_row(ui, t, false, None, "Name", |ui| {
            field(ui, &mut f.name, tl!("First and last name"), f32::INFINITY);
        });
        form_row(ui, t, false, None, "Company", |ui| {
            field(ui, &mut f.org, "", f32::INFINITY);
        });
        form_row(ui, t, false, None, "Job title", |ui| {
            field(ui, &mut f.title, "", f32::INFINITY);
        });
        // Outlook's other work details, kept by Exchange, CardDAV and a .pst alike.
        form_row(ui, t, false, None, "Department", |ui| {
            field(ui, &mut f.department, "", f32::INFINITY);
        });
        form_row(ui, t, false, None, "Office", |ui| {
            field(ui, &mut f.office, "", f32::INFINITY);
        });
        form_row(ui, t, false, None, "Profession", |ui| {
            field(ui, &mut f.profession, "", f32::INFINITY);
        });
        form_row(ui, t, false, None, "Manager", |ui| {
            field(ui, &mut f.manager, "", f32::INFINITY);
        });
        form_row(ui, t, false, None, "Assistant", |ui| {
            field(ui, &mut f.assistant, "", f32::INFINITY);
        });
        form_row(ui, t, false, None, "Web page", |ui| {
            field(ui, &mut f.web, "", f32::INFINITY);
        });
        form_row(ui, t, false, None, "Instant messaging", |ui| {
            field(ui, &mut f.im, "", f32::INFINITY);
        });
        form_row(ui, t, false, None, "Nickname", |ui| {
            field(ui, &mut f.nickname, "", f32::INFINITY);
        });
        form_row(ui, t, false, None, "Spouse", |ui| {
            field(ui, &mut f.spouse, "", f32::INFINITY);
        });
        form_row(ui, t, false, None, "Anniversary", |ui| {
            field(ui, &mut f.anniversary, tl!("YYYY-MM-DD"), 140.0);
        });
        for (i, label) in ["Email", "Email 2", "Email 3"].into_iter().enumerate() {
            if let Some(v) = f.emails.get_mut(i) {
                form_row(ui, t, false, None, label, |ui| {
                    field(ui, v, "", f32::INFINITY);
                });
            }
        }
        for (i, (label, _)) in PHONES.iter().enumerate() {
            if let Some(v) = f.phones.get_mut(i) {
                let label = match *label {
                    "Business" => "Business phone",
                    "Home" => "Home phone",
                    "Mobile" => "Mobile phone",
                    "Fax" => "Business fax",
                    _ => "Other phone",
                };
                form_row(ui, t, false, None, label, |ui| {
                    field(ui, v, "", f32::INFINITY);
                });
            }
        }
        // One address at a time, as Outlook's form: choose which.
        form_row(ui, t, false, None, "Address", |ui| {
            let shown = f.address_shown.min(2);
            let name = |i: usize| match i {
                0 => tl!("Business"),
                1 => tl!("Home"),
                _ => tl!("Other"),
            };
            egui::ComboBox::from_id_salt("contact-address").selected_text(name(shown)).show_ui(ui, |ui| {
                for i in 0..3 {
                    let filled = f.addresses.get(i).is_some_and(|a| !a.is_empty());
                    let label = if filled { format!("{} ✓", name(i)) } else { name(i).to_string() };
                    if ui.selectable_label(shown == i, label).clicked() {
                        f.address_shown = i;
                    }
                }
            });
        });
        let shown = f.address_shown.min(2);
        if let Some(a) = f.addresses.get_mut(shown) {
            form_row(ui, t, false, None, "Street", |ui| {
                area(ui, &mut a.street, "", 2);
            });
            form_row(ui, t, false, None, "Postal code", |ui| {
                field(ui, &mut a.postcode, "", 110.0);
                ui.label(RichText::new(tl!("City")).color(t.dim));
                field(ui, &mut a.city, "", f32::INFINITY);
            });
            form_row(ui, t, false, None, "State/Province", |ui| {
                field(ui, &mut a.region, "", f32::INFINITY);
            });
            form_row(ui, t, false, None, "Country/Region", |ui| {
                field(ui, &mut a.country, "", f32::INFINITY);
            });
        }
        form_row(ui, t, false, None, "Birthday", |ui| {
            field(ui, &mut f.birthday, tl!("YYYY-MM-DD"), 140.0);
        });
        form_row(ui, t, false, None, "Notes", |ui| {
            area(ui, &mut f.note, "", 3);
        });
    });
    if let Some(e) = &f.error {
        ui.label(RichText::new(e).color(t.danger));
    }
    footer_rule(ui, t);
    let mut keep = true;
    let any_email = f.emails.iter().any(|e| !e.trim().is_empty());
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::accent_button(ui, crate::icons::Icon::Person, "Save", t, !f.name.trim().is_empty() || any_email).clicked() {
            f.error = None;
            acts.push(Action::Submit(if f.uid.is_some() { "contact.update" } else { "contact.create" }, f.params()));
        }
        if ui.button(tl!("Cancel")).clicked() {
            keep = false;
        }
    });
    keep
}
