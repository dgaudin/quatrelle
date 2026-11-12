// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Pictures and presence, fetched by the Teams worker for what is on screen, only when the engine
//! asks:
//!
//! * A message's hosted image ([`Picture::Hosted`]): only one of the first
//!   [`HOSTED_PER_MESSAGE`] the copy's message shows, never one of a message the copy holds as
//!   deleted or doesn't hold. The client builds the address from the message's own chat and ids
//!   (`TeamsClient::hosted`, which checks them again), reads [`MAX_PICTURE`] bytes at most, a
//!   chat's at most one a second. Kept in the copy's image cache under its bytes' hash.
//! * Someone's photo ([`Picture::Photo`]): kept the same way. None (404) means initials, and is
//!   remembered a day on Graph's clock.
//! * An image from another site ([`Picture::Outside`]): the user clicked it. Over `https` only, to
//!   the internet only (`http::get_public`: never this computer or its network, whatever the name
//!   resolves to, redirects included), [`MAX_PICTURE`] bytes at most; never kept.
//! * A picture the copy has is answered from it at once, without a request; one that can't be had
//!   (refused, too big, not the message's own) isn't asked again in this run.
//! * Presence ([`Job::Visible`](quatrelle_model::pim::teams::Job::Visible)): the people shown, at
//!   most [`PRESENCE_IDS`], asked every minute in one request, said in events of [`EVENT_ITEMS`],
//!   kept in memory only.
//!
//! The worker never decodes a picture: the window does, off its own thread, within its limits.
//! What is hostile here: what a sender wrote (its hosted images' ids, its pictures' addresses), and
//! the copy read back. What it protects: nothing a sender wrote makes Quatrelle reach out on its
//! own (pictures are asked for what the screen shows, an outside one on a click), the work a
//! message can cause (twenty images of ten megabytes), and a deleted message's pictures staying
//! gone.

use std::collections::{HashSet, VecDeque};
use std::net::IpAddr;
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{self, Attachment, ChatMessage, EVENT_ITEMS, HOSTED_PER_MESSAGE, MAX_PICTURE, PRESENCE_IDS, Picture, Presence};

use super::backfill::room;
use super::graph::{TeamsClient, presence_of};
use super::limits::{Cadence, Route, retry_wait};
use super::store::id_ok;
use super::worker::{Worker, busy, kind, of_the_network};
use crate::NetError;
use crate::http::{self, Url};
use crate::stream::is_public;

/// The most pictures waiting to be fetched: past it, the oldest asked for is let go (what the
/// screen shows now matters more).
const MAX_PENDING: usize = 200;
/// The most pictures remembered as not to be had in this run.
const MAX_REFUSED: usize = 1_000;
/// The most pages of a chat looked through for a picture's message, newest first.
const LOOKUP_PAGES: usize = 200;
/// Short of disk, pictures are looked at again this long after.
const ROOM_LOOK: Duration = Duration::from_secs(10);
/// How long Graph's word that someone has no photo holds.
const DAY_MS: i64 = 86_400_000;
/// The longest outside picture's address fetched.
const MAX_ADDRESS: usize = 4096;

/// What the pictures and the presence hold between two steps, in memory only. Every collection is
/// bounded ([`MAX_PENDING`], [`MAX_REFUSED`], [`PRESENCE_IDS`]).
#[derive(Default)]
pub(crate) struct Pictures {
    /// Asked for and not fetched yet, oldest first, each once.
    pending: VecDeque<Picture>,
    /// Those not to be had in this run: refused, too big, not their message's own.
    refused: HashSet<Picture>,
    /// The people shown (user ids).
    visible: Vec<String>,
    /// The presence's last minute came with nobody shown: someone shown now is asked at once.
    idle: bool,
    /// Short of disk: no picture fetched before then.
    not_before: Option<Instant>,
    /// The page where a message of this chat was last found: looked at first next time.
    found: Option<(String, u32)>,
}

impl Pictures {
    /// Nothing under way any more (switched off, signed out, another sign-in): the people shown
    /// stay, the window still shows them.
    pub(crate) fn halt(&mut self) {
        *self = Pictures { visible: std::mem::take(&mut self.visible), ..Pictures::default() };
    }
}

