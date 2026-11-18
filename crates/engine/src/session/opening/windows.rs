// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A message's own window acts on its message, as Outlook's does on its item, whatever the main
//! window has selected or shows: while one of its commands runs, or its ribbon is drawn, that
//! message stands in as the one selected and read — found by its key even where the list on screen
//! doesn't hold it (another folder shown, a filter, a search) — and the main window's are given
//! back after. Each window keeps its own zoom.

use super::*;

/// What the session keeps of the messages' own windows.
#[derive(Debug, Default)]
pub(crate) struct OwnWindows {
    /// A window's command running (or its ribbon being drawn).
    pub(crate) acting: Option<Acting>,
    /// Each window's message as last seen, in the list or in its folder's: what it acts on where
    /// the list on screen doesn't hold it.
    seen: HashMap<MessageKey, Arc<Summary>>,
    /// Windows whose message a command moved out of its folder or deleted: they close.
    gone: HashSet<MessageKey>,
    /// Each window's zoom (percent), the main window's when it opened.
    zoom: HashMap<MessageKey, u16>,
}

/// A window's command running: its message stands in for the main window's (`stands_in`), or the
/// main window is on that message alone and acts as it does there.
#[derive(Clone, Debug)]
pub(crate) struct Acting {
    /// The window's message.
    key: MessageKey,
    pub(crate) stands_in: bool,
    /// The window's message, where the list on screen doesn't hold it: [`Session::summary`] and
    /// [`Session::selected`] find it all the same.
    unlisted: Option<Arc<Summary>>,
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
    /// For a command ([`Session::enter_window_to_act`]), not a drawing: what changed is looked at.
    to_act: bool,
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
    /// How long the list was: drawn, a list as long is given back as it was (a selection of the
    /// whole list isn't gone through at every frame); after a command, what left the list isn't
    /// selected any more.
    listed_len: usize,
}

impl Session {
    /// Make the message of the window `key` the one selected and read, until
    /// [`Session::leave_window`]: what acts on the list's selection acts on it, wherever it is (the
    /// list on screen holding it or not), what acts on the message read answers it. None when no
    /// window shows it. For a command, [`Session::enter_window_to_act`].
    pub fn enter_window(&mut self, key: &MessageKey) -> Option<WindowScope> {
        self.enter_window_as(key, false)
    }

    /// [`Session::enter_window`] to run a command: the window's message, where the list on screen
    /// doesn't hold it, is looked up in its folder's list as it is now (read from the disk if need
    /// be); gone from there (moved or deleted elsewhere), nothing stands in for it, and what files
    /// or marks mail does nothing.
    pub fn enter_window_to_act(&mut self, key: &MessageKey) -> Option<WindowScope> {
        if self.windows.iter().any(|w| &w.key == key) && self.index_of(key).is_none() {
            let now = self.folder_state(&key.account, &key.folder).messages.iter().find(|m| m.key == *key).cloned();
            match now {
                Some(m) => {
                    self.own_windows.seen.insert(key.clone(), m);
                }
                None => {
                    self.own_windows.seen.remove(key);
                }
            }
        }
        self.enter_window_as(key, true)
    }

    fn enter_window_as(&mut self, key: &MessageKey, to_act: bool) -> Option<WindowScope> {
        let w = self.windows.iter().find(|w| &w.key == key)?.clone();
        let here = self.index_of(key).and_then(|i| self.list.get(i)).cloned();
        let listed = here.is_some();
        if let Some(m) = here {
            self.own_windows.seen.insert(key.clone(), m);
        }
        let unlisted = if listed { None } else { self.own_windows.seen.get(key).cloned() };
        let known = listed || unlisted.is_some();
        // The main window on that message alone: what's done is done as there (deleted, the
        // next one shows there).
        let same = self.selection.as_slice() == std::slice::from_ref(key) && self.reading.as_ref().is_some_and(|r| &r.key == key);
        let main = (!same).then(|| {
            let mine = || known.then(|| key.clone());
            MainView {
                selection: std::mem::replace(&mut self.selection, mine().into_iter().collect()),
                focus: std::mem::replace(&mut self.focus, mine()),
                anchor: std::mem::replace(&mut self.anchor, mine()),
                reading_listed: self.reading.as_ref().is_some_and(|r| self.index_of(&r.key).is_some()),
                listed_len: self.list.len(),
                reading: self.reading.replace(w),
            }
        });
        // Within another window's turn (scopes nest), what stood in stands in still.
        let stands_in = main.is_some() || self.window_stands_in();
        let outer = self.own_windows.acting.replace(Acting { key: key.clone(), stands_in, unlisted });
        Some(WindowScope { key: key.clone(), main, listed, to_act, outer })
    }

