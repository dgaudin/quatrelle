// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: Teams at scale. Two windows are fed the worker's events by a generator of the tests'
//! own, through the queue that takes them four a frame, as the worker's would be: A, 200
//! conversations and a chat of 30,000 messages; B, 2,000 conversations and 300,000 messages, its
//! longest chat of 30,000 as long as A's. Opening the module and receiving 50 new messages with
//! a new message of another chat to announce (each chat at its newest), then going back through
//! the open chat 50 pages (an older page of 100 a time; B's from 10 pages back) cost B's frame no
//! more than twice A's (and 2 ms): what a frame draws and the engine keeps is bounded by the
//! screen and the window, never by the copy. A window full of reactions draws as fast with 50
//! changes going out as without.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use quatrelle_engine::model::pim::teams::{
    Block, BlockKind, ChatMessage, MAX_BLOCKS, MessageKind, News, OutKind, OutState, Outgoing, Reaction, Run, Sender, WINDOW_MAX,
};

use super::chat::{CHAT, ME, TALL, TEAMS_LINK, TENANT_ID, chat_app, group_row, members, pane, para, said, settled_at, user, words};
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

/// A window of 500 messages, each with five reactions of 20 people: a frame of the pane with 50
/// changes going out (edits, deletions and reactions of messages in it) costs no more than twice
/// one without (and 2 ms), the middle of five each, their frames in turn; each under 400 ms.
#[test]
fn a_window_with_full_reactions_and_50_changes_going_out_draws_in_time() {
    let message = |i: usize| {
        let from = if i.is_multiple_of(2) { user(ME, "Riley Moreno") } else { user("u-ana", "Ana Lima") };
        let mut m = said(&format!("m{i:03}"), from, NOW - 6000 + i as i64 * 10, para(vec![words(&format!("Line {i} of the plan"))]));
        m.reactions = (0..5)
            .map(|r| Reaction {
                emoji: char::from_u32(0x1F600 + r).unwrap_or('x').to_string(),
                users: (0..20).map(|k| format!("u-{k:02}")).collect(),
            })
            .collect();
        m
    };
    let messages: Vec<ChatMessage> = (0..WINDOW_MAX).map(message).collect();
    let (mut plain, plain_dir, _) = chat_app("teams-scale-chips", group_row(), TEAMS_LINK, messages.clone());
    let (mut busy, busy_dir, _) = chat_app("teams-scale-changes", group_row(), TEAMS_LINK, messages);
    assert_eq!(busy.session.teams.window.len(), WINDOW_MAX);
    // The user's own messages are the even ones.
    let changes: Vec<Outgoing> = (0..50u64)
        .map(|k| {
            let i = (k as usize * 10) % WINDOW_MAX;
            let (message, created) = (format!("m{i:03}"), (NOW - 6000 + i as i64 * 10) * 1000);
            let (what, text) = match k % 3 {
                0 => (OutKind::Edit { message, created }, "New words of the plan"),
                1 => (OutKind::Delete { message, created }, ""),
                _ => (OutKind::React { message, created, emoji: "\u{1F44D}".into(), on: true }, ""),
            };
            let body = if text.is_empty() { teams::ChatText::default() } else { para(vec![words(text)]) };
            let state = if k.is_multiple_of(2) { OutState::Waiting } else { OutState::NotSent { reason: teams::Unsent::Refused, code: None } };
            Outgoing { seq: k + 1, local: format!("l{k}"), chat: CHAT.into(), body, mentions: Vec::new(), what, state, parts: Vec::new() }
        })
        .collect();
    busy.session.teams_apply(teams::Event::Outbox { chat: CHAT.into(), items: changes, took: None });
    assert!(busy.session.teams.outgoing.len() >= 40, "{}", busy.session.teams.outgoing.len());
    let (plain_ctx, busy_ctx) = (egui::Context::default(), egui::Context::default());
    settled_at(&plain_ctx, &mut plain, TALL);
    settled_at(&busy_ctx, &mut busy, TALL);
    let timed = |ctx: &egui::Context, a: &mut MailApp| {
        let started = Instant::now();
        pane(ctx, a, vec![], TALL);
        started.elapsed()
    };
    let (mut without, mut with) = (Vec::new(), Vec::new());
    for _ in 0..5 {
        without.push(timed(&plain_ctx, &mut plain));
        with.push(timed(&busy_ctx, &mut busy));
    }
    let (a, b) = (median(without.clone()), median(with.clone()));
    let said = format!("without {a:?}, with 50 changes {b:?} (frames {without:?} and {with:?})");
    eprintln!("teams chips: {said}");
    let (_, texts) = pane(&busy_ctx, &mut busy, vec![], TALL);
    assert!(texts.iter().any(|(t, _)| t == "Not edited" || t == "Editing…"), "the changes drawn: {said}");
    assert!(b <= 2 * a + Duration::from_millis(2), "{said}");
    assert!(without.iter().chain(&with).all(|d| *d < Duration::from_millis(400)), "a frame past 400 ms: {said}");
    let _ = std::fs::remove_dir_all(plain_dir);
    let _ = std::fs::remove_dir_all(busy_dir);
}

