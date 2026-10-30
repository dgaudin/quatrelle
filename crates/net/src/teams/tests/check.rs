//! The step 0 check against the fake Graph server: what it counts, that it says nothing
//! private, and that it leaves no test message behind.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;
use crate::teams::check;

fn opts() -> CheckOptions {
    CheckOptions { tenant: None, write_chat: None, hour: None, sleep: |_| {}, renew: None }
}

/// The check run on a fresh client: its result and the lines it said.
fn check(server: &Server, opts: &CheckOptions) -> (crate::Result<()>, Vec<String>) {
    let (c, me) = TeamsClient::connect(&server.base, TOKEN).unwrap();
    let mut lines = Vec::new();
    let r = run_check(&c, &me, opts, &mut |l| lines.push(l.to_string()));
    (r, lines)
}

/// The line that starts with `start`.
fn line<'a>(lines: &'a [String], start: &str) -> &'a str {
    lines.iter().find(|l| l.starts_with(start)).unwrap_or_else(|| panic!("no {start:?} in {lines:#?}"))
}

fn teams_of(server: &Server) -> FakeTeams {
    server.data.lock().unwrap().teams.clone().unwrap()
}

/// This computer's clock in milliseconds (tests only: the check itself never reads it).
fn clock() -> i64 {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap()
}

fn kinded(id: &str, kind: &'static str, start: i64, n: i64) -> FakeChat {
    FakeChat { kind, ..chat(id, start, n) }
}

#[test]
fn check_counts_by_kind_and_reaction_forms() {
    let mut newest = kinded("19:new@thread.v2", "group", T0 + 1000 * MINUTE, 5);
    newest.messages[0].reactions = vec![("like".into(), "u-ana".into()), ("heart".into(), ME.into())];
    newest.messages[2].reactions = vec![("😀".into(), "u-ana".into())];
    newest.messages[3].attachments = vec![("reference".into(), SHARED.into()), ("reference".into(), "https://x.example/b".into())];
    newest.messages[4].attachments = vec![("application/vnd.microsoft.card.adaptive".into(), String::new())];
    let chats = vec![
        kinded("19:a@unq.gbl.spaces", "oneOnOne", T0, 1),
        kinded("19:b@unq.gbl.spaces", "oneOnOne", T0, 2),
        kinded("19:c@unq.gbl.spaces", "oneOnOne", T0, 0),
        newest,
        kinded("19:g@thread.v2", "group", T0, 3),
        kinded("19:meeting_m@thread.v2", "meeting", T0, 1),
    ];
    let mut t = teams_with(chats);
    t.shares.insert(SHARED.into(), b"hello".to_vec());
    let server = serve_teams(t);
    let (r, lines) = check(&server, &opts());
    r.unwrap();
    assert_eq!(line(&lines, "chats:"), "chats: 6 (one-to-one 3, group 2, meeting 1, other 0)");
    assert_eq!(line(&lines, "messages in the newest chat:"), "messages in the newest chat: 5");
    assert_eq!(line(&lines, "reactions:"), "reactions: 3 on 2 messages (by name 2, as emoji 1, other 0)");
    assert_eq!(line(&lines, "file attachments:"), "file attachments: 2, the first downloads: yes, 5 bytes");
    assert!(line(&lines, "in all:").starts_with("in all: requests "));
    assert!(!lines.iter().any(|l| l.starts_with("write") || l.starts_with("hour")), "{lines:#?}");
    // Every message read carried a range.
    assert!(teams_of(&server).message_reads.iter().all(|(_, f)| f.starts_with("lastModifiedDateTime lt ")));
}

#[test]
fn check_caps_listings_at_ten_pages() {
    let mut chats: Vec<FakeChat> = (0..520).map(|i| chat(&format!("19:c{i}@thread.v2"), T0, 0)).collect();
    chats.push(chat(CHAT, T0, 520));
    let server = serve_teams(teams_with(chats));
    let (r, lines) = check(&server, &opts());
    r.unwrap();
    assert!(line(&lines, "chats:").starts_with("chats: at least 500 "), "{lines:#?}");
    assert_eq!(line(&lines, "messages in the newest chat:"), "messages in the newest chat: at least 500");
    assert_eq!((hits(&server, "chats"), hits(&server, "messages")), (10, 10));
}

