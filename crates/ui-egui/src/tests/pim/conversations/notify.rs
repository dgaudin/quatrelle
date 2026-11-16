// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! UI tests: Teams' new messages on the desktop. What the engine announces becomes a system
//! notification, titled with who and where in the interface's language, the mark of an external
//! sender or an application never cut; the window says it isn't looked at while it's minimized or
//! put away in the notification area; the queue of Teams events keeps the news through a burst,
//! what is said once up to its bound, and a chat's outbox once; the options card no longer says
//! announcements come later.

use std::sync::{Arc, Mutex};

use quatrelle_engine::TeamsAnnouncement;
use quatrelle_engine::model::pim::teams::{ChatText, News, OutState, Outgoing};

use super::*;

type Shown = Arc<Mutex<Vec<(String, String)>>>;

const AGENDA: &str = "19:agenda@thread.v2";

/// The app signed in to Teams, its notifications kept, and the events its backend will hand out.
fn notified_app(name: &str) -> (MailApp, std::path::PathBuf, Shown, Shared<MailEvent>) {
    let (mut a, dir, _, events) = teams_app(name, true);
    let shown = Shown::default();
    a.services = Box::new(Recorder(shown.clone()));
    (a, dir, shown, events)
}

/// A new message from Ana Lima of the organisation in `chat` ("Agenda" for a group, none for a
/// one-to-one chat).
fn news(chat: &str, message: &str, chat_name: Option<&str>, text: &str) -> News {
    News {
        chat: chat.into(),
        message: message.into(),
        sender: "Ana Lima".into(),
        external: false,
        app: false,
        mentions_me: false,
        chat_name: chat_name.map(str::to_string),
        direct: chat_name.is_none(),
        hidden: false,
        text: text.into(),
        at: NOW * 1_000,
    }
}

fn news_event(news: Vec<News>) -> MailEvent {
    MailEvent::Teams { event: teams::Event::News { news } }
}

/// The notifications shown in frames run with `events` handed out, those before let go.
fn notified(ctx: &egui::Context, a: &mut MailApp, shown: &Shown, events: &Shared<MailEvent>, given: Vec<MailEvent>) -> Vec<(String, String)> {
    frame(ctx, a, vec![]);
    shown.lock().unwrap().clear();
    *events.borrow_mut() = given;
    for _ in 0..3 {
        frame(ctx, a, vec![]);
    }
    std::mem::take(&mut *shown.lock().unwrap())
}

