//! A big Exchange message: its text on the main connection, so it shows at once, and the whole
//! message on a thread of its own, which nothing waits behind. The thread writes the source only,
//! as downloading ahead does: it never touches the folder's list, so it can't race a sync saving
//! it. Each key waiting keeps a copy of the connection, taken as it was asked for.

use std::sync::PoisonError;

use super::*;

/// Why a message didn't come: the account stopped first (closing, working offline, removed).
const STOPPED: &str = "the account stopped";
/// Why a message didn't come: the thread bringing it died.
const DIED: &str = "the download stopped unexpectedly";

/// What a whole message comes over: a connection of its own, copied when it's asked for.
pub(super) enum Whole {
    Ews(crate::ews::Client),
}

impl Whole {
    fn download(&self, id: &str) -> Result<Vec<u8>> {
        match self {
            Whole::Ews(c) => c.download(id),
        }
    }
}

struct Waiting {
    key: MessageKey,
    id: String,
    how: Whole,
}

/// The whole messages asked for: those waiting, the last asked for first; the one downloading;
/// and whether the thread is up. `up` goes down only with `waiting` empty, in [`next`], or
/// emptied, in [`let_go`].
#[derive(Default)]
struct Queue {
    waiting: VecDeque<Waiting>,
    running: Option<MessageKey>,
    up: bool,
}

impl Queue {
    /// A key downloading needs nothing. One waiting moves to the front with its newer copy of the
    /// connection; a new one goes there too.
    fn put(&mut self, entry: Waiting) {
        if self.running.as_ref() == Some(&entry.key) {
            return;
        }
        self.waiting.retain(|w| w.key != entry.key);
        self.waiting.push_front(entry);
    }
}

/// The next message for the thread. With none, the same lock says the thread is gone, so a key
/// added just before is taken, and one added just after starts another thread: neither is lost.
/// A poisoned lock is an empty queue.
fn next(queue: &Mutex<Queue>) -> Option<Waiting> {
    let mut q = queue.lock().ok()?;
    let entry = q.waiting.pop_front();
    q.running = entry.as_ref().map(|w| w.key.clone());
    q.up = entry.is_some();
    entry
}

/// Whole messages to bring, one at a time, the last asked for first.
pub(super) struct Rest {
    queue: Arc<Mutex<Queue>>,
    /// The account has stopped: nothing more is written.
    stopped: Arc<AtomicBool>,
    store: Store,
    sink: Sink,
}

impl Rest {
    pub(super) fn new(store: Store, sink: Sink) -> Rest {
        Rest { queue: Arc::default(), stopped: Arc::default(), store, sink }
    }

    /// Stop on the pool's flag, set as the account stops: the worker ending, which sets this one
    /// too, can be a slow call away, and a removed account's file would be written meanwhile.
    pub(super) fn stop_on(&mut self, stopped: Arc<AtomicBool>) {
        self.stopped = stopped;
    }

    /// Bring this message after the others asked for since, starting the thread if it's down.
    /// `false`: the thread can't start. The caller brings the message itself.
    pub(super) fn add(&self, key: MessageKey, id: String, how: Whole) -> bool {
        let Ok(mut q) = self.queue.lock() else { return false };
        q.put(Waiting { key: key.clone(), id, how });
        if q.up {
            return true;
        }
        // The lock is held until `up` is set: the thread's first look at the queue comes after.
        let (queue, stopped, store, sink) = (Arc::clone(&self.queue), Arc::clone(&self.stopped), self.store.clone(), self.sink.clone());
        let thread = std::thread::Builder::new().name(format!("whole-{}", key.account)).spawn(move || work(&queue, &stopped, &store, &sink));
        match thread {
            Ok(_) => q.up = true,
            Err(e) => {
                log::info!("{}: no thread for whole messages: {e}", key.account);
                q.waiting.retain(|w| w.key != key);
            }
        }
        q.up
    }
}

/// With the worker: quitting, working offline and a removed account all end here. A download in
/// progress isn't waited for; it tells of its own key when its call returns.
impl Drop for Rest {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        let left = std::mem::take(&mut self.queue.lock().unwrap_or_else(PoisonError::into_inner).waiting);
        for w in left {
            self.sink.emit(Event::BodyFailed { key: w.key, error: STOPPED.into() });
        }
    }
}

