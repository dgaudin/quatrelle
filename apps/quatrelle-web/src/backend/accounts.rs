// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The browser's accounts: starting one with its numbering, connecting it, listing and syncing its
//! folders.

use std::collections::VecDeque;
use std::rc::Rc;

use quatrelle_engine::model::{AccountConfig, Event, Folder, FolderRole, Secret, SyncStatus, sort_folders};
use quatrelle_jmap::{Client, Error, State, SyncOptions};

use super::{Account, Rcs, Task, WINDOWS, client, emit, enqueue, notice, now, poll, push, put_state, remote, state, status, store};
use crate::fetch::Fetch;

/// Start an account (or start it again, with its settings changed): its numbering and folders read
/// from the cache, then connected, polled and listened to.
pub(super) fn start(sh: &Rcs, account: AccountConfig, password: Option<Secret>) {
    let id = account.id.clone();
    let generation = {
        let mut s = sh.borrow_mut();
        let store = s.store.clone();
        let old = s.accounts.remove(&id);
        let generation = old.as_ref().map_or(1, |a| a.generation.wrapping_add(1));
        let password = password.map(|p| p.0).or_else(|| old.as_ref().and_then(|a| a.password.clone()));
        s.accounts.insert(
            id.clone(),
            Account {
                state: State::load(&store, &id),
                folders: store.load_folders(&id).unwrap_or_default(),
                ops: old.map(|a| a.ops).unwrap_or_default(),
                config: account,
                password,
                client: None,
                queue: VecDeque::new(),
                running: false,
                generation,
                pushing: false,
                windows: store.load_account_doc(&id, WINDOWS).unwrap_or_default(),
            },
        );
        generation
    };
    enqueue(sh, &id, Task::Connect);
    wasm_bindgen_futures::spawn_local(poll(sh.clone(), id.clone(), generation));
    wasm_bindgen_futures::spawn_local(push(sh.clone(), id, generation));
}

/// A failure: signing in again, reconnecting later, or just a notice.
pub(super) fn failed(sh: &Rcs, id: &str, e: Error) {
    log::warn!("{id}: {e}");
    match e {
        Error::Auth(m) => {
            if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
                a.client = None;
                a.password = None;
            }
            status(sh, id, SyncStatus::NeedsPassword { error: m });
        }
        // A site the account doesn't trust (its text names it) stops the account as a broken
        // connection does: Server settings' "Add" is how the browser allows it.
        e @ (Error::Io(_) | Error::Closed(_) | Error::Protocol(_) | Error::Tls(_) | Error::Untrusted { .. }) => {
            if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
                a.client = None;
            }
            status(sh, id, SyncStatus::Error { error: e.to_string() });
        }
        e => notice(sh, id, e.to_string()),
    }
}

pub(super) async fn connect(sh: &Rcs, id: &str) {
    let Some((config, password)) = sh.borrow().accounts.get(id).map(|a| (a.config.clone(), a.password.clone())) else { return };
    let Some(url) = config.jmap.clone() else {
        return status(sh, id, SyncStatus::Error { error: "only JMAP accounts work in the browser; use the desktop app for this one".into() });
    };
    let Some(password) = password else {
        return status(sh, id, SyncStatus::NeedsPassword { error: "enter the password: the browser doesn't keep it".into() });
    };
    status(sh, id, SyncStatus::Connecting);
    let user = if config.username.is_empty() { config.email.clone() } else { config.username.clone() };
    let trusted = quatrelle_engine::model::addr::trust::Trust::of(&config, &[]).sites();
    match Client::connect(Fetch, &url, &user, &password, false, &trusted).await {
        Ok(c) => {
            let ops = {
                let mut s = sh.borrow_mut();
                let Some(a) = s.accounts.get_mut(id) else { return };
                a.client = Some(Rc::new(c));
                std::mem::take(&mut a.ops)
            };
            for op in ops {
                remote(sh, id, op).await;
            }
            sync_all(sh, id).await;
        }
        Err(e) => failed(sh, id, e),
    }
}

/// Refresh the folder list; `false` when that failed.
pub(super) async fn relist(sh: &Rcs, id: &str) -> bool {
    let Some(c) = client(sh, id) else { return false };
    let mut st = state(sh, id);
    let r = quatrelle_jmap::list_folders(&c, &mut st, id).await;
    put_state(sh, id, st);
    match r {
        Ok(mut list) => {
            sort_folders(&mut list);
            let store = store(sh);
            let old: Vec<Folder> = sh.borrow().accounts.get(id).map(|a| a.folders.clone()).unwrap_or_default();
            for f in old.iter().filter(|o| !list.iter().any(|f| f.path == o.path)) {
                let _ = store.clear_folder(id, &f.path);
            }
            let _ = store.save_folders(id, &list);
            if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
                a.folders = list.clone();
            }
            emit(sh, Event::Folders { account: id.to_string(), folders: list });
            true
        }
        Err(e) => {
            failed(sh, id, e);
            false
        }
    }
}

pub(super) async fn sync_all(sh: &Rcs, id: &str) {
    if !relist(sh, id).await {
        return;
    }
    let paths: Vec<String> =
        sh.borrow().accounts.get(id).map(|a| a.folders.iter().filter(|f| f.selectable).map(|f| f.path.clone()).collect()).unwrap_or_default();
    // The Inbox first.
    let inbox = sh.borrow().accounts.get(id).and_then(|a| a.folders.iter().find(|f| f.role == FolderRole::Inbox).map(|f| f.path.clone()));
    for p in inbox.iter().chain(paths.iter().filter(|p| Some(*p) != inbox.as_ref())) {
        if client(sh, id).is_none() {
            return;
        }
        sync_one(sh, id, p).await;
    }
    if let Some(c) = client(sh, id) {
        if let Ok(Some((used, limit))) = c.quota().await {
            emit(sh, Event::Quota { account: id.to_string(), used, limit });
        }
        status(sh, id, SyncStatus::Online { at: now() });
    }
}

pub(super) async fn sync_one(sh: &Rcs, id: &str, path: &str) {
    let Some(c) = client(sh, id) else { return };
    status(sh, id, SyncStatus::Syncing { folder: path.to_string() });
    let (store, opts) = {
        let s = sh.borrow();
        // A folder asked to show more keeps more.
        let window = s.accounts.get(id).and_then(|a| a.windows.get(path)).copied().unwrap_or(0).max(s.opts.window);
        (s.store.clone(), SyncOptions { window, ..s.opts })
    };
    let mut st = state(sh, id);
    let r = quatrelle_jmap::sync_folder(&c, &mut st, &store, id, path, &opts, now()).await;
    put_state(sh, id, st);
    match r {
        Ok(r) => {
            let (first, counts) = {
                let mut s = sh.borrow_mut();
                let Some(a) = s.accounts.get_mut(id) else { return };
                let Some(f) = a.folders.iter_mut().find(|f| f.path == path) else { return };
                let first = f.total == 0 && r.total > 0;
                let changed = f.unread != r.unread || f.total != r.total;
                (f.unread, f.total) = (r.unread, r.total);
                (first, changed.then(|| a.folders.clone()))
            };
            if let Some(folders) = counts {
                let _ = store.save_folders(id, &folders);
                emit(sh, Event::Folders { account: id.to_string(), folders });
            }
            if r.touched() || first {
                emit(sh, Event::FolderChanged { account: id.to_string(), folder: path.to_string() });
            }
            status(sh, id, SyncStatus::Online { at: now() });
        }
        Err(e) => failed(sh, id, e),
    }
}
