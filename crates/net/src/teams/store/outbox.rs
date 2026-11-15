// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The outbox, kept in the copy: each message going out is a record of its own,
//! `outbox/<seq>.json` (its number in 20 digits, so the names sort as the numbers do), from when
//! the worker takes it until Teams has it or the user lets it go. Only the worker writes it.
//!
//! It is read back as hostile as the rest of the copy: a record past [`MAX_OP`] bytes, under
//! another number's name, or past its caps ([`OpRecord::broken`]) is passed over, and what is
//! passed over is logged by count, never by what it holds. A record that wouldn't be read back is
//! never written.

use quatrelle_mime::{TEAMS_RUNS, teams_write};
use quatrelle_model::pim::teams::{ChatText, LOCAL_MAX, OUTBOX_MAX, OutState, Outgoing, Unsent};

use super::*;

/// The folder of the outbox's records.
const OUTBOX: &str = "outbox";
/// The most bytes of a record read back, or written.
const MAX_OP: usize = 1 << 20;
/// The most tries a record says.
const MAX_TRIES: u8 = 9;
/// The longest code of Teams' answer kept ([`OpState::NotSent`]).
const MAX_CODE: usize = 64;

/// A message going out, as the outbox keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OpRecord {
    /// Its number in the outbox: from 1, in the order the worker took them.
    pub(crate) seq: u64,
    /// The engine's id for it ([`quatrelle_model::pim::teams::Op::Send`]): at most [`LOCAL_MAX`]
    /// of `0-9`, `a-z`, `.` and `-`.
    pub(crate) local: String,
    pub(crate) chat: String,
    pub(crate) body: ChatText,
    pub(crate) state: OpState,
    /// The times it was posted without an answer saying Teams has it ([`OpState::Unknown`]).
    pub(crate) tries: u8,
    /// The newest creation time known in the chat when it was taken (milliseconds since 1970, on
    /// Graph's clock): a message of the user's made after it may be this one.
    pub(crate) after: Option<i64>,
}

/// Where a message going out stands, as the worker knows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(crate) enum OpState {
    /// Its turn to come.
    Waiting,
    /// Being posted: written so before the request, so that a run cut short meanwhile knows it
    /// may have gone.
    Sending,
    /// Posted without an answer: whether Teams has it is to be checked.
    Unknown,
    /// Not sent, and never sent again unless the user asks ([`OutState::NotSent`]).
    NotSent { reason: Unsent, code: Option<String> },
}

impl OpRecord {
    /// As the conversation shows it: one being checked is shown as being sent.
    #[cfg_attr(not(test), expect(dead_code, reason = "the worker's outbox says it once wired"))]
    pub(crate) fn outgoing(&self) -> Outgoing {
        let state = match &self.state {
            OpState::Waiting => OutState::Waiting,
            OpState::Sending | OpState::Unknown => OutState::Sending,
            OpState::NotSent { reason, code } => OutState::NotSent { reason: *reason, code: code.clone() },
        };
        Outgoing { seq: self.seq, local: self.local.clone(), chat: self.chat.clone(), body: self.body.clone(), state }
    }

    /// The first of the outbox's caps it breaks; `None` for a record the outbox keeps. Its number
    /// is from 1 and short of the last (so the next one always has a number of its own), its chat
    /// id usable ([`id_ok`]), its local id as [`OpRecord::local`] says, at most [`MAX_TRIES`]
    /// tries, Teams' code of `A-Z`, `a-z`, `0-9`, ` `, `_`, `.` and `-` (1 to [`MAX_CODE`]), and
    /// its body at most [`TEAMS_RUNS`] runs (runs that write nothing would cost memory only) and
    /// [`quatrelle_model::pim::teams::SEND_MAX`] bytes once written.
    pub(super) fn broken(&self) -> Option<&'static str> {
        let local = !self.local.is_empty()
            && self.local.len() <= LOCAL_MAX
            && self.local.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'z' | b'.' | b'-'));
        let code = match &self.state {
            OpState::NotSent { code: Some(c), .. } => {
                !c.is_empty() && c.len() <= MAX_CODE && c.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'_' | b'.' | b'-'))
            }
            _ => true,
        };
        let runs: usize = self.body.blocks.iter().map(|b| b.runs.len()).sum();
        match () {
            _ if !(1..u64::MAX).contains(&self.seq) => Some("a number outside the outbox's"),
            _ if !id_ok(&self.chat) => Some("a chat without a usable id"),
            _ if !local => Some("a local id outside its rules"),
            _ if self.tries > MAX_TRIES => Some("more tries than are made"),
            _ if !code => Some("a code of Teams' answer outside its rules"),
            _ if runs > TEAMS_RUNS => Some("a body of too many runs"),
            _ if teams_write(&self.body).is_err() => Some("a body longer than Teams takes"),
            _ => None,
        }
    }
}

