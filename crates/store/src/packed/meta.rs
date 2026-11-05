// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A data file's `META` table, where the file says how it was written: whether everything in it
//! is kept as this build keeps it (so Compact Now needn't look for what an earlier build kept
//! raw), and where compressing it again got to, so a run stopped, slept through or killed goes
//! on from there.
//!
//! It's read from the disk, so it's hostile like the rest: a value that's missing, or can't be
//! read as what it should be, is as if it weren't there, and the run starts from the beginning.

use super::*;

/// How the data file was written, a few values by name. Old files have none, and old builds
/// never open it; `FILES` alone is walked for paths, so none of this is ever taken for one.
pub(super) const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
/// The layout of `META` (a u64): written with every change to it.
const VERSION: &str = "version";
const VERSION_NOW: u64 = 1;
/// Which compression what's kept was written with (a u64): [`DEFLATE_NOW`] once every value is.
pub(super) const DEFLATE: &str = "deflate";
/// Every value written by a build that calls miniz itself (`pack`), or compressed again.
pub(super) const DEFLATE_NOW: u64 = 2;
/// Where a recompress that was stopped got to: the last key it looked at (UTF-8).
pub(super) const CURSOR: &str = "recompress.cursor";
/// How many files it had looked at up to there (a u64): where its progress starts again.
pub(super) const LOOKED: &str = "recompress.looked";
/// The longest key a cursor can be (a store path is at most 512 bytes).
const CURSOR_MAX: usize = 1024;
/// What recompressing has found over every run of it, each a u64 of bytes as they were kept: the
/// files tried that stayed as they were (they don't compress: pictures, archives), and what the
/// others came down by.
pub(super) const STAYED_RAW: &str = "recompress.raw";
pub(super) const SAVED: &str = "recompress.saved";
/// Files kept raw that recompressing passed over, a u64 of their bytes as kept: damaged ones, and
/// those bigger than a run takes.
pub(super) const PASSED: &str = "recompress.passed";
/// Where a file kept raw was moved to while a recompress was under way or stopped (a key it may
/// have passed already): the key before it (UTF-8; empty, the start). The run goes on from there
/// rather than from its cursor, so what was moved behind it is compressed too.
pub(super) const BACK: &str = "recompress.back";

/// What `META` says, each value read on its own.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Meta {
    pub(super) deflate: u64,
    pub(super) cursor: Option<String>,
    pub(super) looked: u64,
    pub(super) stayed_raw: u64,
    pub(super) saved: u64,
    pub(super) passed: u64,
}

/// A u64 as kept (8 bytes, little-endian).
fn u64_of(v: &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(v.get(..8)?.try_into().ok()?))
}

/// A key as kept: UTF-8, not empty, not longer than a store path can be.
fn key_of(v: Vec<u8>) -> Option<String> {
    Some(v).filter(|c| !c.is_empty() && c.len() <= CURSOR_MAX).and_then(|c| String::from_utf8(c).ok())
}

/// The data file's `META`. One that isn't there (an old file), or isn't a table of this kind (a
/// crafted file), says nothing.
pub(super) fn read_meta(tx: &redb::ReadTransaction) -> io::Result<Meta> {
    let table = match tx.open_table(META) {
        Ok(t) => t,
        Err(redb::TableError::Storage(e)) => return Err(other(e)),
        Err(_) => return Ok(Meta::default()),
    };
    let value = |key: &str| -> io::Result<Option<Vec<u8>>> { Ok(table.get(key).map_err(other)?.map(|g| g.value().to_vec())) };
    Ok(Meta {
        deflate: value(DEFLATE)?.as_deref().and_then(u64_of).unwrap_or(0),
        cursor: value(CURSOR)?.and_then(key_of),
        looked: value(LOOKED)?.as_deref().and_then(u64_of).unwrap_or(0),
        stayed_raw: value(STAYED_RAW)?.as_deref().and_then(u64_of).unwrap_or(0),
        saved: value(SAVED)?.as_deref().and_then(u64_of).unwrap_or(0),
        passed: value(PASSED)?.as_deref().and_then(u64_of).unwrap_or(0),
    })
}

