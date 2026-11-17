// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Files in Teams, as the engine keeps them: the files and pictures a message sent takes, checked
//! here and shown at once by name (the worker reads them when their turn comes, sends them up and
//! says how far they went); and a file someone shared in the open chat, asked of the worker once
//! (`teams.openFile`) and handed to the window once saved ([`Session::take_teams_saved`]), to be
//! opened with the system's app, or shown in its folder when it could run code.
//!
//! What is hostile here: the parts' paths (from the window or a script): at most [`PARTS_MAX`],
//! each absolute, at most [`PATH_MAX`] bytes, without a control character. The worker's word on
//! a file asked: taken only for a file of the open chat asked and not answered yet; a path saved
//! only inside the user's own folder of files handed to other apps (`scratch_dir("open")`), not
//! deeper, under a name that is its own `sanitize_filename`; Teams' code only in the client's
//! shape. A name that could run code is never opened ([`teams_could_run`]). What is kept is
//! bounded: the files' states ([`TEAMS_FILES_KEPT`], the oldest answered let go first), the files
//! waiting for the window, the parts of the messages not answered for yet ([`OUTBOX_MAX`]).

use std::path::Path;

use quatrelle_mime::{runs_code, sanitize_filename, teams_write_parts};
use quatrelle_model::pim::teams::{Attachment, ChatText, FileOutcome, FileWhy, Mention, OUTBOX_MAX, OutPart, PARTS_MAX, PATH_MAX, Part};

use super::*;

/// The refusal of `teams.send` with more parts than a message takes ([`PARTS_MAX`]).
pub const TEAMS_TOO_MANY_FILES: &str = "at most 10 files and pictures go with a message";
/// The most files of the open chat whose state is kept, and the most saved files waiting for the
/// window.
pub const TEAMS_FILES_KEPT: usize = 64;
/// The refusal of `teams.retry` for a message the outbox refused with its files: it kept nothing
/// of them, and the engine only their names.
pub(super) const ATTACH_AGAIN: &str = "attach the files again";
/// The longest code of Teams' answer kept, in characters.
const CODE_MAX: usize = 64;
/// The longest extension a saved name keeps (the worker's).
const EXTENSION_MAX: usize = 16;
/// A file's place in a message is written with its GUID: one of the same length stands for it.
const GUID_PLACE: &str = "00000000-0000-0000-0000-000000000000";
/// Extensions of files that run code when opened, beyond those of mail's attachments
/// ([`runs_code`]): web pages and SVGs (their scripts), and Office documents with macros.
const ALSO_RUNS: [&str; 17] =
    ["html", "htm", "xhtml", "shtml", "mht", "mhtml", "svg", "svgz", "docm", "dotm", "xlsm", "xltm", "xlsb", "pptm", "potm", "ppsm", "sldm"];

/// Where a file of the open chat asked for stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TeamsFileState {
    /// Asked of the worker, not answered yet.
    Asked,
    /// Saved at `path`, inside the user's own folder of files handed to other apps.
    Saved { path: String },
    /// Not saved; `code` is Teams' answer when it gave one (`HTTP 403 accessDenied`).
    Failed { why: FileWhy, code: Option<String> },
}

/// A saved file for the window: opened with the system's app when `open`, else shown in its
/// folder (its name could run code: [`teams_could_run`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamsSaved {
    pub path: String,
    pub name: String,
    pub open: bool,
}

/// The files of Teams the engine keeps: the open chat's asked for, the saved ones waiting for the
/// window, and the parts of the messages sent not answered for yet.
#[derive(Default)]
pub(super) struct Files {
    /// (message, attachment, state), oldest first, at most [`TEAMS_FILES_KEPT`]: they go with
    /// their chat when another opens.
    states: VecDeque<(String, String, TeamsFileState)>,
    /// Oldest first, each once, at most [`TEAMS_FILES_KEPT`].
    saved: VecDeque<TeamsSaved>,
    /// By local id, oldest first, at most [`OUTBOX_MAX`]: the outbox's refusal of a message
    /// carries nothing of its parts, which are put back from here.
    parts: VecDeque<(String, Vec<OutPart>)>,
}

impl Files {
    /// Another chat opened: the files asked go with the one left (their answers aren't taken).
    pub(super) fn chat_left(&mut self) {
        self.states.clear();
    }

    fn at(&self, message: &str, attachment: &str) -> Option<usize> {
        self.states.iter().position(|(m, a, _)| m == message && a == attachment)
    }

