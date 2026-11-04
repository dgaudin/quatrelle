// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! How a file is kept in a data file: compressed when that helps, whole or in pieces.

use super::*;

/// The main table: path inside the account's folder → bytes.
pub(super) const FILES: TableDefinition<&str, &[u8]> = TableDefinition::new("files");
/// Big files in pieces: (path, piece) → bytes. The database gives a value whole runs of pages,
/// doubling (a 300 KB message took 512 KB, and imported mail half again its size in waste), so a
/// big file goes in pieces that each fill their pages, and `FILES` keeps a note of it.
pub(super) const PIECES: TableDefinition<(&str, u32), &[u8]> = TableDefinition::new("pieces");
/// The note in `FILES` for a file kept in pieces: this, then its length (8 bytes, little-endian).
/// Nothing Quatrelle keeps starts with a NUL.
pub(super) const PIECED: &[u8] = b"\0Quatrelle:pieces\0";

/// The note as the fork wrote it before it took its name (see `legacy`): a data file keeps it in
/// its own bytes, so the rename can't simply replace them, and both are read.
const PIECED_OLD: &[u8] = crate::legacy::OLD_PIECED;
/// Files up to this size (as kept: compressed when that helps) stay whole: one page's worth. A
/// bigger value took the next power of two of pages, so it goes in pieces.
pub(super) const WHOLE_MAX: usize = PAGE - PAGE_SLACK;
/// A file kept compressed starts with this, then its deflate stream.
pub(super) const DEFLATED: &[u8] = b"\0Quatrelle:deflate\0";

/// The same, as the fork wrote it before it took its name (see `legacy`): read too.
const DEFLATED_OLD: &[u8] = crate::legacy::OLD_DEFLATED;
/// Files smaller than this aren't worth compressing.
const DEFLATE_MIN: usize = 1024;
/// The most a compressed file may come to (a crafted data file can't fill the memory).
pub(super) const INFLATE_MAX: u64 = 1 << 30;
/// The most set aside at once for a file, before it's read: a file in pieces has a note that says
/// how long it is, and a crafted one can say 1 GiB with no pieces at all.
const RESERVE_MAX: usize = 64 << 20;

/// A file as kept: compressed (fast deflate) when that saves a tenth or more.
///
/// miniz_oxide is called directly rather than through flate2: in the app, another crate turns
/// flate2's zlib-rs backend on for everyone, and its fastest level (static Huffman tables) kept
/// base64 attachments at 0.995 of their size, so they were stored raw; miniz's level 1 keeps
/// them at about 0.76, and is faster. The stream is the same raw deflate either way.
pub(super) fn pack(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if bytes.len() < DEFLATE_MIN || bytes.starts_with(DEFLATED) || bytes.starts_with(DEFLATED_OLD) {
        return bytes.into();
    }
    let stream = miniz_oxide::deflate::compress_to_vec(bytes, 1);
    let kept = DEFLATED.len().saturating_add(stream.len());
    if kept >= bytes.len() - bytes.len() / 10 {
        return bytes.into();
    }
    let mut out = Vec::with_capacity(kept);
    out.extend_from_slice(DEFLATED);
    out.extend_from_slice(&stream);
    out.into()
}

/// A file as kept, back as it was written.
pub(super) fn unpack(kept: Vec<u8>) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let Some(stream) = kept.strip_prefix(DEFLATED).or_else(|| kept.strip_prefix(DEFLATED_OLD)) else { return Ok(kept) };
    let mut out = Vec::with_capacity(stream.len().saturating_mul(3).min(RESERVE_MAX));
    flate2::read::DeflateDecoder::new(stream).take(INFLATE_MAX + 1).read_to_end(&mut out)?;
    if out.len() as u64 > INFLATE_MAX {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "a compressed file is too big"));
    }
    Ok(out)
}
/// The database's page, and what a page run needs besides the value (its header and the key).
pub(super) const PAGE: usize = 4096;
pub(super) const PAGE_SLACK: usize = 512;

/// The sizes a file of `len` bytes is cut into: pieces filling 16 pages, then the rest in pieces
/// filling 8, 4, 2 and 1 (the last one smaller).
pub(super) fn piece_sizes(len: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut left = len;
    while left > 0 {
        let fits = [16usize, 8, 4, 2, 1].into_iter().map(|pages| pages * PAGE - PAGE_SLACK).find(|cap| *cap <= left);
        let n = fits.unwrap_or(left).min(left);
        out.push(n);
        left -= n;
    }
    out
}

/// A file kept in pieces: its length.
pub(super) fn pieced_len(note: &[u8]) -> Option<u64> {
    let rest = note.strip_prefix(PIECED).or_else(|| note.strip_prefix(PIECED_OLD))?;
    Some(u64::from_le_bytes(rest.get(..8)?.try_into().ok()?))
}

/// An account's tables in a write transaction: files whole or in pieces.
pub(super) struct Tables<'t> {
    pub(super) files: redb::Table<'t, &'static str, &'static [u8]>,
    pub(super) pieces: redb::Table<'t, (&'static str, u32), &'static [u8]>,
}

