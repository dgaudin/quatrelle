// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a message carries besides its words: its pictures and its cards (its files' cards are
//! `files.rs`', its reactions `reactions.rs`').
//!
//! What is hostile here: a picture kept with the message is asked for when the message is first
//! drawn, at most [`HOSTED_PER_MESSAGE`] (the rest stay placeholders); one from elsewhere waits
//! for a click on "Show picture", and drawing or scrolling never asks for it.

use egui::{RichText, Sense, Ui, Vec2, vec2};
use quatrelle_engine::model::addr::trust::url_host;
use quatrelle_engine::model::ical::launchable_link;
use quatrelle_engine::model::pim::teams::{Attachment, ChatMessage, HOSTED_PER_MESSAGE, Picture, is_bidi};
use serde_json::json;

use super::{ConversationsUi, Pane, Shown};
use crate::Action;
use crate::icons::{self, Icon};
use crate::theme;

/// The largest a thumbnail is drawn.
const THUMB: Vec2 = vec2(240.0, 180.0);
/// A placeholder's size.
const HELD: Vec2 = vec2(160.0, 100.0);
/// The most pictures from elsewhere drawn of a message.
const OUTSIDE_MAX: usize = 20;
/// The most messages remembered as having had their pictures asked for.
const ASKED_MAX: usize = 5_000;

/// An address as it's shown (on a card, on hover): without bidi characters, which could turn
/// what's read around.
pub(super) fn shown(url: &str) -> String {
    url.chars().filter(|&c| !is_bidi(c)).collect()
}

/// Whether a sender's link opens on a click: it is an address a browser may be handed
/// ([`launchable_link`]), as a link of a message always had to be, and so is what the window
/// opens of it when cleaning took it to another host (out of a wrapper taken off when links are
/// `clean`ed: File › Options › Safety). The window checks that one again.
pub(super) fn openable(url: &str, clean: bool) -> bool {
    let opens = if clean { quatrelle_engine::mime::clean_link(url) } else { url.to_string() };
    let host = |s: &str| url_host(s).map(|h| h.to_ascii_lowercase());
    launchable_link(url) && (host(&opens) == host(url) || launchable_link(&opens))
}

/// Where a sender's link goes, as it's shown (on hover, a card's host): out of every wrapper when
/// links are `clean`ed, as the mail shows it; else as it's written.
pub(super) fn destination(url: &str, clean: bool) -> String {
    if clean { quatrelle_engine::mime::link_destination(url) } else { url.to_string() }
}

/// A message's pictures in the order they come: kept with it (their ids, from its text and its
/// attachments, each once), at most [`HOSTED_PER_MESSAGE`], and from elsewhere (their
/// addresses, each once), at most [`OUTSIDE_MAX`]. The rest aren't gathered: a message can carry
/// 20,000 pictures, and this runs every frame.
fn pictures_of(m: &ChatMessage) -> (Vec<&str>, Vec<&str>) {
    let mut hosted: Vec<&str> = Vec::new();
    let mut outside: Vec<&str> = Vec::new();
    fn add<'a>(list: &mut Vec<&'a str>, most: usize, item: &'a str) {
        if list.len() < most && !list.contains(&item) {
            list.push(item);
        }
    }
    for run in m.body.blocks.iter().flat_map(|b| &b.runs) {
        if hosted.len() >= HOSTED_PER_MESSAGE && outside.len() >= OUTSIDE_MAX {
            break;
        }
        if let Some(id) = run.hosted.as_deref() {
            add(&mut hosted, HOSTED_PER_MESSAGE, id);
        }
        if let Some(url) = run.outside.as_deref() {
            add(&mut outside, OUTSIDE_MAX, url);
        }
    }
    for a in &m.attachments {
        if let Attachment::Hosted { id } = a {
            add(&mut hosted, HOSTED_PER_MESSAGE, id.as_str());
        }
    }
    (hosted, outside)
}

