// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Pictures and presence against the fake Graph server, the worker driven by hand at the times
//! the test makes: hosted images fetched once then answered from the copy, kept by their bytes'
//! hash, twenty a message at most, ten megabytes each, none for a message the copy holds as
//! deleted; photos, a missing one asked again only a day later; outside images on `https` to the
//! internet only, never kept; the image cache's bound; presence every minute for the people shown.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{
    self as model, ChatMessage, EVENT_ITEMS, Job, MAX_PICTURE, PRESENCE_IDS, Picture, Presence, Status, TEAMS_ACCOUNT,
};
use quatrelle_store::Store;
use sha2::{Digest, Sha256};

use super::worker::{AtDesk, CLIENT, Kept, Seen, account, desk, emit_into, grant, said, teams_of, turn_until};
use super::{CHAT, MINUTE, T0, TENANT, chat, hits, serve_teams, teams_with};
use crate::fake_graph::{FakeChat, FakeTeams, Server};
use crate::http::Response;
use crate::teams::store::{Copy, Identity};
use crate::teams::worker::Worker;

const DAY: i64 = 24 * 60 * MINUTE;
/// The fake's clock: 40 days after the fixtures' messages.
const NOW: i64 = T0 + 40 * DAY;
/// The step the test's clock moves by between two turns.
const STEP: Duration = Duration::from_millis(100);

/// A worker driven by hand, signed in to a fake Graph, its first copy over.
struct Run {
    w: Worker,
    server: Server,
    seen: Seen,
    t0: Instant,
    /// How far past `t0` the last turn was.
    at: Duration,
    _creds: Arc<Kept>,
}

fn run(mut teams: FakeTeams) -> Run {
    teams.clock.get_or_insert(NOW);
    let server = serve_teams(teams);
    let (desk, seen, creds) = (desk(grant(Some(TENANT))), Seen::default(), Arc::new(Kept::default()));
    let mut w = Worker::new(Store::memory(), creds.clone(), emit_into(&seen), server.base.clone(), Box::new(AtDesk(desk))).unwrap();
    let t0 = Instant::now();
    w.job(Job::SignIn { client_id: CLIENT.into(), tenant: "organizations".into() });
    turn_until(&mut w, t0, "the account", |_| account(&teams_of(&seen)).is_some());
    let mut r = Run { w, server, seen, t0, at: Duration::ZERO, _creds: creds };
    r.until("the first copy's end", Duration::from_secs(120), |r| said(&r.events(), &Status::UpToDate));
    r
}

impl Run {
    fn turn(&mut self) {
        self.at += STEP;
        self.w.turn(self.t0 + self.at);
    }

    fn turns(&mut self, span: Duration) {
        let end = self.at + span;
        while self.at < end {
            self.turn();
        }
    }

    fn until(&mut self, what: &str, span: Duration, done: impl Fn(&Run) -> bool) {
        let end = self.at + span;
        while !done(self) {
            assert!(self.at < end, "{what} never came; said: {:?}", self.events());
            self.turn();
        }
    }

    fn fake<T>(&self, change: impl FnOnce(&mut FakeTeams) -> T) -> T {
        change(self.server.data.lock().unwrap().teams.as_mut().unwrap())
    }

    fn events(&self) -> Vec<model::Event> {
        teams_of(&self.seen)
    }

    /// Every picture said, in order.
    fn pictures(&self) -> Vec<(Picture, Vec<u8>)> {
        self.events()
            .into_iter()
            .filter_map(|e| if let model::Event::Picture { picture, bytes } = e { Some((picture, bytes)) } else { None })
            .collect()
    }

    /// Every presence said: one list an event.
    fn presences(&self) -> Vec<Vec<(String, Presence)>> {
        self.events().into_iter().filter_map(|e| if let model::Event::Presence { presences } = e { Some(presences) } else { None }).collect()
    }

    fn ask(&mut self, picture: Picture) {
        self.w.job(Job::Picture { picture });
    }

    fn copy(&self) -> &Copy {
        self.w.copy.as_ref().unwrap()
    }

