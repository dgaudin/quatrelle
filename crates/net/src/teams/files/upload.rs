// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! A file's bytes sent up to an upload session: ranges of [`CHUNK`] in order, each on a
//! connection of its own, without the token. A range cut without an answer, or failed by Graph's
//! server, is taken up where Graph says, after a wait that doubles; one answered "wait" (429, 503)
//! with a `Retry-After` is taken up once it is over, waited whole on the lane (nothing of the
//! worker waits); [`RESUMES`] times in all. What Graph says of where the upload stands is taken
//! only forward and within what was sent, and read [`STATUS_READS`] times at most. The file is
//! sent only as it was when attached. Cancelled, the session is cancelled too (best effort).

use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

use super::*;
use crate::teams::graph::{CHUNK, RangeAnswer, TransferFail, cancel_upload_on, put_range_on, upload_status_on};
use crate::teams::limits::retry_wait;

/// The times an upload is taken up after a range or a status read cut or asked to wait, in all.
pub(crate) const RESUMES: u32 = 5;
/// The most reads of where an upload stands.
const STATUS_READS: u32 = 64;
/// The wait before going on after the first cut, doubled after each (a hundredth of it in tests).
const RESUME_WAIT: Duration = if cfg!(test) { Duration::from_millis(20) } else { Duration::from_secs(2) };
/// The slices a wait is looked at the cancel in.
const SLICE: Duration = Duration::from_millis(250);
/// How long the cancel of a session waits for a byte (best effort: the upload is given up).
const CANCEL_IO: Duration = Duration::from_secs(2);

/// When a file was last changed, as the outbox keeps it to tell a file changed since: milliseconds
/// since 1970 (0 when the system can't say).
pub(crate) fn modified_of(meta: &Metadata) -> i64 {
    let Ok(t) = meta.modified() else { return 0 };
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_millis()).map_or(i64::MIN, |ms| -ms),
    }
}

/// Send `source` up to `session` (for a client whose API is at `base`): the item OneDrive made.
/// `progress` hears the next byte OneDrive expects after each range but the last. The file must be
/// a regular file of the size and time `source` says, before each range, else [`UploadFail::Gone`];
/// the session gone is [`UploadFail::SessionGone`]; another refusal keeps Graph's answer; cut or
/// failed past the resumes, or answers that don't add up, is [`UploadFail::Network`].
pub(crate) fn send_up(
    session: &UploadSession,
    source: &Source,
    base: &str,
    cancel: &Arc<AtomicBool>,
    floor: (u64, Duration),
    progress: &dyn Fn(u64),
) -> Result<String, UploadFail> {
    let Source::File { path, size, modified } = source;
    let tail = host_tail(&session.url().host);
    let mut up = Up { session, base, cancel, floor, total: *size, next: 0, acked: 0, sent_to: 0, resumes: 0, reads: 0 };
    let got = up.run(path, *modified, progress);
    match &got {
        Ok(_) => log::info!("teams: a file uploaded to {tail}: {size} bytes, {} status reads", up.reads),
        Err(why) => log::info!("teams: an upload to {tail} stopped after {} bytes: {why:?}", up.acked),
    }
    if got == Err(UploadFail::Cancelled) {
        up.cancel_session();
    }
    got
}

/// An upload under way.
struct Up<'a> {
    session: &'a UploadSession,
    base: &'a str,
    cancel: &'a Arc<AtomicBool>,
    floor: (u64, Duration),
    total: u64,
    /// The next byte to send; the furthest OneDrive said it has; the end of the furthest range sent.
    next: u64,
    acked: u64,
    sent_to: u64,
    resumes: u32,
    reads: u32,
}

/// How a request at the session failed.
enum Failed {
    /// Without an answer, or Graph's server failing: asked where it stands, after a wait.
    Cut,
    /// Asked to wait (429, 503) for this long (its `Retry-After`): asked where it stands after it.
    Busy(Duration),
    /// A range already there (416): asked where it stands.
    Ask,
    Stop(UploadFail),
}

