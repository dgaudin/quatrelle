// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Exchange's archive and retention policies on screen: the right-click submenu of a message
//! and of a folder, listing the server's tags of each kind by their period (the server's name on
//! hover) with the one in place ticked, and the line a message's header carries.

use std::collections::HashMap;

use egui::{RichText, Sense, Ui, vec2};
use quatrelle_engine::model::{ArchiveTag, MessageKey, PolicyKind, dates};
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
/// tags, each under its heading, by their period ([`tag_labels`]), the one in place ticked, and « As its
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
    let tags: Vec<&ArchiveTag> = s.policy_tags(account, kind).collect();
    for (tag, label) in tags.iter().zip(tag_labels(&tags)) {
        let mut on = current == Some(tag.id.as_str());
        // Its period; the server's own name for it, as it gives it, on hover.
        if ui.checkbox(&mut on, label).on_hover_text(tag.name.as_str()).clicked() {
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

/// The line in a message's header naming its policies: the archive tag as the submenu shows it,
/// then the retention tag, each with the day the server plans (to move it, to delete it) when it
/// says. None when it carries neither.
pub(crate) fn policy_line(s: &Session, account: &str, archive: Option<&MessagePolicy>, retention: Option<&MessagePolicy>) -> Option<String> {
    let label = |p: &MessagePolicy, kind| tag_label(s, account, kind, &p.id).unwrap_or_else(|| p.name.clone());
    let archive = archive.map(|p| match p.day.as_deref().and_then(planned_day) {
        Some(date) => tf!("Archive: {name} ({date})", name = label(p, PolicyKind::Archive), date = date),
        None => tf!("Archive: {name}", name = label(p, PolicyKind::Archive)),
    });
    let retention = retention.map(|p| match p.day.as_deref().and_then(planned_day) {
        Some(date) => tf!("Deletion: {name} ({date})", name = label(p, PolicyKind::Retention), date = date),
        None => tf!("Deletion: {name}", name = label(p, PolicyKind::Retention)),
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

/// A tag's period, short: « Never » for the one that keeps mail where it is; else whole years,
/// whole months (of thirty days), whole weeks, or days.
fn period_label(tag: &ArchiveTag) -> String {
    match tag.days.filter(|d| *d > 0 && !tag.never) {
        None => tl!("Never").to_string(),
        Some(d) if d % 365 == 0 => match d / 365 {
            1 => tl!("1 year").to_string(),
            n => tf!("{n} years", n = n),
        },
        Some(d) if d % 30 == 0 => match d / 30 {
            1 => tl!("1 month").to_string(),
            n => tf!("{n} months", n = n),
        },
        Some(d) if d % 7 == 0 => match d / 7 {
            1 => tl!("1 week").to_string(),
            n => tf!("{n} weeks", n = n),
        },
        Some(1) => tl!("1 day").to_string(),
        Some(d) => tf!("{n} days", n = d),
    }
}

/// What each of a kind's tags is shown as, in their order: its period ([`period_label`]), or the
/// server's name for those whose period reads as another's (the name tells them apart).
pub(crate) fn tag_labels(tags: &[&ArchiveTag]) -> Vec<String> {
    let periods: Vec<String> = tags.iter().map(|t| period_label(t)).collect();
    let mut alike: HashMap<&str, usize> = HashMap::new();
    for p in &periods {
        *alike.entry(p.as_str()).or_default() += 1;
    }
    periods.iter().zip(tags).map(|(p, t)| if alike.get(p.as_str()).is_some_and(|n| *n > 1) { t.name.clone() } else { p.clone() }).collect()
}

/// How the tag `id` of `kind` is shown, as the submenu shows it; none when it isn't offered.
fn tag_label(s: &Session, account: &str, kind: PolicyKind, id: &str) -> Option<String> {
    let tags: Vec<&ArchiveTag> = s.policy_tags(account, kind).collect();
    let at = tags.iter().position(|t| t.id == id)?;
    tag_labels(&tags).into_iter().nth(at)
}
