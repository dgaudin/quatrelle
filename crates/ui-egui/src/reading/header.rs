// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A message's header in the reading pane (or its window), which stays put while the message
//! scrolls under it, as Outlook's does: the subject and categories, who it's from and to and
//! when, what's to know about it (follow-up, importance, a doubtful sender, its signature or
//! encryption), and under it what can be done with it (Reply… or, in the Outbox, Send now…).

use super::*;
use quatrelle_engine::model::Summary;

/// The narrowest the sender's column gets with the reply group beside it (narrower, the group
/// goes under the header).
const MIN_SENDER: f32 = 220.0;

/// The message's header; whether Reply | Reply all | Forward went in it, at the sender's right
/// ([`actions`] leaves them out then).
#[allow(clippy::too_many_arguments)]
pub(super) fn header(
    s: &mut Session,
    t: Tokens,
    ui: &mut Ui,
    r: &quatrelle_engine::Reading,
    p: &mime::Parsed,
    summary: &Option<Summary>,
    formatted: bool,
    in_window: bool,
    acts: &mut Vec<Action>,
) -> bool {
    let mut placed = false;
    // Subject.
    ui.horizontal_wrapped(|ui| {
        let subject = if p.subject.is_empty() { tl!("(no subject)") } else { p.subject.as_str() };
        crate::bidi::label(ui, subject, theme::bold(21.0), t.text);
        if summary.as_ref().is_some_and(|m| m.flags.flagged) {
            let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), ir, Icon::Flag, t.flag);
        }
    });
    if let Some(m) = &summary {
        let cats = s.categories_of(m);
        if !cats.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for c in cats {
                    let color = theme::category_color(c.color, t.dark);
                    egui::Frame::new().fill(color.gamma_multiply(0.18)).corner_radius(10).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
                        ui.label(RichText::new(&c.name).color(color).small());
                    });
                }
            });
        }
    }
    ui.add_space(10.0);

    // Sender card.
    ui.horizontal_top(|ui| {
        let (ar, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
        // Outlook shows the sender's picture there: the one the directory keeps for
        // their address, asked for once (a server that keeps none is remembered).
        if !s.person_photo_known(&p.from.email) && s.photo_possible_for(&p.from.email) {
            acts.push(Action::Run("contact.photo", json!({ "email": p.from.email.trim() })));
        }
        match crate::people::address_picture(ui, s, &p.from.email) {
            Some(tex) => {
                let side = ar.width().min(ar.height());
                let r = egui::Rect::from_center_size(ar.center(), vec2(side, side));
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(tex.id(), r, uv, egui::Color32::WHITE);
            }
            None => widgets::avatar(ui, ar.center(), 21.0, &p.from.initials(), theme::avatar_color(&p.from.email, t.dark)),
        }
        ui.add_space(4.0);
        // The date to the right, with room kept for it (the address ran under it in a
        // narrow pane); under the sender when the pane is too narrow for both.
        let when = summary.as_ref().map_or(p.date, |m| if p.date > 0 { p.date } else { m.date });
        let date = egui::WidgetText::from(RichText::new(crate::i18n::date(&dates::full_time(when, &s.clock))).color(t.dim).small()).into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Small,
        );
        // In a pane wide enough, the reply group goes above the date, as Outlook's header has it.
        let full = offers_actions(s, r, in_window).then(|| action_width(ui, s, false));
        let with_actions = full.filter(|w| ui.available_width() - w - 16.0 >= MIN_SENDER);
        placed = with_actions.is_some();
        let right = with_actions.map_or(date.size().x, |w| w.max(date.size().x));
        let rest = ui.available_width() - right - 16.0;
        let beside = rest >= 200.0;
        let room = if beside { rest } else { ui.available_width() };
        ui.allocate_ui_with_layout(vec2(room, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_max_width(room);
            ui.horizontal_wrapped(|ui| {
                let shown = crate::bidi::shown(p.from.display()).0;
                let name = person_link(ui, &t, RichText::new(shown).font(theme::bold(15.0)).color(t.text));
                person_menus(s, &t, &name, &p.from, acts);
                if !p.from.name.is_empty() {
                    ui.label(RichText::new(format!("<{}>", p.from.email)).color(t.dim));
                }
                // From outside the organisation (with the tag on; never the user's own mail).
                if s.is_external(&p.from.email) {
                    let _ = external_tag(ui, &t);
                }
                let known = known_contact(ui.ctx(), s, &p.from.email).is_some();
                if !known && !p.from.email.is_empty() && widgets::tiny_icon_button(ui, Icon::Person, "Add to contacts", &t).clicked() {
                    acts.push(Action::Dialog(crate::Dialog::contact_from(&p.from)));
                }
            });
            for (label, list) in [("To:", &p.to), ("Cc:", &p.cc)] {
                if list.is_empty() {
                    continue;
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label(RichText::new(format!("{} ", tl!(label))).color(t.dim).small());
                    for (i, a) in list.iter().take(MAX_PEOPLE_SHOWN).enumerate() {
                        if i > 0 {
                            ui.label(RichText::new("; ").color(t.dim).small());
                        }
                        let r = person_link(ui, &t, RichText::new(crate::bidi::shown(a.display()).0).color(t.dim).small());
                        person_menus(s, &t, &r, a, acts);
                    }
                    if list.len() > MAX_PEOPLE_SHOWN {
                        let more = list.len() - MAX_PEOPLE_SHOWN;
                        ui.label(RichText::new(tf!(" and {n} more", n = more)).color(t.dim).small()).on_hover_text(addr::format_list(list));
                    }
                });
            }
            if !beside {
                ui.label(date.clone());
            }
        });
        if beside {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.allocate_ui_with_layout(vec2(right, 0.0), egui::Layout::top_down(egui::Align::Max), |ui| {
                    if let Some(w) = with_actions {
                        ui.allocate_ui_with_layout(vec2(w, 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            action_row(s, &t, ui, r, p, formatted, false, acts);
                        });
                        ui.add_space(6.0);
                    }
                    ui.label(date);
                });
            });
        }
    });
    ui.add_space(8.0);
    // What Outlook says above a message: that it was replied to or forwarded (and when), its
    // follow-up date (or the day it was done), its importance.
    let due = summary.as_ref().and_then(|m| s.follow_up_due(m));
    let flagged = summary.as_ref().is_some_and(|m| m.flags.flagged);
    let complete = summary.as_ref().is_some_and(|m| m.flags.follow_up_complete());
    let importance = summary.as_ref().map_or(0, |m| m.importance);
    let acted = summary.as_ref().and_then(|m| acted_line(s, &m.flags));
    let policy = s.message_policy(&r.key);
    if flagged || complete || importance != 0 || acted.is_some() || policy.is_some() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if let Some((icon, text)) = acted {
                let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icons::paint(ui.painter(), ir, icon, t.dim);
                ui.label(RichText::new(text).color(t.dim).small());
                ui.add_space(10.0);
            }
            if complete {
                let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icons::paint(ui.painter(), ir, Icon::Done, t.dim);
                let text = match summary.as_ref().and_then(|m| s.follow_up_done(m)) {
                    Some(d) => tf!("Completed on {date}.", date = crate::i18n::date(&dates::day_label(d))),
                    None => tl!("Completed.").to_string(),
                };
                ui.label(RichText::new(text).color(t.dim).small());
                ui.add_space(10.0);
            }
            if flagged {
                let today = dates::today(&s.clock);
                let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icons::paint(ui.painter(), ir, Icon::Flag, t.flag);
                let text = match due {
                    Some(d) if d < today => tf!("Follow up. Overdue: it was due {date}.", date = crate::i18n::date(&dates::day_label(d))),
                    Some(d) if d == today => tl!("Follow up. Due today.").to_string(),
                    Some(d) => tf!("Follow up. Due {date}.", date = crate::i18n::date(&dates::day_label(d))),
                    None => tl!("Follow up.").to_string(),
                };
                let overdue = due.is_some_and(|d| d < today);
                ui.label(RichText::new(text).color(if overdue { t.danger } else { t.dim }).small());
                ui.add_space(10.0);
            }
            if let Some(p) = &policy {
                ui.label(RichText::new(crate::panes::policy::policy_line(p)).color(t.dim).small());
                ui.add_space(10.0);
            }
            if importance > 0 {
                ui.label(RichText::new(tl!("This message was sent with high importance.")).color(t.danger).small());
            } else if importance < 0 {
                ui.label(RichText::new(tl!("This message was sent with low importance.")).color(t.dim).small());
            }
        });
        ui.add_space(6.0);
    }
    if let Some(w) = &r.warning {
        sender_warning(&t, ui, w);
    }
    // The model's finding, below the sender check's banner (which has said the advice already).
    if let Some((score, reasons)) = s.scam_warning(&r.key, &p.message_id).map(|(sc, rs)| (sc, rs.to_vec()))
        && scam_banner(&t, ui, score, &reasons, r.warning.is_none())
    {
        acts.push(Action::Run("scam.notScam", json!({"key": r.key.to_id()})));
    }
    if let Some(sec) = &r.security {
        security::security_bar(s, &t, ui, r, sec);
    }
    placed
}

