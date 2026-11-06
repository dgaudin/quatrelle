// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Where each folder goes in the data file: by its name, as Outlook shows names, or, for the
//! account's Sent, Trash, Drafts and Junk folders (and the folders inside them), in the file's
//! own special folders.

use super::*;
use outlook_pst::UnicodePstSpecialFolder as Special;
use quatrelle_model::Folder;

/// The roles that have a special folder in a data file. Quatrelle has no Outbox folder.
pub(super) const SPECIAL: [(FolderRole, Special); 4] = [
    (FolderRole::Sent, Special::SentItems),
    (FolderRole::Trash, Special::DeletedItems),
    (FolderRole::Drafts, Special::Drafts),
    (FolderRole::Junk, Special::JunkEmail),
];

/// The special folder for `role`, when it has one.
pub(super) fn special(role: FolderRole) -> Option<Special> {
    SPECIAL.iter().find(|(r, _)| *r == role).map(|(_, s)| *s)
}

/// A folder's path as names, the way Outlook shows them.
pub(super) fn names(path: &str, delimiter: Option<char>, role: FolderRole) -> Vec<String> {
    let parts: Vec<&str> = match delimiter {
        Some(d) => path.split(d).collect(),
        None => vec![path],
    };
    let mut out: Vec<String> = parts.iter().map(|p| utf7::decode(p).trim().to_string()).collect();
    if let Some(first) = out.first_mut()
        && (role == FolderRole::Inbox || first.eq_ignore_ascii_case("INBOX"))
    {
        *first = "Inbox".into();
    }
    for n in &mut out {
        if n.is_empty() {
            *n = "Folder".into();
        }
    }
    out
}

/// The folders the file has for each special role: the ones an existing file records, else
/// the names a new file gets.
pub(super) struct Targets(Vec<(Special, Vec<String>)>);

impl Targets {
    /// The file at `path`'s (none when there's no file yet). The file is one being added to, so
    /// anything wrong in it, a panic in the writer's reader too, costs only its record: that
    /// folder gets the name a new file gives it.
    pub(super) fn of(path: Option<&Path>) -> Targets {
        Targets::found(|s| match path {
            Some(p) => UnicodePstFile::special_folder(p, s),
            None => Ok(None),
        })
    }

    /// The ones `lookup` finds.
    fn found(lookup: impl Fn(Special) -> io::Result<Option<Vec<String>>> + std::panic::RefUnwindSafe) -> Targets {
        let found = |s: Special| std::panic::catch_unwind(|| lookup(s)).ok()?.ok().flatten();
        Targets(SPECIAL.iter().map(|(_, s)| (*s, found(*s).unwrap_or_else(|| vec![s.default_name().to_string()]))).collect())
    }

    pub(super) fn get(&self, s: Special) -> Vec<String> {
        self.0.iter().find(|(t, _)| *t == s).map(|(_, names)| names.clone()).unwrap_or_else(|| vec![s.default_name().to_string()])
    }
}

/// Where `f` (one of `folders`) goes: its special folder when its role has one, inside it when
/// a folder above it has such a role, else its own names.
pub(super) fn place(f: &Folder, folders: &[Folder], targets: &Targets) -> Vec<String> {
    let own = names(&f.path, f.delimiter, f.role);
    if let Some(s) = special(f.role) {
        return targets.get(s);
    }
    let above = folders
        .iter()
        .filter_map(|a| Some((a, special(a.role)?)))
        .find(|(a, _)| a.path != f.path && a.delimiter.is_some_and(|d| f.path.strip_prefix(a.path.as_str()).is_some_and(|rest| rest.starts_with(d))));
    match above {
        Some((a, s)) => {
            let depth = names(&a.path, a.delimiter, a.role).len();
            let mut out = targets.get(s);
            out.extend(own.into_iter().skip(depth));
            out
        }
        None => own,
    }
}

/// Record the folders of the account's special roles as the file's own special folders, unless
/// the file records one already (a file added to keeps its own); made when they have no mail.
/// The mail is in by then: a file that can't take the record keeps it as written, and the log
/// says why.
pub(super) fn record_special(path: &Path, folders: &[Folder], targets: &Targets, created: bool) {
    if !created {
        return;
    }
    for (role, special) in SPECIAL {
        if !folders.iter().any(|f| f.selectable && f.role == role) {
            continue;
        }
        match UnicodePstFile::special_folder(path, special) {
            Ok(Some(_)) => continue,
            Ok(None) => {}
            Err(e) => {
                log::info!("the data file's {special:?} can't be read: {e}");
                continue;
            }
        }
        let names = targets.get(special);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        if let Err(e) = UnicodePstFile::set_special_folder(path, special, &names) {
            log::info!("the data file's {special:?} can't be recorded: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names() {
        assert_eq!(names("INBOX/Projects/&AMk-t&AOk-", Some('/'), FolderRole::Other), ["Inbox", "Projects", "Été"]);
        assert_eq!(names("Sent", None, FolderRole::Sent), ["Sent"]);
        assert_eq!(names("a//b", Some('/'), FolderRole::Other), ["a", "Folder", "b"]);
    }

    #[test]
    fn special_folders_and_folders_inside_them() {
        let folder = |path: &str, role| Folder { path: path.into(), delimiter: Some('/'), role, ..Default::default() };
        let folders = [
            folder("[Gmail]/Sent Mail", FolderRole::Sent),
            folder("[Gmail]/Sent Mail/2024", FolderRole::Other),
            folder("Trash", FolderRole::Trash),
            folder("Trashy", FolderRole::Other),
            folder("Projects", FolderRole::Other),
        ];
        let targets = Targets::of(None);
        let at = |i: usize| place(&folders[i], &folders, &targets);
        assert_eq!(at(0), ["Sent Items"]);
        assert_eq!(at(1), ["Sent Items", "2024"]);
        assert_eq!(at(2), ["Deleted Items"]);
        assert_eq!(at(3), ["Trashy"]);
        assert_eq!(at(4), ["Projects"]);
        // A file that records its own: those.
        let targets = Targets(vec![(Special::SentItems, vec!["Gesendet".into()])]);
        assert_eq!(place(&folders[1], &folders, &targets), ["Gesendet", "2024"]);
        assert_eq!(place(&folders[2], &folders, &targets), ["Deleted Items"]);
    }

    /// A lookup in a damaged file that fails, or panics in the writer's reader, costs only its
    /// record: that folder gets the name a new file gives it, and the export goes on.
    #[test]
    fn a_damaged_record_gives_the_default_name() {
        let targets = Targets::found(|s| match s {
            Special::SentItems => Ok(Some(vec!["Gesendet".into()])),
            Special::DeletedItems => Err(io::Error::other("damaged")),
            _ => panic!("damaged"),
        });
        assert_eq!(targets.get(Special::SentItems), ["Gesendet"]);
        assert_eq!(targets.get(Special::DeletedItems), ["Deleted Items"]);
        assert_eq!(targets.get(Special::Drafts), ["Drafts"]);
        assert_eq!(targets.get(Special::JunkEmail), ["Junk Email"]);
    }
}
