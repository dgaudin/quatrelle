// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Exchange's archive and retention policies on screen: the right-click submenu of a message
//! and of a folder, listing the server's tags of each kind with the one in place ticked, and the
//! line a message's header carries.

use egui::{RichText, Sense, Ui, vec2};
use quatrelle_engine::model::{MessageKey, PolicyKind, dates};
use quatrelle_engine::{MessagePolicy, Session};
use serde_json::json;

use crate::Action;
use crate::icons::{self, Icon};
use crate::widgets;

/// What the submenu is for.
#[derive(Clone, Copy)]
pub(crate) enum PolicyFor<'a> {
    /// The selected messages, the clicked one's tag shown.
    Mail(&'a MessageKey),
    /// A folder of the account.
    Folder(&'a str, &'a str),
}

/// The « Policy » submenu, in two parts: the archive tags the server offers, then its retention
/// tags, each under its heading, in the server's words, the one in place ticked, and « As its
/// folder » (the message's own tag of that kind taken off, or the folder's, so its parent's
/// applies). A part the server offers no tag of isn't there; nothing at all when it offers none
/// or the mail is in the online archive.
pub(crate) fn policy_menu(ui: &mut Ui, s: &Session, acts: &mut Vec<Action>, target: PolicyFor<'_>) {
    let (account, folder) = match target {
        PolicyFor::Mail(k) => (k.account.as_str(), k.folder.as_str()),
        PolicyFor::Folder(a, f) => (a, f),
    };
    if !s.offers_policy(account, folder) {
        return;
    }
    ui.menu_button((widgets::menu_icon(Some(Icon::Clock)), tl!("Policy")), |ui| {
        let mut first = true;
        for kind in PolicyKind::ALL {
            if s.policy_tags(account, kind).next().is_none() {
                continue;
            }
            if !first {
                ui.separator();
            }
            first = false;
            policy_part(ui, s, acts, target, kind);
        }
    });
}

/// One part of the submenu: its heading, the tags of its kind, « As its folder ».
fn policy_part(ui: &mut Ui, s: &Session, acts: &mut Vec<Action>, target: PolicyFor<'_>, kind: PolicyKind) {
    let (account, current) = match target {
        PolicyFor::Mail(k) => (k.account.as_str(), s.message_tag(k, kind)),
        PolicyFor::Folder(a, f) => (a, s.folder_tag(a, f, kind)),
    };
    let (icon, heading) = match kind {
        PolicyKind::Archive => (Icon::Archive, tl!("Archive policy")),
        PolicyKind::Retention => (Icon::Delete, tl!("Retention policy")),
    };
    ui.horizontal(|ui| {
        let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        icons::paint(ui.painter(), ir, icon, ui.visuals().weak_text_color());
        ui.label(RichText::new(heading).strong());
    });
    let run = |tag: Option<&str>| -> Action {
        let (mail, folder) = match kind {
            PolicyKind::Archive => ("mail.archivePolicy", "folder.archivePolicy"),
            PolicyKind::Retention => ("mail.retentionPolicy", "folder.retentionPolicy"),
        };
        match target {
            PolicyFor::Mail(_) => Action::Run(mail, json!({"tag": tag})),
            PolicyFor::Folder(a, f) => Action::Run(folder, json!({"account": a, "folder": f, "tag": tag})),
        }
    };
    for tag in s.policy_tags(account, kind) {
        let mut on = current == Some(tag.id.as_str());
        // The server's own name for it, as it gives it.
        if ui.checkbox(&mut on, tag.name.as_str()).clicked() {
            acts.push(run(Some(&tag.id)));
            ui.close();
        }
    }
    let mut none = current.is_none();
    if ui.checkbox(&mut none, tl!("As its folder")).clicked() {
        acts.push(run(None));
        ui.close();
    }
}

/// The line in a message's header naming its policies: the archive tag's name, then the
/// retention tag's, each with the day the server plans (to move it, to delete it) when it says.
/// None when it carries neither.
pub(crate) fn policy_line(archive: Option<&MessagePolicy>, retention: Option<&MessagePolicy>) -> Option<String> {
    let archive = archive.map(|p| match p.day.as_deref().and_then(planned_day) {
        Some(date) => tf!("Archive: {name} ({date})", name = p.name, date = date),
        None => tf!("Archive: {name}", name = p.name),
    });
    let retention = retention.map(|p| match p.day.as_deref().and_then(planned_day) {
        Some(date) => tf!("Deletion: {name} ({date})", name = p.name, date = date),
        None => tf!("Deletion: {name}", name = p.name),
    });
    let parts: Vec<String> = archive.into_iter().chain(retention).collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
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
