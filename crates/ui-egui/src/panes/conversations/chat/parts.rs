// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a message carries besides its words: its pictures, its files as cards, its cards, and its
//! reactions (read only in this delivery).
//!
//! What is hostile here: a picture kept with the message is asked for when the message is first
//! drawn, at most [`HOSTED_PER_MESSAGE`] (the rest stay placeholders); one from elsewhere waits
//! for a click on "Show picture", and drawing or scrolling never asks for it. A file's link
//! is the sender's: its host is written on the card, "Link to another site" (and no file-type
//! icon) when it's neither the user's OneDrive nor their organisation's SharePoint, and it opens
//! only on a click, through [`launchable_link`]. A reaction's emoji is drawn without control or
//! bidi characters, a few characters at most.

use egui::{RichText, Sense, Ui, Vec2, vec2};
use quatrelle_engine::mime::{self, AttachmentKind};
use quatrelle_engine::model::addr::trust::url_host;
use quatrelle_engine::model::ical::launchable_link;
use quatrelle_engine::model::pim::teams::{Attachment, HOSTED_PER_MESSAGE, MAX_REACTIONS, Picture, is_bidi, one_line};
use serde_json::json;

use super::{ConversationsUi, Pane, Shown};
use crate::icons::{self, Icon};
use crate::theme;
use crate::{Action, widgets};

/// The largest a thumbnail is drawn.
const THUMB: Vec2 = vec2(240.0, 180.0);
/// A placeholder's size.
const HELD: Vec2 = vec2(160.0, 100.0);
/// The most pictures from elsewhere drawn of a message.
const OUTSIDE_MAX: usize = 20;
/// The most names a reaction's hover lists.
const NAMES_MAX: usize = 20;
/// The most characters of a reaction's emoji drawn.
const EMOJI_CHARS: usize = 8;
/// The most messages remembered as having had their pictures asked for.
const ASKED_MAX: usize = 5_000;

/// An address as it's shown (on a card, on hover): without bidi characters, which could turn
/// what's read around.
fn shown(url: &str) -> String {
    url.chars().filter(|&c| !is_bidi(c)).collect()
}

/// A message's pictures in the order they come: kept with it (their ids, from its text and its
/// attachments, each once), and from elsewhere (their addresses, each once).
fn pictures_of(m: &quatrelle_engine::model::pim::teams::ChatMessage) -> (Vec<&str>, Vec<&str>) {
    let mut hosted: Vec<&str> = Vec::new();
    let mut outside: Vec<&str> = Vec::new();
    for run in m.body.blocks.iter().flat_map(|b| &b.runs) {
        if let Some(id) = run.hosted.as_deref()
            && !hosted.contains(&id)
        {
            hosted.push(id);
        }
        if let Some(url) = run.outside.as_deref()
            && !outside.contains(&url)
        {
            outside.push(url);
        }
    }
    for a in &m.attachments {
        if let Attachment::Hosted { id } = a
            && !hosted.contains(&id.as_str())
        {
            hosted.push(id);
        }
    }
    (hosted, outside)
}

