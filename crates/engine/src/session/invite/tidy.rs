// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A meeting's mail out of the Inbox once it's dealt with (`Prefs::tidy_answered_invites`, on
//! from the start, as Outlook has it): the message the answer was given in, and the meeting's
//! earlier invitations in the Inbox, go to Deleted Items beside them. An update newer than the
//! version dealt with stays, to be seen and answered. Ordinary moves: Undo puts them back, and
//! other clients see them too.
//!
//! Which messages carry a meeting is only known from their sources, and only those already
//! downloaded are looked at: nothing is fetched for this. The summaries filter first (the
//! meeting's title in the subject, or its organizer as the sender), so that answering doesn't
//! read through a big Inbox.

use std::cmp::Reverse;

use super::*;

/// The Inbox messages looked at for a meeting's mail, the newest first.
const MAX_CANDIDATES: usize = 200;
/// Most of their sources read in one go.
const MAX_READ: usize = 16 << 20;
/// A message bigger than this isn't looked through for a calendar.
const MAX_MESSAGE: usize = 8 << 20;
/// A shorter title (trimmed) names no subject: "ab" is in too many of them.
const MIN_TITLE: usize = 3;

impl Session {
    /// Move a meeting's mail to Deleted Items: the message it was dealt with in (`acted_on`),
    /// unless it's in Deleted Items, Sent Items, Drafts or the Outbox, and the account's Inbox
    /// messages carrying the same meeting (`uid`, by [`same_meeting`]) with one of `methods`,
    /// naming only occurrences in `only` (when given), in no newer version than `upto`, the
    /// version dealt with (its parts). The message read among them: the list goes on to the
    /// one after it that stays. How many moved (none with the switch off).
    pub(super) fn tidy_meeting_mail(
        &mut self,
        account: &str,
        uid: &str,
        methods: &[&str],
        only: Option<&[i64]>,
        upto: &[CalEvent],
        acted_on: Option<&MessageKey>,
    ) -> usize {
        if !self.prefs.tidy_answered_invites {
            return 0;
        }
        let mut going: Vec<Arc<Summary>> = Vec::new();
        if let Some(k) = acted_on
            && self.tidied_from(k)
            && let Some(m) = self.summary_of(k)
        {
            going.push(m);
        }
        let inbox = self.account(account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone());
        if let Some(inbox) = inbox
            && !uid.is_empty()
        {
            let mut read = 0usize;
            for m in self.meeting_candidates(account, &inbox, upto) {
                let size = usize::try_from(m.size).unwrap_or(usize::MAX);
                if going.iter().any(|g| g.key == m.key) || size > MAX_MESSAGE {
                    continue;
                }
                // The budget is checked before reading, by the size the summary gives (used up,
                // a summary saying 0 isn't read either).
                if read >= MAX_READ || read.saturating_add(size) > MAX_READ {
                    break;
                }
                // Never fetched: one not downloaded stays where it is.
                let Some(raw) = self.store.as_ref().and_then(|s| s.read_body(&m.key)) else { continue };
                // The summary's size may be 0 or wrong: the larger of the two counts.
                read = read.saturating_add(size.max(raw.len()));
                if raw.len() <= MAX_MESSAGE && mentions_calendar(&raw) && self.carries_meeting(&raw, uid, methods, only, upto) {
                    going.push(m);
                }
            }
        }
        if going.is_empty() {
            return 0;
        }
        let gone: Vec<MessageKey> = going.iter().map(|m| m.key.clone()).collect();
        let read = self.reading.as_ref().map(|r| r.key.clone()).filter(|k| gone.contains(k));
        let next = read.as_ref().and_then(|k| self.successor_of(k, &gone));
        match self.move_messages(&going, &MoveTarget::Role(FolderRole::Trash)) {
            Ok(moved) => {
                if read.is_some() {
                    self.after_removal(next);
                }
                moved
            }
            Err(e) => {
                log::info!("{account}: the meeting's mail stays where it is: {e}");
                0
            }
        }
    }

    /// The version of a meeting answered in the calendar (`uid`, as the event form names it):
    /// the copy [`Session::find_event`] takes, read after the answer (on Exchange the answer
    /// may only be on its way: what's stored is the version known, and a copy made here the
    /// server answer dropped gives way to the server's), with its moved occurrences in the same
    /// calendar under the same UID. Not the copies in colleagues' shared calendars, which can
    /// be ahead or behind the user's, nor Exchange's other appointments of a series
    /// (`UID#start`). None in `account`: nothing.
    pub(super) fn answered_copy(&self, account: &str, uid: &str) -> Vec<CalEvent> {
        let Some((found, copy)) = self.find_event(uid) else { return Vec::new() };
        if found != account {
            return Vec::new();
        }
        let (calendar, uid) = (&copy.calendar, &copy.event.uid);
        let Some(p) = self.pim.get(account) else { return Vec::new() };
        p.events.iter().filter(|e| &e.calendar == calendar && &e.event.uid == uid).map(|e| e.event.clone()).collect()
    }

    /// Whether a message the user dealt with a meeting in may go: not from Deleted Items (it's
    /// there already), Sent Items, Drafts or the Outbox, nor from a folder not known here.
    fn tidied_from(&self, key: &MessageKey) -> bool {
        if key.folder == OUTBOX {
            return false;
        }
        let kept = |r: FolderRole| matches!(r, FolderRole::Trash | FolderRole::Sent | FolderRole::Drafts);
        self.folder(&key.account, &key.folder).is_some_and(|f| !kept(f.role) && !kept(f.mailbox_role))
    }

