// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Exchange's archive policy on screen: the right-click submenu of a message and of a folder,
//! listing the server's tags with the one in place ticked, and the line a message's header
//! carries.

use egui::Ui;
use quatrelle_engine::model::{MessageKey, dates};
use quatrelle_engine::{MessagePolicy, Session};
use serde_json::json;

use crate::Action;
use crate::icons::Icon;
use crate::widgets;

/// What the submenu is for.
#[derive(Clone, Copy)]
pub(crate) enum PolicyFor<'a> {
    /// The selected messages, the clicked one's tag shown.
    Mail(&'a MessageKey),
    /// A folder of the account.
    Folder(&'a str, &'a str),
}

/// The « Archive policy » submenu: the tags the server offers, in the server's words, the one in
/// place ticked, and « As its folder » (the message's own tag taken off, or the folder's, so its
/// parent's applies). Nothing at all when the server offers no tag or the mail is in the online
/// archive.
pub(crate) fn archive_policy_menu(ui: &mut Ui, s: &Session, acts: &mut Vec<Action>, target: PolicyFor<'_>) {
    let (account, folder, current) = match target {
        PolicyFor::Mail(k) => (k.account.as_str(), k.folder.as_str(), s.message_archive_tag(k)),
        PolicyFor::Folder(a, f) => (a, f, s.folder_archive_tag(a, f)),
    };
    if !s.offers_archive_policy(account, folder) {
        return;
    }
    let run = |tag: Option<&str>| -> Action {
        match target {
            PolicyFor::Mail(_) => Action::Run("mail.archivePolicy", json!({"tag": tag})),
            PolicyFor::Folder(a, f) => Action::Run("folder.archivePolicy", json!({"account": a, "folder": f, "tag": tag})),
        }
    };
    ui.menu_button((widgets::menu_icon(Some(Icon::Archive)), tl!("Archive policy")), |ui| {
        for tag in s.archive_tags(account) {
            let mut on = current == Some(tag.id.as_str());
            // The server's own name for it, as it gives it.
            if ui.checkbox(&mut on, tag.name.as_str()).clicked() {
                acts.push(run(Some(&tag.id)));
                ui.close();
            }
        }
        ui.separator();
        let mut none = current.is_none();
        if ui.checkbox(&mut none, tl!("As its folder")).clicked() {
            acts.push(run(None));
            ui.close();
        }
    });
}

/// The line in a message's header naming its archive policy: the tag's name, and the day the
/// server plans to move it when it says.
pub(crate) fn policy_line(p: &MessagePolicy) -> String {
    let name = tf!("Archive policy: {name}.", name = p.name);
    match p.day.as_deref().and_then(planned_day) {
        Some(day) => format!("{name} {}", tf!("Planned for {date}.", date = day)),
        None => name,
    }
}

/// A `YYYY-MM-DD` day, written as the interface writes days.
fn planned_day(day: &str) -> Option<String> {
    if !quatrelle_engine::model::job::is_day(day) {
        return None;
    }
    let num = |a: usize, z: usize| day.get(a..z)?.parse::<u32>().ok();
    let days = dates::days_from_civil(i64::from(num(0, 4)?), num(5, 7)?, num(8, 10)?);
    Some(crate::i18n::date(&dates::day_label(days)))
}