    /// `message`'s file `attachment` asked, the newest kept; past [`TEAMS_FILES_KEPT`] the oldest
    /// not being downloaded goes, and none going, it isn't asked.
    fn ask(&mut self, message: &str, attachment: &str) -> Result<(), CmdError> {
        if let Some(i) = self.at(message, attachment) {
            self.states.remove(i);
        } else if self.states.len() >= TEAMS_FILES_KEPT {
            let oldest = self.states.iter().position(|(.., s)| *s != TeamsFileState::Asked);
            let oldest = oldest.ok_or_else(|| CmdError::Disabled("too many files are being downloaded".into()))?;
            self.states.remove(oldest);
        }
        self.states.push_back((message.to_string(), attachment.to_string(), TeamsFileState::Asked));
        Ok(())
    }

    /// A saved file for the window, once while it waits; past [`TEAMS_FILES_KEPT`] the oldest goes.
    fn hand(&mut self, saved: TeamsSaved) {
        if self.saved.iter().any(|s| s.path == saved.path) {
            return;
        }
        self.saved.push_back(saved);
        while self.saved.len() > TEAMS_FILES_KEPT {
            self.saved.pop_front();
        }
    }

    /// The parts of the message `local` sent, until the outbox answers for it.
    pub(super) fn sent_with(&mut self, local: &str, parts: Vec<OutPart>) {
        if parts.is_empty() {
            return;
        }
        if self.parts.len() >= OUTBOX_MAX {
            self.parts.pop_front();
        }
        self.parts.push_back((local.to_string(), parts));
    }

    /// The parts of the message `local`, now answered for: let go.
    pub(super) fn answered(&mut self, local: &str) -> Option<Vec<OutPart>> {
        let i = self.parts.iter().position(|(l, _)| l == local)?;
        self.parts.remove(i).map(|(_, parts)| parts)
    }
}

/// Whether opening a file saved under `name` could run code, so that it's shown in its folder
/// instead: what [`runs_code`] says of mail's attachments, a web page, an SVG, a document with
/// macros, and a name without an extension (the system's opener would go by what the file holds).
pub fn teams_could_run(name: &str) -> bool {
    if runs_code(name) {
        return true;
    }
    let clean = sanitize_filename(name);
    let Some((stem, ext)) = clean.rsplit_once('.') else { return true };
    let extension = !stem.trim().is_empty() && (1..=EXTENSION_MAX).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric());
    !extension || ALSO_RUNS.iter().any(|e| ext.eq_ignore_ascii_case(e))
}

/// Whether the parts a message takes can be: at most [`PARTS_MAX`], each by a path that is
/// absolute, at most [`PATH_MAX`] bytes, without a control character.
pub(super) fn parts_ok(parts: &[Part]) -> Result<(), CmdError> {
    if parts.len() > PARTS_MAX {
        return Err(CmdError::Params(TEAMS_TOO_MANY_FILES.into()));
    }
    let usable = |path: &str| !path.is_empty() && path.len() <= PATH_MAX && !path.chars().any(char::is_control) && Path::new(path).is_absolute();
    if parts.iter().all(|(Part::File { path } | Part::Picture { path })| usable(path)) {
        Ok(())
    } else {
        Err(CmdError::Params(format!("a file or picture is given by its full path, at most {PATH_MAX} bytes, without a control character")))
    }
}

/// Whether a message's text and the places of its `parts` fit what Teams takes once written
/// ([`teams::SEND_MAX`]), each file's place as long as its GUID makes it.
pub(super) fn fits(body: &ChatText, mentions: &[Mention], parts: &[Part]) -> bool {
    let pictures = parts.iter().filter(|p| matches!(p, Part::Picture { .. })).count();
    let files = vec![GUID_PLACE; parts.len().saturating_sub(pictures)];
    teams_write_parts(body, mentions, pictures, &files).is_ok()
}

/// A part as the conversation shows it before the worker has it: by the last component of its
/// path, on one line; nothing gone up.
pub(super) fn shown_part(part: &Part) -> OutPart {
    let (Part::File { path } | Part::Picture { path }) = part;
    let name = Path::new(path).file_name().map(|n| one_line(&n.to_string_lossy())).unwrap_or_default();
    OutPart { name, size: 0, picture: matches!(part, Part::Picture { .. }), sent: 0 }
}

