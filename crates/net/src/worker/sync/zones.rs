// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An account's worker: the folders of a zone an Exchange listing couldn't list (a shared
//! mailbox, the public folders or a branch of them, the online archive), kept as the listing
//! before had them — their place, their cache, their numbers — and left as they are till it
//! lists again.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::exchange::missed::{Missed, PublicMissed};

use super::*;

impl Worker {
    /// Whether a folder is kept as it was because its zone didn't list: nothing of it is read,
    /// written nor let go of till it lists again.
    pub(in crate::worker) fn kept_unlisted(&self, path: &str) -> bool {
        self.unlisted_kept.contains(path)
    }
}

/// The folder a path is in, `None` at the top (Exchange's paths are cut at `/`).
fn parent_of(path: &str) -> Option<&str> {
    path.rsplit_once('/').map(|(parent, _)| parent)
}

/// The folders a path is in, the nearest first.
fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(parent_of(path), |p| parent_of(p))
}

/// Whether a public folder may be in what `missed` says of the public folders: all of them, or a
/// branch under one whose id (now, or before) is named, through `up` (the folders it's in that
/// count: its parent alone, or every one).
fn public_missed<'a>(path: &str, missed: &Missed, mut up: impl Iterator<Item = &'a str>, id_of: impl Fn(&str) -> Option<&'a String>) -> bool {
    is_public_folder(path)
        && match &missed.public {
            PublicMissed::None => false,
            PublicMissed::All => true,
            PublicMissed::Under(parents) => up.any(|p| id_of(p).is_some_and(|id| parents.contains(id))),
        }
}

/// The folders of `before` that `missed` couldn't have listed, put back into `list` as they
/// were, with their ids; returns their paths.
///
/// One is put back when it's in a shared mailbox the listing missed (EWS: `Folder::mailbox`;
/// Graph: whose `owners_before` says it is), in any shared mailbox the listing doesn't name when
/// it couldn't tell which the account opens, in the public folders when none listed, in a public
/// branch that didn't list whole (its parent named, or put back), in the online archive when it
/// didn't list; and the folders such a one is in, up to its zone's top. Not one listed now under
/// another path (moved or renamed: its id is in `ids`).
pub(in crate::worker) fn keep_missed(
    before: &[Folder],
    ids_before: &BTreeMap<String, String>,
    owners_before: &BTreeMap<String, String>,
    list: &mut Vec<Folder>,
    ids: &mut BTreeMap<String, String>,
    owners: &mut BTreeMap<String, String>,
    missed: &Missed,
) -> HashSet<String> {
    let mut kept: HashSet<String> = HashSet::new();
    if !missed.any() {
        return kept;
    }
    let listed: HashSet<&str> = list.iter().map(|f| f.path.as_str()).collect();
    let listed_ids: HashSet<&str> = ids.values().map(String::as_str).collect();
    let moved = |path: &str| ids_before.get(path).is_some_and(|id| listed_ids.contains(id.as_str()));
    let named: HashSet<String> =
        list.iter().filter_map(Folder::shared_mailbox).chain(owners.values().map(String::as_str)).map(str::to_lowercase).collect();
    let shared: HashSet<String> = missed.shared.iter().map(|a| a.trim().to_lowercase()).collect();
    let by_path: HashMap<&str, &Folder> = before.iter().map(|f| (f.path.as_str(), f)).collect();
    // Shallow first: a public folder whose parent is put back follows it.
    let mut unnamed: Vec<&Folder> = before.iter().filter(|f| !listed.contains(f.path.as_str()) && !moved(&f.path)).collect();
    unnamed.sort_by_key(|f| f.path.matches('/').count());
    for f in unnamed {
        let owner = f.shared_mailbox().or_else(|| owners_before.get(&f.path).map(String::as_str)).map(str::to_lowercase);
        let in_shared = owner.is_some_and(|o| shared.contains(&o) || (missed.shared_unknown && !named.contains(&o)));
        let parent = parent_of(&f.path);
        let in_public = public_missed(&f.path, missed, parent.into_iter(), |p| ids_before.get(p).or_else(|| ids.get(p)))
            || (matches!(missed.public, PublicMissed::Under(_)) && is_public_folder(&f.path) && parent.is_some_and(|p| kept.contains(p)));
        if in_shared || in_public || (f.archive && missed.archive) {
            kept.insert(f.path.clone());
        }
    }
    // The folders one put back is in, up to its zone's top, go back with it.
    let mut tops: Vec<String> = Vec::new();
    for path in &kept {
        for up in ancestors(path) {
            if listed.contains(up) || kept.contains(up) || !by_path.contains_key(up) || moved(up) {
                break;
            }
            tops.push(up.to_string());
        }
    }
    kept.extend(tops);
    for f in before.iter().filter(|f| kept.contains(&f.path)) {
        list.push(f.clone());
        if let Some(id) = ids_before.get(&f.path) {
            ids.insert(f.path.clone(), id.clone());
        }
        if let Some(owner) = owners_before.get(&f.path) {
            owners.insert(f.path.clone(), owner.clone());
        }
    }
    kept
}

