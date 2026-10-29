//! Exchange telling of changes as they happen, as Outlook's Cached Exchange Mode is told: the
//! folder that changed is read at once — new mail in the Inbox, mail a server rule filed in
//! another folder, a message read on the phone, a shared mailbox's new mail — rather than the
//! Inbox looked at every half minute and every other folder at the next full sync.

use super::*;

/// How long listening waits to start again after it stopped, at first and at most.
#[cfg(not(test))]
const RETRY_FIRST: Duration = Duration::from_secs(5);
#[cfg(test)]
const RETRY_FIRST: Duration = Duration::from_millis(200);
const RETRY_MAX: Duration = Duration::from_secs(15 * 60);
/// Listening this long without a hitch, the next stop waits the shortest again.
const STEADY: Duration = Duration::from_secs(120);
/// While told of changes, the Inbox is still looked at this often, in case the server stopped
/// telling without a word.
pub(super) const LISTEN_POLL_EVERY: Duration = Duration::from_secs(5 * 60);
/// How long one look waits for the server to say something.
const LOOK: Duration = Duration::from_millis(250);
/// What the server tells of together is read together (a rule filing fifty messages is fifty
/// notifications): what came with it is taken too, without waiting for more (what comes while
/// the folders are read is read next), this long at most.
const GATHER: Duration = Duration::from_secs(1);
/// A change told in the calendar, the contacts or the tasks is read this long after (accepting a
/// meeting is several changes), and the calendars' and contacts' sync — every calendar ticked read again, seconds — runs this often
/// at most.
#[cfg(not(test))]
const PIM_GATHER: Duration = Duration::from_secs(2);
#[cfg(test)]
const PIM_GATHER: Duration = Duration::from_millis(100);
#[cfg(not(test))]
const PIM_EVERY: Duration = Duration::from_secs(60);
#[cfg(test)]
const PIM_EVERY: Duration = Duration::from_millis(400);

/// The account's ears on its server.
#[derive(Default)]
pub(super) struct Listening {
    /// One per mailbox: the account's own, and each shared one kept on this computer.
    ears: Vec<crate::ews::Listener>,
    /// Since when, without a hitch.
    since: Option<Instant>,
    /// Not started again before this, after it stopped; and the wait after the next stop.
    retry: Option<Instant>,
    delay: Duration,
    /// The server can't tell of changes: the folders are looked at instead.
    unable: bool,
    /// The account's own calendar and contacts, by id: a change in them is read by their sync.
    pim: Vec<String>,
    /// Their sync is due then (a change was told), and when it last ran.
    pim_due: Option<Instant>,
    pim_last: Option<Instant>,
    /// The account's own Tasks folder, by id, and when it's next read alone (a change was told
    /// in it).
    tasks: Option<String>,
    tasks_due: Option<Instant>,
}

impl Listening {
    pub(super) fn on(&self) -> bool {
        !self.ears.is_empty()
    }

    /// How long until listening is tried again, when it stopped and will be.
    pub(super) fn retry_in(&self) -> Option<Duration> {
        self.retry.filter(|_| !self.unable).map(|at| at.saturating_duration_since(Instant::now()))
    }

    /// A new connection: listening starts again on it at once.
    pub(super) fn reset(&mut self) {
        self.ears.clear();
        self.since = None;
        self.retry = None;
    }
}

impl Worker {
    /// Whether the server tells this account of its changes as they happen; listening starts
    /// here when it can (an Exchange account's main connection).
    pub(super) fn listening(&mut self) -> bool {
        if self.main.is_some() || self.listen.unable || !matches!(self.conn, Some(Conn::Ews(_))) {
            return false;
        }
        if self.listen.on() {
            return true;
        }
        if self.listen.retry.is_some_and(|at| Instant::now() < at) {
            return false;
        }
        let shared = self.shared_to_listen();
        let (own, others, pim) = {
            let Some(Conn::Ews(c)) = self.conn.as_ref() else { return false };
            let own = c.listen(None, &[]);
            let others: Vec<_> =
                if own.is_ok() { shared.iter().map(|(address, ids)| (address.clone(), c.listen(Some(address), ids))).collect() } else { Vec::new() };
            let pim = if own.is_ok() { c.pim_folders() } else { Vec::new() };
            (own, others, pim)
        };
        let mut ears = match own {
            Ok(l) => vec![l],
            // The server can't (older than Exchange 2010 SP1, or with it turned off).
            Err(NetError::No(e)) => {
                log::info!("{}: Exchange doesn't tell of changes as they happen ({e}): the Inbox is looked at every half minute", self.acc.id);
                self.listen.unable = true;
                return false;
            }
            Err(e) => {
                self.listen_stopped(e);
                return false;
            }
        };
        for (address, ear) in others {
            match ear {
                Ok(l) => ears.push(l),
                Err(e) => log::info!("{}: {address}: not told of its changes ({e}): read at each full sync", self.acc.id),
            }
        }
        log::info!("{}: Exchange tells of changes as they happen ({} mailboxes)", self.acc.id, ears.len());
        let mailboxes: Vec<Option<String>> = ears.iter().map(|l| l.mailbox.clone()).collect();
        self.listen.ears = ears;
        self.listen.tasks = pim.iter().find(|(name, _)| *name == "tasks").map(|(_, id)| id.clone());
        self.listen.pim = pim.into_iter().filter(|(name, _)| *name != "tasks").map(|(_, id)| id).collect();
        self.listen.since = Some(Instant::now());
        self.next_poll = Instant::now() + LISTEN_POLL_EVERY;
        // What came between the last look and now wasn't told of: looked at once.
        let inboxes: Vec<String> = mailboxes.iter().filter_map(|m| self.inbox_of(m.as_deref())).collect();
        for inbox in inboxes {
            self.sync_one(&inbox);
        }
        true
    }