/// [`Job::Picture`](quatrelle_model::pim::teams::Job::Picture): answered from the copy at once
/// when it has it, an outside picture fetched now (the user clicked it), any other left to the
/// next steps. One without a usable id is answered with nothing.
pub(crate) fn picture(w: &mut Worker, picture: Picture) {
    let usable = match &picture {
        Picture::Hosted { chat, message, hosted } => [chat, message, hosted].into_iter().all(|id| id_ok(id)),
        Picture::Photo { user } => id_ok(user),
        Picture::Outside { url } => return outside(w, url),
    };
    if !usable {
        return log::info!("teams: a picture without a usable id passed over");
    }
    if let Some(bytes) = kept(w, &picture) {
        return say(w, picture, bytes);
    }
    let p = &mut w.pictures;
    if p.refused.contains(&picture) || p.pending.contains(&picture) {
        return;
    }
    if p.pending.len() >= MAX_PENDING {
        p.pending.pop_front();
    }
    p.pending.push_back(picture);
}

/// Whether a picture waits that may be fetched at `now`, as far as the disk goes: the loop comes
/// back to it at once.
pub(crate) fn waiting(w: &Worker, now: Instant) -> bool {
    !w.pictures.pending.is_empty() && w.pictures.not_before.is_none_or(|at| now >= at)
}

/// One picture fetched: the first one waiting whose route the limiter lets go now. Whether a
/// request was made.
pub(crate) fn step(w: &mut Worker, now: Instant) -> bool {
    if !waiting(w, now) {
        return false;
    }
    if !room(w, now) {
        w.pictures.not_before = now.checked_add(ROOM_LOOK);
        return false;
    }
    w.pictures.not_before = None;
    let routes: Vec<Route> = w.pictures.pending.iter().map(|p| route(w, p)).collect();
    let Some(at) = routes.iter().position(|r| w.limiter.wait(*r, now).is_zero()) else {
        // Told when the first may go: the loop doesn't turn again before.
        if let Some(r) = routes.first() {
            w.may(*r, now);
        }
        return false;
    };
    match w.pictures.pending.remove(at) {
        Some(Picture::Hosted { chat, message, hosted }) => fetch_hosted(w, chat, message, hosted, now),
        Some(Picture::Photo { user }) => fetch_photo(w, user, now),
        Some(Picture::Outside { .. }) | None => false,
    }
}

/// [`Job::Visible`](quatrelle_model::pim::teams::Job::Visible): the people now shown, each once,
/// the first [`PRESENCE_IDS`] with a usable id. Shown after a minute with nobody, they're asked at
/// once; otherwise at the minute.
pub(crate) fn visible(w: &mut Worker, users: Vec<String>) {
    let mut seen = HashSet::new();
    let users: Vec<String> = users.into_iter().filter(|u| id_ok(u) && seen.insert(u.clone())).take(PRESENCE_IDS).collect();
    let someone = !users.is_empty();
    w.pictures.visible = users;
    if someone && std::mem::take(&mut w.pictures.idle) {
        w.limiter.prompt(Cadence::Presence);
    }
}

/// The presence's minute: the people shown asked in one request, said by user id, only those
/// asked for. Whether a request was made.
pub(crate) fn presence(w: &mut Worker, now: Instant) -> bool {
    if !w.limiter.due(Cadence::Presence, now) {
        return false;
    }
    if w.pictures.visible.is_empty() {
        w.limiter.ran(Cadence::Presence, now);
        w.pictures.idle = true;
        return false;
    }
    if !w.may(Route::Presence, now) {
        return false;
    }
    w.limiter.ran(Cadence::Presence, now);
    w.pictures.idle = false;
    let ids = w.pictures.visible.clone();
    match w.request(Route::Presence, now, |c| c.presences(&ids)) {
        Ok(found) => {
            let asked: HashSet<&str> = ids.iter().map(String::as_str).collect();
            let mut said = HashSet::new();
            let presences: Vec<(String, Presence)> =
                found.iter().filter_map(presence_of).filter(|(id, _)| asked.contains(id.as_str()) && said.insert(id.clone())).collect();
            for chunk in presences.chunks(EVENT_ITEMS) {
                w.say(teams::Event::Presence { presences: chunk.to_vec() });
            }
        }
        // Busy or offline: as any cadence. Refused (a permission not granted): the dots stay
        // grey, and the rest goes on.
        Err(e) if busy(&e) || of_the_network(&e) => w.failed(Cadence::Presence, "presence", &e, now),
        Err(e) => log::warn!("teams: presence failed: {}", kind(&e)),
    }
    true
}

