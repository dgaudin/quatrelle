// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Dialogs: previews, calendars and people.

use super::*;

/// What a calendar or contacts file opened with Quatrelle holds, as `file.open` answers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileHolds {
    /// Where the file is, and its own name.
    pub path: String,
    pub name: String,
    /// A calendar file (its events), else a contacts file (its people).
    pub calendar: bool,
    /// How many would be added, and the first few's titles or names.
    pub count: u64,
    pub titles: Vec<String>,
    /// Those left out: already there, not readable, past the most one file adds.
    pub already: u64,
    pub unreadable: u64,
    pub more: u64,
    /// The calendar or address book they would go to, and its account.
    pub into: String,
    pub account: String,
}

impl FileHolds {
    /// From `file.open`'s answer for a calendar or a contacts file.
    pub fn from_answer(v: &serde_json::Value) -> FileHolds {
        let text = |k: &str| v.get(k).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
        let count = |k: &str| v.get(k).and_then(serde_json::Value::as_u64).unwrap_or(0);
        let titles =
            v.get("titles").and_then(serde_json::Value::as_array).map(|l| l.iter().filter_map(|t| t.as_str().map(str::to_string)).take(5).collect());
        FileHolds {
            path: text("path"),
            name: text("name"),
            calendar: text("kind") == "calendar",
            count: count("count"),
            titles: titles.unwrap_or_default(),
            already: count("already"),
            unreadable: count("unreadable"),
            more: count("more"),
            into: text("into"),
            account: text("account"),
        }
    }
}

