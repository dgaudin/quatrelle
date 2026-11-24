// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Quatrelle engine: the session and its commands.
//!
//! **Everything is a command.** Each user-visible action is a [`CommandSpec`] with a stable id
//! (`mail.reply`, `folder.open`), a label, its place in the UI, a default shortcut, a params
//! description and a `run` function. The toolbar, menus, shortcuts, the CLI and agents all call
//! [`Session::run`].
//!
//! **Sans-I/O.** The session reads the local cache ([`Store`]) but never touches the network: it
//! queues [`Job`]s ([`Session::take_jobs`]) and applies the [`Event`]s that come back
//! ([`Session::apply`]). The desktop app connects the two to `quatrelle-net`'s worker pool; tests
//! script the events directly.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod cmd;
mod notice;
pub mod sample;
mod session;

use std::collections::HashMap;

use serde_json::Value;

pub use notice::{Notice, fill};
pub use quatrelle_mime as mime;
pub use quatrelle_model as model;
pub use quatrelle_pgp as pgp;
pub use quatrelle_smime as smime;
pub use quatrelle_store as store;
pub use session::{
    ACCOUNTS_UNREAD, ARCHIVE_READ_ONLY, ARRANGE_HEADINGS, AccountPgp, AccountState, Arrange, AssistKind, Assistant, BodyWork, CardSearch,
    CompactPhase, Compacted, Compose, CtrlEnter, DirectoryAnswer, Expansion, FIELD_NAMES, FieldId, Filed, Filing, FolderRef, FolderSizes,
    HELD_SOURCE_MAX, HeldSource, INDEX_MAX, ImportState, IndexWork, ItemChoice, ItemKind, KnownCert, Learn, Learnt, ListFilter, ListLayout, Lock,
    MAX_SHOWN_MEMBERS, MAX_SUGGESTIONS, MAX_TOTAL, MEETINGS_PROFILE, MailTipsState, MarkWork, MessagePolicy, MoveTarget, NAMES_NOT_FOUND,
    NO_KEY_TO_ENCRYPT, NOT_DOWNLOADED, NameChoice, NotesOrder, Outcome, OwnKey, PANICS_CAUGHT, PEOPLE_DOC, PEOPLE_MAX, PGP_DOC, PaneSection,
    PeopleKeys, PersonKey, PgpCard, Picture, PictureLoading, Prefs, PreparedBody, PreviewWork, PreviewsRead, Quote, QuotePoint, READING_ZOOM,
    RECENT_EMOJI, Reach, ReadInPane, Reading, ReadingPane, RecoverBlock, Recovered, Recovering, Resolved, RevocationCheck, Rewrite, RewriteProblem,
    Row, RulesScope, SMIME_DOC, SOURCE_SHOWN, STILL_COMING, STILL_DOWNLOADING, ScamMiss, ScamTest, ScamToday, Scheme, Seal, SearchClock, SearchScope,
    Security, SendBlock, SendCheck, Sender, SenderAddresses, SenderMarks, ServerSearch, Session, SignedBy, Sizes, SmimeSettings, Source, SourceView,
    Suggestion, UnfinishedImport, UpdateCheck, VacationState, View, WEBEX_ROOM_LINE_MAX, WEBEX_ROOM_LINES_MAX, WEBEX_ROOM_NAME_MAX, WEBEX_ROOMS_MAX,
    WebexRoom, WindowScope, Written, directory_people, fill_name, group_entry,
    invite::{Invite, InviteProblem},
    is_profile_dir,
    junk::{JunkList, junk_entry},
    mail_tips_recipients, newer_release, parse_key,
    pim::{
        ACCOUNT_COLOR, AgendaItem, CONFLICTS, CalMode, CalendarRefusal, ChatMembers, ConflictWords, DueReminder, EVENT_HITS_MAX, EventHit, EventWhen,
        MAX_FILE_CARDS, MAX_FILE_EVENTS, Module, NoteRef, PALETTE_COLOR, PALETTE_COLORS, PICTURE_BYTES_MAX, PICTURES_MAX, PRESENCE_MAX, Person,
        RemovalRefusal, SYNC_ISSUES, ShownReaction, TEAMS_ATTACH_AGAIN, TEAMS_FILES_KEPT, TEAMS_HITS_MAX, TEAMS_LEFT, TEAMS_MAY_BE_IN_TEAMS,
        TEAMS_MEETING_ASKS_MAX, TEAMS_NEEDS_NETWORK, TEAMS_NO_CALL, TEAMS_NOTHING_LEFT, TEAMS_PEOPLE_KEPT, TEAMS_RANGES_MAX, TEAMS_TOO_LONG,
        TEAMS_TOO_MANY, TEAMS_TOO_MANY_DOWNLOADS, TEAMS_TOO_MANY_FILES, TaskChange, TaskRef, TeamsAnnouncement, TeamsAuto, TeamsAutoAsk,
        TeamsCreating, TeamsFileState, TeamsFound, TeamsMeetingAsk, TeamsMeetingPath, TeamsMeetingsState, TeamsOpen, TeamsPage, TeamsPeople,
        TeamsSaved, TeamsState, WebexAsk, WebexState, parse_local, teams_could_run, teams_open,
    },
    quick_steps::{QuickStep, StepWrite},
    scheduling::{Fetched, RoomFinder},
    search_folders::SearchFolder,
};

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum CmdError {
    #[error("unknown command `{0}`")]
    Unknown(String),
    #[error("bad parameters: {0}")]
    Params(String),
    #[error("not available: {0}")]
    Disabled(String),
    #[error("{0}")]
    Failed(String),
}