impl Up<'_> {
    fn run(&mut self, path: &Path, modified: i64, progress: &dyn Fn(u64)) -> Result<String, UploadFail> {
        let mut file = opened(path, self.total, modified)?;
        while self.next < self.total {
            self.going()?;
            unchanged(&file, self.total, modified)?;
            let end = self.next.saturating_add(u64::try_from(CHUNK).unwrap_or(u64::MAX)).min(self.total);
            let bytes = read_at(&mut file, self.next, end)?;
            self.sent_to = self.sent_to.max(end);
            let put = self
                .connected()
                .and_then(|mut s| put_range_on(&mut s, self.session.url(), &bytes, self.next, self.total).map_err(|e| self.failed(&e)));
            match put {
                Ok(RangeAnswer::Done(item)) => return Ok(item),
                Ok(RangeAnswer::Next(n)) => {
                    self.forward(n)?;
                    progress(n);
                }
                Err(Failed::Stop(why)) => return Err(why),
                Err(failed) => self.resume(failed)?,
            }
        }
        // Every byte in, and no file said made.
        Err(UploadFail::Network)
    }

    /// Cancelled: said so.
    fn going(&self) -> Result<(), UploadFail> {
        if self.cancel.load(Ordering::SeqCst) { Err(UploadFail::Cancelled) } else { Ok(()) }
    }

    /// OneDrive expects `n` next: only forward of what it said, and within what was sent.
    fn forward(&mut self, n: u64) -> Result<(), UploadFail> {
        if n < self.acked || n > self.sent_to {
            log::warn!("teams: an upload's next range went back or past what was sent");
            return Err(UploadFail::Network);
        }
        (self.next, self.acked) = (n, n);
        Ok(())
    }

    /// Where the upload stands, read once more, after `failed`: after a cut, each read a resume of
    /// [`RESUMES`], then a wait that doubles; asked to wait, its wait whole first, a resume too;
    /// after a range already there, at once. Gone on from where Graph says.
    fn resume(&mut self, failed: Failed) -> Result<(), UploadFail> {
        let mut failed = failed;
        loop {
            if !matches!(failed, Failed::Ask) {
                self.resumes = self.resumes.saturating_add(1);
                if self.resumes > RESUMES {
                    return Err(UploadFail::Network);
                }
            }
            if let Failed::Busy(wait) = failed {
                self.wait(wait)?;
            }
            self.reads = self.reads.saturating_add(1);
            if self.reads > STATUS_READS {
                return Err(UploadFail::Network);
            }
            let read = self.connected().and_then(|mut s| upload_status_on(&mut s, self.session.url(), self.total).map_err(|e| self.failed(&e)));
            match read {
                Ok(n) => {
                    self.forward(n)?;
                    if matches!(failed, Failed::Cut) {
                        self.wait(self.backoff())?;
                    }
                    return Ok(());
                }
                Err(Failed::Cut) => {
                    failed = Failed::Cut;
                    self.wait(self.backoff())?;
                }
                Err(Failed::Ask) => return Err(UploadFail::Network),
                Err(Failed::Stop(why)) => return Err(why),
                Err(busy) => failed = busy,
            }
        }
    }

    /// The wait after the `resumes`-th cut: 2 s, doubled each time.
    fn backoff(&self) -> Duration {
        RESUME_WAIT.saturating_mul(1u32.checked_shl(self.resumes.saturating_sub(1)).unwrap_or(u32::MAX))
    }

    /// `wait`, in slices that look at the cancel.
    fn wait(&self, wait: Duration) -> Result<(), UploadFail> {
        let until = Instant::now().checked_add(wait).unwrap_or_else(Instant::now);
        while Instant::now() < until {
            self.going()?;
            std::thread::sleep(SLICE.min(until.saturating_duration_since(Instant::now())));
        }
        self.going()
    }

    /// A connection of its own to the session's server, watched. An address on this computer or
    /// its network is refused; the network away is a cut.
    fn connected(&self) -> Result<Watched<Stream>, Failed> {
        connected(self.base, self.session.url(), self.cancel, self.floor).map_err(|e| match e {
            NetError::Io(_) | NetError::Tls(_) => Failed::Cut,
            _ => Failed::Stop(UploadFail::Refused(None)),
        })
    }

    /// What a failed request at the session means.
    fn failed(&self, TransferFail(e, retry_after): &TransferFail) -> Failed {
        if self.cancel.load(Ordering::SeqCst) {
            return Failed::Stop(UploadFail::Cancelled);
        }
        match e {
            // Asked to wait, and for how long: waited whole (an hour at most).
            _ if busy(e) && retry_after.is_some() => Failed::Busy(retry_wait(*retry_after)),
            NetError::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => Failed::Stop(UploadFail::Stalled),
            NetError::Refused(m) if m.contains("session is gone") => Failed::Stop(UploadFail::SessionGone),
            NetError::Refused(_) => Failed::Stop(UploadFail::Refused(None)),
            NetError::Protocol(m) if m == "HTTP 416" => Failed::Ask,
            NetError::Protocol(_) if answer_code(e).is_some() => Failed::Stop(UploadFail::Refused(answer_code(e))),
            // The network, a server's error, or an answer that isn't one: asked where it stands.
            _ => Failed::Cut,
        }
    }

    /// The session cancelled at its address, best effort: on a connection of its own, not
    /// watched (the upload's cancel is set), its waits short.
    fn cancel_session(&self) {
        let url = self.session.url();
        let tail = host_tail(&url.host);
        let done = transfer_stream_at(self.base, url).and_then(|mut s| {
            let sock = match &s {
                Stream::Plain(t) => t,
                Stream::Tls(t) => &t.sock,
            };
            sock.set_read_timeout(Some(CANCEL_IO))?;
            sock.set_write_timeout(Some(CANCEL_IO))?;
            cancel_upload_on(&mut s, url)
        });
        match done {
            Ok(()) => log::info!("teams: an upload's session at {tail} cancelled"),
            Err(e) => log::info!("teams: an upload's session at {tail} couldn't be cancelled: {}", kind(&e)),
        }
    }
}

/// The file, opened, when it is a regular file of the size and time it had when attached.
fn opened(path: &Path, size: u64, modified: i64) -> Result<File, UploadFail> {
    // Looked at before it is opened: a pipe or a device would hold the open.
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return Err(UploadFail::Gone);
    }
    let file = File::open(path).map_err(|_| UploadFail::Gone)?;
    unchanged(&file, size, modified)?;
    Ok(file)
}

/// The file open is still a regular file of that size and time, and not empty.
fn unchanged(file: &File, size: u64, modified: i64) -> Result<(), UploadFail> {
    match file.metadata() {
        Ok(m) if m.is_file() && size > 0 && m.len() == size && modified_of(&m) == modified => Ok(()),
        _ => Err(UploadFail::Gone),
    }
}

/// The file's bytes from `from` to `end`.
fn read_at(file: &mut File, from: u64, end: u64) -> Result<Vec<u8>, UploadFail> {
    let len = usize::try_from(end.saturating_sub(from)).map_err(|_| UploadFail::Gone)?;
    let mut bytes = vec![0u8; len];
    file.seek(SeekFrom::Start(from)).and_then(|_| file.read_exact(&mut bytes)).map_err(|_| UploadFail::Gone)?;
    Ok(bytes)
}
