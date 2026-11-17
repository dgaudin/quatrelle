// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Teams worker's people: the directory searched for a new conversation, and the conversation
//! created, or the one already there found. Both are online actions the engine asks for with a
//! job and hears of with an event; neither is ever made on its own.
//!
//! * One search is kept: the last thing typed (an earlier one is forgotten, an empty one
//!   forgets all). It is asked a second after the last one, and answered with the people found,
//!   never the user, at most [`PEOPLE_FOUND`] and each already cleaned by the client. Nothing
//!   to ask (signed out, working offline) is answered at once: failed, no one.
//! * One creation is kept, waiting or under way. The people are looked for in the copy first
//!   (Review Focus 5): a one-to-one chat with the person, or a group (never a meeting's) whose
//!   members are exactly the people and the user, not left, the most recently active, is the
//!   answer without a request. Else `POST /chats` goes once; then the new chat's members are
//!   read, the chat is kept in the copy with them, its row is said, and then it is created for
//!   the engine. A chat of the copy that Graph gives back (a one-to-one already there that was
//!   left) keeps its record and is no longer left.
//! * `POST /chats` isn't idempotent for a group: a creation is never made twice on its own. One
//!   that got no answer is said "offline" and left; the user sends again.
//!
//! What is hostile here: the directory's answers (read and bounded by the client), the engine's
//! people (ids that aren't object ids, too many, the user, a person twice: dropped with a warning,
//! no request), and Graph's answer to the creation. What it protects: the user's conversations
//! (no second chat, no group made twice) and Graph's limits. The log says counts and keyed
//! hashes: never a name, an address, what was typed or a topic.

use std::collections::BTreeSet;
use std::time::Instant;

use quatrelle_model::pim::teams::{Chat, ChatKind, Event, MAX_MEMBERS, NEW_CHAT_PEOPLE, NotCreated, PEOPLE_FOUND, Person, person_ok};

use super::files::code_of;
use super::graph::{chat_of, member_of, search_term};
use super::limits::Route;
use super::store::{ChatRecord, local_ok};
use super::sync::{dirty, flush};
use super::worker::{Worker, busy, kind, of_the_network};
use crate::NetError;

/// What the worker keeps of the people: the last search asked and not answered, and the creation
/// waiting or under way (one of each).
#[derive(Default)]
pub(crate) struct People {
    search: Option<String>,
    create: Option<Creation>,
}

/// A creation: what the engine asked for, and the chat once Graph has made it (its members not
/// read yet).
struct Creation {
    local: String,
    people: Vec<Person>,
    topic: Option<String>,
    made: Option<Chat>,
}

/// Whether Teams may be asked: signed in, going, and not working offline.
fn online(w: &Worker) -> bool {
    w.polling() && !w.offline() && w.client.is_some()
}

/// [`Job::Directory`]: `query` is the last thing typed; the search waits for [`step`]. An empty
/// one forgets the one waiting.
///
/// [`Job::Directory`]: quatrelle_model::pim::teams::Job::Directory
pub(crate) fn directory(w: &mut Worker, query: String) {
    if query.is_empty() {
        w.people.search = None;
    } else if online(w) {
        w.people.search = Some(query);
    } else {
        w.people.search = None;
        w.say(Event::People { query, people: Vec::new(), failed: true });
    }
}

/// [`Job::Create`]: the conversation `local` with `people` (a topic for a group). People who
/// aren't people are dropped with a warning, no event. Not online: said so. One waiting already:
/// said too many. The copy's chat with the same people is said at once, else the creation waits
/// for [`step`].
///
/// [`Job::Create`]: quatrelle_model::pim::teams::Job::Create
pub(crate) fn create(w: &mut Worker, local: String, people: Vec<Person>, topic: Option<String>) {
    if let Some(why) = unusable(w, &local, &people) {
        log::warn!("teams: a new conversation was dropped: {why}");
        return;
    }
    if !online(w) {
        return not_created(w, local, NotCreated::Offline, None);
    }
    if w.people.create.is_some() {
        return not_created(w, local, NotCreated::TooMany, None);
    }
    if let Some(chat) = found(w, &people) {
        log::info!("teams: a conversation with those people was found in the copy: {}", w.hashed(&chat));
        return w.say(Event::Created { local, chat });
    }
    w.people.create = Some(Creation { local, people, topic, made: None });
}

