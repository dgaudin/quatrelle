// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! People in Teams, as the engine keeps them: the directory searched (the people found for the
//! last query asked, each checked again and remembered), a conversation created with its first
//! message (one at a time, online only), and the call the open chat's members make.
//!
//! What is hostile here: the directory's answer (a person that isn't one, the user, more than
//! [`PEOPLE_FOUND`]: left out; at most [`TEAMS_PEOPLE_KEPT`] remembered), the params of the
//! commands (checked before anything changes: only a person the directory gave can be added to a
//! conversation, once, at most [`NEW_CHAT_PEOPLE`]), and a member's address, which is never trusted
//! to be one participant: the call link refuses an address that isn't plain
//! ([`teams_call_link`]).

use std::collections::VecDeque;

use quatrelle_mime::{Typed, teams_compose_typed};
use quatrelle_model::ical::{TEAMS_CALL_PEOPLE, teams_call_link};
use quatrelle_model::pim::teams::{ChatKind, NEW_CHAT_PEOPLE, NotCreated, PEOPLE_FOUND, PEOPLE_QUERY_CHARS, Part, Person, person_ok};

use super::*;

/// The most people remembered from the directory's answers (those that can be added to a
/// conversation), the oldest out.
pub const TEAMS_PEOPLE_KEPT: usize = 500;
/// The refusal of `teams.create` while Teams can't be reached: a conversation is created online.
pub const TEAMS_NEEDS_NETWORK: &str = "Teams can't be reached: a conversation is created online";
/// The refusal of `teams.call` for a conversation that can't be called from here.
pub const TEAMS_NO_CALL: &str = "this conversation can't be called from here";
/// A conversation is being created.
const BEING_CREATED: &str = "a conversation is already being created";

/// What the directory found for the query last asked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TeamsPeople {
    pub query: String,
    pub people: Vec<Person>,
    /// The directory couldn't be asked.
    pub failed: bool,
    /// The answer is waited for.
    pub waiting: bool,
}

/// Where the creation of a conversation stands (none: no creation going nor failed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TeamsCreating {
    Waiting { local: String },
    Failed { why: NotCreated, code: Option<String> },
}

/// The people the directory gave, for a conversation to be made with: by id (whatever its
/// capitals), the newest last.
#[derive(Default)]
pub(super) struct Known {
    list: VecDeque<Person>,
}

impl Known {
    fn remember(&mut self, p: &Person) {
        self.list.retain(|k| !k.id.eq_ignore_ascii_case(&p.id));
        if self.list.len() >= TEAMS_PEOPLE_KEPT {
            self.list.pop_front();
        }
        self.list.push_back(p.clone());
    }

    fn get(&self, id: &str) -> Option<&Person> {
        self.list.iter().find(|k| k.id.eq_ignore_ascii_case(id))
    }
}

/// The first message of a conversation being created, kept until it exists.
pub(super) struct FirstMessage {
    local: String,
    typed: Typed,
    parts: Vec<Part>,
}

/// What `teams.create` gave back.
pub(crate) enum Created {
    /// A chat there already, shown, the message sent to it.
    Chat { chat: String, local: String },
    /// A creation asked for.
    Creating { local: String },
}

impl TeamsState {
    /// The addresses of the open chat's other members (`me`: the user's id), when it can be
    /// called: a one-to-one chat or a group, listed and not left, with 1 to
    /// [`TEAMS_CALL_PEOPLE`] others, each with an address. Whether the addresses make a link is
    /// [`teams_call_link`]'s to say.
    pub fn callable(&self, me: &str) -> Option<Vec<&str>> {
        let row = self.open.as_deref().and_then(|c| self.row(c))?;
        if row.left || !matches!(row.kind, ChatKind::OneOnOne | ChatKind::Group) {
            return None;
        }
        let others: Vec<&str> = self.members.iter().filter(|m| !m.user_id.eq_ignore_ascii_case(me)).map(|m| m.address.trim()).collect();
        (!others.is_empty() && others.len() <= TEAMS_CALL_PEOPLE && others.iter().all(|a| !a.is_empty())).then_some(others)
    }

