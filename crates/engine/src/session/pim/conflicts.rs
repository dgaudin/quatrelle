// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What a sync drops because the server no longer has it (an appointment or a task changed
//! here, deleted there meanwhile), kept as Outlook keeps it in Sync Issues › Conflicts: a
//! mailbox on this computer only, made at the first one, never synced, with one message for
//! each, the version that was here attached as a calendar file to open to put it back.

use super::*;

/// The mailbox on this computer that keeps them (an account of its own, as a data file opened
/// here is), and its one folder.
pub const SYNC_ISSUES: &str = "sync-issues";
pub const CONFLICTS: &str = "Conflicts";

/// The words of a copy kept: its subject (`{item}`, `{account}`), its text (`{item}`,
/// `{account}`, `{place}`, `{when}`) and the place of a task (the Tasks folder).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictWords {
    pub subject: String,
    pub body: String,
    pub tasks: String,
}

impl Default for ConflictWords {
    fn default() -> Self {
        ConflictWords { subject: SUBJECT.into(), body: BODY.into(), tasks: "Tasks".into() }
    }
}

const SUBJECT: &str = "{item} — deleted on the server ({account})";
const BODY: &str = "{item} was deleted on the server while it was being changed here.\n\nAccount: {account}\nIn: {place}\nWhen: {when}\n\n\
The server's version was kept, so the change made here was dropped. The attachment holds the version that was here: open it to put it back.";

/// What went: its name, where it was (a calendar's name; empty for a task), and the version
/// that was here as a calendar file.
pub(super) struct Dropped {
    name: String,
    place: String,
    ics: String,
}

/// The appointments the server no longer has, changed here (`gone`, by their item there): the
/// server wins, so they go here too, a series with the occurrences changed here that never had
/// an item of their own. Each as it was, to keep a copy and say so.
pub(super) fn drop_gone_events(p: &mut Pim, gone: &[String], now: i64) -> Vec<Dropped> {
    if gone.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut series: HashSet<(String, String)> = HashSet::new();
    for href in gone.iter().filter(|h| !h.is_empty()) {
        let masters: Vec<(String, String)> = p
            .events
            .iter()
            .filter(|e| e.href == *href && e.event.recurrence_id.is_none())
            .map(|e| (e.calendar.clone(), e.event.uid.clone()))
            .collect();
        let going: Vec<&StoredEvent> = p
            .events
            .iter()
            .filter(|e| {
                e.href == *href
                    || (e.href.is_empty() && e.event.recurrence_id.is_some() && masters.contains(&(e.calendar.clone(), e.event.uid.clone())))
            })
            .collect();
        let Some(first) = going.iter().find(|e| e.event.recurrence_id.is_none()).or_else(|| going.first()) else { continue };
        let place = p.calendars.iter().find(|c| c.id == first.calendar).map_or_else(|| first.calendar.clone(), |c| c.name.clone());
        let events: Vec<CalEvent> = going.iter().map(|e| e.event.clone()).collect();
        out.push(Dropped { name: first.event.summary.clone(), place, ics: ical::write(&events, Some("PUBLISH"), now) });
        series.extend(masters);
    }
    p.events.retain(|e| {
        let unsent_occurrence = e.href.is_empty() && e.event.recurrence_id.is_some() && series.contains(&(e.calendar.clone(), e.event.uid.clone()));
        (e.href.is_empty() || !gone.contains(&e.href)) && !unsent_occurrence
    });
    out
}

/// The tasks the server no longer has, changed here: gone here too, each as it was.
pub(super) fn drop_gone_tasks(p: &mut Pim, gone: &[String], now: i64) -> Vec<Dropped> {
    if gone.is_empty() {
        return Vec::new();
    }
    let out = p
        .tasks
        .iter()
        .filter(|t| !t.href.is_empty() && gone.contains(&t.href))
        .map(|t| Dropped { name: t.task.subject.clone(), place: String::new(), ics: ical::write_task(&t.task, now) })
        .collect();
    p.tasks.retain(|t| t.href.is_empty() || !gone.contains(&t.href));
    out
}

/// Most characters of an item's name in its file's name.
const FILE_NAME_MAX: usize = 60;

/// A file name for an item's copy: its name, without what file systems refuse.
fn file_name(name: &str) -> String {
    let clean: String =
        name.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')).take(FILE_NAME_MAX).collect();
    let clean = clean.trim().trim_matches('.');
    format!("{}.ics", if clean.is_empty() { "item" } else { clean })
}