    /// The keys of the copy under `folder`.
    fn kept(&self, folder: &str) -> Vec<String> {
        let mut keys = self.w.store.account_files(TEAMS_ACCOUNT, folder).unwrap();
        keys.sort();
        keys
    }
}

/// The address Graph gives a message's hosted image, in its body.
fn img(message: &str, hosted: &str) -> String {
    format!(r#"<img src="https://graph.microsoft.com/v1.0/chats/{CHAT}/messages/{message}/hostedContents/{hosted}/$value">"#)
}

/// A chat of two messages; the first shows these hosted images, in this order.
fn with_hosted(images: Vec<(&str, Vec<u8>)>) -> FakeChat {
    let mut c = chat(CHAT, T0, 2);
    let id = c.messages[0].id.clone();
    let imgs: String = images.iter().map(|(h, _)| img(&id, h)).collect();
    c.messages[0].body_html = format!("<p>Look{imgs}</p>");
    c.messages[0].hosted = images.into_iter().map(|(h, b)| (h.to_string(), b)).collect();
    c
}

/// The first message's id: the one with the pictures.
fn first(r: &Run) -> String {
    r.fake(|t| t.chat(CHAT).unwrap().messages[0].id.clone())
}

fn hosted(r: &Run, id: &str) -> Picture {
    Picture::Hosted { chat: CHAT.into(), message: first(r), hosted: id.into() }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn a_hosted_image_is_fetched_once_then_from_the_copy() {
    let mut r = run(teams_with(vec![with_hosted(vec![("aDE=", b"PNG one".to_vec())])]));
    let p = hosted(&r, "aDE=");
    assert_eq!(hits(&r.server, "hosted"), 0, "nothing fetched before it's asked for");
    r.ask(p.clone());
    r.until("the picture", Duration::from_secs(10), |r| !r.pictures().is_empty());
    assert_eq!(r.pictures(), [(p.clone(), b"PNG one".to_vec())]);
    assert_eq!(hits(&r.server, "hosted"), 1);
    assert_eq!(r.kept("img"), [sha256(b"PNG one")], "kept under its bytes' hash");

    // Asked again: answered from the copy, at once and without a request.
    r.ask(p.clone());
    assert_eq!(r.pictures().len(), 2, "answered at the job");
    assert_eq!(r.pictures()[1], (p, b"PNG one".to_vec()));
    r.turns(Duration::from_secs(5));
    assert_eq!(hits(&r.server, "hosted"), 1);
}

#[test]
fn the_cache_is_keyed_by_the_bytes() {
    let same = b"the same picture, twice".to_vec();
    let mut r = run(teams_with(vec![with_hosted(vec![("aDE=", same.clone()), ("aDI=", same.clone())])]));
    let (one, two) = (hosted(&r, "aDE="), hosted(&r, "aDI="));
    r.ask(one.clone());
    r.ask(two.clone());
    r.until("both pictures", Duration::from_secs(10), |r| r.pictures().len() == 2);
    assert_eq!(hits(&r.server, "hosted"), 2);
    assert_eq!(r.kept("img"), [sha256(&same)], "one picture");
    assert_eq!(r.kept("imgref").len(), 2, "two places that point at it");
    assert_eq!(r.copy().hosted(CHAT, &first(&r), "aDI=").as_deref(), Some(same.as_slice()));
}

#[test]
fn only_twenty_hosted_images_per_message() {
    let ids: Vec<String> = (0..21).map(|i| format!("img{i:02}")).collect();
    let images = ids.iter().enumerate().map(|(i, h)| (h.as_str(), format!("picture {i}").into_bytes())).collect();
    let mut r = run(teams_with(vec![with_hosted(images)]));
    for h in &ids {
        let p = hosted(&r, h);
        r.ask(p);
    }
    r.until("twenty pictures", Duration::from_secs(60), |r| r.pictures().len() == 20);
    r.turns(Duration::from_secs(5));
    let said: Vec<Picture> = r.pictures().into_iter().map(|(p, _)| p).collect();
    assert_eq!(said.len(), 20);
    assert!(!said.contains(&hosted(&r, "img20")), "the 21st is answered with nothing");
    assert_eq!(hits(&r.server, "hosted"), 20, "nor asked for");
}

/// What the test's stand-in for `get_public` was asked for.
static OUTSIDE: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn outside(url: &str, max: usize) -> crate::Result<Response> {
    assert_eq!(max, MAX_PICTURE);
    OUTSIDE.lock().unwrap().push(url.to_string());
    Ok(Response { status: 200, body: b"GIF89a".to_vec(), ..Response::default() })
}

#[test]
fn an_outside_image_is_https_and_public_only() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 1)]));
    r.w.hooks.outside = Some(outside);
    let refused = [
        "http://images.contoso.example/a.gif",
        "https://127.0.0.1/a.gif",
        "https://10.0.0.1/a.gif",
        "https://192.168.1.20/a.gif",
        "https://169.254.169.254/latest",
        "https://[::1]/a.gif",
        "https://[fd00::1]/a.gif",
        "ftp://images.contoso.example/a.gif",
        "file:///etc/passwd",
        "junk",
        "",
    ];
    for url in refused {
        r.ask(Picture::Outside { url: url.into() });
    }
    r.turns(Duration::from_secs(2));
    assert!(OUTSIDE.lock().unwrap().is_empty(), "no request: {:?}", OUTSIDE.lock().unwrap());
    assert!(r.pictures().is_empty(), "no event");

    // On the internet, over https: fetched at each click, never kept.
    let url = "https://images.contoso.example/a.gif";
    for _ in 0..2 {
        r.ask(Picture::Outside { url: url.into() });
    }
    assert_eq!(*OUTSIDE.lock().unwrap(), [url, url]);
    assert_eq!(r.pictures(), [0, 1].map(|_| (Picture::Outside { url: url.into() }, b"GIF89a".to_vec())));
    assert!(r.kept("img").is_empty() && r.kept("imgref").is_empty());

    // And through the real fetcher, this computer is never reached.
    r.w.hooks.outside = None;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    for url in [format!("https://127.0.0.1:{port}/a.gif"), format!("http://127.0.0.1:{port}/a.gif"), format!("https://localhost:{port}/a.gif")] {
        r.ask(Picture::Outside { url });
    }
    assert!(listener.accept().is_err(), "nothing connected");
    assert_eq!(r.pictures().len(), 2);
}

