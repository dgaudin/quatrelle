// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A file someone shared, saved: its bytes fetched from its download address on a lane's
//! connection, without the token, into a folder of the user's own, under a safe name, never over
//! anything there, readable by the user alone.
//!
//! What the download says of a name is never read: the name is the attachment's own, made safe.
//! The cap is counted while the body comes, whatever its length says, and nothing past what an
//! answer says of its length is written. Whatever goes wrong (a refusal, the cap, the floor, a
//! cancel, the disk, a panic) leaves no file.

use std::cell::Cell;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use quatrelle_mime::sanitize_filename;

use super::*;
use crate::http::{self, Request, Response};
use crate::teams::graph::address;

/// The most characters of a name saved, its number and extension with it: `sanitize_filename`'s.
const NAME_MAX: usize = 180;
/// The longest extension kept apart when a name is cut.
const EXTENSION_MAX: usize = 16;
/// A name taken is tried again with ` (2)` up to ` (999)`.
const NUMBERS: u32 = 999;

/// Save the file at `from` (an address [`transfer_url`](crate::teams::graph::transfer_url) took,
/// for a client whose API is at `base`) in `dir` under `name` made safe: at most `max` bytes, on a
/// connection to an address on the internet ([`connected`]: this computer only for the tests'
/// servers), through [`Watched`]. A redirect or any status but 200 is refused (`HTTP <status>`);
/// past `max` is too big, before a byte is read when its length says so; too slow is stalled; the
/// network away or cancelled is offline; the disk failing is no room.
pub(crate) fn save(
    from: &Url,
    name: &str,
    dir: &Path,
    base: &str,
    cancel: &Arc<AtomicBool>,
    floor: (u64, Duration),
    max: u64,
) -> Result<PathBuf, (FileWhy, Option<String>)> {
    let tail = host_tail(&from.host);
    let got = fetch(from, name, dir, base, cancel, floor, max);
    match &got {
        Ok((_, bytes)) => log::info!("teams: a file downloaded from {tail}: {bytes} bytes"),
        Err((why, code)) => log::info!("teams: a file's download from {tail} stopped: {why:?} {}", code.as_deref().unwrap_or_default()),
    }
    got.map(|(path, _)| path)
}

/// [`save`]: the file's path, and its bytes.
fn fetch(
    from: &Url,
    name: &str,
    dir: &Path,
    base: &str,
    cancel: &Arc<AtomicBool>,
    floor: (u64, Duration),
    max: u64,
) -> Result<(PathBuf, u64), (FileWhy, Option<String>)> {
    if cancel.load(Ordering::SeqCst) {
        return Err((FileWhy::Offline, None));
    }
    let mut s = connected(base, from, cancel, floor).map_err(|e| {
        // An address on this computer or its network is refused; anything else is the network's.
        let why = if matches!(e, NetError::Io(_) | NetError::Tls(_)) { FileWhy::Offline } else { FileWhy::Refused };
        (why, None)
    })?;
    let req = Request::get(&address(from), usize::try_from(max).unwrap_or(usize::MAX));
    let mut partial = Partial(None);
    let (disk, written) = (Cell::new(false), Cell::new(0u64));
    let mut too_big = false;
    let got = http::exchange_into(&mut s, &req, from, |head: &Response| {
        if head.status != 200 {
            return Ok(None);
        }
        if head.header("Content-Length").and_then(|l| l.trim().parse::<u64>().ok()).is_some_and(|n| n > max) {
            too_big = true;
            return Err(NetError::Protocol("a file past its cap".into()));
        }
        let (file, path) = create(dir, name).map_err(|e| {
            disk.set(true);
            NetError::Io(e)
        })?;
        partial.0 = Some(path);
        Ok(Some(Out { file, disk: &disk, written: &written }))
    });
    match got {
        Ok(head) if head.status == 200 => partial.0.take().map(|path| (path, written.get())).ok_or((FileWhy::Refused, None)),
        Ok(head) => Err((FileWhy::Refused, Some(format!("HTTP {}", head.status)))),
        Err(e) => {
            let why = match &e {
                _ if cancel.load(Ordering::SeqCst) => FileWhy::Offline,
                _ if too_big => FileWhy::TooBig,
                NetError::Protocol(m) if m.starts_with("the download is larger") => FileWhy::TooBig,
                _ if disk.get() => FileWhy::NoRoom,
                NetError::Io(io) if io.kind() == io::ErrorKind::TimedOut => FileWhy::Stalled,
                NetError::Io(_) | NetError::Tls(_) | NetError::Closed(_) => FileWhy::Offline,
                _ => FileWhy::Refused,
            };
            Err((why, None))
        }
    }
}