    /// The shared mailboxes kept on this computer, each with its folders' ids.
    fn shared_to_listen(&self) -> Vec<(String, Vec<String>)> {
        let mut out: Vec<(String, Vec<String>)> = Vec::new();
        if self.acc.shared_online {
            return out;
        }
        for f in &self.folders {
            let (Some(address), Some(id)) = (&f.mailbox, self.jstate.mailboxes.get(&f.path)) else { continue };
            match out.iter_mut().find(|(a, _)| a == address) {
                Some((_, ids)) => ids.push(id.clone()),
                None => out.push((address.clone(), vec![id.clone()])),
            }
        }
        out
    }

    /// A mailbox's Inbox: the account's own (`None`), or a shared one's.
    fn inbox_of(&self, mailbox: Option<&str>) -> Option<String> {
        match mailbox {
            None => self.inbox(),
            Some(a) => self.folders.iter().find(|f| f.mailbox.as_deref() == Some(a) && f.mailbox_role == FolderRole::Inbox).map(|f| f.path.clone()),
        }
    }

    /// Wait for the server to tell of a change, until a job comes, the next full sync is due or
    /// a while passed; meanwhile the folders it told of are read, a folder asked for soon (Sent
    /// Items after a send) at its time, and the Inbox now and then in case the server went
    /// quiet without a word.
    pub(super) fn wait_for_changes(&mut self, rx: &Receiver<Job>) {
        let mut end = Instant::now() + IDLE_FOR;
        if let Some(full) = self.last_full {
            end = end.min(full + FULL_SYNC_EVERY);
        }
        while Instant::now() < end {
            self.drain(rx);
            if self.closed || !self.queue.is_empty() {
                return;
            }
            self.sync_soon_due();
            if Instant::now() >= self.next_poll {
                self.next_poll = Instant::now() + LISTEN_POLL_EVERY;
                if let Some(f) = self.inbox() {
                    self.sync_one(&f);
                }
            }
            if self.conn.is_none() || !self.hear(LOOK) {
                return;
            }
        }
    }

