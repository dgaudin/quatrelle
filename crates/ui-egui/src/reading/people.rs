// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The reading pane: the people in a message's header, their menus and contact cards.

use super::*;

/// Most people named in To or Cc above a message (the rest say how many more).
pub(super) const MAX_PEOPLE_SHOWN: usize = 50;

/// The contact someone is saved as, if they are (by address): looked up once, then kept until
/// the address books change (drawn on every frame for each name above a message).
pub(super) fn known_contact(ctx: &egui::Context, s: &Session, email: &str) -> Option<quatrelle_engine::model::vcard::Card> {
    if email.trim().is_empty() {
        return None;
    }
    let id = egui::Id::unique(("known-contact", email.trim().to_ascii_lowercase()));
    if let Some((revision, card)) = ctx.data(|d| d.get_temp::<(u64, Option<quatrelle_engine::model::vcard::Card>)>(id))
        && revision == s.pim_revision
    {
        return card;
    }
    let card = s.person_by_email(email).map(|p| p.card);
    ctx.data_mut(|d| d.insert_temp(id, (s.pim_revision, card.clone())));
    card
}

/// How someone a message went to is named above it, as Outlook names them: the name its header
/// gives, else the name of the contact saved with that address, of what the directory answered
/// for it, or of one's own account; the address when none is known.
pub(super) fn shown_name(ctx: &egui::Context, s: &Session, a: &EmailAddress) -> String {
    let email = a.email.trim();
    if !a.name.trim().is_empty() || email.is_empty() {
        return a.display().to_string();
    }
    let real = |n: &str| Some(n.trim().to_string()).filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case(email));
    known_contact(ctx, s, email)
        .and_then(|c| real(&c.display()))
        .or_else(|| s.directory_known(email).and_then(|d| real(&d.name)))
        .or_else(|| s.accounts.iter().find(|x| x.config.email.trim().eq_ignore_ascii_case(email)).and_then(|x| real(&x.config.name)))
        .unwrap_or_else(|| email.to_string())
}

/// Someone's name above a message, as a link: lit and underlined under the pointer, under its
/// words on each line it is drawn on. A name cut by the end of a line starts after the names
/// before it, and its rectangle spans the lines' whole width: underlined along that, it was
/// underlined under the whole next line.
pub(super) fn person_link(ui: &mut Ui, t: &Tokens, text: RichText) -> egui::Response {
    let (pos, galley, r) = egui::Label::new(text).sense(Sense::click()).selectable(false).layout_in_ui(ui);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), galley.text()));
    if ui.is_rect_visible(r.rect) {
        let colour = ui.style().interact(&r).text_color();
        if r.hovered() {
            for row in galley.rows.iter().filter(|row| !row.glyphs.is_empty()) {
                let line = row.rect_without_leading_space().translate(pos.to_vec2());
                ui.painter().line_segment([line.left_bottom(), line.right_bottom()], egui::Stroke::new(1.0, t.dim));
            }
        }
        ui.painter().galley(pos, galley, colour);
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// What a name above a message opens: a contact card on a click or a right-click, as Outlook's,
/// and the things that can be done with the person on a right-click too.
pub(super) fn person_menus(s: &Session, t: &Tokens, r: &egui::Response, a: &EmailAddress, acts: &mut Vec<Action>) {
    if a.email.trim().is_empty() {
        return;
    }
    let known = known_contact(&r.ctx, s, &a.email);
    let actions_key = actions_key(&a.email);
    if r.secondary_clicked() {
        r.ctx.data_mut(|d| d.insert_temp(actions_key, true));
    } else if r.clicked() {
        r.ctx.data_mut(|d| d.insert_temp(actions_key, false));
    }
    let with_actions = r.ctx.data(|d| d.get_temp::<bool>(actions_key).unwrap_or(false));
    egui::Popup::from_response(r)
        .id(r.id.with("card"))
        .open_memory((r.clicked() || r.secondary_clicked()).then_some(egui::SetOpenCommand::Bool(true)))
        .kind(egui::PopupKind::Popup)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| contact_card(s, ui, t, a, known.as_ref(), with_actions, actions_key, acts));
}