#[test]
fn a_teams_message_is_announced_through_the_system() {
    let (mut a, dir, shown, events) = notified_app("teams-notify");
    let ctx = egui::Context::default();
    let got = notified(&ctx, &mut a, &shown, &events, vec![news_event(vec![news(AGENDA, "m1", Some("Agenda"), "Lunch?")])]);
    assert_eq!(got, [("Ana Lima in Agenda".to_string(), "Lunch?".to_string())]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A name of 200 characters: cut to 24 so " (External)" always shows, even where the desktop cuts
/// a long title; an application's says "(App)".
#[test]
fn an_external_sender_is_marked_and_the_mark_never_cut() {
    let (mut a, dir, shown, events) = notified_app("teams-notify-external");
    let ctx = egui::Context::default();
    let long = "W".repeat(200);
    let external = News { sender: long.clone(), external: true, ..news("19:one@unq.gbl.spaces", "m1", None, "Hello") };
    let app = News { sender: "Planner".into(), app: true, ..news(AGENDA, "m2", Some("Agenda"), "Task due") };
    let got = notified(&ctx, &mut a, &shown, &events, vec![news_event(vec![external, app])]);
    let titles: Vec<&str> = got.iter().map(|(t, _)| t.as_str()).collect();
    let Some(name) = titles.first().and_then(|t| t.strip_suffix(" (External)")) else { panic!("{titles:?}") };
    assert!(name.chars().count() <= 25, "{name}");
    assert!(name.starts_with("WWW") && name.ends_with('…'), "{name}");
    assert_eq!(titles.get(1), Some(&"Planner (App) in Agenda"), "{titles:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_mention_says_so() {
    let (mut a, dir, shown, events) = notified_app("teams-notify-mention");
    let ctx = egui::Context::default();
    let mention = |chat: &str, message: &str| News { mentions_me: true, ..news(chat, message, Some("Agenda"), "@Riley can you look?") };
    let got = notified(&ctx, &mut a, &shown, &events, vec![news_event(vec![mention(AGENDA, "m1")])]);
    assert_eq!(got, [("Ana Lima mentioned you in Agenda".to_string(), "@Riley can you look?".to_string())]);
    let direct = News { mentions_me: true, ..news("19:one@unq.gbl.spaces", "m2", None, "Hi") };
    let got = notified(&ctx, &mut a, &shown, &events, vec![news_event(vec![direct])]);
    assert_eq!(got, [("Ana Lima mentioned you".to_string(), "Hi".to_string())]);
    // In French, through the catalogue.
    a.ui.language = "fr".into();
    let got = notified(&ctx, &mut a, &shown, &events, vec![news_event(vec![mention("19:other@thread.v2", "m3")])]);
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(got, [("Ana Lima vous a mentionné dans Agenda".to_string(), "@Riley can you look?".to_string())]);
    let _ = std::fs::remove_dir_all(dir);
}

/// What the window writes of an announcement the engine groups: a chat's news, a burst's summary,
/// a message with no text.
#[test]
fn grouped_news_and_a_burst_say_how_many() {
    use crate::conversations::{body_of, title_of};
    let one = TeamsAnnouncement {
        chat: AGENDA.into(),
        sender: "Ana Lima".into(),
        chat_name: Some("Agenda".into()),
        text: "Lunch?".into(),
        count: 1,
        chats: 1,
        ..TeamsAnnouncement::default()
    };
    assert_eq!(body_of(&TeamsAnnouncement { text: String::new(), ..one.clone() }), "New message");
    let grouped = TeamsAnnouncement { count: 3, ..one.clone() };
    assert_eq!((title_of(&grouped), body_of(&grouped)), ("Ana Lima in Agenda".to_string(), "3 new messages".to_string()));
    let burst = TeamsAnnouncement { count: 12, chats: 5, mentioned: true, ..TeamsAnnouncement::default() };
    assert_eq!((title_of(&burst), body_of(&burst)), ("Teams".to_string(), "12 new messages in 5 conversations".to_string()));
    // No name at all (a chat Graph won't serve may give none): never a title starting with a
    // space or with "in".
    let nameless = TeamsAnnouncement { sender: String::new(), external: true, ..one.clone() };
    assert_eq!(title_of(&nameless), "(External) in Agenda");
    let nameless = TeamsAnnouncement { sender: String::new(), ..one };
    assert_eq!(title_of(&nameless), "Agenda");
}

/// Where the mail's "new messages" are letters (Russian, Chinese), a chat's are said in Teams'
/// own words.
#[test]
fn teams_messages_are_not_called_mail() {
    use crate::conversations::body_of;
    use crate::i18n::{Lang, set_current, tr};
    let grouped = TeamsAnnouncement { count: 3, chats: 1, ..TeamsAnnouncement::default() };
    let empty = TeamsAnnouncement { count: 1, chats: 1, ..TeamsAnnouncement::default() };
    let mut said = Vec::new();
    for code in ["ru", "zh-hans", "zh-hant"] {
        let Some(lang) = Lang::from_code(code) else { panic!("{code}") };
        set_current(lang);
        said.push((code, body_of(&grouped), body_of(&empty), tr(lang, "{n} new messages").replace("{n}", "3")));
    }
    set_current(Lang::EN);
    for (code, grouped, empty, mail) in &said {
        assert_ne!(grouped, mail, "{code}");
        assert!(!grouped.contains("писем") && !grouped.contains("邮件") && !grouped.contains("郵件"), "{code}: {grouped}");
        assert!(!empty.contains("邮件") && !empty.contains("郵件"), "{code}: {empty}");
    }
}

/// Review Focus 3: the computer wakes after a night, thousands of rows come before the news;
/// once the queue is drained, the news is announced.
#[test]
fn news_are_never_dropped_by_a_burst() {
    let (mut a, dir, shown, _) = notified_app("teams-notify-burst");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    shown.lock().unwrap().clear();
    let mut events: Vec<MailEvent> = (0..12_000).map(|_| chats(vec![row("19:busy@thread.v2", 1)])).collect();
    events.push(news_event(vec![news(AGENDA, "m1", Some("Agenda"), "Lunch?")]));
    a.ui.teams_queue.take(&mut events);
    while a.ui.teams_queue.apply(&mut a.session, crate::conversations::TEAMS_EVENTS_A_FRAME) {}
    frame(&ctx, &mut a, vec![]);
    assert_eq!(shown.lock().unwrap().as_slice(), [("Ana Lima in Agenda".to_string(), "Lunch?".to_string())]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A news waiting before thousands of presences: these go first when the queue is full.
#[test]
fn news_outlast_presences_in_a_full_queue() {
    let mut q = crate::conversations::TeamsQueue::default();
    let mut events = vec![news_event(vec![news(AGENDA, "m1", Some("Agenda"), "Lunch?")])];
    events.extend(
        (0..12_000).map(|i| MailEvent::Teams { event: teams::Event::Presence { presences: vec![(format!("u-{i}"), teams::Presence::Busy)] } }),
    );
    q.take(&mut events);
    assert_eq!(q.len(), 10_000);
    assert!(q.holds(|e| matches!(e, teams::Event::News { .. })), "the news is kept");
}

/// What is said once is kept past the bound, but not without end: past 11,000 (the bound and
/// 1,000 more) the oldest go.
#[test]
fn the_queue_stays_bounded_with_what_is_said_once() {
    let mut q = crate::conversations::TeamsQueue::default();
    let mut events: Vec<MailEvent> = (0..20_000)
        .map(|i| MailEvent::Teams {
            event: teams::Event::Members { chat: format!("19:m{i}@thread.v2"), members: vec![], web_url: None, first: true },
        })
        .collect();
    q.take(&mut events);
    assert!(q.len() <= 11_000, "{}", q.len());
    assert!(q.holds(|e| matches!(e, teams::Event::Members { chat, .. } if chat == "19:m19999@thread.v2")), "the newest kept");
    assert!(!q.holds(|e| matches!(e, teams::Event::Members { chat, .. } if chat == "19:m0@thread.v2")), "the oldest gone");
}

fn outgoing(chat: &str, seq: u64) -> Outgoing {
    Outgoing { seq, local: format!("l{seq}"), chat: chat.into(), body: ChatText::default(), state: OutState::Waiting }
}

/// A chat's outbox as the worker says it: `n` messages going out, and the one it answered for.
fn outbox(chat: &str, n: u64, took: Option<&str>) -> MailEvent {
    MailEvent::Teams {
        event: teams::Event::Outbox { chat: chat.into(), items: (1..=n).map(|i| outgoing(chat, i)).collect(), took: took.map(str::to_string) },
    }
}

/// A later outbox of the same chat stands for the earlier: one kept, the last; one that answered
/// for a message gives it to the later that doesn't, and two that answered both stay. Another
/// chat's is kept, and of the counts unsent, the last.
#[test]
fn a_chats_outbox_snapshots_supersede_each_other() {
    let mut q = crate::conversations::TeamsQueue::default();
    let mut events = vec![outbox(AGENDA, 1, None), outbox(AGENDA, 2, None), outbox(AGENDA, 3, None)];
    q.take(&mut events);
    assert_eq!(q.len(), 1);
    assert!(q.holds(|e| matches!(e, teams::Event::Outbox { items, .. } if items.len() == 3)), "the last kept");
    // One taken in two steps: the earlier with the answer, the later without.
    let mut q = crate::conversations::TeamsQueue::default();
    let mut events = vec![outbox(AGENDA, 1, Some("l1")), outbox(AGENDA, 2, None)];
    q.take(&mut events);
    assert_eq!(q.len(), 1);
    assert!(
        q.holds(|e| matches!(e, teams::Event::Outbox { items, took, .. } if items.len() == 2 && took.as_deref() == Some("l1"))),
        "the answer kept"
    );
    // Over two polls, the same.
    let mut q = crate::conversations::TeamsQueue::default();
    q.take(&mut vec![outbox(AGENDA, 1, Some("l1"))]);
    q.take(&mut vec![outbox(AGENDA, 2, None)]);
    assert_eq!(q.len(), 1);
    assert!(q.holds(|e| matches!(e, teams::Event::Outbox { took, .. } if took.as_deref() == Some("l1"))));
    // Two answers: both stay, in their order; another chat's and the last count unsent too.
    let mut q = crate::conversations::TeamsQueue::default();
    let unsent = |count| MailEvent::Teams { event: teams::Event::Unsent { count } };
    let mut events = vec![outbox(AGENDA, 1, Some("l1")), unsent(1), outbox("19:other@thread.v2", 1, None), outbox(AGENDA, 2, Some("l2")), unsent(2)];
    q.take(&mut events);
    assert_eq!(q.len(), 4, "two answers, another chat's, one count");
    assert!(q.holds(|e| matches!(e, teams::Event::Outbox { took, .. } if took.as_deref() == Some("l1"))));
    assert!(q.holds(|e| matches!(e, teams::Event::Outbox { took, .. } if took.as_deref() == Some("l2"))));
    assert!(q.holds(|e| matches!(e, teams::Event::Unsent { count: 2 })) && !q.holds(|e| matches!(e, teams::Event::Unsent { count: 1 })));
}

/// A message the worker refused (I2) lives only in the outbox it answered with (`seq` 0, "Not
/// sent"): a later outbox of the chat that doesn't carry it never stands for that one, so the
/// refusal reaches the engine, and stays in its conversation.
#[test]
fn a_refusal_is_never_superseded() {
    let refused =
        Outgoing { seq: 0, local: "l2".into(), state: OutState::NotSent { reason: teams::Unsent::TooMany, code: None }, ..outgoing(AGENDA, 0) };
    let refusal =
        MailEvent::Teams { event: teams::Event::Outbox { chat: AGENDA.into(), items: vec![outgoing(AGENDA, 1), refused], took: Some("l2".into()) } };
    let mut events = vec![refusal, outbox(AGENDA, 1, None)];
    let (mut a, dir, _) = module_app("teams-refusal-kept", vec![row(AGENDA, 0)]);
    a.run("teams.open", json!({"chat": AGENDA}));
    let q = &mut a.ui.teams_queue;
    q.take(&mut events);
    assert!(q.holds(|e| matches!(e, teams::Event::Outbox { items, .. } if items.iter().any(|o| o.seq == 0 && o.local == "l2"))), "the refusal waits");
    while a.ui.teams_queue.apply(&mut a.session, crate::conversations::TEAMS_EVENTS_A_FRAME) {}
    let shown: Vec<(u64, &str)> = a.session.teams.outgoing.iter().map(|o| (o.seq, o.local.as_str())).collect();
    assert_eq!(shown, [(1, "l1"), (0, "l2")], "the refused message stays in its conversation");
    let _ = std::fs::remove_dir_all(dir);
}

/// A frame with the window focused or not, minimized or not.
fn frame_as(ctx: &egui::Context, a: &mut MailApp, focused: bool, minimized: bool) {
    let mut input =
        RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), focused, ..Default::default() };
    input.viewports.insert(egui::ViewportId::ROOT, egui::ViewportInfo { minimized: Some(minimized), ..Default::default() });
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
}

/// The chat open in Conversations, the window in front: its news isn't announced. Minimized, put
/// away in the notification area or behind another application, it is.
#[test]
fn the_open_chat_is_announced_while_the_window_is_away() {
    // (focused, minimized, put away, announced)
    for (focused, minimized, put_away, announced) in
        [(true, false, false, false), (true, true, false, true), (true, false, true, true), (false, false, false, true)]
    {
        let (mut a, dir, _) = module_app("teams-notify-away", vec![row(AGENDA, 0)]);
        let shown = Shown::default();
        a.services = Box::new(Recorder(shown.clone()));
        a.run("teams.open", json!({"chat": AGENDA}));
        let ctx = egui::Context::default();
        a.ui.put_away = put_away;
        // The first frame only gets the fonts ready.
        for _ in 0..2 {
            frame_as(&ctx, &mut a, focused, minimized);
        }
        a.session.teams_apply(teams::Event::News { news: vec![news(AGENDA, "m1", Some("Agenda"), "Lunch?")] });
        frame_as(&ctx, &mut a, focused, minimized);
        let got = shown.lock().unwrap().clone();
        assert_eq!(!got.is_empty(), announced, "focused {focused}, minimized {minimized}, put away {put_away}: {got:?}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// The marks read the app sent.
fn marks_read(jobs: &Shared<Job>) -> usize {
    jobs.borrow().iter().filter(|j| matches!(j, Job::Teams { job: teams::Job::Op { op: teams::Op::MarkRead { .. } } })).count()
}

/// B-I2: the File page drawn over Conversations, or the window minimized, the open chat isn't
/// looked at: its news is announced and it isn't marked read; back from the page (or the window
/// shown again), it's marked read once.
#[test]
fn the_open_chat_under_the_file_page_or_minimized_is_announced_not_marked_read() {
    for under in ["the File page", "minimized"] {
        let (mut a, dir, jobs) = module_app("teams-notify-covered", vec![row(AGENDA, 0)]);
        let shown = Shown::default();
        a.services = Box::new(Recorder(shown.clone()));
        a.run("teams.open", json!({"chat": AGENDA}));
        let ctx = egui::Context::default();
        let minimized = under == "minimized";
        for _ in 0..2 {
            frame_as(&ctx, &mut a, true, false);
        }
        if !minimized {
            a.ui.backstage = Some(crate::backstage::Backstage::new(crate::backstage::Page::Options));
        }
        frame_as(&ctx, &mut a, true, minimized);
        jobs.borrow_mut().clear();
        a.session.teams_apply(teams::Event::Chats { rows: vec![row(AGENDA, 1)] });
        a.session.teams_apply(teams::Event::News { news: vec![news(AGENDA, "m1", Some("Agenda"), "Lunch?")] });
        for _ in 0..2 {
            frame_as(&ctx, &mut a, true, minimized);
        }
        assert_eq!(shown.lock().unwrap().as_slice(), [("Ana Lima in Agenda".to_string(), "Lunch?".to_string())], "{under}: announced");
        assert_eq!(marks_read(&jobs), 0, "{under}: not marked read");
        a.ui.backstage = None;
        for _ in 0..3 {
            frame_as(&ctx, &mut a, true, false);
        }
        assert_eq!(marks_read(&jobs), 1, "{under}: marked read once looked at again");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Put away in the notification area, then shown and focused again (a second launch brings it
/// back too, without the icon): no longer put away, so the open chat is looked at again.
#[test]
fn a_window_focused_again_is_no_longer_put_away() {
    let (mut a, dir, _, _) = teams_app("teams-notify-back", true);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.ui.put_away = true;
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.put_away, "nothing said: still put away");
    frame(&ctx, &mut a, vec![egui::Event::WindowFocused(true)]);
    assert!(!a.ui.put_away);
    let _ = std::fs::remove_dir_all(dir);
}

/// A chat's next news waits its turn: the window is woken for it, and never asked to wait less
/// than nothing when the turn is past.
#[test]
fn the_window_wakes_for_the_news_held() {
    use crate::conversations::repaint_in;
    assert_eq!(repaint_in(NOW + 5, NOW), std::time::Duration::from_secs(5));
    assert_eq!(repaint_in(NOW - 5, NOW), std::time::Duration::ZERO, "a turn past");
    assert_eq!(repaint_in(i64::MIN, i64::MAX), std::time::Duration::ZERO);
    assert!(repaint_in(i64::MAX, i64::MIN) <= std::time::Duration::from_secs(60), "a turn far off is looked at again within the minute");
    let (mut a, dir, shown, events) = notified_app("teams-notify-wake");
    let ctx = egui::Context::default();
    let got = notified(
        &ctx,
        &mut a,
        &shown,
        &events,
        vec![news_event(vec![news(AGENDA, "m1", Some("Agenda"), "Lunch?"), news(AGENDA, "m2", Some("Agenda"), "Now?")])],
    );
    assert_eq!(got.len(), 1, "the second waits its turn: {got:?}");
    assert_eq!(a.session.teams_next_announcement(), Some(NOW + 5));
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let delay = out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay);
    assert!(delay.is_some_and(|d| d <= std::time::Duration::from_secs(5)), "{delay:?}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_options_card_no_longer_says_announcements_come_later() {
    let (mut a, dir, _, _) = teams_app("teams-card-announce", true);
    let ctx = egui::Context::default();
    let mut tex = Textures::default();
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    let t = drawn(&ctx, &mut a, &mut tex, "announce");
    assert!(has(&t, "Announce new messages"), "{t:?}");
    assert!(!t.iter().any(|x| x.contains("later version")), "{t:?}");
    let _ = std::fs::remove_dir_all(dir);
}