/// The message's pictures: those kept with it asked for the first time it's on screen (the
/// window's other messages, laid out but scrolled away, wait), then drawn as thumbnails that open
/// big on a click; those from elsewhere as "Show picture".
pub(super) fn pictures(p: &Pane, c: &mut ConversationsUi, ui: &mut Ui, m: &quatrelle_engine::model::pim::teams::ChatMessage, acts: &mut Vec<Action>) {
    let (hosted, outside) = pictures_of(m);
    if hosted.is_empty() && outside.is_empty() {
        return;
    }
    // Not at the chat's first frame: it isn't scrolled to its newest yet.
    let settled = c.chat_view.as_ref().is_some_and(|v| v.0 == m.chat);
    let here = egui::Rect::from_min_size(ui.cursor().min, HELD);
    if !hosted.is_empty() && settled && ui.is_rect_visible(here) && !c.hosted_asked.contains(&m.id) {
        if c.hosted_asked.len() >= ASKED_MAX {
            c.hosted_asked.clear();
        }
        c.hosted_asked.insert(m.id.clone());
        for id in hosted.iter().take(HOSTED_PER_MESSAGE) {
            acts.push(Action::Run("teams.picture", json!({"kind": "hosted", "chat": m.chat, "message": m.id, "hosted": id})));
        }
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (i, id) in hosted.iter().enumerate() {
            let picture = Picture::Hosted { chat: m.chat.clone(), message: m.id.clone(), hosted: (*id).to_string() };
            match p.pics.image(&picture) {
                Some(Shown::Ready(tex)) => {
                    if thumbnail(ui, tex).clicked() {
                        c.viewing = Some(picture);
                    }
                }
                Some(Shown::Failed) => held(p, ui, tl!("This picture can't be shown."), false),
                None => held(p, ui, tl!("Picture"), i < HOSTED_PER_MESSAGE),
            }
        }
        for url in outside.iter().take(OUTSIDE_MAX) {
            let picture = Picture::Outside { url: (*url).to_string() };
            match p.pics.image(&picture) {
                Some(Shown::Ready(tex)) => {
                    if thumbnail(ui, tex).clicked() {
                        c.viewing = Some(picture);
                    }
                }
                Some(Shown::Failed) => held(p, ui, tl!("This picture can't be shown."), false),
                None => elsewhere(p, ui, url, acts),
            }
        }
    });
}

/// A picture drawn at most [`THUMB`], its shape kept.
fn thumbnail(ui: &mut Ui, tex: &egui::TextureHandle) -> egui::Response {
    let size = tex.size_vec2();
    let scale = (THUMB.x / size.x.max(1.0)).min(THUMB.y / size.y.max(1.0)).min(1.0);
    let r = ui.add(egui::Image::new(tex).fit_to_exact_size(size * scale).corner_radius(4.0).sense(Sense::click()));
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    r
}

/// A picture's place, [`HELD`] big: its frame and a picture's icon, and where what goes under it
/// is.
fn place(p: &Pane, ui: &mut Ui) -> egui::Rect {
    let t = &p.t;
    let (r, _) = ui.allocate_exact_size(HELD, Sense::hover());
    ui.painter().rect_stroke(r, 4.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let icon = egui::Rect::from_center_size(egui::pos2(r.center().x, r.top() + 30.0), vec2(22.0, 22.0));
    icons::paint(ui.painter(), icon, Icon::Image, t.faint);
    egui::Rect::from_min_max(egui::pos2(r.left() + 6.0, r.top() + 48.0), egui::pos2(r.right() - 6.0, r.bottom() - 6.0))
}

/// A picture's place while it's waited for (`waiting`), or when it can't be shown.
fn held(p: &Pane, ui: &mut Ui, said: &str, waiting: bool) {
    let under = place(p, ui);
    let g = ui.painter().layout(said.to_string(), theme::regular(11.5), p.t.dim, under.width());
    let at = egui::pos2(under.center().x - g.size().x / 2.0, under.top());
    let below = at.y + g.size().y + 4.0;
    ui.painter().galley(at, g, p.t.dim);
    if waiting {
        egui::Spinner::new().size(14.0).paint_at(ui, egui::Rect::from_center_size(egui::pos2(under.center().x, below + 8.0), vec2(14.0, 14.0)));
    }
}

/// A picture from elsewhere: nothing fetched until "Show picture" is clicked (an `https` address
/// a browser could be handed, else no button).
fn elsewhere(p: &Pane, ui: &mut Ui, url: &str, acts: &mut Vec<Action>) {
    let https = url.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("https://")) && launchable_link(url);
    if !https {
        return held(p, ui, tl!("Picture"), false);
    }
    let under = place(p, ui);
    let button = egui::Rect::from_center_size(egui::pos2(under.center().x, under.top() + 12.0), vec2(under.width(), 24.0));
    if ui.put(button, egui::Button::new(tl!("Show picture"))).on_hover_text(shown(url)).clicked() {
        acts.push(Action::Run("teams.picture", json!({"kind": "outside", "url": url})));
    }
}

