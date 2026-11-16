// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: Teams at scale. Two windows are fed the worker's events by a generator of the tests'
//! own, through the queue that takes them four a frame, as the worker's would be: A, 200
//! conversations and a chat of 30,000 messages; B, 2,000 conversations and 300,000 messages, its
//! longest chat of 30,000 as long as A's. Opening the module and receiving 50 new messages with
//! a new message of another chat to announce (each chat at its newest), then going back through
//! the open chat 50 pages (an older page of 100 a time; B's from 10 pages back) cost B's frame no
//! more than twice A's (and 2 ms): what a frame draws and the engine keeps is bounded by the
//! screen and the window, never by the copy.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use quatrelle_engine::model::pim::teams::{ChatMessage, MessageKind, News, Sender, WINDOW_MAX};

use super::chat::{ME, TEAMS_LINK, TENANT_ID, members, para, words};
use super::*;

/// A copy as the generator makes it up: `rows` conversations, `total` messages in all, the first
/// conversation (the one opened) `long` of them, the others sharing the rest.
#[derive(Clone, Copy)]
struct Made {
    rows: usize,
    total: usize,
    long: usize,
}

/// A conversation's id.
fn chat_id(i: usize) -> String {
    format!("19:s{i:04}@thread.v2")
}

impl Made {
    /// How many messages conversation `i` has: the rest shared as evenly as can be.
    fn count(&self, i: usize) -> usize {
        let (others, rest) = (self.rows - 1, self.total - self.long);
        match i {
            0 => self.long,
            _ => rest / others + usize::from(i <= rest % others),
        }
    }

    /// When conversation `i`'s newest message was made, in milliseconds: the first at [`NOW`],
    /// each next one a minute before.
    fn newest(i: usize) -> i64 {
        (NOW - 60 * i as i64) * 1000
    }

    /// When message `k` of conversation `i` was made: a minute after the one before.
    fn created(&self, i: usize, k: usize) -> i64 {
        Made::newest(i) + (k as i64 - (self.count(i) as i64 - 1)) * 60_000
    }

    /// Message `k` of conversation `i` (0 its oldest; past its count, one new since), from the
    /// user or from Ana in turn.
    fn message(&self, i: usize, k: usize) -> ChatMessage {
        let created = self.created(i, k);
        let sender = if k.is_multiple_of(2) {
            Sender::User { id: "u-ana".into(), name: "Ana Lima".into(), identity_type: Some("aadUser".into()), tenant_id: Some(TENANT_ID.into()) }
        } else {
            Sender::User { id: ME.into(), name: "Riley Moreno".into(), identity_type: Some("aadUser".into()), tenant_id: Some(TENANT_ID.into()) }
        };
        ChatMessage {
            id: format!("m{k:06}"),
            chat: chat_id(i),
            sender,
            created,
            modified: created,
            kind: MessageKind::Message,
            body: para(vec![words(&format!("Line {k} of the plan for conversation {i}"))]),
            ..Default::default()
        }
    }

    /// The list's rows, newest activity first.
    fn rows(&self) -> Vec<ChatRow> {
        (0..self.rows)
            .map(|i| ChatRow {
                name: format!("Conversation {i}"),
                last_line: format!("Ana: Line {} of the plan", self.count(i).saturating_sub(1)),
                at: Some(Made::newest(i)),
                members: 4,
                ..row(&chat_id(i), 0)
            })
            .collect()
    }
}

/// The worker's side, made up: it answers an opening with the conversation's last two pages, its
/// members and its outbox, and a page asked for with the 100 messages before it, as the worker
/// does from the copy, in events of 50; and it hands out what the test adds.
struct Generator {
    made: Made,
    out: Shared<MailEvent>,
}

impl Generator {
    fn messages(&mut self, chat: &str, messages: Vec<ChatMessage>, older: bool) {
        for part in messages.chunks(teams::EVENT_ITEMS) {
            let event = teams::Event::Messages { chat: chat.into(), messages: part.to_vec(), fresh: false, older };
            self.out.borrow_mut().push(MailEvent::Teams { event });
        }
    }

    /// The conversation's index in the list.
    fn index(chat: &str) -> Option<usize> {
        chat.strip_prefix("19:s")?.strip_suffix("@thread.v2")?.parse().ok()
    }
}