/// What can be done with a person, from the card's menu and from a right-click: write to them,
/// change or make their contact, open them in People or as a business card, and copy the address.
fn person_actions(ui: &mut Ui, a: &EmailAddress, known: Option<&quatrelle_engine::model::vcard::Card>, acts: &mut Vec<Action>) {
    if ui.button((widgets::menu_icon(Some(Icon::NewMail)), tl!("Send email"))).clicked() {
        acts.push(Action::Run("mail.new", json!({"to": a.to_string()})));
        ui.close();
    }
    match known {
        Some(card) => {
            if ui.button((widgets::menu_icon(Some(Icon::Person)), tl!("Edit contact…"))).clicked() {
                acts.push(Action::Dialog(crate::Dialog::edit_contact(card)));
                ui.close();
            }
        }
        None => {
            if ui.button((widgets::menu_icon(Some(Icon::Person)), tl!("Add to contacts…"))).clicked() {
                acts.push(Action::Dialog(crate::Dialog::contact_from(a)));
                ui.close();
            }
        }
    }
    if let Some(card) = known {
        ui.separator();
        if ui.button((widgets::menu_icon(Some(Icon::People)), tl!("Open in People"))).clicked() {
            acts.push(Action::OpenPerson { uid: card.uid.clone(), cards: false });
            ui.close();
        }
        if ui.button((widgets::menu_icon(Some(Icon::Layout)), tl!("Open business card"))).clicked() {
            acts.push(Action::OpenPerson { uid: card.uid.clone(), cards: true });
            ui.close();
        }
    }
    ui.separator();
    if ui.button((widgets::menu_icon(Some(Icon::Copy)), tl!("Copy address"))).clicked() {
        ui.ctx().copy_text(a.email.clone());
        ui.close();
    }
}

/// The key the card's unfolded state is kept under, by address (an address book's card is the
/// same person whichever message names them).
fn folded_key(email: &str) -> egui::Id {
    egui::Id::unique(("contact-card-folded", email.trim().to_ascii_lowercase()))
}

/// The key saying whether the card was opened by a right-click: its actions show under it.
fn actions_key(email: &str) -> egui::Id {
    egui::Id::unique(("contact-card-actions", email.trim().to_ascii_lowercase()))
}

/// A small card for someone (Outlook's contact card): who they are, their address (a click
/// writes to them), what can be done, and, unfolded, their numbers, organisation and address
/// books.
pub(super) fn contact_card(
    s: &Session,
    ui: &mut Ui,
    t: &Tokens,
    a: &EmailAddress,
    known: Option<&quatrelle_engine::model::vcard::Card>,
    with_actions: bool,
    actions: egui::Id,
    acts: &mut Vec<Action>,
) {
    let key = folded_key(&a.email);
    let unfolded = ui.data(|d| d.get_temp::<bool>(key).unwrap_or(false));
    ui.set_min_width(300.0);
    ui.set_max_width(360.0);
    ui.horizontal(|ui| {
        let (ar, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        let photo = known.and_then(|c| c.photo.clone()).or_else(|| s.person_photo(a.email.trim()).cloned());
        match photo.and_then(|p| crate::people::card_picture(ui, &a.email.to_ascii_lowercase(), &p)) {
            Some(tex) => {
                let side = ar.width().min(ar.height());
                let r = egui::Rect::from_center_size(ar.center(), vec2(side, side));
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(tex.id(), r, uv, egui::Color32::WHITE);
            }
            None => widgets::avatar(ui, ar.center(), 19.0, &a.initials(), theme::avatar_color(&a.email, t.dark)),
        }
        ui.add_space(4.0);
        ui.vertical(|ui| {
            let name = known.map(|c| c.display()).filter(|n| !n.is_empty()).unwrap_or_else(|| a.display().to_string());
            ui.label(RichText::new(&name).font(theme::bold(15.0)).color(t.text));
            // Their job and company, when saved (not the company again when it's the name).
            if let Some(c) = known {
                let what = [c.title.as_str(), c.org.as_str()]
                    .into_iter()
                    .filter(|x| !x.is_empty() && !x.eq_ignore_ascii_case(&name))
                    .collect::<Vec<_>>()
                    .join(" · ");
                if !what.is_empty() {
                    ui.label(RichText::new(what).color(t.dim).small());
                }
            }
            let mail = ui.add(egui::Label::new(RichText::new(&a.email).color(t.accent).underline()).sense(Sense::click()));
            if mail.on_hover_text(tl!("Send email")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                acts.push(Action::Run("mail.new", json!({"to": a.to_string()})));
                ui.close();
            }
        });
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if widgets::pill_button(ui, Icon::NewMail, "Send email", t).clicked() {
            acts.push(Action::Run("mail.new", json!({"to": a.to_string()})));
            ui.close();
        }
        let (label, dialog) = match known {
            Some(card) => ("Edit contact…", crate::Dialog::edit_contact(card)),
            None => ("Add to contacts…", crate::Dialog::contact_from(a)),
        };
        if widgets::pill_button(ui, Icon::Person, label, t).clicked() {
            acts.push(Action::Dialog(dialog));
            ui.close();
        }
        if ui.button(tl!("Copy address")).clicked() {
            ui.ctx().copy_text(a.email.clone());
            ui.close();
        }
        let more = widgets::more_pill(ui, "More actions", t);
        if more.clicked() {
            ui.data_mut(|d| d.insert_temp(actions, !with_actions));
        }
    });
    if with_actions {
        ui.add_space(6.0);
        ui.separator();
        person_actions(ui, a, known, acts);
    }
    if unfolded {
        ui.add_space(6.0);
        ui.separator();
        card_details(s, ui, t, a, known);
    }
    ui.add_space(4.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let (label, icon) = if unfolded { ("Show less", Icon::ChevronUp) } else { ("Show more", Icon::ChevronDown) };
        let r = ui.add(egui::Button::new((widgets::menu_icon(Some(icon)), tl!(label))).frame(false));
        if r.on_hover_text(tl!(label)).clicked() {
            ui.data_mut(|d| d.insert_temp(key, !unfolded));
        }
    });
}