/// The line saying a message was replied to or forwarded, with its arrow: when, where the server
/// says (Exchange keeps the time of the last); both kept (IMAP), the reply, as the list shows it.
/// Worded as the follow-up's "Completed on …", in Quatrelle's own words (clean room).
fn acted_line(s: &Session, f: &quatrelle_engine::model::Flags) -> Option<(Icon, String)> {
    let when = (f.acted > 0).then(|| crate::i18n::date(&dates::full_time(f.acted, &s.clock)));
    if f.answered {
        Some((Icon::Reply, when.map_or_else(|| tl!("Replied").to_string(), |date| tf!("Replied on {date}.", date = date))))
    } else if f.forwarded {
        Some((Icon::Forward, when.map_or_else(|| tl!("Forwarded").to_string(), |date| tf!("Forwarded on {date}.", date = date))))
    } else {
        None
    }
}

/// Whether the message has Reply | Reply all | Forward beside it: not in a window (its ribbon
/// has them) nor in the Outbox (Send now… instead).
fn offers_actions(s: &Session, r: &quatrelle_engine::Reading, in_window: bool) -> bool {
    !in_window && s.outbox_entry(&r.key).is_none()
}

/// The reply group's labels.
const REPLY_GROUP: [(Icon, &str); 3] = [(Icon::Reply, "Reply"), (Icon::ReplyAll, "Reply all"), (Icon::Forward, "Forward")];