#[test]
fn check_on_an_empty_account() {
    let server = serve_teams(teams_with(Vec::new()));
    let (r, lines) = check(&server, &opts());
    r.unwrap();
    assert_eq!(line(&lines, "chats:"), "chats: 0 (one-to-one 0, group 0, meeting 0, other 0)");
    assert_eq!(line(&lines, "messages in the newest chat:"), "messages in the newest chat: 0 (no chat)");
    assert_eq!(line(&lines, "reactions:"), "reactions: 0 on 0 messages (by name 0, as emoji 0, other 0)");
    assert_eq!(line(&lines, "file attachments:"), "file attachments: 0, the first downloads: none to try");

    // One chat, without a message.
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 0)]));
    let (r, lines) = check(&server, &opts());
    r.unwrap();
    assert_eq!(line(&lines, "chats:"), "chats: 1 (one-to-one 1, group 0, meeting 0, other 0)");
    assert_eq!(line(&lines, "messages in the newest chat:"), "messages in the newest chat: 0");
    assert_eq!(line(&lines, "file attachments:"), "file attachments: 0, the first downloads: none to try");
}

/// A chat to write in, with one message from Ana.
fn write_server(change: impl FnOnce(&mut FakeTeams)) -> Server {
    let mut t = teams_with(vec![chat(CHAT, T0, 1)]);
    change(&mut t);
    serve_teams(t)
}

fn write_opts() -> CheckOptions {
    CheckOptions { write_chat: Some(CHAT.into()), ..opts() }
}

/// The messages the check sent in the write chat: (text, deleted).
fn mine(server: &Server) -> Vec<(String, bool)> {
    let t = teams_of(server);
    let chat = t.chat(CHAT).unwrap();
    chat.messages.iter().filter(|m| m.from.as_deref() == Some(ME)).map(|m| (m.body_html.clone(), m.deleted.is_some())).collect()
}

#[test]
fn check_write_probes_answer_yes() {
    let server = write_server(|_| {});
    let (r, lines) = check(&server, &write_opts());
    r.unwrap();
    for (start, said) in [
        ("write, send (1/2):", "yes"),
        ("write, react:", "yes"),
        ("write, the reaction moves lastModifiedDateTime:", "yes"),
        ("write, unset the reaction:", "yes"),
        ("write, edit:", "yes"),
        ("write, soft delete by /users:", "yes"),
        ("write, send (2/2):", "yes"),
        ("write, soft delete by /me:", "yes"),
        ("write, mark the chat read:", "yes"),
        ("write, test messages left undeleted:", "0"),
    ] {
        assert_eq!(line(&lines, start), format!("{start} {said}"));
    }
    let sent = mine(&server);
    assert_eq!(sent.len(), 2);
    assert!(sent.iter().all(|(_, deleted)| *deleted), "{sent:?}");
    assert_eq!(sent[0].0, "Quatrelle check, please ignore (1/2, edited)");
    assert_eq!(sent[1].0, "Quatrelle check, please ignore (2/2)");
    let t = teams_of(&server);
    assert!(t.chat(CHAT).unwrap().messages.iter().all(|m| m.reactions.is_empty()), "the reaction taken back");
    assert_ne!(t.chat(CHAT).unwrap().read_time, "2026-01-01T00:00:00.000Z", "marked read");
    assert_eq!((hits(&server, "setReaction"), hits(&server, "unsetReaction"), hits(&server, "softDelete")), (1, 1, 2));

    // The tenant given wins over the one Graph says; a wrong one is refused by Graph, and said.
    let server = write_server(|_| {});
    let (r, lines) = check(&server, &CheckOptions { tenant: Some("another".into()), ..write_opts() });
    r.unwrap();
    assert_eq!(line(&lines, "write, mark the chat read:"), "write, mark the chat read: no (400)");
}

