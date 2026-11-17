// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message's own window acts on its message, as Outlook's does on its item, whatever the main
//! window has selected: while one of its commands runs, or its ribbon is drawn, that message stands
//! in as the one selected and read, and the main window's are given back after.

use super::*;

/// A window's command running: its message stands in for the main window's (`stands_in`), or the
/// main window is on that message alone and acts as it does there.
#[derive(Clone, Debug)]
pub(crate) struct Acting {
    pub(crate) stands_in: bool,
}

/// What the main window had selected and read while a window's message stood in for them
/// ([`Session::enter_window`]); given back by [`Session::leave_window`].
#[derive(Debug)]
#[must_use]
pub struct WindowScope {
    key: MessageKey,
    /// The main window's, put aside; none when it was on that message alone.
    main: Option<MainView>,
    /// The message was in the list as the scope began.
    listed: bool,
    /// The window acting before this one, when scopes nest.
    outer: Option<Acting>,
}

/// The main window's selection and message read, put aside.
#[derive(Debug)]
struct MainView {
    selection: Vec<MessageKey>,
    focus: Option<MessageKey>,
    anchor: Option<MessageKey>,
    reading: Option<Reading>,
    /// The message read was in the list (one read from elsewhere stays read).
    reading_listed: bool,
    /// How long the list was: one no longer that long lost messages, which aren't selected any
    /// more (a selection of the whole list isn't gone through at each frame a ribbon is drawn).
    listed_len: usize,
}

impl Session {
    /// Make the message of the window `key` the one selected and read, until
    /// [`Session::leave_window`]: what acts on the list's selection acts on it (nothing, when it
    /// isn't in the list any more: another folder opened since), what acts on the message read
    /// answers it. None when no window shows it.
    pub fn enter_window(&mut self, key: &MessageKey) -> Option<WindowScope> {
        let w = self.windows.iter().find(|w| &w.key == key)?.clone();
        let listed = self.summary(key).is_some();
        // The main window on that message alone: what's done is done as there (deleted, the
        // next one shows there).
        let same = self.selection.as_slice() == std::slice::from_ref(key) && self.reading.as_ref().is_some_and(|r| &r.key == key);
        let main = (!same).then(|| {
            let mine = || listed.then(|| key.clone());
            MainView {
                selection: std::mem::replace(&mut self.selection, mine().into_iter().collect()),
                focus: std::mem::replace(&mut self.focus, mine()),
                anchor: std::mem::replace(&mut self.anchor, mine()),
                reading_listed: self.reading.as_ref().is_some_and(|r| self.summary(&r.key).is_some()),
                listed_len: self.list.len(),
                reading: self.reading.replace(w),
            }
        });
        // Within another window's turn (scopes nest), what stood in stands in still.
        let stands_in = main.is_some() || self.window_stands_in();
        let outer = self.acting_for.replace(Acting { stands_in });
        Some(WindowScope { key: key.clone(), main, listed, outer })
    }

    /// Give the main window back its selection and message read (those that left the list
    /// meanwhile aren't selected any more); the window shows what was done to its message, and
    /// closes once its message was filed away (deleted, archived, moved), as Outlook's does.
    pub fn leave_window(&mut self, scope: WindowScope) {
        let WindowScope { key, main, listed, outer } = scope;
        if let Some(m) = main {
            if let Some(r) = self.reading.take().filter(|r| r.key == key)
                && let Some(w) = self.windows.iter_mut().find(|w| w.key == key)
            {
                *w = r;
            }
            if self.list.len() == m.listed_len {
                (self.selection, self.focus, self.anchor, self.reading) = (m.selection, m.focus, m.anchor, m.reading);
            } else {
                let here = |s: &Session, k: &Option<MessageKey>| k.as_ref().is_some_and(|k| s.summary(k).is_some());
                let selection: Vec<MessageKey> = m.selection.into_iter().filter(|k| self.summary(k).is_some()).collect();
                self.selection = selection;
                self.focus = Some(m.focus).filter(|k| here(self, k)).flatten();
                self.anchor = Some(m.anchor).filter(|k| here(self, k)).flatten();
                self.reading = m.reading.filter(|r| !m.reading_listed || self.summary(&r.key).is_some());
            }
        }
        self.acting_for = outer;
        if listed && self.summary(&key).is_none() && !self.folder_holds(&key) {
            self.close_window(&key);
        }
    }

    /// Run command `id` on the message of the window `key` ([`Session::enter_window`]).
    pub fn run_in_window(&mut self, key: &MessageKey, id: &str, params: &serde_json::Value) -> CmdResult {
        let scope = self.enter_window(key).ok_or_else(|| CmdError::Params(format!("no window shows `{}`", key.to_id())))?;
        let out = self.run(id, params);
        self.leave_window(scope);
        out
    }

    /// Whether a window's message stands in for the main window's now.
    pub(crate) fn window_stands_in(&self) -> bool {
        self.acting_for.as_ref().is_some_and(|a| a.stands_in)
    }

    /// Whether the folder `key` is in still holds it, as far as its cached list says (a list
    /// narrowed by a filter can lose a message that stays in its folder).
    fn folder_holds(&self, key: &MessageKey) -> bool {
        self.cache.get(&(key.account.clone(), key.folder.clone())).is_some_and(|f| f.messages.iter().any(|m| m.key == *key))
    }
}