/// How wide [`action_row`] is, its icons alone when `compact`.
fn action_width(ui: &Ui, s: &Session, compact: bool) -> f32 {
    widgets::button_group_width(ui, &REPLY_GROUP.map(|(_, l)| l), compact) + assistant::width(ui, s, compact) + 6.0 + 34.0
}

/// Reply | Reply all | Forward as one group, as Outlook's, then Assistant (with a language model
/// set) and ⋯ with the rest; their icons alone when `compact`.
#[allow(clippy::too_many_arguments)]
fn action_row(
    s: &Session,
    t: &Tokens,
    ui: &mut Ui,
    r: &quatrelle_engine::Reading,
    p: &mime::Parsed,
    formatted: bool,
    compact: bool,
    acts: &mut Vec<Action>,
) {
    ui.spacing_mut().item_spacing.x = 6.0;
    match widgets::button_group(ui, &REPLY_GROUP, t, compact) {
        Some(0) => acts.push(Action::Run("mail.reply", json!({}))),
        Some(1) => acts.push(Action::Run("mail.replyAll", json!({}))),
        Some(_) => acts.push(Action::Run("mail.forward", json!({}))),
        None => {}
    }
    assistant::button(s, t, ui, compact, acts);
    // More, as Outlook's ⋯ beside Forward: printing and saving too.
    let more = widgets::more_pill(ui, "More actions", t);
    egui::Popup::menu(&more).show(|ui| {
        let item = |ui: &mut Ui, icon: Icon, label: &str| ui.button((widgets::menu_icon(Some(icon)), tl!(label))).clicked();
        if item(ui, Icon::Print, "Print") {
            acts.push(Action::Print);
            ui.close();
        }
        if item(ui, Icon::File, "Save as PDF…") {
            acts.push(Action::ExportPdf);
            ui.close();
        }
        if item(ui, Icon::Inbox, "Save as email (.eml)…") {
            acts.push(Action::SaveMessage);
            ui.close();
        }
        ui.separator();
        if item(ui, Icon::Calendar, "Reply with meeting") {
            // Its text alone: someone it went to can have no address yet, and would be left out.
            acts.push(if r.text_first { Action::Refuse(still_coming(r)) } else { Action::Dialog(crate::Dialog::meeting_from(s, p)) });
            ui.close();
        }
        if item(ui, Icon::Forward, "Forward as attachment") {
            acts.push(Action::Run("mail.forwardAsAttachment", json!({})));
            ui.close();
        }
        if item(ui, Icon::Search, "Find in message") {
            acts.push(Action::FindInMessage);
            ui.close();
        }
        if item(ui, Icon::Info, "Message headers") {
            acts.push(Action::Run("message.headers", json!({})));
            ui.close();
        }
        // One message as plain text, or formatted again; all mail's setting stays.
        super::plain_text_item(ui, &r.key, p, formatted, acts);
        // View › View message source, where Outlook has it (issue #27); the eye is Bcc's icon.
        // "View" as a menu says it (in French « Afficher », not the ribbon tab's « Affichage »).
        let view = crate::i18n::tc("menu", "View");
        ui.menu_button((widgets::menu_icon(Some(Icon::Bcc)), view), |ui| {
            if ui.button((widgets::menu_icon(Some(Icon::File)), tl!("View message source"))).clicked() {
                acts.push(Action::Run("message.source", json!({"key": r.key.to_id()})));
                ui.close();
            }
        });
    });
}

