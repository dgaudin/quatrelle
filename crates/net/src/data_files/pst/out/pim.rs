// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Contacts, contact groups, calendar items, tasks, notes and journal entries as they come in.
//! A repeating appointment's changed occurrences come in as events of their own, each replacing
//! its occurrence (`RECURRENCE-ID`).

use super::super::item::{EXCEPTION, File};
use super::*;

impl Out<'_> {
    /// A contact, contact group, calendar item, task, note or journal entry (anything else in
    /// their folders is left out).
    pub(in super::super) fn add_pim(&mut self, mut it: Item) {
        let Some(x) = it.extra.take() else { return };
        let x = &x;
        let n = self.pim.events.len() + self.pim.cards.len() + self.pim.notes.len() + self.pim.tasks.len();
        let uid = format!("pst-{}-{n}@quatrelle.invalid", self.account);
        let class = it.class.to_ascii_lowercase();
        if class.starts_with("ipm.activity") {
            // Quatrelle has no journal: an entry is a note, saying when it was.
            let text = crate::pst_pim::journal_text(x, &it.subject, &it.body);
            if text.is_empty() {
                self.skipped += 1;
                return;
            }
            let note = Note { uid, text, color: NOTE_YELLOW, modified: x.modified.unwrap_or(it.date) };
            self.pim.notes.push(StoredNote { note, ..Default::default() });
        } else if class.starts_with("ipm.distlist") {
            let card = crate::pst_pim::group_of(x, &it.subject, &it.body, uid);
            self.pim.cards.push(StoredCard { book: LOCAL.into(), card, ..Default::default() });
        } else if class.starts_with("ipm.stickynote") {
            // A note's text is its body (its subject is the first line of it).
            let text = if it.body.trim().is_empty() { it.subject.clone() } else { it.body.replace("\r\n", "\n") };
            if text.trim().is_empty() {
                self.skipped += 1;
                return;
            }
            let note =
                Note { uid, text: text.trim_end().to_string(), color: x.note_color.unwrap_or(NOTE_YELLOW), modified: x.modified.unwrap_or(it.date) };
            self.pim.notes.push(StoredNote { note, ..Default::default() });
        } else if class.starts_with("ipm.task") {
            // A task, done or not; its due day stored as local midnight, the nearest UTC midnight.
            let due = x.due.map(|t| (t + 43_200).div_euclid(86_400));
            let mut task = Task {
                uid,
                subject: it.subject.clone(),
                notes: it.body.trim().to_string(),
                due,
                modified: x.modified.unwrap_or(it.date),
                ..Default::default()
            };
            task.set_complete(x.done, due.unwrap_or(it.date.div_euclid(86_400)));
            self.pim.tasks.push(StoredTask { task, ..Default::default() });
        } else if class.starts_with("ipm.contact") {
            let card = crate::pst_pim::card_of(x, &it.body, uid);
            self.pim.cards.push(StoredCard { book: LOCAL.into(), card, ..Default::default() });
        } else {
            let people: Vec<EmailAddress> = it.to.iter().chain(&it.cc).cloned().collect();
            let Some(event) = crate::pst_pim::event_of(x, &it.subject, &it.body, &it.from, &people, uid.clone()) else {
                self.skipped += 1;
                return;
            };
            // Its changed occurrences are attached to it as items of their own.
            let own: Vec<&Item> = it
                .files
                .iter()
                .filter_map(|f| match f {
                    File::Message(m) if m.class.eq_ignore_ascii_case(EXCEPTION) => Some(&**m),
                    _ => None,
                })
                .collect();
            let guests: Vec<Vec<EmailAddress>> = own.iter().map(|o| o.to.iter().chain(&o.cc).cloned().collect()).collect();
            let none = crate::pst_pim::Extra::default();
            let items: Vec<crate::pst_pim::Own<'_>> = own
                .iter()
                .zip(&guests)
                .map(|(o, guests)| crate::pst_pim::Own { x: o.extra.as_ref().unwrap_or(&none), body: &o.body, people: guests })
                .collect();
            let changed = crate::pst_pim::changed_events(x, &it.subject, &it.body, &it.from, &people, &uid, &items);
            self.pim.events.push(StoredEvent { calendar: LOCAL.into(), event, ..Default::default() });
            for (event, _) in changed {
                self.pim.events.push(StoredEvent { calendar: LOCAL.into(), event, ..Default::default() });
            }
        }
    }
}