/// While the folder list before couldn't be read: the folders the numbering knew of (`ids_before`,
/// the listing before's) that this listing doesn't name, isn't naming under another path, and
/// that may be in a zone it missed, keep their ids (and owners), so that the next listing still
/// knows of them; their paths. Their mailbox isn't known on EWS (the folder list says it): any
/// shared mailbox or archive missed may hold one that isn't public.
pub(in crate::worker) fn keep_unread_ids(
    ids_before: &BTreeMap<String, String>,
    owners_before: &BTreeMap<String, String>,
    ids: &mut BTreeMap<String, String>,
    owners: &mut BTreeMap<String, String>,
    missed: &Missed,
) -> HashSet<String> {
    if !missed.any() {
        return HashSet::new();
    }
    let shared: HashSet<String> = missed.shared.iter().map(|a| a.trim().to_lowercase()).collect();
    let somewhere = !missed.shared.is_empty() || missed.shared_unknown || missed.archive;
    let found: Vec<(&String, &String)> = {
        let listed_ids: HashSet<&str> = ids.values().map(String::as_str).collect();
        ids_before
            .iter()
            .filter(|(path, id)| !ids.contains_key(*path) && !listed_ids.contains(id.as_str()))
            .filter(|(path, _)| {
                if is_public_folder(path) {
                    public_missed(path, missed, ancestors(path), |p| ids_before.get(p).or_else(|| ids.get(p)))
                } else {
                    owners_before.get(*path).map_or(somewhere, |o| shared.contains(&o.to_lowercase()))
                }
            })
            .collect()
    };
    let mut kept = HashSet::new();
    for (path, id) in found {
        ids.insert(path.clone(), id.clone());
        if let Some(owner) = owners_before.get(path) {
            owners.insert(path.clone(), owner.clone());
        }
        kept.insert(path.clone());
    }
    kept
}

/// Whether a folder gone from a listing, which the listing before didn't have either (one kept
/// for a change to the folders waiting), may be in a zone it missed: under a folder put back
/// (`kept`), or in the public folders it missed (`ids`: the folders' ids now).
pub(in crate::worker) fn in_missed_zone(path: &str, kept: &HashSet<String>, ids: &BTreeMap<String, String>, missed: &Missed) -> bool {
    ancestors(path).any(|a| kept.contains(a)) || public_missed(path, missed, ancestors(path), |p| ids.get(p))
}

/// The line a listing that missed something writes: counts only, never a path, an address nor
/// an id.
pub(in crate::worker) fn log_missed(account: &str, kept: usize, missed: &Missed) {
    if !missed.any() {
        return;
    }
    let public = match &missed.public {
        PublicMissed::None => "none".to_string(),
        PublicMissed::All => "all".to_string(),
        PublicMissed::Under(parents) => format!("{} branches", parents.len()),
    };
    log::info!(
        "{account}: {kept} folders kept as they were: {} zones didn't list (shared mailboxes {}, mapped mailboxes untold {}, public folders {public}, online archive {})",
        missed.zones(),
        missed.shared.len(),
        missed.shared_unknown,
        missed.archive
    );
}