/// What limits a picture's request: its chat's hosted contents, or the photos.
fn route(w: &Worker, p: &Picture) -> Route {
    match p {
        Picture::Hosted { chat, .. } => Route::Hosted(w.key_of(chat)),
        Picture::Photo { .. } | Picture::Outside { .. } => Route::Photo,
    }
}

/// A picture as the copy has it.
fn kept(w: &Worker, picture: &Picture) -> Option<Vec<u8>> {
    let copy = w.copy.as_ref()?;
    match picture {
        Picture::Hosted { chat, message, hosted } => copy.hosted(chat, message, hosted),
        Picture::Photo { user } => copy.photo(user).0,
        Picture::Outside { .. } => None,
    }
}

fn say(w: &Worker, picture: Picture, bytes: Vec<u8>) {
    w.say(teams::Event::Picture { picture, bytes });
}

/// A message's hosted image from Graph: only one of the first twenty the copy's message shows, the
/// message not deleted; kept, then said. Whether a request was made.
fn fetch_hosted(w: &mut Worker, chat: String, message: String, hosted: String, now: Instant) -> bool {
    let picture = Picture::Hosted { chat: chat.clone(), message: message.clone(), hosted: hosted.clone() };
    if let Some(bytes) = kept(w, &picture) {
        say(w, picture, bytes);
        return false;
    }
    // Nothing changes the copy between this look and the picture kept: a deletion that purged
    // its pictures is never undone by a late fetch.
    let Some(m) = message_in_copy(w, &chat, &message) else {
        log::info!("teams: a picture of a message the copy doesn't hold passed over");
        return refuse(w, picture);
    };
    if m.deleted.is_some() || !first_hosted(&m).contains(&hosted.as_str()) {
        log::info!("teams: a picture that isn't one of the first {HOSTED_PER_MESSAGE} of a message kept passed over");
        return refuse(w, picture);
    }
    let route = Route::Hosted(w.key_of(&chat));
    match w.request(route, now, |c| c.hosted(&chat, &message, &hosted, MAX_PICTURE)) {
        Ok(bytes) => {
            if let Some(copy) = w.copy.as_ref()
                && copy.put_hosted(&chat, &message, &hosted, &bytes).is_err()
            {
                log::warn!("teams: a picture couldn't be kept in the copy");
            }
            say(w, picture, bytes);
        }
        Err(e) => failed(w, picture, &e, now),
    }
    true
}

/// Someone's photo from Graph, unless Graph said less than a day ago that they have none; kept,
/// then said. Whether a request was made.
fn fetch_photo(w: &mut Worker, user: String, now: Instant) -> bool {
    let picture = Picture::Photo { user: user.clone() };
    let Some(copy) = w.copy.as_ref() else { return false };
    let (kept, missing) = copy.photo(&user);
    if let Some(bytes) = kept {
        say(w, picture, bytes);
        return false;
    }
    // A mark further than a day ahead isn't one this worker wrote: Graph's clock went back, or
    // the copy is damaged. Asked again.
    let today = graph_ms(w);
    if missing.is_some_and(|until| today < until && until <= today.saturating_add(DAY_MS)) {
        return false;
    }
    match w.request(Route::Photo, now, |c| c.photo(&user, MAX_PICTURE)) {
        Ok(Some(bytes)) => {
            if let Some(copy) = w.copy.as_ref()
                && copy.put_photo(&user, &bytes).is_err()
            {
                log::warn!("teams: a photo couldn't be kept in the copy");
            }
            say(w, picture, bytes);
        }
        Ok(None) => {
            let until = graph_ms(w).saturating_add(DAY_MS);
            if let Some(copy) = w.copy.as_ref()
                && copy.photo_missing(&user, until).is_err()
            {
                log::warn!("teams: that someone has no photo couldn't be kept in the copy");
            }
        }
        Err(e) => failed(w, picture, &e, now),
    }
    true
}

