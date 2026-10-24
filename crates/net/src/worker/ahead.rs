//! Messages downloaded whole ahead of reading, as Cached Exchange Mode keeps its offline file:
//! a few at a time while nothing else is asked, the Inbox looked at meanwhile, and never so
//! much that the disk fills.

use super::*;

/// What's left free on the disk the mail is kept on: downloading ahead pauses below the larger
/// of these (a full disk is the whole computer's trouble, not only the mail's).
const LEAVE_FREE: u64 = 5 << 30;
const LEAVE_FREE_SHARE: u64 = 50;
/// How long a pause for room lasts before the disk is looked at again.
const ROOM_AGAIN: Duration = Duration::from_secs(10 * 60);

impl Worker {
    /// Download a few more of the first folder's kept messages whole; the folder leaves the
    /// list when it has them all, when a step got none (a message the server won't give is
    /// tried again at the next full sync, not forever), or when the server refused.
    pub(super) fn ahead_step(&mut self) {
        let Some(path) = self.ahead.front().cloned() else { return };
        let opts = self.options_for(&path);
        let (store, id) = (self.store.clone(), self.id());
        let left = match self.conn.as_mut() {
            Some(Conn::Ews(c)) => crate::ews::download_ahead(c, &self.jstate, &store, &id, &path, &opts, AHEAD_STEP, &mut self.ahead_todo),
            Some(Conn::Graph(c)) => crate::graph::download_ahead(c, &self.jstate, &store, &id, &path, &opts, AHEAD_STEP),
            Some(Conn::Jmap(c)) => jmap::download_ahead(c, &self.jstate, &store, &id, &path, &opts, AHEAD_STEP),
            Some(Conn::Imap(c)) => crate::sync::download_ahead(c, &store, &id, &path, &opts, AHEAD_STEP),
            _ => Ok(0),
        };
        // IMAP, JMAP and Graph write the folder's list with what the sources say: an edit made
        // meanwhile (a read mark) is put back on it, as after a sync.
        let lists = !matches!(self.conn, Some(Conn::Ews(_)));
        if lists && self.reapply_waiting(&path) {
            self.sink.emit(Event::FolderChanged { account: id.clone(), folder: path.clone() });
        }
        let done = match left {
            Ok(0) => true,
            Ok(n) => {
                let stuck = self.ahead_left.as_ref().is_some_and(|(p, before)| *p == path && *before <= n);
                self.ahead_left = Some((path.clone(), n));
                stuck
            }
            Err(e) => {
                log::info!("{}: {}: downloading ahead stopped: {e}", self.acc.id, quatrelle_model::utf7::decode(&path));
                true
            }
        };
        if done {
            self.ahead.pop_front();
            self.ahead_left = None;
            if lists {
                self.sink.emit(Event::FolderChanged { account: id, folder: path });
            }
        }
    }

    /// While messages are downloaded ahead, the Inbox is looked at every half minute (IDLE and
    /// push wait until it's done), and a folder asked for soon (Sent Items after a send) is read.
    pub(super) fn look_between_downloads(&mut self) {
        self.ahead_poll = Instant::now() + AHEAD_POLL;
        // Told of changes, the Inbox needn't be looked at but now and then.
        if !self.listen.on() || Instant::now() >= self.next_poll {
            self.next_poll = Instant::now() + self.poll_every();
            if let Some(f) = self.inbox() {
                self.sync_one(&f);
            }
        }
        self.sync_soon_due();
    }

    /// Whether there's room to download ahead ([`enough_room`] on the disk the mail is kept on).
    /// Short of it, downloading pauses — told once — and the disk is looked at again ten minutes
    /// on; new mail still comes, and a message opened is still read.
    pub(super) fn room_to_download(&mut self) -> bool {
        if self.room_again.is_some_and(|at| Instant::now() < at) {
            return false;
        }
        let Some(stats) = self.store.data_dir().and_then(|dir| fs4::statvfs(dir).ok()) else { return true };
        if enough_room(stats.available_space(), stats.total_space()) {
            if self.room_again.take().is_some() {
                log::info!("{}: room on the disk again: downloading ahead goes on", self.acc.id);
            }
            return true;
        }
        if self.room_again.is_none() {
            let left = stats.available_space() >> 30;
            log::warn!("{}: the disk is almost full ({left} GB left): downloading ahead paused", self.acc.id);
            self.notice(format!(
                "the disk is almost full ({left} GB left): mail is no longer downloaded ahead; a message opened is read from the server"
            ));
        }
        self.room_again = Some(Instant::now() + ROOM_AGAIN);
        false
    }
}

/// Whether a disk with `available` of `total` bytes free has room for more mail: the larger of
/// 5 GB and a fiftieth of it is left.
pub(super) fn enough_room(available: u64, total: u64) -> bool {
    available >= LEAVE_FREE.max(total / LEAVE_FREE_SHARE)
}