/// Message `k` of a heavy window, Ana's, a minute after the one before: `paragraphs` paragraphs,
/// "m{k} p{i}" each.
fn heavy(k: usize, paragraphs: usize) -> ChatMessage {
    let blocks = (0..paragraphs).map(|i| Block { kind: BlockKind::Paragraph, runs: vec![words(&format!("m{k} p{i}"))] }).collect();
    said(&format!("h{k:03}"), user("u-ana", "Ana Lima"), NOW - 86_000 + k as i64 * 60, teams::ChatText { blocks })
}

/// The paragraphs of heavy messages a frame drew.
fn paragraphs_drawn(texts: &[(String, egui::Rect)]) -> usize {
    texts.iter().filter(|(t, _)| t.starts_with('m') && t.contains(" p")).count()
}

/// Security review 1.5 (content n° 2): a message of 3,500 paragraphs (a copy kept before the
/// reader's cap can hold one) draws [`MAX_BLOCKS`] of them, then "…".
#[test]
fn a_message_draws_at_most_its_cap_of_blocks() {
    let (mut a, dir, _) = chat_app("teams-scale-blocks", group_row(), TEAMS_LINK, vec![heavy(0, 3_500)]);
    let ctx = egui::Context::default();
    let texts = settled_at(&ctx, &mut a, TALL);
    assert_eq!(paragraphs_drawn(&texts), MAX_BLOCKS);
    assert!(has_text(&texts, "…"), "the cut says so");
    let _ = std::fs::remove_dir_all(dir);
}