#[test]
fn pictures_over_10_mb_are_refused_while_reading() {
    let images = vec![("Zml0cw==", vec![1u8; MAX_PICTURE]), ("YmlnZw==", vec![2u8; MAX_PICTURE + 1])];
    let mut r = run(teams_with(vec![with_hosted(images)]));
    let (fits, big) = (hosted(&r, "Zml0cw=="), hosted(&r, "YmlnZw=="));
    r.ask(big.clone());
    r.ask(fits.clone());
    r.until("the picture that fits", Duration::from_secs(10), |r| !r.pictures().is_empty());
    r.turns(Duration::from_secs(3));
    let said = r.pictures();
    assert_eq!(said.len(), 1);
    assert_eq!((&said[0].0, said[0].1.len()), (&fits, MAX_PICTURE));
    assert_eq!(hits(&r.server, "hosted"), 2);
    assert_eq!(r.kept("img"), [sha256(&vec![1u8; MAX_PICTURE])], "the one too big isn't kept");
    // Not asked again for this run.
    r.ask(big);
    r.turns(Duration::from_secs(3));
    assert_eq!(hits(&r.server, "hosted"), 2);
}

#[test]
fn a_missing_photo_is_asked_again_only_after_a_day() {
    let mut teams = teams_with(vec![chat(CHAT, T0, 1)]);
    teams.photos.insert("u-ana".into(), b"JPEG of Ana".to_vec());
    let mut r = run(teams);
    let (ana, bo) = (Picture::Photo { user: "u-ana".into() }, Picture::Photo { user: "u-bo".into() });
    r.ask(ana.clone());
    r.until("Ana's photo", Duration::from_secs(10), |r| !r.pictures().is_empty());
    assert_eq!(r.pictures(), [(ana.clone(), b"JPEG of Ana".to_vec())]);
    r.ask(ana.clone());
    assert_eq!(r.pictures().len(), 2, "from the copy, at once");
    assert_eq!(hits(&r.server, "photo"), 1);

    // Bo has none: initials, and nothing asked again before a day has passed on Graph's clock.
    r.ask(bo.clone());
    r.turns(Duration::from_secs(2));
    assert_eq!(hits(&r.server, "photo"), 2);
    assert_eq!(r.pictures().len(), 2, "nothing said of Bo");
    let until = r.copy().photo("u-bo").1.expect("remembered in the copy");
    assert!((NOW + DAY - 5_000..=NOW + DAY + 5_000).contains(&until), "{until}");
    r.ask(bo.clone());
    r.turns(Duration::from_secs(2));
    r.fake(|t| t.clock = Some(NOW + DAY - MINUTE));
    r.turns(Duration::from_secs(11));
    r.ask(bo.clone());
    r.turns(Duration::from_secs(2));
    assert_eq!(hits(&r.server, "photo"), 2);
    r.fake(|t| t.clock = Some(NOW + DAY + MINUTE));
    r.turns(Duration::from_secs(11));
    r.ask(bo);
    r.turns(Duration::from_secs(2));
    assert_eq!(hits(&r.server, "photo"), 3);
}