    /// The folders asked for soon whose time has come, read.
    pub(super) fn sync_soon_due(&mut self) {
        let now = Instant::now();
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.soon).into_iter().partition(|(at, _)| *at <= now);
        self.soon = later;
        let mut folders: Vec<String> = due.into_iter().map(|(_, f)| f).collect();
        folders.dedup();
        for f in folders {
            self.sync_one(&f);
        }
    }

    /// Read what the server told of, waiting at most `wait` for it, and the folders that changed
    /// (the folder list too, when the tree did); `false` when listening stopped (it starts again
    /// a while later, the Inbox looked at meanwhile).
    pub(super) fn hear(&mut self, wait: Duration) -> bool {
        if !self.listen.on() {
            return false;
        }
        if self.listen.pim_due.is_some_and(|at| Instant::now() >= at) {
            self.sync_dav();
        }
        if self.listen.tasks_due.is_some_and(|at| Instant::now() >= at) {
            self.sync_tasks();
        }
        let mut told: Vec<(Option<String>, crate::ews::Heard)> = Vec::new();
        let mut stopped = None;
        // When the first of it came here, by this computer's clock (milliseconds since 1970).
        let mut heard_at = None;
        {
            let Some(Conn::Ews(c)) = self.conn.as_ref() else { return false };
            let each = wait / u32::try_from(self.listen.ears.len()).unwrap_or(1).max(1);
            let began = Instant::now();
            let mut first = true;
            loop {
                let mut more = false;
                for ear in &mut self.listen.ears {
                    match ear.heard(c, if first { each } else { Duration::ZERO }) {
                        Ok(h) if h.is_empty() => {}
                        Ok(h) => {
                            more = true;
                            heard_at = heard_at.or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis()));
                            told.push((ear.mailbox.clone(), h));
                        }
                        Err(e) => {
                            stopped = Some(e);
                            break;
                        }
                    }
                }
                first = false;
                if !more || stopped.is_some() || began.elapsed() >= GATHER {
                    break;
                }
            }
        }
        self.read_told(told, heard_at);
        if let Some(e) = stopped {
            self.listen_stopped(e);
            return false;
        }
        if self.listen.since.is_some_and(|t| t.elapsed() >= STEADY) {
            self.listen.delay = RETRY_FIRST;
        }
        true
    }

    /// The folders the server told of, read (those kept on this computer); the folder list
    /// first when the tree changed where the account has folders. `heard_at`: when it came here.
    fn read_told(&mut self, told: Vec<(Option<String>, crate::ews::Heard)>, heard_at: Option<u128>) {
        if told.is_empty() {
            return;
        }
        let path_of = |id: &str| self.jstate.mailboxes.iter().find(|(_, v)| v.as_str() == id).map(|(p, _)| p.clone());
        let mut folders: Vec<String> = Vec::new();
        let mut relist = false;
        let (mut pim, mut tasks) = (false, false);
        for (mailbox, heard) in &told {
            let mut unknown = false;
            for id in &heard.folders {
                match path_of(id) {
                    Some(p) => folders.push(p),
                    None if mailbox.is_none() && self.listen.tasks.as_ref() == Some(id) => tasks = true,
                    None if mailbox.is_none() && self.listen.pim.contains(id) => pim = true,
                    None => unknown = true,
                }
            }
            relist |= heard.tree.iter().any(|id| path_of(id).is_some());
            // A shared mailbox's folder named some other way than its list names it: the change
            // is most likely new mail.
            if unknown
                && mailbox.is_some()
                && let Some(inbox) = self.inbox_of(mailbox.as_deref())
            {
                folders.push(inbox);
            }
        }
        // The calendar or the contacts: their sync, a moment later (and not too often).
        if pim && self.listen.pim_due.is_none() {
            log::info!("{}: told of changes in the calendar or the contacts", self.acc.id);
            let soon = Instant::now() + PIM_GATHER;
            self.listen.pim_due = Some(self.listen.pim_last.map_or(soon, |last| soon.max(last + PIM_EVERY)));
        }
        // The tasks: their folder alone, a moment later however soon after the last sync, as it
        // reads in a tenth of a second (a task made in Outlook just after one waited a minute).
        if tasks && self.listen.tasks_due.is_none() {
            log::info!("{}: told of changes in the tasks", self.acc.id);
            self.listen.tasks_due = Some(Instant::now() + PIM_GATHER);
        }
        if relist && !self.relist() {
            return;
        }
        folders.sort();
        folders.dedup();
        let began = Instant::now();
        for f in &folders {
            if self.conn.is_none() {
                return;
            }
            if !self.read_online(f) && self.folders.iter().any(|x| x.path == *f) {
                self.sync_one(f);
            }
        }
        // How long Quatrelle took once told, to set beside how soon the server told: a new
        // mail, by the server's clock, set beside when it was told of here (Outlook showed one
        // 4 s sooner, 29/10).
        if !folders.is_empty() {
            let names: Vec<String> = folders.iter().map(|f| quatrelle_model::utf7::decode(f)).collect();
            let clock = match &self.conn {
                Some(Conn::Ews(c)) => c.clock_ahead(),
                _ => None,
            };
            let new_mail = told.iter().filter_map(|(_, h)| h.new_mail).max();
            let late = match (new_mail, heard_at, clock) {
                (Some(came), Some(at), Some(ahead)) => format!(
                    "; a new mail the server had at {} (its clock {ahead:+} s from this one's), told of {:.1} s later (to the second)",
                    crate::ews_pim::utc(came).get(11..19).unwrap_or(""),
                    told_after(came, ahead, at)
                ),
                _ => String::new(),
            };
            log::info!("{}: told of changes in {}, read in {} ms{late}", self.acc.id, names.join(", "), began.elapsed().as_millis());
        }
    }

    /// The calendars' and contacts' sync ran: what was told of before is read, the tasks too.
    pub(super) fn pim_synced(&mut self) {
        self.listen.pim_due = None;
        self.listen.pim_last = Some(Instant::now());
        self.listen.tasks_due = None;
    }

    /// The Tasks folder was read alone: what was told of in it before is read.
    pub(super) fn tasks_synced(&mut self) {
        self.listen.tasks_due = None;
    }

    /// Listening stopped (the connection dropped, the subscription is gone, the server ended
    /// it at once): it starts again after a while, longer each time it stops soon, and the Inbox
    /// is looked at meanwhile as without it.
    fn listen_stopped(&mut self, e: NetError) {
        let delay = self.listen.delay.max(RETRY_FIRST);
        log::info!("{}: no longer told of changes ({e}); trying again in {} s", self.acc.id, delay.as_secs());
        self.listen.ears.clear();
        self.listen.since = None;
        self.listen.retry = Some(Instant::now() + delay);
        self.listen.delay = (delay * 2).min(RETRY_MAX);
        self.next_poll = Instant::now();
    }
}

/// How long after a new mail came to the server (`came`, by its clock, `ahead` seconds from this
/// computer's) it was told of here (`at`, milliseconds since 1970 by this computer's), in seconds.
pub(super) fn told_after(came: i64, ahead: i64, at: u128) -> f64 {
    let came_here = i128::from(came.saturating_sub(ahead)).saturating_mul(1000);
    (i128::try_from(at).unwrap_or(i128::MAX).saturating_sub(came_here)) as f64 / 1000.0
}