#[test]
fn check_write_probes_clean_up_after_a_failure() {
    // Posting refused: said, nothing sent, nothing else tried on a message.
    let server = write_server(|t| t.forbid_post.push(CHAT.into()));
    let (r, lines) = check(&server, &write_opts());
    r.unwrap();
    assert_eq!(line(&lines, "write, send (1/2):"), "write, send (1/2): no (403)");
    assert_eq!(line(&lines, "write, send (2/2):"), "write, send (2/2): no (403)");
    assert_eq!(line(&lines, "write, react:"), "write, react: not tried");
    assert_eq!(line(&lines, "write, test messages left undeleted:"), "write, test messages left undeleted: 0");
    assert!(mine(&server).is_empty());
    assert_eq!(hits(&server, "softDelete"), 0);

    // The reaction fails: the rest goes on, and both messages end deleted.
    let server = write_server(|t| _ = t.fail_next.insert("setReaction".into(), 1));
    let (r, lines) = check(&server, &write_opts());
    r.unwrap();
    assert_eq!(line(&lines, "write, react:"), "write, react: no (500)");
    assert_eq!(line(&lines, "write, edit:"), "write, edit: yes");
    let sent = mine(&server);
    assert_eq!(sent.len(), 2);
    assert!(sent.iter().all(|(_, deleted)| *deleted), "{sent:?}");

    // A soft delete fails: the message is deleted on the way out all the same.
    let server = write_server(|t| _ = t.fail_next.insert("softDelete".into(), 1));
    let (r, lines) = check(&server, &write_opts());
    r.unwrap();
    assert_eq!(line(&lines, "write, soft delete by /users:"), "write, soft delete by /users: no (500)");
    assert_eq!(line(&lines, "write, test messages left undeleted:"), "write, test messages left undeleted: 0");
    let sent = mine(&server);
    assert!(sent.len() == 2 && sent.iter().all(|(_, deleted)| *deleted), "{sent:?}");
    assert_eq!(hits(&server, "softDelete"), 3);

    // Every way of deleting fails: both forms tried, and what is left said.
    let server = write_server(|t| _ = t.fail_next.insert("softDelete".into(), 100));
    let (r, lines) = check(&server, &write_opts());
    r.unwrap();
    assert_eq!(line(&lines, "write, test messages left undeleted:"), "write, test messages left undeleted: 2");
    assert_eq!(hits(&server, "softDelete"), 2 + 2 * 2);
}

#[test]
fn check_sweeps_a_send_whose_answer_was_lost() {
    // The first post is made but its answer lost (503): the check can't know its id, so it
    // looks for its own text among the chat's newest messages and deletes it.
    let server = write_server(|t| _ = t.answer_lost.insert("post".into(), 1));
    let (r, lines) = check(&server, &write_opts());
    r.unwrap();
    assert_eq!(line(&lines, "write, send (1/2):"), "write, send (1/2): no (503)");
    assert_eq!(line(&lines, "write, send (2/2):"), "write, send (2/2): yes");
    assert_eq!(line(&lines, "write, stray test messages swept:"), "write, stray test messages swept: 1");
    assert_eq!(line(&lines, "write, test messages left undeleted:"), "write, test messages left undeleted: 0");
    let sent = mine(&server);
    assert_eq!(sent.len(), 2, "posted once each: {sent:?}");
    assert!(sent.iter().all(|(_, deleted)| *deleted), "{sent:?}");
    // Ana's message, and one of mine from before the check, are left alone.
    let server = write_server(|t| {
        _ = t.answer_lost.insert("post".into(), 2);
        let c = &mut t.chats[0];
        c.messages.push(FakeMessage::new("old-mine", Some(ME), &rfc3339(T0 + MINUTE), "Quatrelle check, please ignore (1/2)"));
    });
    let (r, lines) = check(&server, &write_opts());
    r.unwrap();
    assert_eq!(line(&lines, "write, stray test messages swept:"), "write, stray test messages swept: 2");
    let t = teams_of(&server);
    let deleted: Vec<&str> = t.chat(CHAT).unwrap().messages.iter().filter(|m| m.deleted.is_some()).map(|m| m.id.as_str()).collect();
    assert_eq!(deleted.len(), 2, "{deleted:?}");
    assert!(!deleted.contains(&"old-mine"));
    // Without a lost answer, nothing is swept, and the line isn't said.
    let server = write_server(|_| {});
    let (_, lines) = check(&server, &write_opts());
    assert!(!lines.iter().any(|l| l.starts_with("write, stray")), "{lines:#?}");
}