/// Whether a file's host is the user's own: their OneDrive's, or their organisation's
/// SharePoint (the OneDrive's host without `-my`).
fn own_host(p: &Pane, host: &str) -> bool {
    let Some(drive) = p.s.teams.drive_host.as_deref().or(p.s.prefs.teams.drive_host.as_deref()) else { return false };
    let site = drive.split_once('.').and_then(|(first, rest)| Some(format!("{}.{rest}", first.strip_suffix("-my")?)));
    host.eq_ignore_ascii_case(drive) || site.is_some_and(|s| host.eq_ignore_ascii_case(&s))
}

/// The message's files, each a card: its icon by type, its name, its size when known and the host
/// its link goes to; "Link to another site" when that isn't the user's own; "Open in browser".
pub(super) fn files(p: &Pane, ui: &mut Ui, m: &quatrelle_engine::model::pim::teams::ChatMessage, width: f32, acts: &mut Vec<Action>) {
    let t = &p.t;
    for a in &m.attachments {
        let Attachment::File { name, url, size, .. } = a else { continue };
        // Judged as it is; written without what could turn it around.
        let host = url_host(url).map(|h| h.to_ascii_lowercase());
        let ours = host.as_deref().is_some_and(|h| own_host(p, h));
        let host = host.map(|h| shown(&h));
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
                    let name = one_line(name);
                    let g = widgets::one_line(ui, &name, theme::bold(13.0), t.text, ui.available_width());
                    ui.label(g);
                    let said: Vec<String> = size.map(widgets::size_text).into_iter().chain(host.clone()).collect();
                    if !said.is_empty() {
                        ui.label(RichText::new(said.join(" · ")).size(11.5).color(t.dim));
                    }
                    if !ours {
                        ui.label(RichText::new(tl!("Link to another site")).size(11.5).color(t.warn));
                    }
                    if launchable_link(url) && ui.button(tl!("Open in browser")).on_hover_text(shown(url)).clicked() {
                        acts.push(Action::OpenUrl(url.clone()));
                    }
                });
            });
        });
    }
}

/// What Quatrelle doesn't draw (cards, polls): said so, with "Open in Teams".
pub(super) fn cards(p: &Pane, ui: &mut Ui, m: &quatrelle_engine::model::pim::teams::ChatMessage, acts: &mut Vec<Action>) {
    if !m.attachments.iter().any(|a| matches!(a, Attachment::Card)) {
        return;
    }
    let t = &p.t;
    ui.add_space(4.0);
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6).inner_margin(8).show(ui, |ui| {
        ui.label(RichText::new(tl!("This content can be seen in Teams")).size(12.5).color(t.dim));
        p.open_in_teams(ui, acts);
    });
}

/// The message's reactions, as chips (the emoji and how many); hovered, who reacted (the chat's
/// members, by name).
pub(super) fn reactions(p: &Pane, ui: &mut Ui, m: &quatrelle_engine::model::pim::teams::ChatMessage) {
    if m.reactions.is_empty() {
        return;
    }
    let t = &p.t;
    ui.horizontal_wrapped(|ui| {
        for r in m.reactions.iter().take(MAX_REACTIONS) {
            let emoji: String = r.emoji.chars().filter(|&c| !(c.is_control() || is_bidi(c))).take(EMOJI_CHARS).collect();
            if emoji.is_empty() {
                continue;
            }
            let mine = r.users.iter().any(|u| !p.me.is_empty() && u.eq_ignore_ascii_case(p.me));
            let stroke = egui::Stroke::new(1.0, if mine { t.accent } else { t.border });
            let chip = egui::Frame::new()
                .stroke(stroke)
                .corner_radius(10)
                .inner_margin(egui::Margin::symmetric(7, 1))
                .show(ui, |ui| ui.label(RichText::new(format!("{emoji} {}", r.users.len())).size(12.0).color(t.text)))
                .response;
            let names: Vec<String> = r
                .users
                .iter()
                .filter_map(|u| p.s.teams.members.iter().find(|x| x.user_id == *u))
                .map(|x| one_line(&x.name))
                .filter(|n| !n.is_empty())
                .take(NAMES_MAX)
                .collect();
            if !names.is_empty() {
                chip.on_hover_text(names.join(", "));
            }
        }
    });
}