/// The file being written: removed unless it is kept (a failure, a cancel, a panic).
struct Partial(Option<PathBuf>);

impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Where a body goes: the file, its bytes counted, a failure of the disk told from the network's.
struct Out<'a> {
    file: File,
    disk: &'a Cell<bool>,
    written: &'a Cell<u64>,
}

impl Write for Out<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.file.write(buf).inspect_err(|_| self.disk.set(true))?;
        self.written.set(self.written.get().saturating_add(u64::try_from(n).unwrap_or(u64::MAX)));
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush().inspect_err(|_| self.disk.set(true))
    }
}

/// A new file in `dir` for `name`, readable by the user alone: its [`saved_name`], with a number
/// when taken. Never over a file or a link there (`create_new`).
fn create(dir: &Path, name: &str) -> io::Result<(File, PathBuf)> {
    for n in 1..=NUMBERS {
        let path = dir.join(saved_name(name, n));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        match options.open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "every name is taken"))
}

/// The name a file is saved under, the `n`-th tried: `name` made safe (`sanitize_filename`), its
/// extension (up to 16 letters and digits) kept apart, ` (n)` before it from 2 on, all within
/// `sanitize_filename`'s 180 characters, and its own safe name.
pub(super) fn saved_name(name: &str, n: u32) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let (stem, ext) = match last.rsplit_once('.') {
        Some((stem, ext))
            if !stem.trim().is_empty() && (1..=EXTENSION_MAX).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            (stem, Some(ext))
        }
        _ => (last, None),
    };
    let number = if n > 1 { format!(" ({n})") } else { String::new() };
    let room = NAME_MAX.saturating_sub(number.len()).saturating_sub(ext.map_or(0, |e| e.len() + 1));
    // A cut can leave an end to trim or a name Windows keeps: made safe again, twice at most
    // (once it starts with `_`, nothing more is added).
    let mut stem = sanitize_filename(stem);
    for _ in 0..3 {
        if stem.chars().count() <= room {
            break;
        }
        stem = sanitize_filename(&stem.chars().take(room).collect::<String>());
    }
    let whole = match ext {
        Some(ext) => format!("{stem}{number}.{ext}"),
        None => format!("{stem}{number}"),
    };
    if whole.chars().count() <= NAME_MAX && sanitize_filename(&whole) == whole { whole } else { format!("attachment{number}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P-1: one cap, `sanitize_filename`'s, for the whole name with its number; the extension kept;
    /// each its own safe name.
    #[test]
    fn a_saved_name_is_its_own_safe_name_with_its_number_and_extension() {
        let long = format!("{}.pdf", "x".repeat(400));
        let spaced = format!("CON{}x.txt", " ".repeat(200));
        for name in
            ["plan.pdf", "../../x.txt", "CON.txt", "a\u{202E}fdp.exe", "", " .pdf", "archive.tar.gz", "a.verylongextension12345", &long, &spaced]
        {
            for n in [1, 2, 999] {
                let saved = saved_name(name, n);
                assert_eq!(sanitize_filename(&saved), saved, "{name:?} {n}");
                assert!(saved.chars().count() <= NAME_MAX, "{name:?} {n}: {}", saved.chars().count());
            }
        }
        assert_eq!(saved_name("plan.pdf", 1), "plan.pdf");
        assert_eq!(saved_name("plan.pdf", 2), "plan (2).pdf");
        assert_eq!(saved_name("archive.tar.gz", 3), "archive.tar (3).gz");
        assert_eq!(saved_name("notes", 2), "notes (2)");
        assert_eq!(saved_name("", 1), "attachment");
        assert_eq!(saved_name("CON.txt", 1), "_CON.txt");
        assert_eq!(saved_name("CON.txt", 2), "_CON (2).txt");
        let cut = saved_name(&long, 999);
        assert!(cut.ends_with(" (999).pdf") && cut.chars().count() == NAME_MAX, "{cut}");
        assert_eq!(saved_name(&spaced, 1), "_CON.txt", "cut to its first word, a name Windows keeps");
    }
}
