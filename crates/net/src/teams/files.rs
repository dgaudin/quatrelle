// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The Teams worker's files: a file someone shared in a chat, downloaded through Graph and saved
//! for the user alone, and a file's bytes sent up to an upload session of the user's OneDrive.
//!
//! * Each transfer runs on a lane, a thread started at its first use (`teams-download`,
//!   `teams-upload`), one transfer at a time a lane, behind `fenced`. A lane is handed an address
//!   that carries its own authorisation and nothing else: the token never leaves the worker's
//!   thread.
//! * A file asked for is the copy's: its message, of that chat, not deleted, carrying a file of
//!   that id ([`open`]). The worker asks Graph for its download address with the token ([`step`]);
//!   the lane fetches it without, from Microsoft's file hosts only, on an address on the internet
//!   ([`transfer_stream_at`]), and saves it under a safe name in a folder of the user's own
//!   ([`save`]). One runs and [`DOWNLOADS_WAITING`] wait; a file asked again before it is said is
//!   asked once.
//! * A lane's connection reads and writes in slices of [`LANE_IO`], watched ([`Watched`]): a
//!   transfer cancelled (switched off, signed out) or slower than [`FLOOR`] over [`WINDOW`] is
//!   given up. The worker never waits for a lane: a cancelled transfer is let go at once and its
//!   late word dropped.
//! * Nothing an address carries reaches an event, an error or the log: the log says a file by its
//!   chat's keyed hash, sizes, statuses and a host's last two labels ([`host_tail`]).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{Attachment, Event, FILE_MAX, FileOutcome, FileWhy};

use super::backfill::room;
use super::graph::{UploadSession, host_tail, transfer_stream_at};
use super::limits::Route;
use super::outbox::answer_code;
use super::store::id_ok;
use super::worker::{Worker, busy, kind, of_the_network};
use crate::NetError;
use crate::http::Url;
use crate::stream::Stream;
use crate::worker::connect::fenced;

mod download;
mod upload;
mod watched;

pub(crate) use download::save;
pub(crate) use upload::modified_of;
pub(crate) use upload::send_up;
use watched::Watched;

/// The least a transfer moves, in bytes a second over [`WINDOW`]: slower, it is given up.
pub(crate) const FLOOR: u64 = 16 << 10;
pub(crate) const WINDOW: Duration = Duration::from_secs(60);
/// The downloads that wait behind the one that runs, at most.
pub(crate) const DOWNLOADS_WAITING: usize = 8;
/// How long a lane's connection waits for a byte before the cancel and the floor are looked at
/// again.
const LANE_IO: Duration = Duration::from_millis(250);
/// How soon the worker looks again while a transfer runs or a file waits.
const LOOK: Duration = Duration::from_millis(200);

/// A file asked for: its chat, its message and its attachment's id.
pub(crate) type FileKey = (String, String, String);

/// What goes up: a file by its path, as it was when attached (its size, and when it was last
/// changed: [`modified_of`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    File { path: PathBuf, size: u64, modified: i64 },
}

/// Why an upload stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UploadFail {
    /// The file is no longer as it was attached: changed, gone, or not a file.
    Gone,
    /// The upload session is gone: the upload starts over in a new one.
    SessionGone,
    /// Slower than the floor.
    Stalled,
    /// The network, or Graph's answers that don't add up.
    Network,
    /// Refused, with Graph's answer when it gave one.
    Refused(Option<String>),
    Cancelled,
}

/// What a lane says. None carries an address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Done {
    Saved {
        key: FileKey,
        path: PathBuf,
    },
    NotSaved {
        key: FileKey,
        why: FileWhy,
        code: Option<String>,
    },
    /// The bytes of a part gone up so far.
    Sent {
        seq: u64,
        part: usize,
        sent: u64,
    },
    /// A part gone up: the item OneDrive made.
    Uploaded {
        seq: u64,
        part: usize,
        item: String,
    },
    NotUploaded {
        seq: u64,
        part: usize,
        why: UploadFail,
    },
}

/// A file asked for, waiting for its download address: what the copy's message says of it.
struct Wanted {
    key: FileKey,
    url: String,
    name: String,
}

/// A transfer handed to a lane, running or waiting its turn there: its ticket (what its words
/// come back with), what it is, and its cancel flag.
struct Flight<K> {
    ticket: u64,
    key: K,
    cancel: Arc<AtomicBool>,
}