/// A picture's request failed. Busy (429, 503): nothing at all is asked of Teams while its
/// `Retry-After` runs, and the picture is the first fetched after. The network: "Offline", tried
/// again in a while. A refused token was dealt with where it was met. Anything else (refused, not
/// found, too big): not asked again in this run.
fn failed(w: &mut Worker, picture: Picture, e: &NetError, now: Instant) {
    log::warn!("teams: a picture failed: {}", kind(e));
    if busy(e) {
        let wait = w.client.as_ref().and_then(TeamsClient::retry_after);
        if let Some(until) = now.checked_add(retry_wait(wait)) {
            w.limiter.pause_all(until);
        }
        w.pictures.pending.push_front(picture);
    } else if of_the_network(e) {
        w.pause(e, now);
        w.pictures.pending.push_front(picture);
    } else if !matches!(e, NetError::Auth(_)) {
        refuse(w, picture);
    }
}

/// `picture` isn't to be had in this run. No request was made: `false`.
fn refuse(w: &mut Worker, picture: Picture) -> bool {
    if w.pictures.refused.len() >= MAX_REFUSED {
        w.pictures.refused.clear();
    }
    w.pictures.refused.insert(picture);
    false
}

/// A message as the copy holds it, looked for from the page where one of its chat was last found,
/// then from the newest pages back.
fn message_in_copy(w: &mut Worker, chat: &str, message: &str) -> Option<ChatMessage> {
    let hint = w.pictures.found.as_ref().filter(|(c, _)| c == chat).map(|(_, page)| *page);
    let (m, page) = w.copy.as_ref()?.message(chat, message, hint, LOOKUP_PAGES)?;
    w.pictures.found = Some((chat.to_string(), page));
    Some(m)
}

/// The first [`HOSTED_PER_MESSAGE`] hosted images a message shows, each once: those in its text in
/// their order, then those it carries.
fn first_hosted(m: &ChatMessage) -> Vec<&str> {
    let shown = m.body.blocks.iter().flat_map(|b| &b.runs).filter_map(|r| r.hosted.as_deref());
    let carried = m.attachments.iter().filter_map(|a| if let Attachment::Hosted { id } = a { Some(id.as_str()) } else { None });
    let mut ids: Vec<&str> = Vec::with_capacity(HOSTED_PER_MESSAGE);
    for id in shown.chain(carried) {
        if ids.len() >= HOSTED_PER_MESSAGE {
            break;
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// The time on Graph's clock (milliseconds since 1970), by the newest `Date` it sent.
fn graph_ms(w: &Worker) -> i64 {
    w.client.as_ref().and_then(TeamsClient::server_time).and_then(|s| s.checked_mul(1000)).unwrap_or_default()
}

/// An outside picture the user clicked: fetched when its address is `https` to a site on the
/// internet, said as it came (the window decodes it); never kept.
fn outside(w: &Worker, url: &str) {
    if !public_https(url) {
        return log::info!("teams: an outside picture refused: not an https address on the internet");
    }
    match fetch_outside(w, url) {
        Ok(r) if r.status == 200 => say(w, Picture::Outside { url: url.to_string() }, r.body),
        Ok(r) => log::info!("teams: an outside picture wasn't given (HTTP {})", r.status),
        Err(e) => log::info!("teams: an outside picture couldn't be fetched: {}", kind(&e)),
    }
}

/// Whether an outside picture's address may be fetched: `https`, plain, and not an address of this
/// computer or its network written as one (a name is judged by `http::get_public` on the addresses
/// it resolves to).
fn public_https(url: &str) -> bool {
    if url.len() > MAX_ADDRESS || url.chars().any(char::is_control) {
        return false;
    }
    let Ok(u) = Url::parse(url) else { return false };
    if !u.https {
        return false;
    }
    match u.host.parse::<IpAddr>() {
        Ok(ip) => is_public(ip),
        Err(_) => !u.host.is_empty(),
    }
}

/// `http::get_public`, or a test's stand-in for it.
fn fetch_outside(#[cfg_attr(not(test), allow(unused_variables))] w: &Worker, url: &str) -> crate::Result<http::Response> {
    #[cfg(test)]
    if let Some(fetch) = w.hooks.outside {
        return fetch(url, MAX_PICTURE);
    }
    http::get_public(url, MAX_PICTURE)
}
