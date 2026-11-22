// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The pool: a data file read into an account on a thread of its own, told as it goes, stopped
//! when asked and gone on with later.

use quatrelle_model::job::IMPORT_FOLDERS_UNREAD;

use super::*;

/// An import's end.
fn ended(account: &str, messages: usize, skipped: usize, error: Option<String>) -> Event {
    Event::Imported { account: account.to_string(), messages, skipped, total: None, folder: String::new(), stopped: false, done: true, error }
}

impl Pool {
    /// Import a data file on a thread of its own: an Outlook one (`.pst`), or Quatrelle's own
    /// (`.quatrelle`, copied in as it is). `resume`: go on with the account's import from its
    /// journal instead of starting one (`Some(true)`: even when the file has changed since).
    pub(super) fn import_pst(&mut self, account: String, path: String, resume: Option<bool>) {
        let stop = Arc::new(AtomicBool::new(false));
        match self.imports.lock() {
            // One import of an account at a time: the one running goes on.
            Ok(running) if running.contains_key(&account) => return log::info!("{account}: its import is already running"),
            Ok(mut running) => {
                running.insert(account.clone(), stop.clone());
            }
            Err(_) => {
                return self.sink.emit(ended(&account, kept(&self.store, &account, resume), 0, Some("import: no import can be started".into())));
            }
        }
        let (sink, store, imports, id) = (self.sink.clone(), self.store.clone(), self.imports.clone(), account.clone());
        let spawned = std::thread::Builder::new().name("mail-import".into()).spawn(move || {
            let done = read_in(&store, &sink, &account, &path, resume, &stop);
            // Out of the running ones before its end is told: it can be gone on with at once.
            if let Ok(mut running) = imports.lock() {
                running.remove(&account);
            }
            sink.emit(done);
        });
        // No thread, no import: it ends here, as a failed one (the account isn't left importing).
        if let Err(e) = spawned {
            if let Ok(mut running) = self.imports.lock() {
                running.remove(&id);
            }
            self.sink.emit(ended(&id, kept(&self.store, &id, resume), 0, Some(format!("import: {e}"))));
        }
    }

    /// Stop the account's import at its next item (none running: nothing to stop).
    pub(super) fn stop_import(&mut self, account: &str) {
        if let Ok(running) = self.imports.lock()
            && let Some(stop) = running.get(account)
        {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

/// The messages an account's folders hold when an import that was to go on with it fails: said
/// with the error, so what an earlier run brought in isn't taken for nothing. Its folder list
/// that can't be read (the disk failing) isn't taken for none either: what the import's journal
/// noted is said then, read as it is (a damaged one is left for the import to say).
pub(in crate::worker) fn kept(store: &Store, account: &str, resume: Option<bool>) -> usize {
    if resume.is_none() {
        return 0;
    }
    match store.load_folders(account) {
        Ok(folders) => folders.iter().fold(0usize, |n, f| n.saturating_add(usize::try_from(f.total).unwrap_or(usize::MAX))),
        Err(e) => {
            log::warn!("{account}: the folder list couldn't be read, the import's journal says what came in: {e}");
            store.load_account_doc_strict::<quatrelle_model::ImportJournal>(account, quatrelle_model::IMPORT_DOC).map_or(0, |j| j.messages)
        }
    }
}

/// Read the data file in, telling how it goes: how it ended.
fn read_in(store: &Store, sink: &Sink, account: &str, path: &str, resume: Option<bool>, stop: &AtomicBool) -> Event {
    if path.to_ascii_lowercase().ends_with(".quatrelle") {
        // Copied in whole: nothing to go on with.
        if resume.is_some() {
            return ended(account, kept(store, account, resume), 0, Some(quatrelle_model::IMPORT_NO_JOURNAL.into()));
        }
        return match store.import_data_file(account, std::path::Path::new(path)) {
            Ok(messages) => said_copied(store, sink, account, messages),
            Err(e) => ended(account, 0, 0, Some(e.to_string())),
        };
    }
    let mut progress = |p: &crate::pst::Progress<'_>| {
        sink.emit(Event::Imported {
            account: account.to_string(),
            messages: p.messages,
            skipped: 0,
            total: p.total,
            folder: p.folder.to_string(),
            stopped: false,
            done: false,
            error: None,
        });
    };
    let asked = || stop.load(Ordering::Relaxed);
    let run = crate::pst::Run { stop: Some(&asked), resume: resume.is_some(), allow_changed: resume == Some(true) };
    match crate::pst::import_run(std::path::Path::new(path), store, account, run, &mut progress) {
        Ok(done) => {
            let paths: Vec<String> = done.folders.iter().map(|f| f.path.clone()).collect();
            sink.emit(Event::Folders { account: account.to_string(), folders: done.folders });
            for folder in paths {
                sink.emit(Event::FolderChanged { account: account.to_string(), folder });
            }
            // A last save that failed is said with what came in (the account stays).
            Event::Imported {
                account: account.to_string(),
                messages: done.messages,
                skipped: done.skipped,
                total: done.total,
                folder: String::new(),
                stopped: done.stopped,
                done: true,
                error: done.last_save,
            }
        }
        Err(e) => ended(account, kept(store, account, resume), 0, Some(e)),
    }
}

/// A data file copied in whole (`messages` of them): its folders told, and the import's end. Its
/// folder list read back can fail (the disk failing): that's said with the end, as the engine
/// knows it ([`IMPORT_FOLDERS_UNREAD`]: all of it is here, nothing to import again), and no list
/// is told (found in review: the account showed no folders, and the import said all was well).
pub(in crate::worker) fn said_copied(store: &Store, sink: &Sink, account: &str, messages: usize) -> Event {
    match store.load_folders(account) {
        Ok(folders) => {
            let paths: Vec<String> = folders.iter().map(|f| f.path.clone()).collect();
            sink.emit(Event::Folders { account: account.to_string(), folders });
            for folder in paths {
                sink.emit(Event::FolderChanged { account: account.to_string(), folder });
            }
            ended(account, messages, 0, None)
        }
        Err(e) => {
            log::warn!("{account}: the folder list of the data file copied couldn't be read back: {e}");
            ended(account, messages, 0, Some(IMPORT_FOLDERS_UNREAD.into()))
        }
    }
}