    /// The directory's answer for `query`, when it's the query last asked.
    fn take_people(&mut self, query: &str, people: Vec<Person>, failed: bool, me: &str) {
        let Some(t) = self.people.as_mut().filter(|t| t.waiting && t.query == query) else { return };
        let people: Vec<Person> = people.into_iter().filter(|p| person_ok(p) && !p.id.eq_ignore_ascii_case(me)).take(PEOPLE_FOUND).collect();
        for p in &people {
            self.known.remember(p);
        }
        *t = TeamsPeople { query: t.query.clone(), people, failed, waiting: false };
    }

    /// The listed one-to-one chat with `person`, the user still in it and Teams letting it be read.
    fn chat_with(&self, person: &str) -> Option<&str> {
        let row = self.rows.iter().find(|r| {
            r.kind == ChatKind::OneOnOne && !r.left && !r.unreadable && r.person.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(person))
        });
        row.map(|r| r.id.as_str())
    }
}

/// A topic as a group's name: on one line, at most [`NAME_CHARS`] characters, none when empty.
fn topic_of(topic: Option<&str>) -> Option<String> {
    topic.map(one_line).filter(|t| !t.is_empty())
}

impl Session {
    /// The directory asked for `query` (trimmed, at most [`PEOPLE_QUERY_CHARS`] characters): an
    /// empty one forgets the people. Whether it was asked: not again while the same query waits.
    pub fn teams_people(&mut self, query: &str) -> Result<bool, CmdError> {
        let query = query.trim();
        if query.chars().count() > PEOPLE_QUERY_CHARS {
            return Err(CmdError::Params(format!("a search is at most {PEOPLE_QUERY_CHARS} characters")));
        }
        if query.is_empty() {
            self.teams.people = None;
        } else if self.teams.people.as_ref().is_some_and(|t| t.waiting && t.query == query) {
            return Ok(false);
        } else {
            self.teams.people = Some(TeamsPeople { query: query.to_string(), waiting: true, ..TeamsPeople::default() });
        }
        self.push_job(Job::Teams { job: teams::Job::Directory { query: query.to_string() } });
        Ok(true)
    }

    /// What the first message of a conversation to be made would be: what it takes to be sent
    /// ([`Session::teams_send`]'s checks, but no mention: the members aren't known yet).
    fn first_message_ok(&self, typed: &Typed, parts: &[Part]) -> Result<(), CmdError> {
        self.teams.mentions_ok("", typed, &self.prefs.teams.tenant_id, &[])?;
        files::parts_ok(parts)?;
        let (body, mentions) = teams_compose_typed(typed);
        if body.blocks.is_empty() && parts.is_empty() {
            return Err(CmdError::Params("there's nothing to send".into()));
        }
        if !files::fits(&body, &mentions, parts) {
            return Err(CmdError::Params(TEAMS_TOO_LONG.into()));
        }
        Ok(())
    }

    /// A conversation with `people` (ids the directory gave, 1 to [`NEW_CHAT_PEOPLE`], once each,
    /// not the user), named `topic` from two people on, begun with the message `typed` and `parts`.
    /// A one-to-one chat there already is shown and the message sent to it. Else one creation at
    /// a time, and only while Teams can be reached: the message is kept, and sent once the chat
    /// exists ([`teams::Event::Created`]). A creation is never asked twice on its own.
    pub(crate) fn teams_create(&mut self, people: &[String], topic: Option<&str>, typed: &Typed, parts: Vec<Part>) -> Result<Created, CmdError> {
        if people.is_empty() || people.len() > NEW_CHAT_PEOPLE {
            return Err(CmdError::Params(format!("a conversation is with 1 to {NEW_CHAT_PEOPLE} people")));
        }
        let me = self.prefs.teams.object_id.clone();
        let mut chosen: Vec<Person> = Vec::with_capacity(people.len());
        for id in people {
            let again = chosen.iter().any(|p| p.id.eq_ignore_ascii_case(id));
            if again || (!me.is_empty() && id.eq_ignore_ascii_case(&me)) {
                return Err(CmdError::Params("each person once, and not yourself".into()));
            }
            let known = self.teams.known.get(id).ok_or_else(|| CmdError::Params("a person the directory didn't give".into()))?;
            chosen.push(known.clone());
        }
        self.first_message_ok(typed, &parts)?;
        if matches!(self.teams.creating, Some(TeamsCreating::Waiting { .. })) {
            return Err(CmdError::Disabled(BEING_CREATED.into()));
        }
        if let [only] = &chosen[..]
            && let Some(chat) = self.teams.chat_with(&only.id).map(str::to_string)
        {
            // Queued first: refused, nothing changes and the pane keeps what was typed.
            let local = self.teams_send(&chat, typed, parts)?;
            self.teams_show(Some(chat.clone()));
            self.teams.created = Some(chat.clone());
            return Ok(Created::Chat { chat, local });
        }
        if self.teams.status == Status::Offline {
            return Err(CmdError::Disabled(TEAMS_NEEDS_NETWORK.into()));
        }
        let topic = topic_of(topic).filter(|_| chosen.len() >= 2);
        let n = self.fresh_id();
        let local = format!("{}-{n}", self.clock.now);
        self.teams.creating = Some(TeamsCreating::Waiting { local: local.clone() });
        self.teams.first = Some(FirstMessage { local: local.clone(), typed: typed.clone(), parts });
        self.push_job(Job::Teams { job: teams::Job::Create { local: local.clone(), people: chosen, topic } });
        Ok(Created::Creating { local })
    }

