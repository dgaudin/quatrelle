// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message's files as cards: the icon by type, the name, the size, the host the link goes to,
//! and what can be done with it: "Open" (the engine downloads it, the window opens the saved
//! file: `app/actions/teams.rs`) and "Open in browser".
//!
//! What is hostile here: a file's link is the sender's. The host it goes to (out of its wrappers
//! when links are cleaned, as the mail shows it) is written on the card; "Link to another site"
//! (and no file-type icon) when what a click opens isn't the user's OneDrive nor their
//! organisation's SharePoint (a link check's wrapper stays: a sender can write one around any
//! address); the link opens only on a click, and only one a browser may be handed, as written
//! and as the window opens it (`parts::openable`); the pointer on "Open in browser" shows where.
//! "Open" asks the engine, which downloads by the file's id through the user's own sign-in, never
//! by the link. A file that could run a program says so on its card: it is shown in its folder,
//! not opened.

use egui::{RichText, Sense, Ui, vec2};
use quatrelle_engine::mime::{self, AttachmentKind};
use quatrelle_engine::model::addr::trust::url_host;
use quatrelle_engine::model::pim::teams::{Attachment, ChatMessage, FileWhy, one_line};
use quatrelle_engine::{TeamsFileState, teams_could_run};
use serde_json::json;

use super::Pane;
use super::parts::{destination, openable, shown};
use crate::icons::{self, Icon};
use crate::theme;
use crate::{Action, widgets};

/// Whether a file's host is the user's own: their OneDrive's, or their organisation's
/// SharePoint (the OneDrive's host without `-my`).
fn own_host(p: &Pane, host: &str) -> bool {
    let Some(drive) = p.s.teams.drive_host.as_deref().or(p.s.prefs.teams.drive_host.as_deref()) else { return false };
    let site = drive.split_once('.').and_then(|(first, rest)| Some(format!("{}.{rest}", first.strip_suffix("-my")?)));
    host.eq_ignore_ascii_case(drive) || site.is_some_and(|s| host.eq_ignore_ascii_case(&s))
}

/// Why a file wasn't downloaded, in a sentence; Teams' own code in brackets after "Teams refused
/// it" (the engine kept it only in the client's shape).
fn why_said(why: FileWhy, code: Option<&str>) -> String {
    match why {
        FileWhy::TooBig => tl!("It is bigger than Quatrelle downloads (100 MB): open it in the browser").to_string(),
        FileWhy::NoRoom => tl!("There isn't enough room on the disk").to_string(),
        FileWhy::Refused => match code {
            Some(code) => format!("{} ({})", tl!("Teams refused it"), one_line(code)),
            None => tl!("Teams refused it").to_string(),
        },
        FileWhy::Offline => tl!("Teams can't be reached").to_string(),
        FileWhy::Stalled => tl!("The download was too slow").to_string(),
        FileWhy::Busy => tl!("Too many files are being downloaded").to_string(),
        FileWhy::Gone => tl!("This file is no longer in the conversation").to_string(),
    }
}

/// The message's files, each a card: its icon by type, its name, its size when known and the host
/// its link goes to; "Link to another site" when that isn't the user's own; "Open" (not while it
/// downloads: "Downloading…"; a failure says why, and "Open" tries again) and "Open in browser".
pub(super) fn files(p: &Pane, ui: &mut Ui, m: &ChatMessage, width: f32, acts: &mut Vec<Action>) {
    let t = &p.t;
    for a in &m.attachments {
        let Attachment::File { id, name, url, size } = a else { continue };
        // The user's own by what a click opens, as it is; written where it goes, without what
        // could turn it around.
        let clean = p.s.prefs.clean_links;
        let goes = destination(url, clean);
        let opens = if clean { mime::clean_link(url) } else { url.clone() };
        let ours = url_host(&opens).is_some_and(|h| own_host(p, &h.to_ascii_lowercase()));
        let host = url_host(&goes).map(|h| shown(&h.to_ascii_lowercase()));
        let state = p.s.teams.file_state(&m.id, id);
        ui.add_space(4.0);
        egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(8).show(ui, |ui| {
            ui.set_max_width(width.min(340.0));
            ui.horizontal_top(|ui| {
                if ours {
                    let icon = match mime::kind_of("", name) {
                        AttachmentKind::Image => Icon::Image,
                        AttachmentKind::Video => Icon::Video,
                        _ => Icon::File,
                    };
                    let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                    icons::paint(ui.painter(), r, icon, t.accent);
                }
                ui.vertical(|ui| {
                    let line = one_line(name);
                    let g = widgets::one_line(ui, &line, theme::bold(13.0), t.text, ui.available_width());
                    ui.label(g);
                    let said: Vec<String> = size.map(widgets::size_text).into_iter().chain(host.clone()).collect();
                    if !said.is_empty() {
                        ui.label(RichText::new(said.join(" · ")).size(11.5).color(t.dim));
                    }
                    if !ours {
                        ui.label(RichText::new(tl!("Link to another site")).size(11.5).color(t.warn));
                    }
                    if teams_could_run(name) {
                        ui.label(RichText::new(tl!("This file could run a program: it is shown in its folder, not opened.")).size(11.5).color(t.dim));
                    }
                    match state {
                        Some(TeamsFileState::Asked) => {
                            ui.label(RichText::new(tl!("Downloading…")).size(11.5).color(t.dim));
                        }
                        Some(TeamsFileState::Failed { why, code }) => {
                            ui.label(RichText::new(tl!("This file couldn't be downloaded")).size(11.5).color(t.danger));
                            ui.label(RichText::new(why_said(*why, code.as_deref())).size(11.5).color(t.dim));
                        }
                        _ => {}
                    }
                    ui.horizontal_wrapped(|ui| {
                        // Not while it downloads (it would be asked twice); nothing to ask for
                        // without an address to a file.
                        if !url.is_empty() && !matches!(state, Some(TeamsFileState::Asked)) && ui.button(tl!("Open")).clicked() {
                            acts.push(Action::Run("teams.openFile", json!({"chat": m.chat, "message": m.id, "attachment": id})));
                        }
                        if openable(url, clean) && ui.button(tl!("Open in browser")).on_hover_text(shown(&goes)).clicked() {
                            acts.push(Action::OpenUrl(url.clone()));
                        }
                    });
                });
            });
        });
    }
}