/// Whether `code` is Teams' answer as the client says it: `HTTP`, a status of three digits, and
/// the error's code (letters, digits, `_`, `.` and `-`) when there was one; at most
/// [`CODE_MAX`] characters.
fn code_ok(code: &str) -> bool {
    let Some(rest) = code.strip_prefix("HTTP ") else { return false };
    let (status, word) = rest.split_once(' ').map_or((rest, None), |(s, w)| (s, Some(w)));
    code.len() <= CODE_MAX
        && status.len() == 3
        && status.bytes().all(|b| b.is_ascii_digit())
        && word.is_none_or(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-')))
}

/// The worker's word on where a file was saved, taken when it's a file of the user's own folder of
/// files handed to other apps (not deeper, not elsewhere) under a name that is its own
/// `sanitize_filename`: its path, made again from that folder and the name.
fn in_open_folder(path: &str) -> Option<String> {
    let dir = crate::store::scratch_dir("open")?;
    let given = Path::new(path);
    let name = given.file_name()?.to_str()?;
    if given.parent() != Some(dir.as_path()) || sanitize_filename(name) != name {
        return None;
    }
    dir.join(name).into_os_string().into_string().ok()
}

/// A saved file at `path` for the window, by its name.
fn handed(path: &str) -> TeamsSaved {
    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    TeamsSaved { open: !teams_could_run(&name), path: path.to_string(), name }
}

impl TeamsState {
    /// The open chat's file `attachment` of its message `message`: where it stands, once asked.
    pub fn file_state(&self, message: &str, attachment: &str) -> Option<&TeamsFileState> {
        self.files.at(message, attachment).and_then(|i| self.files.states.get(i)).map(|(.., s)| s)
    }
}

impl Session {
    /// The open chat's file `attachment` of its message `message` opened: asked of the worker
    /// ([`teams::Job::File`]) unless it's being downloaded already, or handed to the window again
    /// when it was saved (never downloaded twice). Only a file with an address of a message in the
    /// window, not deleted, of a chat Teams lets be read here (a chat left is read only: its files
    /// still open). Whether it was asked.
    pub fn teams_open_file(&mut self, chat: &str, message: &str, attachment: &str) -> Result<bool, CmdError> {
        let t = &self.teams;
        if t.open.as_deref() != Some(chat) {
            return Err(CmdError::Disabled("that conversation isn't open".into()));
        }
        if t.row(chat).is_none_or(|r| r.unreadable) {
            return Err(CmdError::Disabled("Teams doesn't let this conversation be read here".into()));
        }
        let m = t.window.iter().find(|m| m.id == message && m.deleted.is_none());
        let m = m.ok_or_else(|| CmdError::Disabled("that message isn't in the open conversation".into()))?;
        if !m.attachments.iter().any(|a| matches!(a, Attachment::File { id, url, .. } if id == attachment && !url.is_empty())) {
            return Err(CmdError::Disabled("that isn't a file of the message".into()));
        }
        let created = m.created;
        match t.file_state(message, attachment) {
            Some(TeamsFileState::Asked) => return Ok(false),
            Some(TeamsFileState::Saved { path }) => {
                let saved = handed(path);
                self.teams.files.hand(saved);
                return Ok(false);
            }
            Some(TeamsFileState::Failed { .. }) | None => {}
        }
        self.teams.files.ask(message, attachment)?;
        let (chat, message, attachment) = (chat.to_string(), message.to_string(), attachment.to_string());
        self.push_job(Job::Teams { job: teams::Job::File { chat, message, created, attachment } });
        Ok(true)
    }

    /// The next saved file for the window to open (or show in its folder), once each, oldest
    /// first.
    pub fn take_teams_saved(&mut self) -> Option<TeamsSaved> {
        self.teams.files.saved.pop_front()
    }

    /// [`teams::Event::File`]: how a file asked ended, taken only for one of the open chat asked
    /// and not answered yet. Not saved: kept to say so (Teams' code only in the client's shape).
    /// Saved: taken only inside the folder of files handed to other apps, under a name of its own
    /// (else refused), marked as come from elsewhere, and handed to the window.
    pub(super) fn teams_file(&mut self, chat: &str, message: &str, attachment: &str, outcome: FileOutcome) {
        if self.teams.open.as_deref() != Some(chat) || self.teams.file_state(message, attachment) != Some(&TeamsFileState::Asked) {
            return;
        }
        let state = match outcome {
            FileOutcome::Failed { why, code } => TeamsFileState::Failed { why, code: code.filter(|c| code_ok(c)) },
            FileOutcome::Saved { path } => match in_open_folder(&path) {
                Some(path) => {
                    crate::cmd::mark_from_internet(Path::new(&path));
                    self.teams.files.hand(handed(&path));
                    TeamsFileState::Saved { path }
                }
                None => {
                    log::warn!("teams: a file said saved outside the folder of files opened was passed over");
                    TeamsFileState::Failed { why: FileWhy::Refused, code: None }
                }
            },
        };
        if let Some(i) = self.teams.files.at(message, attachment)
            && let Some((.., s)) = self.teams.files.states.get_mut(i)
        {
            *s = state;
        }
    }
}