/// The thread is gone without emptying the queue: left as it is, every later key would count on
/// it. The keys it had are told, and the next one asked for starts another.
fn let_go(queue: &Mutex<Queue>, sink: &Sink) {
    let (running, waiting) = {
        let mut q = queue.lock().unwrap_or_else(PoisonError::into_inner);
        q.up = false;
        (q.running.take(), std::mem::take(&mut q.waiting))
    };
    for key in running.into_iter().chain(waiting.into_iter().map(|w| w.key)) {
        sink.emit(Event::BodyFailed { key, error: DIED.into() });
    }
}

/// Runs [`let_go`] when the thread unwinds.
struct Unwinding<'a> {
    queue: &'a Mutex<Queue>,
    sink: &'a Sink,
}

impl Drop for Unwinding<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let_go(self.queue, self.sink);
        }
    }
}

/// The thread: bring messages until the queue is empty.
fn work(queue: &Mutex<Queue>, stopped: &AtomicBool, store: &Store, sink: &Sink) {
    let _unwinding = Unwinding { queue, sink };
    while let Some(entry) = next(queue) {
        let told = bring(entry, stopped, store);
        // Before it's told of: a key reopened from now on is taken as any other, where one that
        // met `running` here would be dropped, and keep its spinner.
        if let Ok(mut q) = queue.lock() {
            q.running = None;
        }
        sink.emit(told);
    }
}

/// Bring one message; what to tell of it. Its failures are told of the message only: the main
/// connection meets the same trouble on its own.
fn bring(entry: Waiting, stopped: &AtomicBool, store: &Store) -> Event {
    let Waiting { key, id, how } = entry;
    if stopped.load(Ordering::SeqCst) {
        return Event::BodyFailed { key, error: STOPPED.into() };
    }
    // Written meanwhile (downloading ahead, a sync), which tells no one: told here, or the message
    // would wait for ever. A body that can't be read is downloaded.
    if let Some(raw) = store.read_body(&key) {
        return Event::Body { key, raw };
    }
    let asked = Instant::now();
    let raw = how.download(&id);
    if stopped.load(Ordering::SeqCst) {
        return Event::BodyFailed { key, error: STOPPED.into() };
    }
    let written = raw.and_then(|raw| {
        store.write_body(&key, &raw)?;
        Ok(raw)
    });
    match written {
        Ok(raw) => {
            log::info!("{}: the rest of it came in {:.1} s ({} bytes)", key.account, asked.elapsed().as_secs_f32(), raw.len());
            Event::Body { key, raw }
        }
        Err(e) => Event::BodyFailed { key, error: e.to_string() },
    }
}

impl Worker {
    /// A big message's text on this connection, then the whole of it on a copy. Whatever can't
    /// go that way is brought whole here ([`Worker::fetch`]), which tells of any failure.
    pub(super) fn fetch_text_first(&mut self, key: MessageKey) {
        if let Some(raw) = self.store.read_body(&key) {
            return self.sink.emit(Event::Body { key, raw });
        }
        if self.text_refused {
            return self.fetch(key);
        }
        // Told here, not left to `fetch`, which would connect again: on Active Directory two
        // refused sign-ins for each message opened can lock the account.
        if self.conn.is_none()
            && let Err(e) = self.connect()
        {
            self.sink.emit(Event::BodyFailed { key, error: e.to_string() });
            if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                self.failed(e);
            }
            return;
        }
        let Some(Conn::Ews(c)) = self.conn.as_ref() else { return self.fetch(key) };
        let (asked, numbered) = (Instant::now(), self.jstate.len());
        let text = crate::ews::fetch_text(c, &mut self.jstate, &self.store, &key);
        let how = Whole::Ews(c.sibling());
        if self.jstate.len() != numbered {
            // A message found again by its Message-ID: the numbering is worth keeping.
            self.save_jstate();
        }
        let (raw, id) = match text {
            Ok(Some(found)) => found,
            Ok(None) => {
                log::info!("{}: a message comes whole, without its text first", self.acc.id);
                return self.fetch(key);
            }
            Err(NetError::No(e)) if crate::ews_pim::FIELD_REFUSED.iter().any(|r| e.contains(r)) => {
                self.text_refused = true;
                log::warn!("{}: the server refuses a message's text alone ({e}): messages come whole from now on", self.acc.id);
                return self.fetch(key);
            }
            Err(e) => {
                log::info!("{}: a message's text didn't come ({e}): it comes whole", self.acc.id);
                return self.fetch(key);
            }
        };
        log::info!("{}: a message's text came first: {} bytes in {} ms", self.acc.id, raw.len(), asked.elapsed().as_millis());
        // In this order, a message's first text can't arrive after the whole of it. Reopened as
        // it downloads, its text can come after `Event::Body`, which it can't overwrite; or, the
        // key no longer running, `add` brings it again and `Event::Body` follows.
        self.sink.emit(Event::TextFirst { key: key.clone(), raw });
        if !self.rest.add(key.clone(), id, how) {
            self.fetch(key);
        }
    }
}