/// Why people can't be created a conversation with, if they can't: 1 to [`NEW_CHAT_PEOPLE`], each
/// one a person once and never the user, and a local id as the outbox's.
fn unusable(w: &Worker, local: &str, people: &[Person]) -> Option<&'static str> {
    let me = w.settings.object_id.to_ascii_lowercase();
    let mut ids = BTreeSet::new();
    if !local_ok(local) {
        Some("not a local id")
    } else if people.is_empty() || people.len() > NEW_CHAT_PEOPLE {
        Some("not 1 to 20 people")
    } else if !people.iter().all(person_ok) {
        Some("someone who isn't a person")
    } else if people.iter().any(|p| p.id.eq_ignore_ascii_case(&me)) {
        Some("the user among the people")
    } else if !people.iter().all(|p| ids.insert(p.id.to_ascii_lowercase())) {
        Some("someone twice")
    } else {
        None
    }
}

/// The chat of the copy these people are the other members of, if there's one: a one-to-one chat
/// for one person, a group (not a meeting's, not left) for several, whose members are exactly
/// them and the user; the most recently active.
fn found(w: &Worker, people: &[Person]) -> Option<String> {
    let copy = w.copy.as_ref()?;
    let mut wanted: BTreeSet<String> = people.iter().map(|p| p.id.to_ascii_lowercase()).collect();
    wanted.insert(w.settings.object_id.to_ascii_lowercase());
    let kind = if people.len() == 1 { ChatKind::OneOnOne } else { ChatKind::Group };
    copy.chats()
        .into_iter()
        .map(|r| r.chat)
        .filter(|c| !c.left && c.kind == kind && c.members.iter().map(|m| m.user_id.to_ascii_lowercase()).collect::<BTreeSet<_>>() == wanted)
        .max_by(|a, b| (a.last_activity, &a.id).cmp(&(b.last_activity, &b.id)))
        .map(|c| c.id)
}

/// Whether a search or a creation waits for a request.
pub(crate) fn waiting(w: &Worker) -> bool {
    w.people.search.is_some() || w.people.create.is_some()
}

/// Nothing more is asked of Teams: what waits is given up and said, so that the window doesn't
/// wait for it (a creation already made, its members not read, is found by the list's next pass).
pub(crate) fn stop(w: &mut Worker) {
    let People { search, create } = std::mem::take(&mut w.people);
    if let Some(query) = search {
        w.say(Event::People { query, people: Vec::new(), failed: true });
    }
    if let Some(c) = create {
        not_created(w, c.local, NotCreated::Offline, None);
    }
}

/// One step, a request at most: the creation, else the search. Whether a request went.
pub(crate) fn step(w: &mut Worker, now: Instant) -> bool {
    match w.people.create.as_ref().map(|c| c.made.is_some()) {
        Some(false) => make(w, now),
        Some(true) => members(w, now),
        None => search(w, now),
    }
}

fn not_created(w: &Worker, local: String, why: NotCreated, code: Option<String>) {
    w.say(Event::NotCreated { local, why, code });
}

/// The creation says it was not made, and is forgotten: never made again on its own.
fn give_up(w: &mut Worker, why: NotCreated, code: Option<String>) {
    if let Some(c) = w.people.create.take() {
        not_created(w, c.local, why, code);
    }
}

/// `POST /chats`, once.
fn make(w: &mut Worker, now: Instant) -> bool {
    let Some((me, people, topic)) = w.people.create.as_ref().map(|c| (w.settings.object_id.clone(), c.people.clone(), c.topic.clone())) else {
        return false;
    };
    if !w.may(Route::Create, now) {
        return false;
    }
    let made = w.request(Route::Create, now, |c| c.create_chat(&me, &people, topic.as_deref()));
    match made {
        Ok(value) => match chat_of(&value) {
            Some(chat) => {
                if let Some(c) = w.people.create.as_mut() {
                    c.made = Some(chat);
                }
            }
            None => {
                log::warn!("teams: a conversation was created but Graph's answer couldn't be read");
                give_up(w, NotCreated::Refused, None);
            }
        },
        Err(e) => {
            log::warn!("teams: a conversation wasn't created: {}", kind(&e));
            let (why, code) = failure(w, &e, now);
            give_up(w, why, code);
        }
    }
    true
}

