// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message's own window as Outlook's: the tabs File, Message and Help, and the Message tab's
//! ribbon, its groups as Outlook's (Delete, Respond, Quick steps, Move, Tags, Find, Zoom), each
//! button one of Quatrelle's commands, named as Quatrelle names it. Drawn while the window's
//! message stands in as the one selected and read ([`Session::enter_window`]): what's offered is
//! for that message, and the window hands on what's asked as done to it. Outlook's buttons for what
//! Quatrelle doesn't do (Send to OneNote, All apps, Read aloud, Immersive reader, Translate)
//! aren't there.

use crate::*;

/// Where a message window's ribbon fit is kept from one frame to the next (`ribbon_groups`): one
/// per window, as each has its own width (one for all had two windows take each other's at every
/// frame, and never settle).
pub(super) fn fit_key(key: &MessageKey) -> String {
    format!("message-window/{}", key.to_id())
}

/// The window's tabs, then the Message tab's ribbon; too narrow for every group's large
/// buttons, its last groups drawn compact, as the main window's (worked out from this frame for
/// the next). `main`: there's a main window to open File and Help in.
pub(super) fn message_ribbon(ui: &mut Ui, s: &Session, st: &mut UiState, w: &quatrelle_engine::Reading, main: bool, acts: &mut Vec<Action>) {
    let t = st.tokens;
    tabs(ui, &t, &w.key, main, acts);
    ui.add_space(2.0);
    let fit_key = fit_key(&w.key);
    let fit = st.ribbon_groups.get(&fit_key).cloned().unwrap_or_default();
    let gid = widgets::group_fit_id();
    let room_right = ui.max_rect().right();
    ui.ctx().data_mut(|d| d.insert_temp(gid, widgets::GroupFit::start(fit.compact_from, fit.columns)));
    egui::ScrollArea::horizontal().id_salt(("message-ribbon", w.key.to_id())).show(ui, |ui| {
        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            groups(ui, s, &t, w, acts);
        });
    });
    let gfit = ui.ctx().data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<widgets::GroupFit>(gid)));
    let next = RibbonGroups {
        compact_from: gfit.fit(room_right),
        columns: gfit.groups.iter().map(|g| g.2.clone()).collect(),
        overflow: gfit.right - room_right,
    };
    if st.ribbon_groups.get(&fit_key).is_none_or(|g| g.compact_from != next.compact_from || g.columns != next.columns) {
        ui.ctx().request_repaint();
    }
    st.ribbon_groups.insert(fit_key, next);
}

/// File and Help open the main window's File page (Help: its About page), brought forward;
/// Message is the tab shown. Drawn inside the page (the browser), the window has no title bar to
/// close it from: Close at the right.
fn tabs(ui: &mut Ui, t: &Tokens, key: &MessageKey, main: bool, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let tab = |ui: &mut Ui, label: &str, on: bool| {
            let font = if on { theme::bold(13.5) } else { theme::regular(13.5) };
            ui.add(egui::Button::new(RichText::new(label).color(if on { t.accent } else { t.dim }).font(font)).frame_when_inactive(false))
        };
        if main && tab(ui, tl!("File"), false).clicked() {
            acts.push(Action::Backstage(backstage::Page::Info));
            acts.push(Action::FocusMain);
        }
        let r = tab(ui, tl!("Message"), true);
        let line = [egui::pos2(r.rect.left() + 6.0, r.rect.bottom() + 1.0), egui::pos2(r.rect.right() - 6.0, r.rect.bottom() + 1.0)];
        ui.painter().line_segment(line, egui::Stroke::new(2.0, t.accent));
        if main && tab(ui, tl!("Help"), false).on_hover_text(tl!("About")).clicked() {
            acts.push(Action::Backstage(backstage::Page::About));
            acts.push(Action::FocusMain);
        }
        if ui.ctx().embed_viewports() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::tiny_icon_button(ui, Icon::Close, "Close", t).clicked() {
                    acts.push(Action::Run("message.closeWindow", json!({"key": key.to_id()})));
                }
            });
        }
    });
}