/// A calendar or contacts file: what it holds, where it would go, and what is left out.
fn add_from_file(ui: &mut Ui, f: &FileHolds, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(460.0);
    let (most, held) = if f.calendar {
        title(ui, t, "Add these events to your calendar?");
        (quatrelle_engine::MAX_FILE_EVENTS, tf!("Events in {file}: {n}", file = f.name, n = f.count))
    } else {
        title(ui, t, "Add these people to your contacts?");
        (quatrelle_engine::MAX_FILE_CARDS, tf!("People in {file}: {n}", file = f.name, n = f.count))
    };
    ui.label(RichText::new(held).color(t.text));
    for line in &f.titles {
        ui.label(RichText::new(format!("• {}", widgets::shorten(line, 60))).color(t.dim).small());
    }
    ui.add_space(4.0);
    if f.count > 0 {
        ui.label(RichText::new(tf!("They go to {place} ({account}).", place = f.into, account = f.account)).color(t.text));
    }
    if f.already > 0 {
        ui.label(RichText::new(tf!("Already there, left as they are: {n}", n = f.already)).color(t.dim).small());
    }
    if f.unreadable > 0 {
        ui.label(RichText::new(tf!("Left out because they couldn't be read: {n}", n = f.unreadable)).color(t.dim).small());
    }
    if f.more > 0 {
        ui.label(RichText::new(tf!("Left out because one file adds {max} at most: {n}", max = most, n = f.more)).color(t.dim).small());
    }
    ui.add_space(4.0);
    if f.calendar {
        ui.label(RichText::new(tl!("Nobody is invited: the guests the file names are left out.")).color(t.dim).small());
    }
    ui.label(RichText::new(tl!("Add a file only when you know where it comes from.")).color(t.dim).small());
    let (ok, cancel) = buttons(ui, t, "Add", f.count > 0);
    if ok {
        acts.push(Action::AddFile(f.path.clone()));
        return false;
    }
    !cancel
}

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, pics: &mut crate::reading::Pictures, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::ConfirmDeleteContact { uid, name } => {
            ui.set_width(380.0);
            if name.trim().is_empty() {
                title(ui, t, "Delete this contact?");
            } else {
                title(ui, t, &tf!("Delete “{name}”?", name = name));
            }
            ui.label(RichText::new(tl!("This contact will be deleted, along with everything kept about them. This can't be undone.")).color(t.dim));
            let (ok, cancel) = buttons(ui, t, "Delete", true);
            if ok {
                acts.push(Action::Run("contact.delete", json!({"uid": uid})));
                return false;
            }
            !cancel
        }
        // Delete on an appointment of a series, as Outlook asks: this one, or all of them?
        Dialog::DeleteOccurrence { uid, occurrence, name } => {
            ui.set_width(380.0);
            title(ui, t, &tf!("Delete “{name}”?", name = name));
            ui.add_space(10.0);
            let mut keep = true;
            button_row(ui, |ui| {
                if ui.button(tl!("Cancel")).clicked() {
                    keep = false;
                }
                if ui.button(tl!("Delete the series")).clicked() {
                    acts.push(Action::Run("event.delete", json!({"uid": uid})));
                    keep = false;
                }
                if widgets::primary_button(ui, "Delete this occurrence", t, true).clicked() {
                    acts.push(Action::Run("event.delete", json!({"uid": uid, "occurrence": occurrence})));
                    keep = false;
                }
            });
            keep
        }
        Dialog::OpenShare { link, server, path, always } => {
            ui.set_width(460.0);
            title(ui, t, "Open a network share?");
            ui.label(RichText::new(tf!("This link opens {path} on the server {server}.", path = path, server = server)).color(t.text));
            ui.add_space(4.0);
            ui.label(
                RichText::new(tl!("The server can ask for your sign-in: open a share only when you trust the person who sent the link."))
                    .color(t.dim)
                    .small(),
            );
            // A program there is never started from a link, whoever the server is.
            if quatrelle_engine::mime::share_runs_code(link) {
                ui.add_space(4.0);
                ui.label(RichText::new(tl!("This file could run a program: it is shown in its folder, not opened.")).color(t.dim).small());
            }
            ui.add_space(6.0);
            ui.checkbox(always, tl!("Don't ask again for this server"));
            let (ok, cancel) = buttons(ui, t, "Open", true);
            if ok {
                if *always {
                    acts.push(Action::Run("link.trustShareServer", json!({"server": server})));
                }
                acts.push(Action::OpenShare(link.clone()));
                return false;
            }
            !cancel
        }
        Dialog::AddFromFile(f) => add_from_file(ui, f, t, acts),
        Dialog::PicturePreview { key, index, name } => {
            let w = (ui.ctx().content_rect().width() * 0.75).clamp(360.0, 1200.0);
            let h = (ui.ctx().content_rect().height() * 0.7).max(240.0);
            ui.set_width(w);
            title(ui, t, name);
            // Its message's text alone here: the picture comes with the whole of it.
            let got = if s.text_first(key) { crate::reading::Got::Pending } else { pics.get(key, *index, crate::reading::Want::Big) };
            match got {
                crate::reading::Got::Ready(tex, _) => {
                    let sz = tex.size_vec2();
                    let scale = (w / sz.x).min(h / sz.y).min(1.0);
                    ui.vertical_centered(|ui| ui.add(egui::Image::new(&tex).fit_to_exact_size(sz * scale)));
                }
                crate::reading::Got::Pending => {
                    ui.vertical_centered(|ui| ui.spinner());
                }
                crate::reading::Got::Failed => {
                    ui.label(RichText::new(tl!("This picture can't be shown.")).color(t.dim));
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("Open")).clicked() {
                    acts.push(Action::InWindow(key.clone(), Box::new(Action::OpenAttachment(*index))));
                }
                close_row(ui)
            })
            .inner
        }
        Dialog::Preview { title: name, header, text, warning } => {
            let w = (ui.ctx().content_rect().width() * 0.7).clamp(360.0, 900.0);
            ui.set_width(w);
            title(ui, t, name);
            if let Some(warning) = warning {
                crate::reading::sender_warning(t, ui, warning);
            }
            for (k, v) in header.iter() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(k.as_str()).color(t.dim).small());
                    ui.label(RichText::new(v.as_str()).small());
                });
            }
            ui.add_space(6.0);
            let h = (ui.ctx().content_rect().height() * 0.6).max(200.0);
            egui::ScrollArea::both().max_height(h).auto_shrink([false, true]).show(ui, |ui| {
                let mut shown = text.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut shown).font(egui::FontId::monospace(13.0)).desired_width(f32::INFINITY).frame(egui::Frame::NONE),
                );
            });
            ui.add_space(6.0);
            button_row(ui, |ui| {
                let keep = !ui.button(tl!("Close")).clicked();
                if ui.button(tl!("Copy")).clicked() {
                    ui.ctx().copy_text(text.clone());
                }
                keep
            })
        }
        Dialog::Certificate { account, path, data, password } => {
            ui.set_width(420.0);
            let unlock = path.is_empty() && data.is_none();
            title(ui, t, if unlock { "Unlock certificate" } else { "Import certificate" });
            let file = path.rsplit(['/', '\\']).next().unwrap_or(path.as_str()).to_string();
            let ask = if unlock {
                tl!("Your certificate's password, to use it in this browser until the page is closed:").to_string()
            } else {
                tf!("The password {file} was saved with:", file = file)
            };
            ui.add(egui::Label::new(RichText::new(ask).color(t.dim)).wrap());
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(password).password(true).desired_width(f32::INFINITY));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(if cfg!(target_arch = "wasm32") {
                        tl!("The browser keeps the file, locked with its password; the password itself isn't kept.")
                    } else {
                        tl!("Its private key is kept in the system's keychain, not in Quatrelle's files.")
                    })
                    .color(t.faint)
                    .small(),
                )
                .wrap(),
            );
            let (ok, cancel) = buttons(ui, t, if unlock { "Unlock" } else { "Import" }, true);
            if ok || enter {
                let params = match data {
                    Some(d) => json!({"account": account, "data": d, "password": password}),
                    None if unlock => json!({"account": account, "kept": true, "password": password}),
                    None => json!({"account": account, "path": path, "password": password}),
                };
                acts.push(Action::Run("security.import", params));
                return false;
            }
            !cancel
        }
        Dialog::SmartCard { account, module, pin } => smart_card(ui, s, t, acts, account, module, pin),
        Dialog::SubscribeCalendar { url, name, error } => {
            ui.set_width(440.0);
            title(ui, t, "Subscribe to a calendar");
            ui.add(
                egui::Label::new(
                    RichText::new(tl!(
                        "Paste the calendar's link (iCal or webcal://). It shows beside yours, read-only, and is read again every few hours."
                    ))
                    .color(t.dim),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(url).hint_text("webcal://example.com/calendar.ics").desired_width(f32::INFINITY));
            // Before the focus request below, which would hide the Enter that ended input.
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            ui.add_space(6.0);
            ui.label(RichText::new(tl!("Name (optional)")).color(t.dim).small());
            ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
            if let Some(e) = error {
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(e.as_str()).color(t.danger).small()).wrap());
            }
            let ready = quatrelle_engine::model::pim::calendar_link(url).is_some();
            let (ok, cancel) = buttons(ui, t, "Subscribe", ready);
            if (ok || enter) && ready {
                acts.push(Action::Submit("calendar.subscribe", json!({"url": url.trim(), "name": name.trim()})));
            }
            !cancel
        }
        Dialog::Event(f) => crate::pim_dialogs::event(ui, f, s, t, acts),
        Dialog::Contact(f) => crate::pim_dialogs::contact(ui, f, t, acts),
        // Every other kind is shown by its own group (`body`).
        _ => true,
    }
}