#[test]
fn check_says_nothing_private() {
    let secret_tenant = "tenant-7f3e-zq";
    let me_secret = FakeUser {
        id: "oid-5c1d-zq".into(),
        display_name: "Zebulon Quixote".into(),
        mail: "zq@secret.example".into(),
        upn: "zq@secret.example".into(),
    };
    let other = FakeUser::new("oid-9b2a-other", "Philippa Nightjar");
    let mut c = FakeChat::new("19:quixote-chat@thread.v2", "group", Some("Project Nightingale"), vec![me_secret.clone(), other.clone()]);
    let now = clock();
    for i in 0..4 {
        let at = now - (10 - i) * MINUTE;
        let mut m = FakeMessage::new(&format!("msg-quixote-{i}"), Some("oid-9b2a-other"), &rfc3339(at), "<p>secret-banana</p>");
        m.reactions = vec![("like".into(), "oid-9b2a-other".into())];
        c.messages.push(m);
    }
    let secret_url = "https://contoso.sharepoint.com/sites/nightingale/secret-banana.pdf";
    c.messages[1].attachments = vec![("reference".into(), secret_url.into())];
    let mut t = FakeTeams { chats: vec![c], ..FakeTeams::new(me_secret.clone(), secret_tenant) };
    t.shares.insert(secret_url.into(), b"secret-banana".to_vec());
    let server = serve_teams(t);
    let all = CheckOptions {
        tenant: Some(secret_tenant.into()),
        write_chat: Some("19:quixote-chat@thread.v2".into()),
        hour: Some(Duration::from_millis(1500)),
        sleep: |d| std::thread::sleep(d.min(Duration::from_millis(50))),
        renew: None,
    };
    let (r, mut lines) = check(&server, &all);
    r.unwrap();
    // The tenant found by the check itself, and a write chat Graph doesn't know.
    let (r, more) = check(&server, &CheckOptions { tenant: None, hour: None, ..all });
    r.unwrap();
    lines.extend(more);
    let (r, more) = check(&server, &CheckOptions { write_chat: Some("19:no-such-quixote@thread.v2".into()), ..opts() });
    r.unwrap();
    lines.extend(more);
    assert!(lines.iter().any(|l| l == "write, mark the chat read: yes"), "{lines:#?}");
    let t = teams_of(&server);
    let mut private: Vec<String> =
        ["Zebulon", "Quixote", "quixote", "zq@", "secret", "Nightingale", "Nightjar", "Philippa", "banana", "sharepoint", secret_tenant, "oid-"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    for c in &t.chats {
        private.push(c.id.clone());
        private.extend(c.messages.iter().map(|m| m.id.clone()));
    }
    for l in &lines {
        for p in &private {
            assert!(!l.contains(p.as_str()), "{p:?} in {l:?}");
        }
        assert!(l.is_ascii() && !l.contains('@') && !l.contains("//"), "{l:?}");
    }
}

#[test]
fn check_survives_throttling() {
    let mut c = chat(CHAT, T0, 3);
    c.messages[0].attachments = vec![("reference".into(), SHARED.into())];
    let mut t = teams_with(vec![c]);
    t.shares.insert(SHARED.into(), vec![1; 10]);
    t.throttle_every = Some(3);
    let server = serve_teams(t);
    let (r, lines) = check(&server, &opts());
    r.unwrap();
    assert_eq!(line(&lines, "messages in the newest chat:"), "messages in the newest chat: 3");
    assert_eq!(line(&lines, "file attachments:"), "file attachments: 1, the first downloads: yes, 10 bytes");
    let all = line(&lines, "in all:");
    let throttled: u32 = all.rsplit("throttled ").next().unwrap().parse().unwrap();
    assert!(throttled > 0, "{all}");
}

#[test]
fn check_hour_runs_the_cadences() {
    let now = clock();
    let recent = |id: &str, ago_min: i64| chat(id, now - ago_min * MINUTE, 2);
    let chats = vec![
        recent("19:newest@thread.v2", 3),
        recent("19:yesterday@thread.v2", 24 * 60),
        recent("19:last-week@thread.v2", 6 * 24 * 60),
        recent("19:last-month@thread.v2", 30 * 24 * 60),
    ];
    let server = serve_teams(teams_with(chats));
    let opts = CheckOptions { hour: Some(Duration::from_millis(2500)), sleep: |d| std::thread::sleep(d.min(Duration::from_millis(50))), ..opts() };
    super::super::check::REPORT_EVERY_IN_TESTS.set(Some(Duration::from_secs(1)));
    let (r, lines) = check(&server, &opts);
    super::super::check::REPORT_EVERY_IN_TESTS.set(None);
    r.unwrap();
    let hour: Vec<&String> = lines.iter().filter(|l| l.starts_with("hour")).collect();
    let (reports, end) = hour.split_at(hour.len() - 1);
    assert!(!reports.is_empty() && reports.iter().all(|l| l.starts_with("hour, after ") && l.contains(" min: requests ")), "{lines:#?}");
    assert!(end[0].starts_with("hour, 2 s: requests ") && end[0].ends_with(", failed 0, 429 rate 0.0 %"), "{}", end[0]);
    // What happens at the start is certain; what the clock adds after depends on the machine.
    let t = teams_of(&server);
    let lists = hits(&server, "chats");
    assert!(lists >= 2, "the check's list, then the cadence's: {lists}");
    let reads = |chat: &str| t.message_reads.iter().filter(|(c, _)| c == chat).count();
    assert!(t.message_reads.iter().all(|(_, f)| f.starts_with("lastModifiedDateTime gt ") || f.starts_with("lastModifiedDateTime lt ")));
    assert!(reads("19:newest@thread.v2") >= 2, "the check's read, then the cadence's: {:?}", t.message_reads);
    assert_eq!((reads("19:yesterday@thread.v2"), reads("19:last-week@thread.v2"), reads("19:last-month@thread.v2")), (1, 1, 0));
    for c in &t.chats {
        assert!(reads(&c.id) <= lists as usize * 4, "{}: {} reads for {lists} lists", c.id, reads(&c.id));
    }
}

#[test]
fn check_hour_stops_at_its_deadline() {
    // Every request throttled once (a second each): the first pass alone, over twenty active
    // chats, would take twenty seconds. The hour stops at its end, one request late at most.
    let now = clock();
    let chats = (0..22).map(|i| chat(&format!("19:busy{i}@thread.v2"), now - (i + 1) * MINUTE, 1)).collect();
    let mut t = teams_with(chats);
    t.throttle_every = Some(2);
    let server = serve_teams(t);
    let opts = CheckOptions { hour: Some(Duration::from_secs(2)), sleep: |d| std::thread::sleep(d.min(Duration::from_millis(50))), ..opts() };
    let (r, lines) = check(&server, &opts);
    r.unwrap();
    let end = line(&lines, "hour, ");
    let secs: u64 = end.trim_start_matches("hour, ").split(' ').next().unwrap().parse().unwrap();
    assert!(secs <= 2 + 1, "a request's wait past the end at most: {end}");
    assert!(end.ends_with(", cut short at the end"), "{end}");
    assert!(teams_of(&server).message_reads.len() < 10, "{}", teams_of(&server).message_reads.len());
}

#[test]
fn check_without_a_date_uses_graphs_newest_time() {
    // No `Date` anywhere: "now" is just after the newest time Graph wrote, never this computer's.
    let chats = r#"{"value": [
        {"id": "19:old@thread.v2", "chatType": "group", "lastMessagePreview": {"createdDateTime": "2026-10-20T08:00:00Z"}},
        {"id": "19:new@thread.v2", "chatType": "oneOnOne", "lastMessagePreview": {"createdDateTime": "2026-10-29T10:00:00.500Z"}}
    ]}"#;
    let (base, h) = crate::http::tests::serve(vec![answer(200, ME_JSON), answer(200, chats), answer(200, r#"{"value": []}"#)]);
    let (c, me) = TeamsClient::connect(&base, "t").unwrap();
    let mut lines = Vec::new();
    run_check(&c, &me, &opts(), &mut |l| lines.push(l.to_string())).unwrap();
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 3);
    assert!(
        seen[2].starts_with("GET /chats/19%3Anew%40thread.v2/messages?$filter=lastModifiedDateTime%20lt%202026-10-29T10%3A00%3A00.501Z&"),
        "{}",
        seen[2]
    );
    assert_eq!(line(&lines, "messages in the newest chat:"), "messages in the newest chat: 0");
}

#[test]
fn check_hour_goes_on_past_a_failing_chat() {
    // One active chat's reads fail every time: the other active chats are read all the same.
    let now = clock();
    let recent = |id: &str, ago_min: i64| chat(id, now - ago_min * MINUTE, 2);
    let chats = vec![recent("19:newest@thread.v2", 1), recent("19:broken@thread.v2", 2), recent("19:b@thread.v2", 3), recent("19:c@thread.v2", 4)];
    let mut t = teams_with(chats);
    t.fail_reads.push("19:broken@thread.v2".into());
    let server = serve_teams(t);
    let opts = CheckOptions { hour: Some(Duration::from_millis(500)), sleep: |d| std::thread::sleep(d.min(Duration::from_millis(50))), ..opts() };
    let (r, lines) = check(&server, &opts);
    r.unwrap();
    let t = teams_of(&server);
    let reads = |chat: &str| t.message_reads.iter().filter(|(c, _)| c == chat).count();
    assert_eq!((reads("19:broken@thread.v2"), reads("19:b@thread.v2"), reads("19:c@thread.v2")), (1, 1, 1), "{:?}", t.message_reads);
    assert!(line(&lines, "hour, ").contains(", failed 1, "), "{lines:#?}");
    assert!(!line(&lines, "hour, ").contains("cut short"), "{lines:#?}");
}

#[test]
fn the_sweep_matches_only_the_sent_texts() {
    let since = T0;
    let msg = |from: &str, text: &str, at: i64| {
        serde_json::json!({"id": "1", "from": {"user": {"id": from}}, "body": {"contentType": "text", "content": text},
            "createdDateTime": rfc3339(at), "deletedDateTime": null})
    };
    assert!(check::stray(&msg(ME, "Quatrelle check, please ignore (1/2)", since), ME, since));
    assert!(check::stray(&msg(ME, "<p>Quatrelle check, please ignore (2/2)</p>", since + 1), ME, since));
    // The edited text comes only from a message the check knows: a lost send never carries it.
    assert!(!check::stray(&msg(ME, "Quatrelle check, please ignore (1/2, edited)", since), ME, since));
    assert!(!check::stray(&msg("u-ana", "Quatrelle check, please ignore (1/2)", since), ME, since));
    assert!(!check::stray(&msg(ME, "Quatrelle check, please ignore (1/2)", since - 1), ME, since));
    assert!(!check::stray(&msg(ME, "Quatrelle check, please ignore (1/2) and more", since), ME, since));
    let mut deleted = msg(ME, "Quatrelle check, please ignore (1/2)", since);
    deleted["deletedDateTime"] = serde_json::json!(rfc3339(since));
    assert!(!check::stray(&deleted, ME, since));
}

#[test]
fn check_without_a_date_sweeps_nothing() {
    // No `Date`: the check can't say when it began, so a lost send isn't looked for among the
    // chat's messages (Graph's older times would let a real message in); it is said as left.
    let chats =
        r#"{"value": [{"id": "19:new@thread.v2", "chatType": "oneOnOne", "lastMessagePreview": {"createdDateTime": "2026-10-20T10:00:00Z"}}]}"#;
    let lost = || b"HTTP/1.1 503 X\r\nContent-Length: 0\r\n\r\n".to_vec();
    let (base, h) =
        crate::http::tests::serve(vec![answer(200, ME_JSON), answer(200, chats), answer(200, r#"{"value": []}"#), lost(), lost(), answer(204, "")]);
    let (c, me) = TeamsClient::connect(&base, "t").unwrap();
    let mut lines = Vec::new();
    let opts = CheckOptions { tenant: Some(TENANT.into()), ..write_opts() };
    run_check(&c, &me, &opts, &mut |l| lines.push(l.to_string())).unwrap();
    assert_eq!(line(&lines, "write, stray test messages swept:"), "write, stray test messages swept: no (no server time)");
    assert_eq!(line(&lines, "write, test messages left undeleted:"), "write, test messages left undeleted: 2");
    assert_eq!(h.join().unwrap().len(), 6, "no read of the chat for the sweep");
}

/// Chats for the hour: the newest, then three more active this week.
fn hour_chats() -> Vec<FakeChat> {
    let now = clock();
    let recent = |id: &str, ago_min: i64| chat(id, now - ago_min * MINUTE, 2);
    vec![recent("19:newest@thread.v2", 1), recent("19:a@thread.v2", 2), recent("19:b@thread.v2", 3), recent("19:c@thread.v2", 4)]
}

/// A short hour, renewing the token with `renew` (its calls counted).
fn renewing(renew: impl Fn() -> Option<String> + 'static) -> (CheckOptions, std::rc::Rc<std::cell::Cell<u32>>) {
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = calls.clone();
    let opts = CheckOptions {
        hour: Some(Duration::from_millis(500)),
        sleep: |d| std::thread::sleep(d.min(Duration::from_millis(50))),
        renew: Some(Box::new(move || {
            counted.set(counted.get() + 1);
            renew()
        })),
        ..opts()
    };
    (opts, calls)
}

#[test]
fn check_hour_renews_an_expired_token_and_goes_on() {
    // The token runs out at the hour's first list: renewed, the list is asked again and the
    // hour reads its chats.
    let mut t = teams_with(hour_chats());
    t.token_expires = Some(("chats", 2, "renewed".into()));
    let server = serve_teams(t);
    let (opts, calls) = renewing(|| Some("renewed".into()));
    let (r, lines) = check(&server, &opts);
    r.unwrap();
    let end = line(&lines, "hour, ");
    assert!(end.contains(", failed 0, ") && end.ends_with(", sign-in renewed 1"), "{end}");
    assert_eq!(calls.get(), 1);
    let t = teams_of(&server);
    let reads = |chat: &str| t.message_reads.iter().filter(|(c, _)| c == chat).count();
    assert_eq!((reads("19:newest@thread.v2"), reads("19:a@thread.v2"), reads("19:c@thread.v2")), (2, 1, 1), "{:?}", t.message_reads);

    // Run out in the middle of the active chats' pass: renewed there, the pass goes on.
    let mut t = teams_with(hour_chats());
    t.token_expires = Some(("messages", 4, "renewed".into()));
    let server = serve_teams(t);
    let (opts, calls) = renewing(|| Some("renewed".into()));
    let (r, lines) = check(&server, &opts);
    r.unwrap();
    assert!(line(&lines, "hour, ").ends_with(", failed 0, 429 rate 0.0 %, sign-in renewed 1"), "{lines:#?}");
    assert_eq!(calls.get(), 1);
    let t = teams_of(&server);
    let reads = |chat: &str| t.message_reads.iter().filter(|(c, _)| c == chat).count();
    assert_eq!((reads("19:a@thread.v2"), reads("19:b@thread.v2"), reads("19:c@thread.v2")), (1, 1, 1), "{:?}", t.message_reads);
    assert_eq!(hits(&server, "messages"), 6, "the check's, the open chat's, a, b refused, b again, c");
}

#[test]
fn check_hour_stops_at_once_when_the_token_cant_be_renewed() {
    // The token runs out at the first active chat's read, and can't be renewed: the hour stops
    // there, without asking for the other chats with a token refused.
    let mut t = teams_with(hour_chats());
    t.token_expires = Some(("messages", 3, "renewed".into()));
    let server = serve_teams(t);
    let (opts, calls) = renewing(|| None);
    let (r, lines) = check(&server, &opts);
    r.unwrap();
    assert!(line(&lines, "hour, ").contains(", failed 1, 429 rate 0.0 %, stopped (401)"), "{lines:#?}");
    assert_eq!(calls.get(), 1);
    let t = teams_of(&server);
    assert_eq!(t.message_reads.len(), 2, "the check's and the open chat's: {:?}", t.message_reads);
    assert_eq!(hits(&server, "messages"), 3, "and the refused one, no other");
    assert_eq!(hits(&server, "chats"), 2);

    // Renewed, but the new token is refused too: a second 401 in a row stops the hour.
    let mut t = teams_with(hour_chats());
    t.token_expires = Some(("chats", 2, "renewed".into()));
    let server = serve_teams(t);
    let (opts, calls) = renewing(|| Some("not-the-one".into()));
    let (r, lines) = check(&server, &opts);
    r.unwrap();
    assert!(line(&lines, "hour, ").contains(", failed 1, 429 rate 0.0 %, stopped (401)"), "{lines:#?}");
    assert_eq!(calls.get(), 1);
    assert_eq!(hits(&server, "chats"), 3, "the check's list, the refused one, its one retry");
    assert_eq!(teams_of(&server).message_reads.len(), 1, "the check's read only");
}