/// What can be done with the message (Reply… or, in the Outbox, Send now…): under the header
/// and outside its scrolling, so a long header never hides it (a reading pane at the bottom, too
/// short for the whole header, cut the buttons in half). Reply | Reply all | Forward come here
/// when the header had no room for them (`placed`: it had), their icons alone when even this
/// line is too narrow for their words.
#[allow(clippy::too_many_arguments)]
pub(super) fn actions(
    s: &mut Session,
    t: Tokens,
    ui: &mut Ui,
    r: &quatrelle_engine::Reading,
    p: &mime::Parsed,
    formatted: bool,
    in_window: bool,
    placed: bool,
    acts: &mut Vec<Action>,
) {
    if let Some((_, item)) = s.outbox_entry(&r.key) {
        egui::Frame::new().fill(t.warn.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            let why = match item.error.as_deref() {
                Some(e) if e != "offline" => tf!("Waiting in the Outbox. Last try: {error}", error = widgets::shorten(e, 140)),
                _ if item.not_before > s.clock.now => {
                    tf!("Held in the Outbox: it will be sent {when}.", when = crate::i18n::date(&dates::full_time(item.not_before, &s.clock)))
                }
                _ => tl!("Waiting in the Outbox: it will be sent when you're back online.").to_string(),
            };
            ui.label(RichText::new(why).color(t.text));
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if widgets::pill_button(ui, Icon::Send, "Send now", &t).clicked() {
                acts.push(Action::Run("outbox.send", json!({"key": r.key.to_id()})));
            }
            if widgets::pill_button(ui, Icon::Drafts, "Edit", &t).clicked() {
                acts.push(Action::Run("outbox.edit", json!({"key": r.key.to_id()})));
            }
            if widgets::pill_button(ui, Icon::Delete, "Delete", &t).clicked() {
                delete(s, &r.key, in_window, acts);
            }
        });
    } else if s.folder(&r.key.account, &r.key.folder).is_some_and(|f| f.role == quatrelle_engine::model::FolderRole::Drafts)
        || s.summary(&r.key).is_some_and(|m| m.flags.draft)
    {
        // A draft: carried on with, as it was saved.
        egui::Frame::new().fill(t.accent_soft).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(tl!("Not sent yet: this is a draft.")).color(t.text));
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if widgets::pill_button(ui, Icon::Drafts, "Edit", &t).clicked() {
                acts.push(Action::Run("draft.edit", json!({"key": r.key.to_id()})));
            }
            if widgets::pill_button(ui, Icon::Delete, "Delete", &t).clicked() {
                delete(s, &r.key, in_window, acts);
            }
        });
    } else if !in_window && !placed {
        // On one line: their icons alone when it has no room for their words.
        let compact = ui.available_width() < action_width(ui, s, false);
        ui.horizontal(|ui| action_row(s, &t, ui, r, p, formatted, compact, acts));
    }
}

/// Delete pressed on the message read: asked first where it's for good, as the Delete key does;
/// in a window of its own, that window's message (not the list's selection), as its toolbar does.
fn delete(s: &Session, key: &MessageKey, in_window: bool, acts: &mut Vec<Action>) {
    if in_window {
        acts.extend(crate::app::delete_from_window(s, key, false));
    } else {
        acts.push(Action::DeleteMail { for_good: false });
    }
}