/// A certificate on a smart card: the card's module and PIN, the certificates it has, one chosen.
fn smart_card(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>, account: &str, module: &mut String, pin: &mut String) -> bool {
    ui.set_width(460.0);
    title(ui, t, "Use a smart card");
    ui.add(egui::Label::new(RichText::new(tl!("The module your card came with (OpenSC's reads most cards):")).color(t.dim)).wrap());
    ui.add_space(4.0);
    ui.add(egui::TextEdit::singleline(module).hint_text("opensc-pkcs11").desired_width(f32::INFINITY));
    ui.add_space(8.0);
    ui.label(RichText::new(tl!("The card's PIN")).color(t.dim));
    let r = ui.add(egui::TextEdit::singleline(pin).password(true).desired_width(f32::INFINITY));
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.add_space(8.0);
    let search = s.card_search();
    ui.horizontal(|ui| {
        let ready = !module.trim().is_empty() && !search.searching;
        if ui.add_enabled(ready, egui::Button::new(tl!("Find certificates"))).clicked() || (enter && ready) {
            acts.push(Action::Run("security.findKeys", json!({"module": module.trim(), "pin": pin})));
        }
        if cfg!(target_os = "macos") && ui.add_enabled(!search.searching, egui::Button::new(tl!("Look in the Keychain"))).clicked() {
            acts.push(Action::Run("security.findKeys", json!({"keychain": true})));
        }
    });
    if search.searching {
        ui.add_space(6.0);
        ui.label(RichText::new(tl!("Looking…")).color(t.faint).small());
    }
    if let Some(e) = &search.error {
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(tl!(e)).color(t.danger).small()).wrap());
    }
    let mut chosen = false;
    for (i, k) in search.keys.iter().enumerate() {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let info = quatrelle_engine::smime::cert::info(&k.cert).ok();
            ui.vertical(|ui| {
                ui.label(RichText::new(&k.label).color(t.text));
                if let Some(info) = &info {
                    let who = info.emails.first().cloned().unwrap_or_else(|| info.name.clone());
                    ui.label(RichText::new(format!("{who} · {} · {}", info.issuer, info.key)).color(t.faint).small());
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tl!("Use")).clicked() {
                    acts.push(Action::Run("security.useKey", json!({"account": account, "index": i})));
                    chosen = true;
                }
            });
        });
    }
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(RichText::new(tl!("The key stays on the card; its PIN is kept in the system's keychain.")).color(t.faint).small()).wrap(),
    );
    !chosen && close_row(ui)
}