/// Security review 1.5 (content n° 2): a window of 200 messages of 300 paragraphs each draws only
/// the messages in view (a frame draws a few hundred paragraphs, not 60,000), and opens at its
/// newest.
#[test]
fn only_the_messages_in_view_are_drawn() {
    let window: Vec<ChatMessage> = (0..200).map(|k| heavy(k, 300)).collect();
    let (mut a, dir, _) = chat_app("teams-scale-view", group_row(), TEAMS_LINK, window);
    let ctx = egui::Context::default();
    let texts = settled_at(&ctx, &mut a, (1280, 800));
    let drawn = paragraphs_drawn(&texts);
    assert!(drawn <= 600, "{drawn} paragraphs drawn");
    // The messages in view (twice when the frame is laid out again) and 2,000 blocks' worth more,
    // the last message measured past them; a message costs its 300 blocks and a block a 8 runs.
    let (laid, each) = (a.ui.conversations.heights.laid_out, 300 + 300 / 8);
    assert!(laid <= 2_000 + each + 2 * 2 * each, "{laid} blocks laid out");
    let last = rect_of(&texts, |t| t == "m199 p299").expect("the newest's last paragraph");
    assert!(last.top() > 0.0 && last.bottom() < 800.0, "at the newest: {last:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A frame of 1280 × 800 with `events`, then the texts of a frame without any.
fn heavy_frame(ctx: &egui::Context, a: &mut MailApp, events: Vec<Event>) -> Vec<(String, egui::Rect)> {
    frame_sized(ctx, a, events, egui::vec2(1280.0, 800.0));
    texts_of(&frame_shapes(ctx, a, vec![], egui::vec2(1280.0, 800.0)))
}

/// Security review 1.5 (content n° 2): opened at a message in the middle of a heavy window (a
/// search's hit), the chat is scrolled to it, though the heights around it are guessed at first;
/// only what's near it is drawn.
#[test]
fn a_heavy_windows_message_is_gone_to() {
    let (mut a, dir, _) = chat_app("teams-scale-go", group_row(), TEAMS_LINK, Vec::new());
    a.run("teams.open", json!({"chat": CHAT, "message": "h100"}));
    a.session.teams_apply(teams::Event::Members { chat: CHAT.into(), members: members(), web_url: Some(TEAMS_LINK.into()), first: true });
    let window: Vec<ChatMessage> = (0..200).map(|k| heavy(k, 300)).collect();
    for part in window.chunks(teams::EVENT_ITEMS) {
        a.session.teams_apply(teams::Event::Messages { chat: CHAT.into(), messages: part.to_vec(), fresh: false, older: false });
    }
    a.session.teams_apply(teams::Event::Placed { chat: CHAT.into(), message: "h100".into(), newest: false });
    let ctx = egui::Context::default();
    let texts = settled_at(&ctx, &mut a, (1280, 800));
    let near = texts.iter().filter(|(t, r)| t.starts_with("m100 p") && r.top() >= 0.0 && r.bottom() <= 800.0).count();
    assert!(near > 0, "the message gone to in view: {:?}", texts.iter().filter(|(t, _)| t.contains(" p")).take(5).collect::<Vec<_>>());
    assert!(paragraphs_drawn(&texts) <= 900, "{} paragraphs drawn", paragraphs_drawn(&texts));
    let _ = std::fs::remove_dir_all(dir);
}

/// Security review 1.5 (content n° 2): scrolled up a heavy window, an older page comes in above,
/// heavy too: what's on screen stays where it is, while the frames that follow learn the new
/// messages' heights out of view; a frame lays out the messages in view and at most 2,000 blocks
/// more.
#[test]
fn a_heavy_window_keeps_its_place_as_heights_become_known() {
    let (mut a, dir, _) = chat_app("teams-scale-place", group_row(), TEAMS_LINK, (100..200).map(|k| heavy(k, 300)).collect());
    let ctx = egui::Context::default();
    settled_at(&ctx, &mut a, (1280, 800));
    let over = egui::pos2(900.0, 400.0);
    let notch = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 900.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: Modifiers::NONE,
    };
    heavy_frame(&ctx, &mut a, vec![Event::PointerMoved(over)]);
    for _ in 0..3 {
        heavy_frame(&ctx, &mut a, vec![Event::PointerMoved(over), notch.clone()]);
    }
    let mut texts = Vec::new();
    for _ in 0..20 {
        texts = heavy_frame(&ctx, &mut a, vec![Event::PointerMoved(over)]);
    }
    let (seen, was) = texts.iter().find(|(t, r)| t.contains(" p") && r.top() > 300.0 && r.bottom() < 700.0).cloned().expect("a paragraph in view");
    for part in (0..100).map(|k| heavy(k, 300)).collect::<Vec<_>>().chunks(teams::EVENT_ITEMS) {
        a.session.teams_apply(teams::Event::Messages { chat: CHAT.into(), messages: part.to_vec(), fresh: false, older: true });
    }
    for round in 0..30 {
        let texts = heavy_frame(&ctx, &mut a, vec![Event::PointerMoved(over)]);
        let now = rect_of(&texts, |t| t == seen).unwrap_or_else(|| panic!("frame {round}: {seen} gone"));
        assert!((now.top() - was.top()).abs() < 4.0, "frame {round}: {seen} moved: {was:?} → {now:?}");
        // The messages in view (twice when the frame is laid out again) and 2,000 blocks' worth
        // more, the last message measured past them; a message costs its 300 blocks and a block a
        // 8 runs.
        let (laid, each) = (a.ui.conversations.heights.laid_out, 300 + 300 / 8);
        assert!(laid <= 2_000 + each + 2 * 2 * each, "frame {round}: {laid} blocks laid out");
    }
    assert_eq!(a.session.teams.window.len(), 200);
    let _ = std::fs::remove_dir_all(dir);
}

/// Re-review n° 2: a window of 500 messages of one block of 2,000 runs each (styles in turn) isn't
/// measured all in one frame: a frame lays out the messages in view and some 2,000 blocks' worth
/// more, runs counted, not the 500 messages' million runs.
#[test]
fn a_window_of_many_runs_is_measured_a_part_a_frame() {
    let message = |k: usize| {
        let runs = (0..2_000)
            .map(|i| Run { text: if i % 2 == 0 { "a".into() } else { "b".into() }, bold: i % 2 == 0, italic: i % 2 == 1, ..Default::default() })
            .collect();
        said(
            &format!("r{k:03}"),
            user("u-ana", "Ana Lima"),
            NOW - 86_000 + k as i64 * 60,
            teams::ChatText { blocks: vec![Block { kind: BlockKind::Paragraph, runs }] },
        )
    };
    let (mut a, dir, _) = chat_app("teams-scale-runs", group_row(), TEAMS_LINK, (0..WINDOW_MAX).map(message).collect());
    let ctx = egui::Context::default();
    for frame in 0..6 {
        frame_sized(&ctx, &mut a, vec![], egui::vec2(1280.0, 800.0));
        let laid = a.ui.conversations.heights.laid_out;
        assert!(laid <= 5_000, "frame {frame}: {laid} blocks' worth laid out");
    }
    let _ = std::fs::remove_dir_all(dir);
}