#[cfg(test)]
mod text_first_tests {
    use super::*;

    fn entry(uid: u32, id: &str) -> Waiting {
        let client = crate::ews::Client::new("https://nowhere.invalid/EWS/Exchange.asmx", "user", "pw", "user@nowhere.invalid").unwrap();
        Waiting { key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid }, id: id.into(), how: Whole::Ews(client) }
    }

    fn waiting(q: &Queue) -> Vec<(u32, &str)> {
        q.waiting.iter().map(|w| (w.key.uid, w.id.as_str())).collect()
    }

    #[test]
    fn new_keys_go_first() {
        let mut q = Queue::default();
        for uid in 1..=3 {
            q.put(entry(uid, "id"));
        }
        assert_eq!(waiting(&q), [(3, "id"), (2, "id"), (1, "id")]);
    }

    /// The copy of the connection waiting may be old by now: the newest stays.
    #[test]
    fn the_same_key_twice_stays_one_at_the_front_with_the_new_copy() {
        let mut q = Queue::default();
        q.put(entry(1, "old"));
        q.put(entry(2, "other"));
        q.put(entry(1, "new"));
        assert_eq!(waiting(&q), [(1, "new"), (2, "other")]);
    }

    #[test]
    fn a_key_running_stays_running() {
        let queue = Mutex::new(Queue::default());
        queue.lock().unwrap().put(entry(1, "id"));
        let running = next(&queue).unwrap();
        queue.lock().unwrap().put(entry(1, "id"));
        let q = queue.lock().unwrap();
        assert!(q.waiting.is_empty());
        assert_eq!(q.running, Some(running.key));
    }

    #[test]
    fn the_next_key_is_the_first_and_runs() {
        let queue = Mutex::new(Queue { up: true, ..Default::default() });
        queue.lock().unwrap().put(entry(1, "id"));
        queue.lock().unwrap().put(entry(2, "id"));
        let first = next(&queue).unwrap();
        let q = queue.lock().unwrap();
        assert_eq!((first.key.uid, q.running.as_ref().map(|k| k.uid), q.up), (2, Some(2), true));
    }

    /// One lock finds the queue empty and says the thread is gone, so a key added just before
    /// is taken by the thread, and one added just after starts another.
    #[test]
    fn an_empty_queue_lets_the_thread_go_at_once() {
        let queue = Mutex::new(Queue { running: Some(entry(1, "id").key), up: true, ..Default::default() });
        assert!(next(&queue).is_none());
        let q = queue.lock().unwrap();
        assert!(q.running.is_none() && !q.up);
    }

    #[test]
    fn a_poisoned_queue_is_an_empty_one() {
        let queue = Arc::new(Mutex::new(Queue::default()));
        let held = Arc::clone(&queue);
        let _ = std::thread::spawn(move || {
            let _guard = held.lock().unwrap();
            panic!("poisoned on purpose");
        })
        .join();
        assert!(next(&queue).is_none());
    }

    fn sink() -> (Sink, Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        (Sink { tx, wake: Arc::new(Mutex::new(None)) }, rx)
    }

    fn failed(rx: &Receiver<Event>) -> Vec<u32> {
        rx.try_iter().filter_map(|e| if let Event::BodyFailed { key, .. } = e { Some(key.uid) } else { None }).collect()
    }

    /// A key reopened once its download has ended is taken as any other: told of first, it would
    /// meet `running` and be dropped, and keep its spinner.
    #[test]
    fn a_key_is_no_longer_running_when_it_is_told_of() {
        let store = crate::fake::tests_store("rest-text-first-told");
        let queue = Arc::new(Mutex::new(Queue { up: true, ..Default::default() }));
        queue.lock().unwrap().put(entry(1, "id"));
        let (sink, _rx) = sink();
        let (there, running) = (Arc::clone(&queue), Arc::new(Mutex::new(Vec::new())));
        let saw = Arc::clone(&running);
        *sink.wake.lock().unwrap() = Some(Arc::new(move || saw.lock().unwrap().push(there.lock().unwrap().running.is_some())));
        work(&queue, &AtomicBool::new(true), &store, &sink);
        assert_eq!(*running.lock().unwrap(), [false]);
        let _ = std::fs::remove_dir_all(store.root());
    }

    /// The thread dies mid-way: no key is left waiting on it, and the next one starts another.
    #[test]
    fn a_thread_that_dies_lets_go_of_the_queue() {
        let store = crate::fake::tests_store("rest-text-first-dies");
        let queue = Arc::new(Mutex::new(Queue { up: true, ..Default::default() }));
        for uid in 1..=3 {
            queue.lock().unwrap().put(entry(uid, "id"));
        }
        let (sink, rx) = sink();
        *sink.wake.lock().unwrap() = Some(Arc::new(|| panic!("dies on purpose")));
        let (there, kept) = (Arc::clone(&queue), store.clone());
        assert!(std::thread::spawn(move || work(&there, &AtomicBool::new(true), &kept, &sink)).join().is_err());
        let q = queue.lock().unwrap();
        assert!(q.running.is_none() && !q.up && q.waiting.is_empty());
        assert_eq!(failed(&rx), [3, 2, 1], "the one it died on, then those waiting");
        let _ = std::fs::remove_dir_all(store.root());
    }

    /// The same when it dies with a key running, whatever state the lock is in.
    #[test]
    fn letting_go_tells_the_key_running_and_those_waiting() {
        let queue = Arc::new(Mutex::new(Queue { running: Some(entry(1, "id").key), up: true, ..Default::default() }));
        queue.lock().unwrap().put(entry(2, "id"));
        let held = Arc::clone(&queue);
        let _ = std::thread::spawn(move || {
            let _guard = held.lock().unwrap();
            panic!("poisoned on purpose");
        })
        .join();
        let (sink, rx) = sink();
        let_go(&queue, &sink);
        assert_eq!(failed(&rx), [1, 2]);
        let q = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(q.running.is_none() && !q.up && q.waiting.is_empty());
    }

    /// A poisoned lock loses no key at the stop.
    #[test]
    fn the_stop_reads_a_poisoned_queue() {
        let store = crate::fake::tests_store("rest-text-first-poisoned");
        let (sink, rx) = sink();
        let rest = Rest::new(store.clone(), sink);
        rest.queue.lock().unwrap().put(entry(1, "id"));
        let held = Arc::clone(&rest.queue);
        let _ = std::thread::spawn(move || {
            let _guard = held.lock().unwrap();
            panic!("poisoned on purpose");
        })
        .join();
        drop(rest);
        assert_eq!(failed(&rx), [1]);
        let _ = std::fs::remove_dir_all(store.root());
    }

    /// The account stops: each key waiting is told, and the flag tells the thread.
    #[test]
    fn the_stop_tells_each_key_waiting() {
        let (tx, rx) = mpsc::channel();
        let store = crate::fake::tests_store("rest-text-first-stop");
        let rest = Rest::new(store.clone(), Sink { tx, wake: Arc::new(Mutex::new(None)) });
        rest.queue.lock().unwrap().put(entry(1, "id"));
        rest.queue.lock().unwrap().put(entry(2, "id"));
        let stopped = Arc::clone(&rest.stopped);
        drop(rest);
        assert!(stopped.load(Ordering::SeqCst));
        let told: Vec<(u32, String)> =
            rx.try_iter().filter_map(|e| if let Event::BodyFailed { key, error } = e { Some((key.uid, error)) } else { None }).collect();
        assert_eq!(told, [(2, STOPPED.to_string()), (1, STOPPED.to_string())]);
        let _ = std::fs::remove_dir_all(store.root());
    }
}