/// Where record `seq` is kept.
fn op_key(seq: u64) -> String {
    format!("{OUTBOX}/{seq:020}.json")
}

/// The number a record's name says: 20 digits, from 1 and short of the last ([`op_key`]).
fn seq_of(name: &str) -> Option<u64> {
    let digits = name.strip_suffix(".json").filter(|d| d.len() == 20 && d.bytes().all(|b| b.is_ascii_digit()))?;
    digits.parse().ok().filter(|seq| (1..u64::MAX).contains(seq))
}

impl Copy {
    /// A message going out kept under its number, or kept again as its state moves. One the
    /// outbox wouldn't read back ([`OpRecord::broken`], or past [`MAX_OP`] bytes) is refused.
    pub(crate) fn put_op(&self, op: &OpRecord) -> Result<()> {
        if let Some(why) = op.broken() {
            return Err(refused(why));
        }
        let bytes = serde_json::to_vec(op).map_err(|source| StoreError::Json { path: OUTBOX.into(), source })?;
        if bytes.len() > MAX_OP {
            return Err(refused("an outbox record past its size"));
        }
        self.store.write_account_files(TEAMS_ACCOUNT, &[(op_key(op.seq), bytes)])
    }

    /// The messages going out, by number: the first [`OUTBOX_MAX`] records, less those passed
    /// over. None when the outbox can't be listed.
    pub(crate) fn ops(&self) -> Vec<OpRecord> {
        let names = self.op_names().unwrap_or_default();
        let mut seqs: Vec<u64> = names.iter().filter_map(|n| seq_of(n)).collect();
        let mut passed = names.len().saturating_sub(seqs.len());
        seqs.sort_unstable();
        seqs.truncate(OUTBOX_MAX);
        let ops: Vec<OpRecord> = seqs
            .into_iter()
            .filter_map(|seq| {
                let op = self.op_at(seq);
                passed = passed.saturating_add(usize::from(op.is_none()));
                op
            })
            .collect();
        if passed > 0 {
            log::warn!("Teams copy: {passed} records of the outbox passed over");
        }
        ops
    }

    /// Record `seq`, when it's sound and under its own name.
    fn op_at(&self, seq: u64) -> Option<OpRecord> {
        let bytes = self.store.account_file(TEAMS_ACCOUNT, &op_key(seq)).ok()??;
        if bytes.len() > MAX_OP {
            return None;
        }
        let op: OpRecord = serde_json::from_slice(&bytes).ok()?;
        (op.seq == seq && op.broken().is_none()).then_some(op)
    }

    /// Record `seq` gone; none there is no error.
    pub(crate) fn remove_op(&self, seq: u64) -> Result<()> {
        self.store.remove_account_files(TEAMS_ACCOUNT, &op_key(seq))
    }

    /// Every record of the outbox gone, whatever it holds; the rest of the copy stays.
    pub(crate) fn clear_ops(&self) -> Result<()> {
        self.store.remove_account_files(TEAMS_ACCOUNT, OUTBOX)
    }

    /// The number of the next message taken: one past the highest under a record's name (read
    /// back or passed over, so none is written over), from 1. A number comes again once the
    /// highest is removed, or the outbox emptied. When the outbox can't be listed, `u64::MAX`: a
    /// number [`Copy::put_op`] refuses, so a message waiting is never written over.
    pub(crate) fn next_seq(&self) -> u64 {
        match self.op_names() {
            Some(names) => names.iter().filter_map(|n| seq_of(n)).max().map_or(1, |seq| seq.saturating_add(1)),
            None => u64::MAX,
        }
    }

    /// The names in the outbox's folder; `None` when it can't be listed.
    fn op_names(&self) -> Option<Vec<String>> {
        match self.store.account_files(TEAMS_ACCOUNT, OUTBOX) {
            Ok(names) => Some(names),
            Err(_) => {
                log::warn!("Teams copy: its outbox couldn't be listed");
                None
            }
        }
    }
}