/// A lane's word: its transfer's ticket, and what it says.
type Word = (u64, Done);

/// The two lanes, what they run or hold, and the files waiting for their download address.
pub(crate) struct Transfers {
    waiting: VecDeque<Wanted>,
    downloads: Vec<Flight<FileKey>>,
    uploads: Vec<Flight<(u64, usize)>>,
    down: Option<Sender<Fetch>>,
    up: Option<Sender<Put>>,
    tickets: u64,
    words: (Sender<Word>, Receiver<Word>),
}

impl Default for Transfers {
    fn default() -> Self {
        Transfers { waiting: VecDeque::new(), downloads: Vec::new(), uploads: Vec::new(), down: None, up: None, tickets: 0, words: mpsc::channel() }
    }
}

impl Transfers {
    /// The file `key` downloaded from `from` (an address [`transfer_url`](super::graph::transfer_url)
    /// took) and saved in `dir` for a client whose API is at `base`, on the download lane: false
    /// when one runs and [`DOWNLOADS_WAITING`] wait already, or the lane can't be had.
    pub(crate) fn download(&mut self, key: FileKey, from: Url, name: String, dir: PathBuf, base: &str, floor: (u64, Duration)) -> bool {
        if self.downloads.len() > DOWNLOADS_WAITING {
            return false;
        }
        if self.down.is_none() {
            self.down = lane("teams-download", self.words.0.clone());
        }
        let flight = self.flight(key.clone());
        let task = Fetch { ticket: flight.ticket, key, from, name, dir, base: base.to_string(), floor, cancel: flight.cancel.clone() };
        let handed = self.down.as_ref().is_some_and(|lane| lane.send(task).is_ok());
        if handed {
            self.downloads.push(flight);
        } else {
            self.down = None;
        }
        handed
    }

    /// Part `part` of the message `seq` sent up to `session` from `source`, on the upload lane:
    /// false when one of that message's runs or waits already, or the lane can't be had.
    pub(crate) fn upload(&mut self, seq: u64, part: usize, session: UploadSession, source: Source, base: &str, floor: (u64, Duration)) -> bool {
        if self.uploads.iter().any(|f| f.key.0 == seq) {
            return false;
        }
        if self.up.is_none() {
            self.up = lane("teams-upload", self.words.0.clone());
        }
        let flight = self.flight((seq, part));
        let task = Put { ticket: flight.ticket, seq, part, session, source, base: base.to_string(), floor, cancel: flight.cancel.clone() };
        let handed = self.up.as_ref().is_some_and(|lane| lane.send(task).is_ok());
        if handed {
            self.uploads.push(flight);
        } else {
            self.up = None;
        }
        handed
    }

    /// The uploads of the message `seq` cancelled and let go: nothing of them is heard after.
    pub(crate) fn cancel_upload(&mut self, seq: u64) {
        self.uploads.retain(|f| {
            let theirs = f.key.0 == seq;
            if theirs {
                f.cancel.store(true, Ordering::SeqCst);
            }
            !theirs
        });
    }

    /// Every transfer cancelled and let go, and the files waiting forgotten: the files asked for
    /// that won't be said now, in order.
    pub(crate) fn cancel_all(&mut self) -> Vec<FileKey> {
        for cancel in self.downloads.iter().map(|f| &f.cancel).chain(self.uploads.iter().map(|f| &f.cancel)) {
            cancel.store(true, Ordering::SeqCst);
        }
        self.uploads.clear();
        let mut asked: Vec<FileKey> = self.downloads.drain(..).map(|f| f.key).collect();
        asked.extend(self.waiting.drain(..).map(|w| w.key));
        asked
    }

    /// Whether a transfer runs or waits on a lane.
    pub(crate) fn busy(&self) -> bool {
        !self.downloads.is_empty() || !self.uploads.is_empty()
    }

    /// Whether an upload of the message `seq` runs or waits on its lane.
    pub(crate) fn uploading(&self, seq: u64) -> bool {
        self.uploads.iter().any(|f| f.key.0 == seq)
    }