impl Tables<'_> {
    pub(super) fn has(&self, key: &str) -> io::Result<bool> {
        Ok(self.files.get(key).map_err(other)?.is_some())
    }

    /// Keep `bytes` at `key` (replacing what was there), compressed when that helps.
    pub(super) fn put(&mut self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.put_kept(key, &pack(bytes))
    }

    /// Keep bytes already as kept (compressed or not), whole or in pieces.
    pub(super) fn put_kept(&mut self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.drop_pieces(key)?;
        if bytes.len() <= WHOLE_MAX {
            self.files.insert(key, bytes).map_err(other)?;
            return Ok(());
        }
        let mut note = PIECED.to_vec();
        note.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        self.files.insert(key, note.as_slice()).map_err(other)?;
        let mut at = 0;
        for (i, n) in piece_sizes(bytes.len()).into_iter().enumerate() {
            let piece = bytes.get(at..at + n).ok_or_else(|| other("piece out of range"))?;
            self.pieces.insert((key, u32::try_from(i).map_err(other)?), piece).map_err(other)?;
            at += n;
        }
        Ok(())
    }

    /// Take the file at `key` out: its bytes as kept (still compressed), when it was there.
    pub(super) fn take_kept(&mut self, key: &str) -> io::Result<Option<Vec<u8>>> {
        let Some(v) = self.files.remove(key).map_err(other)?.map(|g| g.value().to_vec()) else { return Ok(None) };
        let Some(len) = pieced_len(&v) else { return Ok(Some(v)) };
        let mut out = Vec::with_capacity(usize::try_from(len).unwrap_or(0).min(RESERVE_MAX));
        for (_, piece) in self.pieces.extract_from_if((key, 0)..=(key, u32::MAX), |_, _| true).map_err(other)?.flatten() {
            out.extend_from_slice(piece.value());
        }
        Ok(Some(out))
    }

    fn drop_pieces(&mut self, key: &str) -> io::Result<()> {
        self.pieces.retain_in((key, 0)..=(key, u32::MAX), |_, _| false).map_err(other)
    }
}

/// A file from a read transaction: whole, or put together from its pieces, and uncompressed.
///
/// A compressed file in pieces is uncompressed as its pieces are read, never held compressed
/// whole besides: a source of 30 MB kept in 23 MB held both at once while it was uncompressed,
/// and the search index's readers, several such sources at a time, held most of what its pass
/// added to the app's memory.
pub(super) fn read_file(
    tx: &redb::ReadTransaction,
    files: &impl ReadableTable<&'static str, &'static [u8]>,
    key: &str,
) -> io::Result<Option<Vec<u8>>> {
    use std::io::Read;
    let Some(v) = files.get(key).map_err(other)?.map(|g| g.value().to_vec()) else { return Ok(None) };
    let Some(len) = pieced_len(&v) else { return unpack(v).map(Some) };
    let pieces = tx.open_table(PIECES).map_err(other)?;
    let mut reader = PieceReader { pieces: pieces.range((key, 0)..=(key, u32::MAX)).map_err(other)?, piece: None, at: 0, read: 0, failed: false };
    // Whether it's compressed: its first bytes, read before anything else (both prefixes are
    // the same length, so either is told from the same bytes).
    let mut head = Vec::with_capacity(DEFLATED.len());
    (&mut reader).take(DEFLATED.len() as u64).read_to_end(&mut head)?;
    let out = if head == DEFLATED || head == DEFLATED_OLD {
        let mut out = Vec::with_capacity(usize::try_from(len).unwrap_or(0).saturating_mul(3).min(RESERVE_MAX));
        let inflated = flate2::read::DeflateDecoder::new(&mut reader).take(INFLATE_MAX + 1).read_to_end(&mut out);
        if reader.failed {
            return Err(inflated.err().unwrap_or_else(|| io::Error::other("a piece couldn't be read")));
        }
        // What follows the stream, if anything, counted with the rest: a piece missing is told
        // as such whether or not the stream ended well without it.
        io::copy(&mut reader, &mut io::sink())?;
        if reader.read != len {
            return Err(pieces_missing(key));
        }
        inflated?;
        if out.len() as u64 > INFLATE_MAX {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "a compressed file is too big"));
        }
        out
    } else {
        let mut out = Vec::with_capacity(usize::try_from(len).unwrap_or(0).min(RESERVE_MAX));
        out.extend_from_slice(&head);
        reader.read_to_end(&mut out)?;
        if reader.read != len {
            return Err(pieces_missing(key));
        }
        out
    };
    Ok(Some(out))
}

fn pieces_missing(key: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{key}: pieces missing"))
}

/// A file's pieces read one after another, as one stream of its bytes as kept, counted.
struct PieceReader<'a> {
    pieces: redb::Range<'a, (&'static str, u32), &'static [u8]>,
    piece: Option<redb::AccessGuard<'a, &'static [u8]>>,
    /// How far into `piece`.
    at: usize,
    /// Bytes read, all told.
    read: u64,
    /// A piece couldn't be read (the error was given).
    failed: bool,
}

impl io::Read for PieceReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if let Some(p) = &self.piece {
                let rest = p.value().get(self.at..).unwrap_or_default();
                if !rest.is_empty() {
                    let n = rest.len().min(buf.len());
                    if let (Some(to), Some(from)) = (buf.get_mut(..n), rest.get(..n)) {
                        to.copy_from_slice(from);
                    }
                    self.at = self.at.saturating_add(n);
                    self.read = self.read.saturating_add(n as u64);
                    return Ok(n);
                }
            }
            match self.pieces.next() {
                Some(Ok((_, piece))) => {
                    self.piece = Some(piece);
                    self.at = 0;
                }
                Some(Err(e)) => {
                    self.failed = true;
                    return Err(other(e));
                }
                None => {
                    self.piece = None;
                    return Ok(0);
                }
            }
        }
    }
}