/// The message's pictures: those kept with it asked for the first time it's on screen (the
/// window's other messages, laid out but scrolled away, wait), and again once let go for room,
/// then drawn as thumbnails that open big on a click; those from elsewhere as "Show picture".
/// A picture not there yet is a still placeholder: nothing keeps the window drawing for it.
pub(super) fn pictures(p: &Pane, c: &mut ConversationsUi, ui: &mut Ui, m: &ChatMessage, acts: &mut Vec<Action>) {
    let (hosted, outside) = pictures_of(m);
    if hosted.is_empty() && outside.is_empty() {
        return;
    }
    let picture = |id: &str| Picture::Hosted { chat: m.chat.clone(), message: m.id.clone(), hosted: id.to_string() };
    ui.add_space(4.0);
    let drawn = ui.horizontal_wrapped(|ui| {
        for id in &hosted {
            let picture = picture(id);
            match p.pics.image(&picture) {
                Some(Shown::Ready(tex)) => {
                    if thumbnail(ui, tex).clicked() {
                        c.viewing = Some(picture);
                    }
                }
                Some(Shown::Failed) => held(p, ui, tl!("This picture can't be shown.")),
                Some(Shown::Unavailable) => held(p, ui, tl!("Picture not available")),
                None => held(p, ui, tl!("Picture")),
            }
        }
        for url in &outside {
            let picture = Picture::Outside { url: (*url).to_string() };
            match p.pics.image(&picture) {
                Some(Shown::Ready(tex)) => {
                    if thumbnail(ui, tex).clicked() {
                        c.viewing = Some(picture);
                    }
                }
                Some(Shown::Failed) => held(p, ui, tl!("This picture can't be shown.")),
                Some(Shown::Unavailable) => held(p, ui, tl!("Picture not available")),
                None => elsewhere(p, ui, url, acts),
            }
        }
    });
    // Asked for once some of them are on screen (not at the chat's first frame: it isn't
    // scrolled to its newest yet); those still kept aren't asked again.
    let settled = c.chat_view.as_ref().is_some_and(|v| v.0 == m.chat);
    if !hosted.is_empty() && settled && ui.is_rect_visible(drawn.response.rect) && !c.hosted_asked.contains(&m.id) {
        if c.hosted_asked.len() >= ASKED_MAX {
            c.hosted_asked.clear();
        }
        c.hosted_asked.insert(m.id.clone());
        for id in hosted.iter().filter(|id| p.pics.image(&picture(id)).is_none()) {
            acts.push(Action::Run("teams.picture", json!({"kind": "hosted", "chat": m.chat, "message": m.id, "hosted": id})));
        }
    }
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

/// A picture's place while it's waited for, or when it can't be shown: still (a spinner would
/// draw the window again and again).
fn held(p: &Pane, ui: &mut Ui, said: &str) {
    let under = place(p, ui);
    let g = ui.painter().layout(said.to_string(), theme::regular(11.5), p.t.dim, under.width());
    let at = egui::pos2(under.center().x - g.size().x / 2.0, under.top());
    ui.painter().galley(at, g, p.t.dim);
}

/// A picture from elsewhere: nothing fetched until "Show picture" is clicked (an `https` address
/// a browser could be handed, else no button).
fn elsewhere(p: &Pane, ui: &mut Ui, url: &str, acts: &mut Vec<Action>) {
    let https = url.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("https://")) && launchable_link(url);
    if !https {
        return held(p, ui, tl!("Picture"));
    }
    let under = place(p, ui);
    let button = egui::Rect::from_center_size(egui::pos2(under.center().x, under.top() + 12.0), vec2(under.width(), 24.0));
    // Waiting for the answer: not clicked again (a double click would fetch twice).
    let waiting = p.s.teams.is_asked(&Picture::Outside { url: url.to_string() });
    let r = ui
        .add_enabled_ui(!waiting, |ui| {
            ui.put(button, egui::Button::new(tl!("Show picture"))).on_hover_text(shown(url)).on_disabled_hover_text(shown(url))
        })
        .inner;
    if r.clicked() {
        acts.push(Action::Run("teams.picture", json!({"kind": "outside", "url": url})));
    }
}

/// What Quatrelle doesn't draw (cards, polls): said so, with "Open in Teams".
pub(super) fn cards(p: &Pane, ui: &mut Ui, m: &ChatMessage, acts: &mut Vec<Action>) {
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