    /// The next word of a lane about a transfer still awaited; the words of one cancelled since
    /// are dropped.
    pub(crate) fn take(&mut self) -> Option<Done> {
        while let Ok((ticket, done)) = self.words.1.try_recv() {
            if let Some(i) = self.downloads.iter().position(|f| f.ticket == ticket) {
                self.downloads.remove(i);
                return Some(done);
            }
            if let Some(i) = self.uploads.iter().position(|f| f.ticket == ticket) {
                if !matches!(done, Done::Sent { .. }) {
                    self.uploads.remove(i);
                }
                return Some(done);
            }
        }
        None
    }

    /// Whether a file waits for its download address.
    fn waits(&self) -> bool {
        !self.waiting.is_empty()
    }

    /// Whether `key` is asked for and not said yet: waiting, or on the lane.
    fn asked(&self, key: &FileKey) -> bool {
        self.waiting.iter().any(|w| w.key == *key) || self.downloads.iter().any(|f| f.key == *key)
    }

    /// The files asked for and not said yet.
    fn asked_count(&self) -> usize {
        self.waiting.len().saturating_add(self.downloads.len())
    }

    fn flight<K>(&mut self, key: K) -> Flight<K> {
        self.tickets = self.tickets.wrapping_add(1);
        Flight { ticket: self.tickets, key, cancel: Arc::new(AtomicBool::new(false)) }
    }
}

impl Drop for Transfers {
    /// The worker gone (Quatrelle closing): what the lanes run is cancelled, their threads end
    /// with their channels.
    fn drop(&mut self) {
        self.cancel_all();
    }
}

/// What a lane runs.
trait Task: Send + 'static {
    fn ticket(&self) -> u64;
    fn cancelled(&self) -> bool;
    /// What is said of it when it panicked: given up.
    fn broken(&self) -> Done;
    fn run(self, words: &Sender<Word>) -> Done;
}

/// A lane: a thread named `name` running its transfers one at a time, in order, each behind
/// `fenced` (a panic costs that transfer), a cancelled one passed over; its words go to `words`.
fn lane<T: Task>(name: &str, words: Sender<Word>) -> Option<Sender<T>> {
    let (tx, rx) = mpsc::channel::<T>();
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || {
        for task in rx {
            if task.cancelled() {
                continue;
            }
            let (ticket, broken) = (task.ticket(), task.broken());
            let done = fenced(|| task.run(&words)).unwrap_or_else(|| {
                log::error!("teams: internal error: a transfer was given up");
                broken
            });
            if words.send((ticket, done)).is_err() {
                break;
            }
        }
    });
    match spawned {
        Ok(_) => Some(tx),
        Err(e) => {
            log::error!("teams: no thread for a transfer ({:?})", e.kind());
            None
        }
    }
}

/// What the download lane is handed.
struct Fetch {
    ticket: u64,
    key: FileKey,
    from: Url,
    name: String,
    dir: PathBuf,
    base: String,
    floor: (u64, Duration),
    cancel: Arc<AtomicBool>,
}