    /// A message's summary: from the list shown, else from its folder.
    fn summary_of(&mut self, key: &MessageKey) -> Option<Arc<Summary>> {
        if let Some(m) = self.index_of(key).and_then(|i| self.list.get(i)) {
            return Some(m.clone());
        }
        self.folder_state(&key.account, &key.folder).messages.iter().find(|m| &m.key == key).cloned()
    }

    /// The Inbox messages that may carry the meeting, the newest first, at most
    /// [`MAX_CANDIDATES`]: those whose subject holds its title (case aside, one of
    /// [`MIN_TITLE`] characters or more) or whose sender is its organizer. The title and
    /// organizer are the series' in `upto`, else its first part's.
    fn meeting_candidates(&mut self, account: &str, inbox: &str, upto: &[CalEvent]) -> Vec<Arc<Summary>> {
        let main = upto.iter().find(|e| e.recurrence_id.is_none()).or_else(|| upto.first());
        let title = main.map(|e| e.summary.trim().to_lowercase()).filter(|t| t.chars().count() >= MIN_TITLE);
        let organizer = main.and_then(|e| e.organizer.as_ref()).map(|o| o.email.clone()).filter(|o| !without_mailto(o).is_empty());
        if title.is_none() && organizer.is_none() {
            return Vec::new();
        }
        let mut found: Vec<Arc<Summary>> = self
            .folder_state(account, inbox)
            .messages
            .iter()
            .filter(|m| {
                title.as_deref().is_some_and(|t| contains_folded(&m.subject, t))
                    || organizer.as_deref().is_some_and(|o| same_address(o, &m.from.email))
            })
            .cloned()
            .collect();
        found.sort_unstable_by_key(|m| Reverse((m.date, m.key.uid)));
        found.truncate(MAX_CANDIDATES);
        found
    }

    /// Whether a message's source carries the meeting with one of these methods, naming only
    /// the occurrences in `only` (when given), in no newer version than `upto`.
    fn carries_meeting(&self, raw: &[u8], uid: &str, methods: &[&str], only: Option<&[i64]>, upto: &[CalEvent]) -> bool {
        let Some((cal, _)) = self.calendar_in(raw) else { return false };
        if !cal.method.as_deref().is_some_and(|m| methods.iter().any(|x| m.trim().eq_ignore_ascii_case(x))) {
            return false;
        }
        let parts: Vec<&CalEvent> = cal.events.iter().filter(|e| same_meeting(&e.uid, uid)).collect();
        !parts.is_empty()
            && only.is_none_or(|only| parts.iter().all(|e| e.recurrence_id.is_some_and(|r| only.contains(&r))))
            && !parts.iter().any(|e| newer_than(e, upto))
    }
}

/// The occurrences a meeting's parts name when they don't name the series (an answer or
/// cancellation of some occurrences only); `None` when the series is among them.
pub(super) fn occurrences_only(parts: &[CalEvent]) -> Option<Vec<i64>> {
    (!parts.iter().any(|e| e.recurrence_id.is_none())).then(|| parts.iter().filter_map(|e| e.recurrence_id).collect())
}

/// Whether a message's part is a newer version than the one dealt with: compared with the part
/// of `upto` for the same occurrence, else with its series, by `SEQUENCE` only (a `DTSTAMP` is
/// the time a copy was written, and with the organizer's clock ahead the calendar's copy looked
/// older than its own invitation). Nothing to compare with: not newer.
fn newer_than(part: &CalEvent, upto: &[CalEvent]) -> bool {
    let same = upto.iter().find(|u| u.recurrence_id == part.recurrence_id);
    same.or_else(|| upto.iter().find(|u| u.recurrence_id.is_none())).is_some_and(|that| part.sequence > that.sequence)
}

/// Whether a source names a calendar at all (`text/calendar` or `.ics`, ASCII case aside): most
/// mail doesn't, and isn't parsed.
fn mentions_calendar(raw: &[u8]) -> bool {
    contains_ascii_folded(raw, b"text/calendar") || contains_ascii_folded(raw, b".ics")
}

/// Whether `hay` holds `needle`, ASCII case aside.
fn contains_ascii_folded(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

/// Whether `text` holds `lower` (already in lower case), case aside: without a copy of `text`
/// when `lower` is plain ASCII.
fn contains_folded(text: &str, lower: &str) -> bool {
    if lower.is_ascii() { contains_ascii_folded(text.as_bytes(), lower.as_bytes()) } else { text.to_lowercase().contains(lower) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(recurrence_id: Option<i64>, sequence: u32) -> CalEvent {
        CalEvent { recurrence_id, sequence, ..Default::default() }
    }

    #[test]
    fn versions_compare_by_sequence_with_the_same_part_else_the_series() {
        let upto = [part(None, 2), part(Some(100), 4)];
        assert!(!newer_than(&part(None, 2), &upto), "the same version");
        assert!(newer_than(&part(None, 3), &upto), "a later series");
        assert!(!newer_than(&part(Some(100), 3), &upto), "an occurrence against its own");
        assert!(newer_than(&part(Some(200), 3), &upto), "another occurrence against the series");
        assert!(!newer_than(&part(Some(200), 9), &[part(Some(100), 1)]), "nothing to compare with");
        assert!(!newer_than(&part(None, 1), &[]));
    }

    #[test]
    fn titles_and_markers_are_found_case_aside() {
        assert!(contains_folded("Updated: MIX Review", "mix review"));
        assert!(contains_folded("Réunion ÉTÉ", "réunion été"));
        assert!(!contains_folded("Mix", "mix review"));
        assert!(mentions_calendar(b"Content-Type: TEXT/Calendar; method=REQUEST"));
        assert!(mentions_calendar(b"name=\"invite.ICS\""));
        assert!(!mentions_calendar(b"plain text"));
    }
}
