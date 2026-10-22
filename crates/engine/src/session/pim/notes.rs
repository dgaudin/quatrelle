//! Outlook's Notes: each account's, kept with its calendar and address book.

use super::*;

/// A note with the account it's in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteRef {
    pub account: String,
    pub note: quatrelle_model::pim::Note,
}

impl Session {
    /// Every note matching a search (in its text), the most recently changed first.
    pub fn notes(&self, query: &str) -> Vec<NoteRef> {
        let q = query.trim().to_lowercase();
        let mut out: Vec<NoteRef> = self
            .pim
            .iter()
            .flat_map(|(a, p)| {
                p.notes
                    .iter()
                    .filter(|n| q.is_empty() || n.note.text.to_lowercase().contains(&q))
                    .map(|n| NoteRef { account: a.clone(), note: n.note.clone() })
            })
            .collect();
        // A note's title is its first line, compared without case.
        let title = |n: &NoteRef| n.note.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_lowercase();
        match self.prefs.notes_order {
            NotesOrder::Modified => out.sort_by(|a, b| b.note.modified.cmp(&a.note.modified).then_with(|| a.note.uid.cmp(&b.note.uid))),
            NotesOrder::Title => out.sort_by_cached_key(|n| (title(n), n.note.uid.clone())),
            NotesOrder::Color => out.sort_by_cached_key(|n| (n.note.color, title(n), n.note.uid.clone())),
        }
        out
    }

    fn note_account(&self, uid: &str) -> Result<String, CmdError> {
        self.pim
            .iter()
            .find(|(_, p)| p.notes.iter().any(|n| n.note.uid == uid))
            .map(|(a, _)| a.clone())
            .ok_or_else(|| CmdError::Params(format!("no note `{uid}`")))
    }

    /// Notes go to the server only where it keeps them (Exchange's Notes folder).
    fn notes_sync(&self, account: &str) -> bool {
        self.account(account).is_some_and(|a| !a.is_local() && a.config.ews.is_some())
    }

    /// A new note; returns its uid.
    pub fn create_note(&mut self, account: Option<&str>, text: &str, color: Option<u8>) -> CmdResult {
        let account = self.pim_account(account)?;
        let note = quatrelle_model::pim::Note {
            uid: self.new_uid(),
            text: cap_note(text),
            color: color.unwrap_or(quatrelle_model::pim::NOTE_YELLOW).min(quatrelle_model::pim::NOTE_COLORS - 1),
            modified: self.clock.now,
        };
        let uid = note.uid.clone();
        let remote = self.notes_sync(&account);
        let p = self.pim_mut(&account);
        if p.notes.len() >= MAX_NOTES {
            return Err(CmdError::Params(format!("an account keeps at most {MAX_NOTES} notes")));
        }
        p.notes.push(quatrelle_model::pim::StoredNote { note, dirty: remote, ..Default::default() });
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid, "account": account}))
    }

    /// Change a note's text or colour.
    pub fn update_note(&mut self, uid: &str, text: Option<&str>, color: Option<u8>) -> CmdResult {
        let account = self.note_account(uid)?;
        let remote = self.notes_sync(&account);
        let now = self.clock.now;
        let p = self.pim_mut(&account);
        let Some(n) = p.notes.iter_mut().find(|n| n.note.uid == uid) else { return Err(CmdError::Params(format!("no note `{uid}`"))) };
        let before = n.note.clone();
        if let Some(t) = text {
            n.note.text = cap_note(t);
        }
        if let Some(c) = color {
            n.note.color = c.min(quatrelle_model::pim::NOTE_COLORS - 1);
        }
        if n.note == before {
            return Ok(serde_json::json!({"uid": uid, "changed": false}));
        }
        n.note.modified = now;
        n.dirty |= remote;
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid}))
    }

    pub fn delete_note(&mut self, uid: &str) -> CmdResult {
        let account = self.note_account(uid)?;
        let p = self.pim_mut(&account);
        let mut remote = false;
        for n in p.notes.iter().filter(|n| n.note.uid == uid && !n.href.is_empty()) {
            remote = true;
            p.deleted.push(Tombstone { href: n.href.clone(), etag: n.etag.clone(), ..Default::default() });
        }
        p.notes.retain(|n| n.note.uid != uid);
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"deleted": uid}))
    }
}

/// Most notes an account keeps, and most characters in one.
const MAX_NOTES: usize = 5_000;
const MAX_NOTE_CHARS: usize = 100_000;

fn cap_note(text: &str) -> String {
    text.chars().take(MAX_NOTE_CHARS).collect()
}