/// How a creation that failed is said. Graph busy: waited for, and refused all the same (never
/// made again on its own). No answer (the network, a connection closed) or the sign-in refused:
/// offline. Anything else, a server's error too, is Graph's answer: refused, with its code.
fn failure(w: &mut Worker, e: &NetError, now: Instant) -> (NotCreated, Option<String>) {
    if busy(e) {
        w.busy_wait(now);
        return (NotCreated::Refused, code_of(e));
    }
    if of_the_network(e) {
        w.pause(e, now);
        return (NotCreated::Offline, None);
    }
    if matches!(e, NetError::Auth(_)) {
        return (NotCreated::Offline, None);
    }
    (NotCreated::Refused, code_of(e))
}

/// The made chat's members, read; then it is kept in the copy, its row said, and it is created.
/// Members that can't be read leave the chat kept without them (the list's next pass reads them).
/// Graph busy: asked again later.
fn members(w: &mut Worker, now: Instant) -> bool {
    let Some(chat) = w.people.create.as_ref().and_then(|c| c.made.as_ref()).map(|c| c.id.clone()) else { return false };
    if !w.may(Route::Members, now) {
        return false;
    }
    let read = w.request(Route::Members, now, |c| c.members_all(&chat));
    // A sign-in refused twice has stopped the worker, and what waited has been said.
    let Some(creation) = w.people.create.take() else { return true };
    let Some(made) = creation.made else { return true };
    let read = match read {
        Ok(values) => Some(values.iter().filter_map(member_of).take(MAX_MEMBERS).collect::<Vec<_>>()),
        Err(e) if busy(&e) => {
            w.busy_wait(now);
            w.people.create = Some(Creation { made: Some(made), ..creation });
            return true;
        }
        Err(e) => {
            log::warn!("teams: the members of a conversation created couldn't be read: {}", kind(&e));
            None
        }
    };
    keep(w, creation.local, made, read);
    true
}

/// The chat in the copy (its record kept when it's there already, and no longer left), its row
/// said, then [`Event::Created`].
fn keep(w: &mut Worker, local: String, made: Chat, members: Option<Vec<quatrelle_model::pim::teams::Member>>) {
    let id = made.id.clone();
    let Some(copy) = &w.copy else { return not_created(w, local, NotCreated::Offline, None) };
    let mut record = match copy.chat(&id) {
        Some(mut r) => {
            r.chat.left = false;
            r
        }
        None => ChatRecord::new(Chat { updated: None, ..made.clone() }),
    };
    if let Some(members) = members {
        record.chat.members = members;
        record.chat.updated = made.updated;
    }
    if let Err(e) = copy.put_chat(&record) {
        log::warn!("teams: a conversation created couldn't be kept: {}", kind(&NetError::Store(e)));
        return not_created(w, local, NotCreated::Refused, None);
    }
    log::info!("teams: a conversation created: {}", w.hashed(&id));
    dirty(w, &id);
    flush(w);
    w.say(Event::Created { local, chat: id });
}

/// The directory searched for the last thing typed, once.
fn search(w: &mut Worker, now: Instant) -> bool {
    let Some(query) = w.people.search.clone() else { return false };
    if search_term(&query).is_empty() {
        w.people.search = None;
        w.say(Event::People { query, people: Vec::new(), failed: false });
        return false;
    }
    if !w.may(Route::Directory, now) {
        return false;
    }
    // A newer search typed while this one runs replaces it: it is answered when it is asked.
    w.people.search = None;
    let asked = query.clone();
    match w.request(Route::Directory, now, |c| c.users_search(&asked)) {
        Ok(found) => {
            let me = w.settings.object_id.clone();
            let people: Vec<Person> = found.into_iter().filter(|p| !p.id.eq_ignore_ascii_case(&me)).take(PEOPLE_FOUND).collect();
            log::info!("teams: a directory search found {} people", people.len());
            w.say(Event::People { query, people, failed: false });
        }
        Err(e) => {
            log::warn!("teams: a directory search failed: {}", kind(&e));
            if busy(&e) {
                w.busy_wait(now);
            } else if of_the_network(&e) {
                w.pause(&e, now);
            }
            w.say(Event::People { query, people: Vec::new(), failed: true });
        }
    }
    true
}