impl From<quatrelle_store::StoreError> for CmdError {
    fn from(e: quatrelle_store::StoreError) -> Self {
        CmdError::Failed(e.to_string())
    }
}

pub type CmdResult = Result<Value, CmdError>;

/// A registered command.
#[derive(Clone)]
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Where it lives in the UI: `Home › Respond`, `View › Layout`…
    pub location: &'static str,
    /// Default shortcut in a portable form (`Mod+R`, `Mod+Shift+R`, `F9`). `Mod` = Ctrl, or ⌘ on macOS.
    pub shortcut: &'static str,
    /// Parameter description for agents (`{"folder": string}`).
    pub params: &'static str,
    /// `None` = enabled; `Some(reason)` = disabled.
    pub enabled: fn(&Session) -> Option<&'static str>,
    pub run: fn(&mut Session, &Value) -> CmdResult,
}

fn always(_: &Session) -> Option<&'static str> {
    None
}

impl CommandSpec {
    pub const fn new(id: &'static str, label: &'static str, location: &'static str, run: fn(&mut Session, &Value) -> CmdResult) -> Self {
        CommandSpec { id, label, location, shortcut: "", params: "{}", enabled: always, run }
    }
    pub const fn key(mut self, s: &'static str) -> Self {
        self.shortcut = s;
        self
    }
    pub const fn params(mut self, p: &'static str) -> Self {
        self.params = p;
        self
    }
    pub const fn when(mut self, f: fn(&Session) -> Option<&'static str>) -> Self {
        self.enabled = f;
        self
    }
}

/// All commands by id.
pub struct Registry {
    specs: Vec<CommandSpec>,
    by_id: HashMap<&'static str, usize>,
}

impl Registry {
    pub fn new(specs: Vec<CommandSpec>) -> Self {
        let by_id = specs.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
        Registry { specs, by_id }
    }
    pub fn get(&self, id: &str) -> Option<&CommandSpec> {
        self.by_id.get(id).and_then(|i| self.specs.get(*i))
    }
    pub fn all(&self) -> &[CommandSpec] {
        &self.specs
    }
    /// The command list as JSON (for agents and `quatrelle-cli commands`).
    pub fn describe(&self) -> Value {
        Value::Array(
            self.specs
                .iter()
                .map(|s| serde_json::json!({"id": s.id, "label": s.label, "location": s.location, "shortcut": s.shortcut, "params": s.params}))
                .collect(),
        )
    }
    /// The command bound to a shortcut, if any.
    pub fn by_shortcut(&self, key: &str) -> Option<&CommandSpec> {
        self.specs.iter().find(|s| !s.shortcut.is_empty() && s.shortcut.split(" / ").any(|k| k.eq_ignore_ascii_case(key)))
    }
}

/// Parameter helpers.
pub mod p {
    use serde_json::Value;

    use crate::CmdError;

    pub fn str<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
        v.get(k).and_then(Value::as_str)
    }
    pub fn req_str<'a>(v: &'a Value, k: &str) -> Result<&'a str, CmdError> {
        str(v, k).ok_or_else(|| CmdError::Params(format!("`{k}` (string) is required")))
    }
    pub fn u64(v: &Value, k: &str) -> Option<u64> {
        v.get(k).and_then(|x| x.as_u64().or_else(|| x.as_f64().filter(|f| *f >= 0.0 && f.is_finite()).map(|f| f as u64)))
    }
    pub fn bool(v: &Value, k: &str) -> Option<bool> {
        v.get(k).and_then(Value::as_bool)
    }
    pub fn i64(v: &Value, k: &str) -> Option<i64> {
        v.get(k).and_then(|x| x.as_i64().or_else(|| x.as_f64().filter(|f| f.is_finite() && f.abs() < 9.0e15).map(|f| f as i64)))
    }
    /// A file's bytes given as `k` (a list of numbers, 0 to 255), at most `max` of them: `None`
    /// when there's no list, "too big" as for a file named by its path (`read_capped`), and read
    /// where it is (each caller decoded its own way: one without the cap, two after a copy).
    pub fn bytes(v: &Value, k: &str, max: usize) -> Result<Option<Vec<u8>>, CmdError> {
        let Some(list) = v.get(k).and_then(Value::as_array) else { return Ok(None) };
        if list.len() > max {
            return Err(CmdError::Params(format!("`{k}` is too big")));
        }
        let not_bytes = || CmdError::Params(format!("`{k}` must be bytes"));
        list.iter().map(|b| b.as_u64().and_then(|x| u8::try_from(x).ok()).ok_or_else(not_bytes)).collect::<Result<_, _>>().map(Some)
    }
}

#[cfg(test)]
mod bench;
#[cfg(test)]
mod tests;