impl Session {
    /// One item dropped from `account`: a copy kept in Sync Issues › Conflicts, and said once.
    pub(super) fn keep_conflict(&mut self, account: &str, d: Dropped) {
        if self.file_conflict(account, &d) {
            self.notice_with(
                "{event}: deleted on the server meanwhile; the change made here was dropped (a copy is in Sync Issues › Conflicts)",
                &[("event", &d.name)],
                true,
            );
        } else {
            self.notice_with("{event}: deleted on the server meanwhile; the change made here was dropped", &[("event", &d.name)], true);
        }
    }

    /// The copy, as a message in Sync Issues › Conflicts (the mailbox made if it isn't there).
    /// Without a cache (nowhere to keep it), none.
    fn file_conflict(&mut self, account: &str, d: &Dropped) -> bool {
        if self.store.is_none() {
            return false;
        }
        let (who, email) = self
            .account(account)
            .map(|a| (if a.config.name.trim().is_empty() { a.config.email.clone() } else { a.config.name.clone() }, a.config.email.clone()))
            .unwrap_or_else(|| (account.to_string(), String::new()));
        self.ensure_sync_issues();
        let now = self.clock.now;
        let words = self.conflict_words.clone();
        let place = if d.place.is_empty() { words.tasks.as_str() } else { d.place.as_str() };
        let when = quatrelle_model::dates::full_time(now, &self.clock);
        let values = [("item", d.name.as_str()), ("account", who.as_str()), ("place", place), ("when", when.as_str())];
        let draft = mime::Draft {
            reply_to: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: crate::fill(&words.subject, &values),
            body: crate::fill(&words.body, &values),
            in_reply_to: String::new(),
            references: Vec::new(),
            attachments: vec![mime::OutAttachment {
                name: file_name(&d.name),
                mime: "text/calendar; method=PUBLISH".into(),
                data: d.ics.clone().into_bytes(),
                ..Default::default()
            }],
            html: None,
            inline: Vec::new(),
            importance: 0,
            read_receipt: false,
            voting: String::new(),
            seven_bit: false,
        };
        let seed = {
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            (account, &d.name, &d.ics).hash(&mut h);
            h.finish()
        };
        let id = mime::new_message_id(&email, now, seed);
        let raw = match mime::build(&draft, &quatrelle_model::EmailAddress::new("Quatrelle", email), now, &id) {
            Ok(raw) => raw,
            Err(e) => {
                log::warn!("{account}: the copy of an item dropped couldn't be written: {e}");
                return false;
            }
        };
        match self.file_locally(SYNC_ISSUES, CONFLICTS, &raw, true) {
            Ok(_) => true,
            Err(e) => {
                log::warn!("{account}: the copy of an item dropped couldn't be kept: {e}");
                false
            }
        }
    }

    /// Sync Issues and its Conflicts folder, made the first time (an account on this computer,
    /// as a data file opened here is: no server, never synced), the folder made again if the
    /// user deleted it.
    fn ensure_sync_issues(&mut self) {
        let folder = Folder { account: SYNC_ISSUES.into(), path: CONFLICTS.into(), selectable: true, ..Default::default() };
        if self.account(SYNC_ISSUES).is_none() {
            let config = AccountConfig {
                id: SYNC_ISSUES.into(),
                name: "Sync Issues".into(),
                // Never an address, so never "me" (as a data file's is its file's name).
                email: "Sync Issues".into(),
                color: u8::try_from(self.accounts.len() % usize::from(ACCOUNT_COLORS)).unwrap_or(0),
                ..Default::default()
            };
            self.accounts.push(AccountState {
                config,
                folders: Vec::new(),
                status: SyncStatus::Offline,
                quota: None,
                vacation: VacationState::Unknown,
            });
            self.save_accounts();
        }
        let Some(a) = self.account_mut(SYNC_ISSUES) else { return };
        if a.folders.iter().any(|f| f.path == CONFLICTS) {
            return;
        }
        a.folders.push(folder);
        let folders = a.folders.clone();
        // Written after the marks of its messages made before (which write its counts).
        self.finish_local_marks();
        if let Some(store) = &self.store
            && let Err(e) = store.save_folders(SYNC_ISSUES, &folders)
        {
            log::warn!("Sync Issues: its folder not saved: {e}");
        }
    }
}