impl Task for Fetch {
    fn ticket(&self) -> u64 {
        self.ticket
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn broken(&self) -> Done {
        Done::NotSaved { key: self.key.clone(), why: FileWhy::Refused, code: None }
    }

    fn run(self, _: &Sender<Word>) -> Done {
        match save(&self.from, &self.name, &self.dir, &self.base, &self.cancel, self.floor, FILE_MAX) {
            Ok(path) => Done::Saved { key: self.key, path },
            Err((why, code)) => Done::NotSaved { key: self.key, why, code },
        }
    }
}

/// What the upload lane is handed.
struct Put {
    ticket: u64,
    seq: u64,
    part: usize,
    session: UploadSession,
    source: Source,
    base: String,
    floor: (u64, Duration),
    cancel: Arc<AtomicBool>,
}

impl Task for Put {
    fn ticket(&self) -> u64 {
        self.ticket
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn broken(&self) -> Done {
        Done::NotUploaded { seq: self.seq, part: self.part, why: UploadFail::Network }
    }

    fn run(self, words: &Sender<Word>) -> Done {
        let (ticket, seq, part) = (self.ticket, self.seq, self.part);
        let said = |sent: u64| {
            let _ = words.send((ticket, Done::Sent { seq, part, sent }));
        };
        match send_up(&self.session, &self.source, &self.base, &self.cancel, self.floor, &said) {
            Ok(item) => Done::Uploaded { seq, part, item },
            Err(why) => Done::NotUploaded { seq, part, why },
        }
    }
}

/// A lane's connection to a transfer's server ([`transfer_stream_at`]: on the internet only, but
/// for the tests' servers), reading and writing in slices of [`LANE_IO`], watched for the cancel
/// and the floor.
fn connected(base: &str, url: &Url, cancel: &Arc<AtomicBool>, floor: (u64, Duration)) -> crate::Result<Watched<Stream>> {
    let s = transfer_stream_at(base, url)?;
    let sock = match &s {
        Stream::Plain(t) => t,
        Stream::Tls(t) => &t.sock,
    };
    sock.set_read_timeout(Some(LANE_IO))?;
    sock.set_write_timeout(Some(LANE_IO))?;
    Ok(Watched::new(s, cancel.clone(), floor.0, floor.1))
}

/// [`Job::File`](quatrelle_model::pim::teams::Job::File): the file `attachment` of the message
/// `message` (made at `created`) of `chat`, to download and save. Ids that can't be the copy's are
/// dropped. Signed out or offline: "Offline". Not a file the copy's message carries (that chat's,
/// not deleted, with an address): "Gone". Asked already and not said: nothing more. Past one
/// running and [`DOWNLOADS_WAITING`] waiting: "Busy". Else it waits for [`step`].
pub(crate) fn open(w: &mut Worker, chat: String, message: String, created: i64, attachment: String) {
    if ![&chat, &message, &attachment].into_iter().all(|id| id_ok(id)) {
        return log::warn!("teams: a file asked for by ids that can't be a message's: dropped");
    }
    let key = (chat, message, attachment);
    if !w.polling() || w.offline() {
        return tell(w, key, failed(FileWhy::Offline, None));
    }
    let Some((url, name)) = carried(w, &key, created) else {
        log::info!("teams: a file of chat {} isn't one its message carries", w.hashed(&key.0));
        return tell(w, key, failed(FileWhy::Gone, None));
    };
    if w.transfers.asked(&key) {
        return;
    }
    if w.transfers.asked_count() > DOWNLOADS_WAITING {
        log::info!("teams: a file of chat {}: {DOWNLOADS_WAITING} wait already", w.hashed(&key.0));
        return tell(w, key, failed(FileWhy::Busy, None));
    }
    w.transfers.waiting.push_back(Wanted { key, url, name });
}

/// The address and name of the file `key` names, when the copy's message carries it: that chat's
/// message, not deleted, with a file of that id and an address.
fn carried(w: &Worker, (chat, message, attachment): &FileKey, created: i64) -> Option<(String, String)> {
    let m = w.copy.as_ref()?.message_at(chat, message, created)?;
    if m.deleted.is_some() {
        return None;
    }
    m.attachments.into_iter().find_map(|a| match a {
        Attachment::File { id, name, url, .. } if id == *attachment && !url.is_empty() => Some((url, name)),
        _ => None,
    })
}

/// One file waiting, a request at most: with room on the disk and a folder to save in, Graph asked
/// for its download address (with the token), which goes to the download lane. Graph busy: it
/// waits. The network away: "Offline", and nothing is asked for a while. Refused: said with Graph's
/// answer. Whether a request went.
pub(crate) fn step(w: &mut Worker, now: Instant) -> bool {
    if !w.transfers.waits() || !w.may(Route::Drive, now) {
        return false;
    }
    let Some(file) = w.transfers.waiting.pop_front() else { return false };
    let chat = w.hashed(&file.key.0);
    // The disk first: the folder isn't made on a disk without room.
    let dir = if room(w, now) { w.files_dir() } else { None };
    let Some(dir) = dir else {
        log::warn!("teams: a file of chat {chat}: no room to save it");
        tell(w, file.key, failed(FileWhy::NoRoom, None));
        return false;
    };
    let url = file.url.clone();
    match w.request(Route::Drive, now, |c| c.shared_location(&url)) {
        Ok(from) => {
            log::info!("teams: a file of chat {chat}: its download, from {}", host_tail(&from.host));
            let (base, floor) = (w.graph.clone(), w.floor());
            if !w.transfers.download(file.key.clone(), from, file.name, dir, &base, floor) {
                tell(w, file.key, failed(FileWhy::Busy, None));
            }
        }
        Err(e) if busy(&e) => {
            w.busy_wait(now);
            w.transfers.waiting.push_front(file);
        }
        Err(e) => {
            log::warn!("teams: a file of chat {chat}: its address wasn't given: {}", kind(&e));
            let outcome = if of_the_network(&e) || matches!(e, NetError::Auth(_)) {
                failed(FileWhy::Offline, None)
            } else {
                failed(FileWhy::Refused, code_of(&e))
            };
            if of_the_network(&e) {
                w.pause(&e, now);
            }
            tell(w, file.key, outcome);
        }
    }
    true
}

/// Graph's answer as an outcome keeps it: its status and code (`HTTP 404 NotFound`), a server's
/// error's as a refusal's.
pub(crate) fn code_of(e: &NetError) -> Option<String> {
    match e {
        NetError::Closed(m) => answer_code(&NetError::Protocol(m.clone())),
        e => answer_code(e),
    }
}

/// Every word of the lanes: a file saved or not, said; the uploads' words, for the outbox.
pub(crate) fn done(w: &mut Worker, now: Instant) {
    while let Some(word) = w.transfers.take() {
        match word {
            Done::Saved { key, path } => saved(w, key, path),
            Done::NotSaved { key, why, code } => {
                log::info!("teams: a file of chat {} not saved: {why:?}", w.hashed(&key.0));
                tell(w, key, FileOutcome::Failed { why, code });
            }
            Done::Sent { .. } | Done::Uploaded { .. } | Done::NotUploaded { .. } => super::outbox::uploaded(w, word, now),
        }
    }
}

/// A file saved, said by its path; one whose path can't be said in text is removed.
fn saved(w: &Worker, key: FileKey, path: PathBuf) {
    let chat = w.hashed(&key.0);
    match path.into_os_string().into_string() {
        Ok(path) => {
            log::info!("teams: a file of chat {chat} saved");
            tell(w, key, FileOutcome::Saved { path });
        }
        Err(odd) => {
            let _ = std::fs::remove_file(&odd);
            log::warn!("teams: a file of chat {chat} saved where a path can't be said: removed");
            tell(w, key, failed(FileWhy::NoRoom, None));
        }
    }
}

/// Teams switched off, signed out or signing in again: every transfer cancelled at once (a lane
/// stops at its next slice, a partial file removed, an upload's session cancelled), and each file
/// asked for said "Offline".
pub(crate) fn stop(w: &mut Worker) {
    let asked = w.transfers.cancel_all();
    if !asked.is_empty() {
        log::info!("teams: {} files asked for were stopped", asked.len());
    }
    for key in asked {
        tell(w, key, failed(FileWhy::Offline, None));
    }
}

/// When the loop looks again for the files: within [`LOOK`] while a transfer runs (its word is
/// taken soon after it comes), or while a file waits and Teams may be asked.
pub(crate) fn wake(w: &Worker, now: Instant, wake: Option<Instant>) -> Option<Instant> {
    let waits = w.polling() && !w.offline() && w.transfers.waits();
    if !w.transfers.busy() && !waits {
        return wake;
    }
    let soon = now.checked_add(LOOK).unwrap_or(now);
    Some(wake.map_or(soon, |at| at.min(soon)))
}

fn failed(why: FileWhy, code: Option<String>) -> FileOutcome {
    FileOutcome::Failed { why, code }
}

fn tell(w: &Worker, (chat, message, attachment): FileKey, outcome: FileOutcome) {
    w.say(Event::File { chat, message, attachment, outcome });
}

impl Worker {
    /// Where a file opened is saved: a test's folder, else the user's own folder of files handed
    /// to other apps (`open`); `None` when there's none.
    pub(crate) fn files_dir(&self) -> Option<PathBuf> {
        #[cfg(test)]
        if let Some(dir) = &self.hooks.files_dir {
            return Some(dir.clone());
        }
        quatrelle_store::scratch_dir("open")
    }

    /// The floor a transfer keeps to: a test's, else [`FLOOR`] over [`WINDOW`].
    pub(crate) fn floor(&self) -> (u64, Duration) {
        #[cfg(test)]
        if let Some(floor) = self.hooks.floor {
            return floor;
        }
        (FLOOR, WINDOW)
    }
}