#[test]
fn presence_is_asked_for_the_visible_people_every_minute_only() {
    let mut teams = teams_with(vec![chat(CHAT, T0, 1)]);
    teams.presences.insert("u-ana".into(), "Available".into());
    teams.presences.insert("u-bo".into(), "Busy".into());
    let mut r = run(teams);
    // Nobody shown: nothing asked.
    r.turns(Duration::from_secs(130));
    assert_eq!(hits(&r.server, "presence"), 0);

    // Shown: asked at once, each once, an id that can't be one left out.
    r.w.job(Job::Visible { users: ["u-ana", "u-bo", "u-ana", "", "u-x"].map(String::from).to_vec() });
    r.turn();
    assert_eq!(hits(&r.server, "presence"), 1);
    let mut said = r.presences().concat();
    said.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(said, [("u-ana".into(), Presence::Available), ("u-bo".into(), Presence::Busy), ("u-x".into(), Presence::Unknown)]);

    // Others shown within the minute: asked at the minute, not before.
    r.w.job(Job::Visible { users: vec!["u-ana".into()] });
    r.turns(Duration::from_secs(58));
    assert_eq!(hits(&r.server, "presence"), 1);
    r.turns(Duration::from_secs(3));
    assert_eq!(hits(&r.server, "presence"), 2);
    assert_eq!(r.presences().last().unwrap(), &[("u-ana".to_string(), Presence::Available)]);

    // Nobody again, then someone: still a minute apart at least.
    r.w.job(Job::Visible { users: Vec::new() });
    r.turns(Duration::from_secs(30));
    r.w.job(Job::Visible { users: vec!["u-bo".into()] });
    r.turns(Duration::from_secs(20));
    assert_eq!(hits(&r.server, "presence"), 2);
    r.turns(Duration::from_secs(15));
    assert_eq!(hits(&r.server, "presence"), 3);

    // 700 shown: the first 650 asked, said in events of 50 at most; never written to the copy.
    let before = r.presences().len();
    r.w.job(Job::Visible { users: (0..700).map(|i| format!("u-{i}")).collect() });
    r.turns(Duration::from_secs(61));
    let events = &r.presences()[before..];
    assert!(events.iter().all(|e| e.len() <= EVENT_ITEMS), "{:?}", events.iter().map(Vec::len).collect::<Vec<_>>());
    assert_eq!(events.iter().map(Vec::len).sum::<usize>(), PRESENCE_IDS);
    assert!(r.kept("").iter().all(|k| !k.contains("presence")), "{:?}", r.kept(""));
}

