// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The card of the person picked: their picture, name, buttons and details.

use super::*;

/// The main pane: the person picked, or everyone as cards or a phone list.
pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    match st.people_view {
        PeopleView::Cards => return cards(s, st, ui, acts),
        PeopleView::Phone => return phone_list(s, st, ui, acts),
        PeopleView::People => {}
    }
    let Some(p) = st.person.as_deref().and_then(|u| s.find_person(u)) else {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.35);
                let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::People, t.border);
                ui.add_space(8.0);
                ui.label(RichText::new(tl!("Pick someone to see their details")).color(t.faint));
            });
        });
        return;
    };
    let c = &p.card;
    let name = c.display();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 24 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                // Outlook shows the picture an address book carries; without one, and for a
                // colleague, the one the directory keeps (asked for once).
                let email = c.emails.first().map(|e| e.value.trim()).unwrap_or_default();
                if c.photo.is_none() && !s.person_photo_known(email) && s.photo_possible_for(email) {
                    acts.push(Action::Run("contact.photo", json!({ "email": email })));
                }
                let photo = picture(s, c);
                let texture = photo.and_then(|p| photo_texture(ui, &c.uid, p));
                match texture {
                    Some(tex) => {
                        ui.put(r, egui::Image::new(&tex).fit_to_exact_size(r.size()));
                    }
                    None => {
                        let initials: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
                        widgets::avatar(ui, r.center(), 34.0, &initials, theme::avatar_color(&name, t.dark));
                    }
                }
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&name).font(theme::bold(22.0)).color(t.text));
                    let role = [c.title.as_str(), c.org.as_str()].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(" · ");
                    if !role.is_empty() {
                        ui.label(RichText::new(role).color(t.dim));
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if let Some(e) = c.emails.first()
                            && widgets::pill_button(ui, Icon::NewMail, "Email", &t).clicked()
                        {
                            acts.extend(write_to(&name, &e.value));
                        }
                        // Someone in a shared address book (an organisation's) is only read.
                        if s.person_read_only(&c.uid) {
                            ui.label(RichText::new(tl!("In a shared address book: read-only")).color(t.faint).small());
                        } else {
                            if widgets::pill_button(ui, Icon::Drafts, "Edit", &t).clicked() {
                                acts.push(Action::Dialog(Dialog::edit_contact(c)));
                            }
                            if widgets::pill_button(ui, Icon::Delete, "Delete", &t).clicked() {
                                acts.push(Action::Dialog(Dialog::ConfirmDeleteContact { uid: c.uid.clone(), name: c.display() }));
                            }
                        }
                    });
                });
            });
            ui.add_space(18.0);
            let field = |ui: &mut Ui, t: &Tokens, icon: Icon, value: &str, kind: &str| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    icons::paint(ui.painter(), r, icon, t.accent);
                    ui.label(RichText::new(value).color(t.text));
                    if !kind.is_empty() {
                        ui.label(RichText::new(kind).color(t.faint).small());
                    }
                });
            };
            for e in &c.emails {
                field(ui, &t, Icon::Unread, &e.value, &e.kind);
            }
            // Each number with what it is (Outlook's Business, Home, Mobile…).
            for ph in &c.phones {
                field(ui, &t, Icon::Phone, &ph.value, phone_kind(&ph.kind));
            }
            for a in c.addresses.iter().filter(|a| !a.is_empty()) {
                let kind = match a.kind.as_str() {
                    "work" => tl!("Business"),
                    "home" => tl!("Home"),
                    _ => tl!("Other"),
                };
                let lines =
                    [a.street.clone(), format!("{} {}", a.postcode.trim(), a.city.trim()).trim().to_string(), a.region.clone(), a.country.clone()]
                        .into_iter()
                        .filter(|x| !x.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                field(ui, &t, Icon::Location, &lines, kind);
            }
            if !c.birthday.is_empty() {
                field(ui, &t, Icon::Calendar, &crate::i18n::date(&quatrelle_engine::model::dates::birthday(&c.birthday)), tl!("Birthday"));
            }
            // Outlook's contact card: the work details an address book carries beside the name.
            if !c.department.is_empty() {
                field(ui, &t, Icon::Gear, &c.department, tl!("Department"));
            }
            if !c.office.is_empty() {
                field(ui, &t, Icon::Location, &c.office, tl!("Office"));
            }
            if !c.profession.is_empty() {
                field(ui, &t, Icon::Person, &c.profession, tl!("Profession"));
            }
            if !c.manager.is_empty() {
                field(ui, &t, Icon::Person, &c.manager, tl!("Manager"));
            }
            if !c.assistant.is_empty() {
                field(ui, &t, Icon::Person, &c.assistant, tl!("Assistant"));
            }
            if !c.web.is_empty() {
                field(ui, &t, Icon::Info, &c.web, tl!("Web page"));
            }
            if !c.im.is_empty() {
                field(ui, &t, Icon::Phone, &c.im, tl!("Instant messaging"));
            }
            if !c.nickname.is_empty() {
                field(ui, &t, Icon::Tag, &c.nickname, tl!("Nickname"));
            }
            if !c.spouse.is_empty() {
                field(ui, &t, Icon::Person, &c.spouse, tl!("Spouse"));
            }
            if !c.anniversary.is_empty() {
                field(ui, &t, Icon::Calendar, &c.anniversary, tl!("Anniversary"));
            }
            // The files kept with them (a `.pst` import's), as a message's attachments.
            let files = s.card_files(&c.uid);
            if !files.is_empty() {
                ui.add_space(10.0);
                let of = crate::pim_dialogs::FilesOf::Contact { uid: c.uid.clone() };
                crate::pim_dialogs::files::strip(ui, &t, &of, &files, acts);
            }
            if !c.note.is_empty() {
                ui.add_space(10.0);
                ui.label(RichText::new(tl!("Notes")).font(theme::bold(13.0)).color(t.dim));
                ui.label(RichText::new(&c.note).color(t.text));
            }
            ui.add_space(14.0);
            if let Some(a) = s.account(&p.account) {
                ui.label(RichText::new(tf!("In the address book of {account}", account = a.config.email)).color(t.faint).small());
            }
        });
    });
}

/// What a card's Email button does: a new message to the person, in Mail. The name is written
/// as a recipient's is (one with a comma quoted, as an Outlook "Last, First" name has): pasted
/// in bare, the comma parted it into two recipients, the first not an address.
pub(crate) fn write_to(name: &str, email: &str) -> [Action; 3] {
    let to = quatrelle_engine::model::EmailAddress::new(name, email).to_string();
    [Action::Run("mail.new", json!({})), Action::Run("compose.update", json!({ "to": to })), Action::Run("view.module", json!({"name": "mail"}))]
}

/// What a number is, as Outlook names it.
fn phone_kind(kind: &str) -> &'static str {
    match kind {
        "work" | "voice" => tl!("Business"),
        "home" => tl!("Home"),
        "cell" | "mobile" => tl!("Mobile"),
        "fax" => tl!("Fax"),
        "pager" => tl!("Pager"),
        _ => tl!("Other"),
    }
}