    /// The chat the last creation ended in (or the one a creation found already there), once: the
    /// window leaves the pane for it.
    pub fn take_teams_created(&mut self) -> Option<String> {
        self.teams.created.take()
    }

    /// [`teams::Event::Created`]: for the creation waited for only, the chat listed is shown and
    /// the first message sent to it; a chat never listed (its row is said before) is a refusal.
    pub(super) fn teams_created(&mut self, local: &str, chat: String) {
        if !matches!(&self.teams.creating, Some(TeamsCreating::Waiting { local: l }) if l == local) {
            return;
        }
        let first = self.teams.first.take().filter(|f| f.local == local);
        if !self.teams.has_chat(&chat) {
            self.teams.creating = Some(TeamsCreating::Failed { why: NotCreated::Refused, code: None });
            return;
        }
        // The first message is queued before the chat is shown: refused (the outbox full, too long),
        // the creation ends as failed, the pane stays with what was typed, and the chat, there in
        // Teams, shows in the list; sending again opens it (a one-to-one chat) instead of making
        // another.
        if let Some(FirstMessage { typed, parts, .. }) = first
            && self.teams_send(&chat, &typed, parts).is_err()
        {
            self.teams.creating = Some(TeamsCreating::Failed { why: NotCreated::Refused, code: None });
            self.notice_with("Your message to the new conversation wasn't sent", &[], true);
            return;
        }
        self.teams.creating = None;
        self.teams_show(Some(chat.clone()));
        self.teams.created = Some(chat);
    }

    /// [`teams::Event::NotCreated`]: for the creation waited for only; what was typed stays in the
    /// window.
    pub(super) fn teams_not_created(&mut self, local: &str, why: NotCreated, code: Option<String>) {
        if !matches!(&self.teams.creating, Some(TeamsCreating::Waiting { local: l }) if l == local) {
            return;
        }
        self.teams.first = None;
        let code = code.filter(|c| c.len() <= MAX_ID).map(|c| one_line(&c));
        self.teams.creating = Some(TeamsCreating::Failed { why, code });
    }

    /// [`teams::Event::People`]. The user's id is left out of what is found.
    pub(super) fn teams_found_people(&mut self, query: &str, people: Vec<Person>, failed: bool) {
        let me = self.prefs.teams.object_id.clone();
        self.teams.take_people(query, people, failed, &me);
    }

    /// The open chat called: Teams' call link, with video or not, handed to the window as a page.
    /// Refused when the chat can't be called from here, or its members' addresses don't make a
    /// link.
    pub fn teams_call(&mut self, video: bool) -> Result<String, CmdError> {
        let me = self.prefs.teams.object_id.clone();
        let link = self.teams.callable(&me).and_then(|a| teams_call_link(&a, video)).ok_or_else(|| CmdError::Disabled(TEAMS_NO_CALL.into()))?;
        self.teams.open_page(TeamsPage::Call(link.clone()));
        Ok(link)
    }
}