/// A file kept raw moved to `key` (by `tx`, `files` its `FILES`): a recompress under way
/// (`running`) or stopped goes on from before it ([`BACK`]), in a file not yet all kept as this
/// build keeps it. A `META` that can't be opened (a crafted file) says nothing to do.
pub(super) fn pull_back(
    tx: &redb::WriteTransaction,
    files: &redb::Table<'_, &'static str, &'static [u8]>,
    key: &str,
    running: bool,
) -> io::Result<()> {
    let (deflate, stopped, back) = match tx.open_table(META) {
        Ok(t) => {
            let value = |k: &str| -> io::Result<Option<Vec<u8>>> { Ok(t.get(k).map_err(other)?.map(|g| g.value().to_vec())) };
            (value(DEFLATE)?.as_deref().and_then(u64_of).unwrap_or(0), value(CURSOR)?.is_some(), value(BACK)?)
        }
        Err(_) => return Ok(()),
    };
    if deflate >= DEFLATE_NOW || !(running || stopped) {
        return Ok(());
    }
    let before = files.range::<&str>(..key).map_err(other)?.next_back().transpose().map_err(other)?.map(|(k, _)| k.value().to_string());
    let mut to = before.unwrap_or_default();
    if let Some(b) = back.map(|b| String::from_utf8_lossy(&b).into_owned()).filter(|b| *b < to) {
        to = b;
    }
    write_meta(tx, &[(BACK, to.as_bytes())])
}

/// [`BACK`] taken out of `META`, when a move put it there.
pub(super) fn take_back(tx: &redb::WriteTransaction) -> io::Result<Option<String>> {
    let mut table = tx.open_table(META).map_err(other)?;
    let back = table.remove(BACK).map_err(other)?.map(|g| String::from_utf8_lossy(g.value()).into_owned());
    // A crafted one longer than a key can be: from the start.
    Ok(back.map(|b| if b.len() > CURSOR_MAX { String::new() } else { b }))
}

/// Where a run goes on from, after `cursor` (`None`: the start), once a move pulled it back to
/// after `back` (empty: the start): whichever comes first.
pub(super) fn earlier(cursor: Option<String>, back: Option<String>) -> Option<String> {
    match (cursor, back) {
        (c, None) => c,
        (None, Some(_)) => None,
        (Some(_), Some(b)) if b.is_empty() => None,
        (Some(c), Some(b)) => Some(c.min(b)),
    }
}

/// The cursor in `META` set to `cursor`, with how many files were looked at up to it (`None`:
/// taken out, a run from the start).
pub(super) fn put_cursor(tx: &redb::WriteTransaction, cursor: Option<&str>, looked: u64) -> io::Result<()> {
    match cursor {
        Some(c) => write_meta(tx, &[(CURSOR, c.as_bytes()), (LOOKED, looked.to_le_bytes().as_slice())]),
        None => {
            let mut t = tx.open_table(META).map_err(other)?;
            t.remove(CURSOR).map_err(other)?;
            t.remove(LOOKED).map(drop).map_err(other)
        }
    }
}

/// Values written to `META` in the transaction `tx` (a u64 as 8 bytes), with its version.
pub(super) fn write_meta(tx: &redb::WriteTransaction, values: &[(&str, &[u8])]) -> io::Result<()> {
    let mut table = tx.open_table(META).map_err(other)?;
    table.insert(VERSION, VERSION_NOW.to_le_bytes().as_slice()).map_err(other)?;
    for (k, v) in values {
        table.insert(*k, *v).map_err(other)?;
    }
    Ok(())
}

/// A data file just made says, in a sealed commit of its own, that everything in it is kept as
/// this build keeps it: Compact Now has nothing to compress again in it. One that can't is only
/// looked through once by the first Compact Now.
pub(super) fn mark_new(db: &Database, path: &Path) {
    let done = db.begin_write().map_err(other).and_then(|mut tx| {
        tx.set_quick_repair(true);
        write_meta(&tx, &[(DEFLATE, DEFLATE_NOW.to_le_bytes().as_slice())])?;
        tx.commit().map_err(other)
    });
    if let Err(e) = done {
        log::warn!("{}: made, but couldn't say how it's kept yet: {e}", path.display());
    }
}