impl crate::Backend for Generator {
    fn submit(&mut self, job: Job) {
        let Job::Teams { job } = job else { return };
        match job {
            teams::Job::Open { chat: Some(chat) } => {
                let Some(i) = Generator::index(&chat) else { return };
                let n = self.made.count(i);
                let last: Vec<ChatMessage> = (n.saturating_sub(200)..n).map(|k| self.made.message(i, k)).collect();
                self.messages(&chat, last, false);
                let mut out = self.out.borrow_mut();
                let members = teams::Event::Members { chat: chat.clone(), members: members(), web_url: Some(TEAMS_LINK.into()), first: true };
                out.push(MailEvent::Teams { event: members });
                out.push(MailEvent::Teams { event: teams::Event::Outbox { chat, items: Vec::new(), took: None } });
            }
            teams::Job::Page { chat, before } => {
                let Some(i) = Generator::index(&chat) else { return };
                // The messages made before `before`: those below the first made at it or after.
                let ks: Vec<usize> = (0..self.made.count(i)).collect();
                let end = ks.partition_point(|&k| self.made.created(i, k) < before);
                let page = (end.saturating_sub(100)..end).map(|k| self.made.message(i, k)).collect();
                self.messages(&chat, page, true);
            }
            _ => {}
        }
    }

    fn poll(&mut self) -> Vec<MailEvent> {
        std::mem::take(&mut *self.out.borrow_mut())
    }
}

/// Services whose clock the test moves, and which keep the notifications shown.
struct Clocked {
    now: Arc<AtomicI64>,
    shown: Arc<Mutex<Vec<(String, String)>>>,
}

impl crate::Services for Clocked {
    fn clock(&self) -> (i64, i32) {
        (self.now.load(Ordering::SeqCst), 0)
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn notify(&mut self, title: &str, body: &str) {
        self.shown.lock().unwrap().push((title.into(), body.into()));
    }
}

/// A window over a made-up copy, its generator's queue, its clock and its notifications.
struct Scaled {
    a: MailApp,
    dir: std::path::PathBuf,
    ctx: egui::Context,
    made: Made,
    out: Shared<MailEvent>,
    now: Arc<AtomicI64>,
    shown: Arc<Mutex<Vec<(String, String)>>>,
}

/// The middle of what a step's timed frames took.
fn median(mut took: Vec<Duration>) -> Duration {
    took.sort();
    took.get(took.len() / 2).copied().unwrap_or_default()
}

impl Scaled {
    /// Signed in to Teams, the rows listed from the generator, the first conversation opened.
    fn new(name: &str, made: Made) -> Scaled {
        let (mut a, dir, _, _) = teams_app(name, true);
        let out: Shared<MailEvent> = Rc::new(RefCell::new(Vec::new()));
        a.backend = Box::new(Generator { made, out: out.clone() });
        let (now, shown) = (Arc::new(AtomicI64::new(NOW)), Arc::new(Mutex::new(Vec::new())));
        a.services = Box::new(Clocked { now: now.clone(), shown: shown.clone() });
        let mut s = Scaled { a, dir, ctx: egui::Context::default(), made, out, now, shown };
        {
            let mut out = s.out.borrow_mut();
            out.push(MailEvent::Teams { event: teams::Event::Status { status: Status::UpToDate } });
            out.push(MailEvent::Teams {
                event: teams::Event::Account {
                    name: "Riley Moreno".into(),
                    address: "riley@contoso.example".into(),
                    object_id: ME.into(),
                    tenant_id: TENANT_ID.into(),
                    drive_host: None,
                },
            });
            for part in made.rows().chunks(teams::EVENT_ITEMS) {
                out.push(chats(part.to_vec()));
            }
        }
        s.settle();
        assert_eq!(s.a.session.teams.rows.len(), made.rows, "every row listed");
        s.a.run("view.module", json!({"name": "conversations"}));
        s.a.run("teams.open", json!({"chat": chat_id(0)}));
        s.settle();
        assert_eq!(s.a.session.teams.window.len(), 200, "the last two pages");
        s
    }

    /// Frames until the queue is empty, and two more.
    fn settle(&mut self) {
        for _ in 0..2_000 {
            self.frame(vec![]);
            if self.out.borrow().is_empty() && !self.a.ui.teams_queue.apply(&mut self.a.session, 0) {
                break;
            }
        }
        self.frame(vec![]);
        self.frame(vec![]);
    }

    /// A frame with `events`: how long it took.
    fn frame(&mut self, events: Vec<Event>) -> Duration {
        let input =
            RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let start = Instant::now();
        let mut out = self.ctx.run_ui(input, |ui| self.a.ui(ui));
        let took = start.elapsed();
        out.textures_delta.clear();
        took
    }