/// The card's unfolded part: the ways to reach them, their work details, the address books that
/// keep them, and their note.
fn card_details(s: &Session, ui: &mut Ui, t: &Tokens, a: &EmailAddress, known: Option<&quatrelle_engine::model::vcard::Card>) {
    detail_heading(ui, t, "Contact");
    let mut any = false;
    if let Some(c) = known {
        for e in &c.emails {
            detail_row(ui, t, Icon::NewMail, &e.value);
            any = true;
        }
        for ph in &c.phones {
            detail_row(ui, t, Icon::Phone, &ph.value);
            any = true;
        }
        for ad in c.addresses.iter().filter(|x| !x.is_empty()) {
            detail_row(ui, t, Icon::Location, &ad.one_line());
            any = true;
        }
    }
    if !any {
        detail_row(ui, t, Icon::NewMail, a.email.trim());
    }
    if let Some(c) = known {
        let work = [
            (tl!("Job title"), c.title.as_str()),
            (tl!("Company"), c.org.as_str()),
            (tl!("Department"), c.department.as_str()),
            (tl!("Office"), c.office.as_str()),
            (tl!("Manager"), c.manager.as_str()),
            (tl!("Assistant"), c.assistant.as_str()),
        ];
        if work.iter().any(|(_, v)| !v.is_empty()) {
            detail_heading(ui, t, "Organisation");
            for (what, value) in work {
                if !value.is_empty() {
                    detail_row(ui, t, Icon::Person, &format!("{}: {}", what, value));
                }
            }
        }
    }
    let books = address_books(s, &a.email);
    if !books.is_empty() {
        detail_heading(ui, t, "Address books");
        for book in books.iter().take(6) {
            detail_row(ui, t, Icon::AddressBook, book);
        }
        if books.len() > 6 {
            detail_row(ui, t, Icon::AddressBook, &tf!("and {n} more", n = books.len() - 6));
        }
    }
    if let Some(c) = known
        && !c.note.trim().is_empty()
    {
        detail_heading(ui, t, "Notes");
        ui.label(RichText::new(c.note.trim()).color(t.text));
    }
}

fn detail_heading(ui: &mut Ui, t: &Tokens, label: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(tl!(label)).font(theme::bold(12.5)).color(t.dim));
}

fn detail_row(ui: &mut Ui, t: &Tokens, icon: Icon, value: &str) {
    if value.trim().is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        icons::paint(ui.painter(), r, icon, t.dim);
        ui.add(egui::Label::new(RichText::new(value.trim()).color(t.text)).wrap());
    });
}

/// The address books that keep this address, by their name (Outlook's membership of a person:
/// the books they are in).
fn address_books(s: &Session, email: &str) -> Vec<String> {
    let email = email.trim();
    if email.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for pim in s.pim.values() {
        for card in &pim.cards {
            if !card.card.emails.iter().any(|e| e.value.trim().eq_ignore_ascii_case(email)) {
                continue;
            }
            let name = pim
                .books
                .iter()
                .find(|b| b.id == card.book)
                .map(|b| {
                    if b.id == quatrelle_engine::model::pim::LOCAL || b.name.is_empty() || b.name == "Contacts" {
                        tl!("Contacts").to_string()
                    } else {
                        b.name.clone()
                    }
                })
                .unwrap_or_else(|| tl!("Contacts").to_string());
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}