    /// Give the main window back its selection and message read (after a command, those that left
    /// the list meanwhile aren't selected any more); the window shows what was done to its
    /// message, and closes once its message was filed away (deleted, archived, moved), as
    /// Outlook's does.
    pub fn leave_window(&mut self, scope: WindowScope) {
        let WindowScope { key, main, listed, to_act, outer } = scope;
        if let Some(m) = main {
            if let Some(r) = self.reading.take().filter(|r| r.key == key)
                && let Some(w) = self.windows.iter_mut().find(|w| w.key == key)
            {
                *w = r;
            }
            if !to_act && self.list.len() == m.listed_len {
                (self.selection, self.focus, self.anchor, self.reading) = (m.selection, m.focus, m.anchor, m.reading);
            } else {
                let here = |s: &Session, k: &Option<MessageKey>| k.as_ref().is_some_and(|k| s.index_of(k).is_some());
                let selection: Vec<MessageKey> = m.selection.into_iter().filter(|k| self.index_of(k).is_some()).collect();
                self.selection = selection;
                self.focus = Some(m.focus).filter(|k| here(self, k)).flatten();
                self.anchor = Some(m.anchor).filter(|k| here(self, k)).flatten();
                self.reading = m.reading.filter(|r| !m.reading_listed || self.index_of(&r.key).is_some());
            }
        }
        self.own_windows.acting = outer;
        if !to_act {
            return;
        }
        // What was done to it (read, flagged, categorized), its ribbon shows.
        let now = self
            .index_of(&key)
            .and_then(|i| self.list.get(i).cloned())
            .or_else(|| self.cache.get(&(key.account.clone(), key.folder.clone())).and_then(|f| f.messages.iter().find(|m| m.key == key).cloned()));
        if let Some(m) = now {
            self.own_windows.seen.insert(key.clone(), m);
        }
        let filed = self.own_windows.gone.remove(&key) || (listed && self.index_of(&key).is_none() && !self.folder_holds(&key));
        if filed {
            self.close_window(&key);
        }
    }

    /// Run command `id` on the message of the window `key` ([`Session::enter_window_to_act`]).
    pub fn run_in_window(&mut self, key: &MessageKey, id: &str, params: &serde_json::Value) -> CmdResult {
        let scope = self.enter_window_to_act(key).ok_or_else(|| CmdError::Params(format!("no window shows `{}`", key.to_id())))?;
        let out = self.run(id, params);
        self.leave_window(scope);
        out
    }

    /// Whether a window's message stands in for the main window's now.
    pub(crate) fn window_stands_in(&self) -> bool {
        self.own_windows.acting.as_ref().is_some_and(|a| a.stands_in)
    }

    /// The message of the window acting now, if one is.
    pub(crate) fn window_acting_key(&self) -> Option<&MessageKey> {
        self.own_windows.acting.as_ref().map(|a| &a.key)
    }

    /// The list made again while a window acts (a search run again after a step of a Quick
    /// Step, a filter) doesn't hold its message: it stands in all the same, as its folder's list
    /// has it now, for the steps after.
    pub(crate) fn window_left_list(&mut self, key: &MessageKey) {
        let now = self
            .cache
            .get(&(key.account.clone(), key.folder.clone()))
            .and_then(|f| f.messages.iter().find(|m| m.key == *key).cloned())
            .or_else(|| self.index_of(key).and_then(|i| self.list.get(i).cloned()))
            .or_else(|| self.window_unlisted(key).cloned());
        if let Some(a) = self.own_windows.acting.as_mut().filter(|a| a.key == *key) {
            a.unlisted = now;
        }
    }

    /// The message of the window acting now, where the list on screen doesn't hold it.
    pub(crate) fn window_unlisted(&self, key: &MessageKey) -> Option<&Arc<Summary>> {
        self.own_windows.acting.as_ref()?.unlisted.as_ref().filter(|m| m.key == *key)
    }

    /// A change about to be made while a window acts: the windows whose message it moves out of
    /// its folder or deletes close once it's done.
    pub(crate) fn note_window_removals(&mut self, op: &Op) {
        if self.own_windows.acting.is_none() {
            return;
        }
        let removed: Vec<&MessageKey> = match op {
            Op::Move { keys, to } => keys.iter().filter(|k| &k.folder != to).collect(),
            Op::Expunge { keys, .. } => keys.iter().collect(),
            _ => return,
        };
        for k in removed {
            if self.windows.iter().any(|w| w.key == *k) {
                self.own_windows.gone.insert(k.clone());
            }
        }
    }

    /// The zoom of the window showing `key` (percent): its own, from the main window's when it
    /// opened.
    pub fn window_zoom(&self, key: &MessageKey) -> u16 {
        self.own_windows.zoom.get(key).copied().unwrap_or(self.prefs.reading_zoom)
    }

    /// Set the zoom of the window showing `key`, the main window's left as it is.
    pub(crate) fn set_window_zoom(&mut self, key: &MessageKey, percent: u16) -> Result<u16, CmdError> {
        if !self.windows.iter().any(|w| &w.key == key) {
            return Err(CmdError::Params(format!("no window shows `{}`", key.to_id())));
        }
        self.own_windows.zoom.insert(key.clone(), percent);
        Ok(percent)
    }

    /// A window opening (its message in the list): at the main window's zoom, its own from then on;
    /// its message as it is now, for when the list on screen no longer holds it.
    pub(super) fn window_opened(&mut self, key: &MessageKey) {
        self.own_windows.zoom.insert(key.clone(), self.prefs.reading_zoom);
        if let Some(m) = self.index_of(key).and_then(|i| self.list.get(i)).cloned() {
            self.own_windows.seen.insert(key.clone(), m);
        }
    }

    /// A window closed: what's kept of it goes.
    pub(super) fn window_closed(&mut self, key: &MessageKey) {
        self.own_windows.seen.remove(key);
        self.own_windows.gone.remove(key);
        self.own_windows.zoom.remove(key);
    }

    /// Whether the folder `key` is in still holds it, as far as its cached list says (a list
    /// narrowed by a filter can lose a message that stays in its folder).
    fn folder_holds(&self, key: &MessageKey) -> bool {
        self.cache.get(&(key.account.clone(), key.folder.clone())).is_some_and(|f| f.messages.iter().any(|m| m.key == *key))
    }
}
