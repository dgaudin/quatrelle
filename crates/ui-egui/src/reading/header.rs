//! A message's header in the reading pane (or its window), which stays put while the message
//! scrolls under it, as Outlook's does: the subject and categories, who it's from and to and
//! when, what's to know about it (follow-up, importance, a doubtful sender, its signature or
//! encryption), and what can be done with it (Reply… or, in the Outbox, Send now…).

use super::*;
use quatrelle_engine::model::Summary;

#[allow(clippy::too_many_arguments)]
pub(super) fn header(
    s: &mut Session,
    t: Tokens,
    ui: &mut Ui,
    r: &quatrelle_engine::Reading,
    p: &mime::Parsed,
    summary: &Option<Summary>,
    in_window: bool,
    acts: &mut Vec<Action>,
) {
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
        let rest = ui.available_width() - date.size().x - 16.0;
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
                ui.label(date);
            });
        }
    });
    ui.add_space(8.0);
    // What Outlook says above a message: its follow-up date, its importance.
    let due = summary.as_ref().and_then(|m| s.follow_up_due(m));
    let flagged = summary.as_ref().is_some_and(|m| m.flags.flagged);
    let importance = summary.as_ref().map_or(0, |m| m.importance);
    if flagged || importance != 0 {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
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
    if let Some(sec) = &r.security {
        security::security_bar(s, &t, ui, sec);
    }
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
                acts.push(Action::Run("mail.delete", json!({})));
            }
        });
    } else if !in_window {
        // Wrapped: a narrow pane cut off Save and More.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            if widgets::pill_button(ui, Icon::Reply, "Reply", &t).clicked() {
                acts.push(Action::Run("mail.reply", json!({})));
            }
            if widgets::pill_button(ui, Icon::ReplyAll, "Reply all", &t).clicked() {
                acts.push(Action::Run("mail.replyAll", json!({})));
            }
            if widgets::pill_button(ui, Icon::Forward, "Forward", &t).clicked() {
                acts.push(Action::Run("mail.forward", json!({})));
            }
            assistant::button(s, &t, ui, acts);
            ui.add_space(6.0);
            if widgets::pill_button(ui, Icon::Print, "Print", &t).on_hover_text(tl!("Open as a PDF to print")).clicked() {
                acts.push(Action::Print);
            }
            let more = widgets::pill_button(ui, Icon::File, "Save", &t);
            egui::Popup::menu(&more).show(|ui| {
                if ui.button(tl!("Save as PDF…")).clicked() {
                    acts.push(Action::ExportPdf);
                    ui.close();
                }
                if ui.button(tl!("Save as email (.eml)…")).clicked() {
                    acts.push(Action::SaveMessage);
                    ui.close();
                }
            });
            // More, as Outlook's ⋯ beside Forward.
            let more = widgets::more_pill(ui, "More actions", &t);
            egui::Popup::menu(&more).show(|ui| {
                if ui.button(tl!("Reply with meeting")).clicked() {
                    acts.push(Action::Dialog(crate::Dialog::meeting_from(s, p)));
                    ui.close();
                }
                if ui.button(tl!("Forward as attachment")).clicked() {
                    acts.push(Action::Run("mail.forwardAsAttachment", json!({})));
                    ui.close();
                }
                if ui.button(tl!("Message headers")).clicked() {
                    acts.push(Action::Run("message.headers", json!({})));
                    ui.close();
                }
            });
        });
    }
}
