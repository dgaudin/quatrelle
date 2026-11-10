// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Contacts, contact groups, calendar items, tasks, notes and journal entries as they come in.
//! A repeating appointment's changed occurrences come in as events of their own, each replacing
//! its occurrence (`RECURRENCE-ID`); an exception item that is none of them is counted. The files
//! of a contact or calendar item come in on its card or event (`pst_pim::files`); a task, a note
//! and a journal entry have nowhere to keep one, and theirs are counted. The account's contacts
//! and calendar are one document, written whole at each change, and written as one `.ics` and one
//! `.vcf` that Quatrelle must read back (in full up to 16 MiB each), so the files kept there are
//! held to [`MAX_FILE`] each and [`MAX_FILES`] in all, and those past either are counted as too
//! big.

use super::super::item::{EXCEPTION, File};
use super::*;

/// A file of a contact or calendar item kept at most, as its line holds it (base64)…
const MAX_FILE: usize = 1 << 20;
/// …and all of them in the account's contacts and calendar: half what a calendar or address book
/// file is read to, leaving the rest to their text and the folding of their lines.
const MAX_FILES: usize = 8 << 20;

impl Out<'_> {
    /// A contact, contact group, calendar item, task, note or journal entry (anything else in
    /// their folders is left out).
    pub(in super::super) fn add_pim(&mut self, it: Item) {
        let (before, events) = (self.pim_len(), self.pim.events.len());
        self.pim_item(it);
        // Something came in, under the number worked out for it: the next takes a higher one.
        if self.pim_len() > before {
            self.pim_changed = true;
            self.pim_floor = before.max(self.pim_floor).saturating_add(1);
            self.event_again(events);
        }
    }

    /// The contacts, calendar items, notes and tasks held.
    fn pim_len(&self) -> usize {
        self.pim.events.len() + self.pim.cards.len() + self.pim.notes.len() + self.pim.tasks.len()
    }

    fn pim_item(&mut self, mut it: Item) {
        let Some(x) = it.extra.take() else { return };
        let x = &x;
        // Its files left out as they were read count, whatever becomes of it.
        for (reason, n) in std::mem::take(&mut it.left_out) {
            self.lose(reason, n);
        }
        let files = std::mem::take(&mut it.files);
        // The number of those held, as ever; an import gone on with never takes one again that
        // an earlier run gave out.
        let n = self.pim_len().max(self.pim_floor);
        let uid = format!("pst-{}-{n}@quatrelle.invalid", self.account);
        let class = it.class.to_ascii_lowercase();
        if class.starts_with("ipm.activity") {
            // Quatrelle has no journal: an entry is a note, saying when it was.
            let text = crate::pst_pim::journal_text(x, &it.subject, &it.body);
            if text.is_empty() {
                self.skipped += 1;
                return;
            }
            let note = Note { uid, text, color: NOTE_YELLOW, modified: x.modified.unwrap_or(it.date), ..Default::default() };
            self.pim.notes.push(StoredNote { note, ..Default::default() });
            self.no_place(&files);
        } else if class.starts_with("ipm.distlist") {
            let mut card = crate::pst_pim::group_of(x, &it.subject, &it.body, uid.clone());
            card.extra.extend(self.card_files(files, &uid));
            self.pim.cards.push(StoredCard { book: LOCAL.into(), card, ..Default::default() });
        } else if class.starts_with("ipm.stickynote") {
            // A note's text is its body (its subject is the first line of it).
            let text = if it.body.trim().is_empty() { it.subject.clone() } else { it.body.replace("\r\n", "\n") };
            if text.trim().is_empty() {
                self.skipped += 1;
                return;
            }
            let note = Note {
                uid,
                text: text.trim_end().to_string(),
                color: x.note_color.unwrap_or(NOTE_YELLOW),
                modified: x.modified.unwrap_or(it.date),
                ..Default::default()
            };
            self.pim.notes.push(StoredNote { note, ..Default::default() });
            self.no_place(&files);
        } else if class.starts_with("ipm.task") {
            let task = crate::pst_pim::task_of(x, &it.subject, &it.body, it.importance, it.date, uid);
            self.pim.tasks.push(StoredTask { task, ..Default::default() });
            self.no_place(&files);
        } else if class.starts_with("ipm.contact") {
            let mut card = crate::pst_pim::card_of(x, &it.body, uid.clone());
            card.extra.extend(self.card_files(files, &uid));
            self.pim.cards.push(StoredCard { book: LOCAL.into(), card, ..Default::default() });
        } else {
            let people: Vec<EmailAddress> = it.to.iter().chain(&it.cc).cloned().collect();
            let Some(mut event) = crate::pst_pim::event_of(x, &it.subject, &it.body, &it.from, &people, uid.clone()) else {
                self.skipped += 1;
                return;
            };
            // Its changed occurrences are attached to it as items of their own; its files besides.
            let (own, files): (Vec<File>, Vec<File>) =
                files.into_iter().partition(|f| matches!(f, File::Message(m) if m.class.eq_ignore_ascii_case(EXCEPTION)));
            let own: Vec<Item> = own.into_iter().filter_map(|f| if let File::Message(m) = f { Some(*m) } else { None }).collect();
            let lines = self.event_files(files, &uid);
            with_files(&mut event, lines);
            let changed = {
                let guests: Vec<Vec<EmailAddress>> = own.iter().map(|o| o.to.iter().chain(&o.cc).cloned().collect()).collect();
                let none = crate::pst_pim::Extra::default();
                let items: Vec<crate::pst_pim::Own<'_>> = own
                    .iter()
                    .zip(&guests)
                    .map(|(o, guests)| crate::pst_pim::Own { x: o.extra.as_ref().unwrap_or(&none), body: &o.body, people: guests })
                    .collect();
                crate::pst_pim::changed_events(x, &it.subject, &it.body, &it.from, &people, &uid, &items)
            };
            self.pim.events.push(StoredEvent { calendar: LOCAL.into(), event, ..Default::default() });
            // Each with the files of its own item.
            let mut own: Vec<Option<Item>> = own.into_iter().map(Some).collect();
            for (mut e, found) in changed {
                if let Some(mut item) = found.and_then(|i| own.get_mut(i)).and_then(Option::take) {
                    for (reason, n) in std::mem::take(&mut item.left_out) {
                        self.lose(reason, n);
                    }
                    let lines = self.event_files(item.files, &uid);
                    with_files(&mut e, lines);
                }
                self.pim.events.push(StoredEvent { calendar: LOCAL.into(), event: e, ..Default::default() });
            }
            // One that isn't any occurrence (no time of its own, or one taken already) is
            // counted, its files with it.
            let unused = own.iter().flatten().count();
            if unused > 0 {
                log::info!("{unused} changed occurrences of a repeating appointment could not be placed");
                self.skipped += unused;
            }
        }
    }

    /// `n` files left out for `reason`.
    pub(super) fn lose(&mut self, reason: FileLeftOut, n: usize) {
        let c = self.files_left_out.entry(reason).or_default();
        *c = c.saturating_add(n);
    }

    /// The files of an item that has nowhere to keep them, counted.
    fn no_place(&mut self, files: &[File]) {
        if !files.is_empty() {
            log::info!("{} files of a task, note or journal entry were left out: it has nowhere to keep them", files.len());
            self.lose(FileLeftOut::NoPlace, files.len());
        }
    }

    /// An event's files as its lines, those that fit.
    fn event_files(&mut self, files: Vec<File>, uid: &str) -> Vec<String> {
        let files: Vec<_> = files.into_iter().enumerate().filter_map(|(i, f)| self.file_of(f, &format!("{i}.{uid}"))).collect();
        files.into_iter().filter_map(|(name, mime, data)| self.keep_file(&data, || crate::pst_pim::event_file(&name, &mime, &data))).collect()
    }

    /// A card's files as its lines, those that fit (its picture is its own).
    fn card_files(&mut self, files: Vec<File>, uid: &str) -> Vec<String> {
        let files: Vec<_> = files.into_iter().enumerate().filter_map(|(i, f)| self.file_of(f, &format!("{i}.{uid}"))).collect();
        files.into_iter().filter_map(|(name, mime, data)| self.keep_file(&data, || crate::pst_pim::card_file(&name, &mime, &data))).collect()
    }

    /// A file's name, type and bytes; an attached message's are its `.eml` (made with
    /// `fallback_id` when it has no id of its own).
    fn file_of(&mut self, f: File, fallback_id: &str) -> Option<(String, String, Vec<u8>)> {
        match f {
            File::Data { name, mime, data, .. } => Some((name, mime, data)),
            File::Message(inner) => {
                let subject = if inner.subject.trim().is_empty() { "message".to_string() } else { inner.subject.trim().to_string() };
                match super::super::rebuild::to_raw(*inner, fallback_id) {
                    Ok(raw) => Some((format!("{subject}.eml"), "message/rfc822".to_string(), raw)),
                    Err(e) => {
                        log::info!("a message attached to a contact or calendar item could not be rebuilt: {e}");
                        self.lose(FileLeftOut::MessageUnreadable, 1);
                        None
                    }
                }
            }
        }
    }

    /// The line holding a file of bytes `data` (made by `line`, once it's known to fit in what the
    /// contacts and calendar keep of files); else it's counted as too big.
    fn keep_file(&mut self, data: &[u8], line: impl FnOnce() -> String) -> Option<String> {
        let kept = self.pim_files;
        let fits = |n: usize| n <= MAX_FILE && kept.saturating_add(n) <= MAX_FILES;
        // Its bytes in base64, before they're written out.
        let line = fits(data.len().div_ceil(3).saturating_mul(4)).then(line).filter(|l| fits(l.len()));
        match &line {
            Some(l) => self.pim_files = kept.saturating_add(l.len()),
            None => {
                log::info!("a file of {} bytes on a contact or calendar item was left out", data.len());
                self.lose(FileLeftOut::TooBig, 1);
            }
        }
        line
    }
}

/// `lines` (an event's files) into the event, before any part of it (an alarm).
fn with_files(e: &mut quatrelle_model::ical::CalEvent, lines: Vec<String>) {
    let at = e.extra.iter().position(|l| l.trim_start().get(..6).is_some_and(|b| b.eq_ignore_ascii_case("BEGIN:"))).unwrap_or(e.extra.len());
    e.extra.splice(at..at, lines);
}