/// The Message tab's groups, each button acting on the window's message.
fn groups(ui: &mut Ui, s: &Session, tokens: &Tokens, w: &quatrelle_engine::Reading, acts: &mut Vec<Action>) {
    let t = *tokens;
    let key = &w.key;
    let p = w.parsed.as_ref();
    let on = |id: &str| s.registry().get(id).is_some_and(|c| (c.enabled)(s).is_none());
    // What acts on the list's mail can't reach a message no longer in it (another folder opened
    // since): greyed, and said why.
    let listed = s.summary(key).is_some();
    let why =
        |r: egui::Response| if listed { r } else { r.on_hover_text(tl!("This message is no longer in the list: open its folder to change it.")) };
    let mut groups = widgets::Groups::new(ui);

    let ignored = s.summary(key).is_some_and(|m| s.is_ignored(m));
    let tip = if ignored { tl!("Stop ignoring the conversation") } else { tl!("Its messages, and replies that arrive later, go to Deleted Items") };
    if why(widgets::big_button(ui, Icon::Ignore, "Ignore", &t, on("mail.ignore")).on_hover_text(tip)).clicked() {
        acts.push(Action::Run("mail.ignore", json!({})));
    }
    // Junk, as Outlook's drop-down: reported (or not, in Junk Email), the sender's lists.
    let r = why(widgets::big_button(ui, Icon::Junk, "Junk", &t, listed));
    egui::Popup::menu(&r).show(|ui| {
        let in_junk = s.folder(&key.account, &key.folder).is_some_and(|f| f.role == quatrelle_engine::model::FolderRole::Junk);
        let (id, label) = if in_junk { ("mail.notJunk", tl!("Not junk")) } else { ("mail.junk", tl!("Report junk")) };
        if ui.add_enabled(on(id), egui::Button::new(label)).clicked() {
            acts.push(Action::Run(id, json!({})));
            ui.close();
        }
        ui.separator();
        crate::junk_menu(ui, s, acts);
    });
    // Asked first where it's for good, as the Delete key does.
    if why(widgets::big_button(ui, Icon::Delete, "Delete", &t, on("mail.delete")).on_hover_text(tl!("Delete (Del)"))).clicked() {
        acts.push(Action::DeleteMail { for_good: false });
    }
    if why(widgets::big_button(ui, Icon::Archive, "Archive", &t, on("mail.archive"))).clicked() {
        acts.push(Action::Run("mail.archive", json!({})));
    }
    groups.end(ui, &t, "Delete");

    for (id, icon, label, tip) in [
        ("mail.reply", Icon::Reply, "Reply", "Reply (Ctrl+R)"),
        ("mail.replyAll", Icon::ReplyAll, "Reply all", "Reply all (Ctrl+Shift+R)"),
        ("mail.forward", Icon::Forward, "Forward", "Forward (Ctrl+F)"),
    ] {
        if widgets::big_button(ui, icon, label, &t, on(id)).on_hover_text(tl!(tip)).clicked() {
            acts.push(Action::Run(id, json!({})));
        }
    }
    // More, beside Forward (its arrow joins Forward when the group is drawn compact).
    let r = widgets::big_more(ui, "More actions", &t);
    egui::Popup::menu(&r).show(|ui| {
        let item = egui::Button::new((widgets::menu_icon(Some(Icon::Forward)), tl!("Forward as attachment")));
        if ui.add_enabled(on("mail.forwardAsAttachment"), item).clicked() {
            acts.push(Action::Run("mail.forwardAsAttachment", json!({})));
            ui.close();
        }
    });
    // Reply with a meeting: the people it went to invited.
    if widgets::big_button(ui, Icon::Calendar, "Meeting", &t, p.is_some()).on_hover_text(tl!("Reply with meeting")).clicked()
        && let Some(p) = p
    {
        // Its text alone: someone it went to can have no address yet, and would be left out.
        acts.push(if w.text_first { Action::Refuse(crate::reading::still_coming(w)) } else { Action::Dialog(Dialog::meeting_from(s, p)) });
    }
    groups.end(ui, &t, "Respond");

    let r = why(widgets::big_button(ui, Icon::Steps, "Quick steps", &t, listed));
    egui::Popup::menu(&r).show(|ui| crate::quick_steps_menu(ui, s, acts));
    groups.end(ui, &t, "Quick steps");

    if why(widgets::big_button(ui, Icon::Move, "Move to", &t, on("mail.move"))).clicked() {
        acts.push(Action::Dialog(Dialog::move_to()));
    }
    // Rules: one made from this message's sender, or all of them.
    let r = widgets::big_button(ui, Icon::MailRules, "Rules", &t, true);
    egui::Popup::menu(&r).show(|ui| {
        if let Some(p) = p
            && ui.button((widgets::menu_icon(Some(Icon::MailRules)), tl!("Create rule…"))).clicked()
        {
            // A colleague's address may be Exchange's internal one, kept as a name alone: the
            // rule goes by the name then.
            let sender = if p.from.email.trim().is_empty() { p.from.name.trim() } else { p.from.email.trim() };
            let server = s.server_kind(&key.account).is_some();
            acts.push(Action::Dialog(Dialog::rule_from(sender, Some(&key.account), server, Some(&key.folder))));
            ui.close();
        }
        if ui.button(tl!("Manage rules")).clicked() {
            acts.push(Action::Dialog(Dialog::Rules));
            ui.close();
        }
    });
    // Outlook's Actions: what else is done with the message as a whole.
    let r = widgets::big_button(ui, Icon::File, "Actions", &t, p.is_some());
    egui::Popup::menu(&r).show(|ui| {
        let item = |ui: &mut Ui, icon: Icon, label: &str| ui.button((widgets::menu_icon(Some(icon)), label)).clicked();
        for (icon, label, act) in [
            (Icon::Print, tl!("Print"), Action::Print),
            (Icon::File, tl!("Save as PDF…"), Action::ExportPdf),
            (Icon::Inbox, tl!("Save as email (.eml)…"), Action::SaveMessage),
            (Icon::Info, tl!("Message headers"), Action::Run("message.headers", json!({}))),
            (Icon::Bcc, tl!("View message source"), Action::Run("message.source", json!({"key": key.to_id()}))),
        ] {
            if item(ui, icon, label) {
                acts.push(act);
                ui.close();
            }
        }
    });
    groups.end(ui, &t, "Move");

    // Exchange's retention and archive policies, where the server offers some.
    if s.offers_policy(&key.account, &key.folder) {
        let r = why(widgets::big_button(ui, Icon::Clock, "Policy", &t, listed));
        egui::Popup::menu(&r).show(|ui| crate::panes::policy::policy_parts(ui, s, acts, crate::panes::policy::PolicyFor::Mail(key)));
    }
    let unread = s.summary(key).is_some_and(|m| !m.flags.seen);
    let (id, icon, label) = if unread { ("mail.markRead", Icon::Read, "Mark read") } else { ("mail.markUnread", Icon::Unread, "Mark unread") };
    if why(widgets::big_button(ui, icon, label, &t, on(id))).clicked() {
        acts.push(Action::Run(id, json!({})));
    }
    let r = why(widgets::big_button(ui, Icon::Tag, "Categorize", &t, listed));
    egui::Popup::menu(&r).show(|ui| organize::categorize_menu(ui, s, &t, acts));
    let r = why(widgets::big_button(ui, Icon::Flag, "Follow up", &t, on("mail.flag")));
    egui::Popup::menu(&r).show(|ui| organize::follow_up_menu(ui, s, &t, acts));
    groups.end(ui, &t, "Tags");

    if widgets::big_button(ui, Icon::Search, "Find", &t, p.is_some()).on_hover_text(tl!("Find in message")).clicked() {
        acts.push(Action::FindInMessage);
    }
    let r = widgets::big_button(ui, Icon::Conversations, "Find related", &t, p.is_some());
    if let Some(p) = p {
        let sender = if p.from.email.trim().is_empty() { p.from.name.trim() } else { p.from.email.trim() };
        egui::Popup::menu(&r).show(|ui| organize::find_related(ui, &p.subject, sender, acts));
    }
    groups.end(ui, &t, "Find");

    // The message's zoom (all mail read is shown at it, as the status bar's).
    let now = s.prefs.reading_zoom;
    let r = widgets::big_button(ui, Icon::FontGrow, "Zoom", &t, true).on_hover_text(tf!("{percent}%", percent = now));
    egui::Popup::menu(&r).show(|ui| {
        for percent in ZOOMS {
            if ui.radio(now == percent, tf!("{percent}%", percent = percent)).clicked() {
                acts.push(Action::Run("view.zoom", json!({"percent": percent})));
                ui.close();
            }
        }
    });
    groups.last(ui, &t, "Zoom");
}

/// The zooms offered, as Outlook's Zoom offers them (all within [`quatrelle_engine::READING_ZOOM`]).
const ZOOMS: [u16; 8] = [50, 75, 100, 125, 150, 200, 300, 400];
