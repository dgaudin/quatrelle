// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Compacting a data file, as Outlook's Compact Now: the space freed inside it (mail deleted,
//! folders no longer kept) goes back to the disk. The file is reused as it is written, but it
//! never shrinks by itself.
//!
//! It needs the file to itself, for as long as it takes (about a minute for 40 GB): from the
//! start, whatever else wants the account's file waits for the end rather than failing, and the
//! file is taken once those already using it have let it go. But the window's thread never waits
//! to read it ([`no_waiting`]): its read fails at once, as the idea of upstream's `0151859a`.

use std::time::{Duration, Instant};

use super::*;

/// How long whoever is using the file may keep it before the compaction gives up, and how long
/// anything else waits for a compaction at most.
const LET_GO: Duration = Duration::from_secs(30);
const WAIT_AT_MOST: Duration = Duration::from_secs(30 * 60);

thread_local! {
    /// This thread mustn't wait for a data file being compacted to read it ([`no_waiting`]).
    static NO_WAITING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The calling thread (the window's) is never kept waiting to read a data file being compacted:
/// the read fails at once ([`io::ErrorKind::WouldBlock`], "being compacted") rather than freezing
/// the window for as long as that takes. A write still waits its turn: it's never refused for
/// that.
pub(crate) fn no_waiting() {
    NO_WAITING.with(|w| w.set(true));
}

/// Why a read from a thread that doesn't wait ([`no_waiting`]) is refused.
fn being_compacted() -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, "this data file is being compacted; it's ready in a moment")
}

/// Marks an account as being compacted until dropped (a panic on the way included).
struct Compacting<'a> {
    packs: &'a Packs,
    account: String,
}

impl Drop for Compacting<'_> {
    fn drop(&mut self) {
        Packs::lock(&self.packs.compacting).remove(&self.account);
        self.packs.compacted.notify_all();
    }
}

impl Packs {
    /// Wait while the account's data file is being compacted; a read (not `write`) from the
    /// window's thread is refused at once instead ([`no_waiting`]).
    pub(super) fn wait_while_compacted(&self, account: &str, write: bool) -> io::Result<()> {
        let began = Instant::now();
        let refuse = !write && NO_WAITING.with(std::cell::Cell::get);
        let mut busy = Packs::lock(&self.compacting);
        while busy.contains(account) {
            if refuse {
                return Err(being_compacted());
            }
            if began.elapsed() > WAIT_AT_MOST {
                return Err(other("the data file is still being compacted"));
            }
            busy = match self.compacted.wait_timeout(busy, Duration::from_secs(1)) {
                Ok((g, _)) => g,
                Err(p) => p.into_inner().0,
            };
        }
        Ok(())
    }

    /// Compact an account's data file: its size before and after; `None` when it has none.
    pub(crate) fn compact(&self, account: &str) -> io::Result<Option<(u64, u64)>> {
        let Some(path) = self.file(account).filter(|f| f.is_file()) else { return Ok(None) };
        let size = || fs::metadata(&path).map_or(0, |m| on_disk(&m));
        let before = size();
        if !Packs::lock(&self.compacting).insert(account.to_string()) {
            return Err(other("the data file is being compacted already"));
        }
        let _mark = Compacting { packs: self, account: account.to_string() };
        // Taken out of the open files: whoever has it finishes what they're doing with it.
        let mut held = Packs::lock(&self.open).remove(account);
        let began = Instant::now();
        let mut db = loop {
            match held.take() {
                None => break open_file(&path).map_err(other)?,
                Some(arc) => match Arc::try_unwrap(arc) {
                    Ok(db) => break db,
                    Err(arc) if began.elapsed() > LET_GO => {
                        Packs::lock(&self.open).insert(account.to_string(), arc);
                        return Err(other("the data file is busy: try again in a moment"));
                    }
                    Err(arc) => {
                        held = Some(arc);
                        std::thread::sleep(Duration::from_millis(20));
                    }
                },
            }
        };
        let began = Instant::now();
        let done = loop {
            match db.compact() {
                Ok(true) => {}
                Ok(false) => break Ok(()),
                Err(e) => break Err(other(e)),
            }
        };
        // The compaction's own commits don't keep the file's free space (redb skips it there), so
        // a crash after them had the whole file gone through as it reopened, on the window's
        // thread. A sealed commit after them (as `Packs::change` makes) grew a file compacted
        // tight, which redb doubles under 4 GiB: 306 MB kept ended in 623 MB. Closed instead,
        // the file records its free space as redb closes one, from its lowest pages and trimmed
        // after (redb's issue 1165), whether the compaction went to the end or not; it's opened
        // again at once, with nothing to check.
        drop(db);
        let reopened = open_file(&path).map_err(other);
        let after = size();
        log::info!("{}: compacted from {before} to {after} bytes in {:.1} s", path.display(), began.elapsed().as_secs_f32());
        // One that can't be opened again is opened (or set aside) by whatever wants it next.
        let done = match reopened {
            Ok(db) => {
                Packs::lock(&self.open).insert(account.to_string(), Arc::new(db));
                done
            }
            Err(e) => done.and(Err(e)),
        };
        done.map(|()| Some((before, after)))
    }
}