    /// Opening the module from Mail, five times: the frame that first draws it each time.
    fn opening(&mut self) -> Duration {
        let mut took = Vec::new();
        for _ in 0..5 {
            self.a.run("view.module", json!({"name": "mail"}));
            self.settle();
            self.a.run("view.module", json!({"name": "conversations"}));
            took.push(self.frame(vec![]));
            self.settle();
        }
        median(took)
    }

    /// Five times, 50 new messages of the open conversation and a new message of another one,
    /// handed out together: the frame that takes them in and announces it.
    fn receiving(&mut self) -> Duration {
        let mut took = Vec::new();
        for round in 0..5usize {
            // Past the last announcement's span: this one goes at once.
            self.now.fetch_add(10, Ordering::SeqCst);
            let n = self.made.count(0);
            let fresh: Vec<ChatMessage> = (0..50).map(|j| self.made.message(0, n + 50 * round + j)).collect();
            let news = News {
                chat: chat_id(1 + round),
                message: format!("n{round}"),
                sender: "Ana Lima".into(),
                external: false,
                app: false,
                mentions_me: false,
                chat_name: Some(format!("Conversation {}", 1 + round)),
                direct: false,
                hidden: false,
                text: "Lunch at noon?".into(),
                at: Made::newest(1 + round) + 60_000,
            };
            {
                let mut out = self.out.borrow_mut();
                out.push(MailEvent::Teams { event: teams::Event::Messages { chat: chat_id(0), messages: fresh, fresh: true, older: false } });
                out.push(MailEvent::Teams { event: teams::Event::News { news: vec![news] } });
            }
            self.shown.lock().unwrap().clear();
            took.push(self.frame(vec![]));
            let shown = std::mem::take(&mut *self.shown.lock().unwrap());
            assert_eq!(shown, [(format!("Ana Lima in Conversation {}", 1 + round), "Lunch at noon?".to_string())], "announced in the frame");
            let newest = self.a.session.teams.window.back().map(|m| m.id.clone());
            assert_eq!(newest, Some(format!("m{:06}", n + 50 * round + 49)), "the new messages taken in the frame");
            self.settle();
        }
        median(took)
    }

    /// `pages` older pages asked for (the request goes at a frame's end, its answer comes at the
    /// next), the wheel turned towards the top each time: the frames that took each page in, the
    /// last five's.
    fn scrolling(&mut self, pages: usize) -> Duration {
        let over = egui::pos2(900.0, 400.0);
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 600.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        };
        let mut took = Vec::new();
        for _ in 0..pages {
            let front = self.a.session.teams.window.front().map(|m| m.created);
            self.a.run("teams.older", json!({"chat": chat_id(0)}));
            self.frame(vec![]);
            took.push(self.frame(vec![Event::PointerMoved(over), wheel.clone()]));
            self.settle();
            assert!(self.a.session.teams.window.front().map(|m| m.created) < front, "the page before taken in");
        }
        assert!(self.a.session.teams.window.len() <= WINDOW_MAX);
        median(took.split_off(took.len().saturating_sub(5)))
    }
}

impl Drop for Scaled {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Opening Conversations, going back through a long chat and receiving new messages keep a
/// frame's work flat from 200 conversations and a chat of 30,000 messages to 2,000 conversations
/// and 300,000 messages (spec, Testing › Scale).
#[test]
fn opening_scrolling_and_receiving_stay_flat_at_2000_chats_and_300000_messages() {
    // A's other conversations as long as B's: 135 messages each.
    let small = Made { rows: 200, total: 30_000 + 199 * 135, long: 30_000 };
    let big = Made { rows: 2_000, total: 300_000, long: 30_000 };
    let mut a = Scaled::new("teams-scale-a", small);
    let mut b = Scaled::new("teams-scale-b", big);
    let opening = (a.opening(), b.opening());
    let receiving = (a.receiving(), b.receiving());
    // B's long chat gone back through 10 pages before its 50 are timed.
    b.scrolling(10);
    let scrolling = (a.scrolling(50), b.scrolling(50));
    let said = format!("opening {opening:?}, scrolling {scrolling:?}, receiving {receiving:?} (A, B)");
    eprintln!("teams scale: {said}");
    for (what, (a, b)) in [("opening", opening), ("scrolling", scrolling), ("receiving", receiving)] {
        assert!(b <= 2 * a + Duration::from_millis(2), "{what}: B {b:?} past twice A {a:?} and 2 ms; {said}");
        assert!(a < Duration::from_secs(2) && b < Duration::from_secs(2), "{what}: a frame past 2 s; {said}");
    }
}