#[test]
fn a_deleted_message_brings_no_picture_back() {
    let mut r = run(teams_with(vec![with_hosted(vec![("aDE=", b"one".to_vec()), ("aDI=", b"two".to_vec())])]));
    let (one, two) = (hosted(&r, "aDE="), hosted(&r, "aDI="));
    r.ask(one.clone());
    r.until("the first picture", Duration::from_secs(10), |r| r.pictures().len() == 1);
    // Graph says it was deleted: what it showed goes from the copy.
    let m: ChatMessage = r.copy().last_pages(CHAT, 1).into_iter().find(|m| m.id == first(&r)).unwrap();
    let gone = ChatMessage { deleted: Some(NOW), modified: NOW, ..m };
    r.copy().upsert(CHAT, &[gone]).unwrap();
    assert!(r.kept("img").is_empty());
    // A window that still shows it asks for both: neither is fetched, nor kept again.
    r.ask(one);
    r.ask(two);
    r.turns(Duration::from_secs(5));
    assert_eq!(hits(&r.server, "hosted"), 1);
    assert_eq!(r.pictures().len(), 1);
    assert!(r.kept("img").is_empty() && r.kept("imgref").is_empty());
}

#[test]
fn nothing_is_asked_under_an_empty_id() {
    let mut r = run(teams_with(vec![with_hosted(vec![("aDE=", b"one".to_vec())])]));
    let m = first(&r);
    for p in [
        Picture::Hosted { chat: String::new(), message: m.clone(), hosted: "aDE=".into() },
        Picture::Hosted { chat: CHAT.into(), message: String::new(), hosted: "aDE=".into() },
        Picture::Hosted { chat: CHAT.into(), message: m.clone(), hosted: String::new() },
        // Not one of the message's own.
        Picture::Hosted { chat: CHAT.into(), message: m.clone(), hosted: "b3RoZXI=".into() },
        Picture::Hosted { chat: CHAT.into(), message: "1".into(), hosted: "aDE=".into() },
        Picture::Photo { user: String::new() },
    ] {
        r.ask(p);
    }
    r.w.job(Job::Visible { users: vec![String::new()] });
    r.turns(Duration::from_secs(70));
    assert_eq!((hits(&r.server, "hosted"), hits(&r.server, "photo"), hits(&r.server, "presence")), (0, 0, 0));
    assert!(r.pictures().is_empty());
}

fn who() -> Identity {
    Identity { tenant_id: "t-1".into(), object_id: "u-1".into() }
}

#[test]
fn the_image_cache_lets_the_least_recently_used_go() {
    let s = Store::memory();
    let (c, _) = Copy::open(s.clone(), &who()).unwrap();
    c.cap_images(250, 100);
    let a = |i: u8| vec![i; 100];
    c.put_hosted(CHAT, "1", "a", &a(1)).unwrap();
    c.put_hosted(CHAT, "2", "b", &a(2)).unwrap();
    // The first is shown again: the second is now the least recently used, and goes.
    assert_eq!(c.hosted(CHAT, "1", "a"), Some(a(1)));
    c.put_hosted(CHAT, "3", "c", &a(3)).unwrap();
    assert_eq!(c.hosted(CHAT, "2", "b"), None);
    assert_eq!(c.hosted(CHAT, "1", "a"), Some(a(1)));
    assert_eq!(c.hosted(CHAT, "3", "c"), Some(a(3)));
    let kept = |s: &Store| s.account_files(TEAMS_ACCOUNT, "img").unwrap().len();
    assert_eq!(kept(&s), 2);

    // The order is the copy's, from one opening to the next.
    drop(c);
    let (c, _) = Copy::open(s.clone(), &who()).unwrap();
    c.cap_images(250, 100);
    c.put_photo("u-ana", &a(4)).unwrap();
    assert_eq!(c.hosted(CHAT, "1", "a"), None, "used before the third");
    assert_eq!(c.hosted(CHAT, "3", "c"), Some(a(3)));
    assert_eq!(c.photo("u-ana").0, Some(a(4)));
    assert_eq!(kept(&s), 2);

    // A bound on how many, whatever their sizes.
    c.cap_images(1 << 30, 2);
    c.put_hosted(CHAT, "5", "e", &[5]).unwrap();
    assert_eq!(kept(&s), 2);
    assert_eq!(c.hosted(CHAT, "3", "c"), None);
}
