// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Exchange's calendars, contacts, tasks and notes read as hostile XML: an appointment (its
//! times, pattern, guests, categories and reminder), an occurrence of a series changed on its own,
//! a contact card, a note and a task, and the navigation shortcuts Outlook keeps for the calendars
//! in groups of its Calendar pane ([MS-OXOCFG] 2.2.9; their binary address book entry ids are
//! length-checked before they are read). Guests, categories, deleted occurrences and shortcuts
//! stay within their caps; nothing is read outside the bytes it says it has.
#![no_main]

use libfuzzer_sys::fuzz_target;
use quatrelle_net::ews;
use quatrelle_net::{card_of, event_of, exception_of, note_of, own_organizer, shortcuts_listed, task_of};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let root = ews::parse(&text);
    let mut items = Vec::new();
    root.all("Message", &mut items);
    root.all("CalendarItem", &mut items);
    root.all("Contact", &mut items);
    root.all("Task", &mut items);
    for it in items.iter().take(512) {
        if let Some(e) = event_of(it) {
            assert!(!e.uid.is_empty(), "an event with no uid");
            assert!(e.attendees.len() <= 1500, "{} guests", e.attendees.len());
            assert!(e.categories.len() <= 20, "{} categories", e.categories.len());
            assert!(e.exdates.len() <= 1000, "{} deleted occurrences", e.exdates.len());
            assert!(e.end >= e.start, "{e:?}");
            assert!(e.rrule.as_ref().is_none_or(|r| r.len() <= 64 * 1024));
        }
        if let Some(x) = exception_of(it) {
            assert!(x.recurrence_id.is_some(), "an occurrence with no original start");
            assert!(x.rrule.is_none());
        }
        let _ = card_of(it);
        let _ = note_of(it);
        if let Some(t) = task_of(it) {
            assert!(t.href.starts_with("task:"));
            assert!(t.task.categories.len() <= 64 * 1024);
        }
        // The account's own response as the organizer of its meetings.
        if let Some(mut e) = event_of(it) {
            own_organizer(&mut e, it, "zoe@studio.example");
            if let Some(o) = &e.organizer {
                assert!(o.email.contains('@'));
            }
        }
    }
    // Outlook's grouped calendars: each shortcut names a directory entry (an X.500 name, ASCII,
    // at most 1024 bytes) and a named group, at most the cap.
    let shortcuts = shortcuts_listed(&root);
    assert!(shortcuts.len() <= 300, "{} shortcuts", shortcuts.len());
    for (dn, _name, group) in &shortcuts {
        assert!(dn.starts_with('/') && dn.is_ascii() && dn.len() <= 1024, "{dn:?}");
        assert!(group.chars().count() <= 4096, "{group:?}");
    }
});
