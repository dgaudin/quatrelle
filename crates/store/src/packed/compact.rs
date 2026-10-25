//! Compacting a data file, as Outlook's Compact Now: the space freed inside it (mail deleted,
//! folders no longer kept) goes back to the disk. The file is reused as it is written, but it
//! never shrinks by itself.
//!
//! It needs the file to itself, for as long as it takes (about a minute for 40 GB): from the
//! start, whatever else wants the account's file waits for the end rather than failing, and the
//! file is taken once those already using it have let it go.

use std::time::{Duration, Instant};

use super::*;

/// How long whoever is using the file may keep it before the compaction gives up, and how long
/// anything else waits for a compaction at most.
const LET_GO: Duration = Duration::from_secs(30);
const WAIT_AT_MOST: Duration = Duration::from_secs(30 * 60);

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
    /// Wait while the account's data file is being compacted.
    pub(super) fn wait_while_compacted(&self, account: &str) -> io::Result<()> {
        let began = Instant::now();
        let mut busy = Packs::lock(&self.compacting);
        while busy.contains(account) {
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
        let size = || fs::metadata(&path).map_or(0, |m| m.len());
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
                None => break builder(&path).create(&path).map_err(other)?,
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
        let after = size();
        log::info!("{}: compacted from {before} to {after} bytes in {:.1} s", path.display(), began.elapsed().as_secs_f32());
        Packs::lock(&self.open).insert(account.to_string(), Arc::new(db));
        done.map(|()| Some((before, after)))
    }
}
