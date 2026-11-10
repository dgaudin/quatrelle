// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The account's Notes and Tasks folders read by their changes, from the state kept with them
//! ([`Pim::notes_state`], [`Pim::tasks_state`]). A sync gives the engine the whole list, as a whole
//! read does (what's kept here and unchanged, with what changed): it keeps those changed here
//! meanwhile, and those never sent.

use quatrelle_model::pim::StoredTask;

use super::super::tasks::task_of;
use super::*;

/// A list's folder read: by its changes when a state is kept and something it stands for is here
/// (`here`), else whole. The items read, the items touched (`None`: read whole), and the state.
fn read_list(
    c: &Client,
    folder: &str,
    label: &str,
    kept: &str,
    here: bool,
    again: &[String],
    shape: Shape,
) -> Result<(Vec<El>, Option<HashSet<String>>, String)> {
    let element = distinguished(folder);
    if let Some(read) = read_changes(c, &element, label, if here { kept } else { "" }, again, shape)? {
        log::info!("{label} read by their changes: {} made or changed, {} deleted", read.changed, read.deleted);
        return Ok((read.found, Some(read.touched), read.state));
    }
    let state = state_now(c, &element, label)?.unwrap_or_default();
    let shape = match shape {
        Shape::All(extra) => extra,
        Shape::Appointment => "",
    };
    Ok((items_in(c, &element, shape, None)?, None, state))
}

/// The ids of the items `prefix` (`note:`, `task:`) names among `again` (deletions refused).
fn ids(again: &[String], prefix: &str) -> Vec<String> {
    again.iter().filter_map(|h| h.strip_prefix(prefix)).map(str::to_string).collect()
}

/// The server's notes now, and the state they were read up to; a mailbox without a Notes folder
/// keeps them on this computer (an error).
pub(in crate::exchange::ews_pim) fn notes_read(c: &Client, pim: &Pim, again: &[String]) -> Result<(Vec<StoredNote>, String)> {
    let here = pim.notes.iter().any(|n| n.href.starts_with("note:"));
    let (found, touched, state) = read_list(c, "notes", "the notes", &pim.notes_state, here, &ids(again, "note:"), Shape::All(NOTE_FIELDS))?;
    let mut list: Vec<StoredNote> = found.iter().filter_map(note_of).collect();
    if let Some(touched) = touched {
        // Those kept, unchanged there; one changed here is the engine's.
        let keep = |n: &&StoredNote| n.href.strip_prefix("note:").is_some_and(|id| !touched.contains(id)) && !n.dirty;
        list.extend(pim.notes.iter().filter(keep).cloned());
    }
    Ok((list, state))
}

/// The server's tasks now, and the state they were read up to; a mailbox without a Tasks folder
/// keeps them on this computer (an error).
pub(in crate::exchange::ews_pim) fn tasks_read(c: &Client, pim: &Pim, again: &[String]) -> Result<(Vec<StoredTask>, String)> {
    let here = pim.tasks.iter().any(|t| t.href.starts_with("task:"));
    let (found, touched, state) = read_list(c, "tasks", "the tasks", &pim.tasks_state, here, &ids(again, "task:"), Shape::All(""))?;
    let mut list: Vec<StoredTask> = found.iter().filter_map(task_of).collect();
    if let Some(touched) = touched {
        let keep = |t: &&StoredTask| t.href.strip_prefix("task:").is_some_and(|id| !touched.contains(id)) && !t.dirty;
        list.extend(pim.tasks.iter().filter(keep).cloned());
    }
    Ok((list, state))
}
