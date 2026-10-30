//! The Teams Graph client against the fake Graph server (and, where the exact request line
//! matters, against a server that only records what it is sent).

use std::collections::HashSet;

use super::*;
use crate::NetError;
use crate::fake_graph::{Data, FakeChat, FakeMessage, FakeTeams, FakeUser, Server, TOKEN, rfc3339, serve};

/// The step 0 check.
mod check;

/// 2026-10-01T00:00:00Z in milliseconds.
const T0: i64 = 1_790_812_800_000;
const MINUTE: i64 = 60_000;
const ME: &str = "u-me";
const TENANT: &str = "tenant-1";
const CHAT: &str = "19:a1b2@thread.v2";

fn me() -> FakeUser {
    FakeUser::new(ME, "Morgan Lee")
}

/// A chat with `n` messages a minute apart from `start`, alternately from Ana and from me.
fn chat(id: &str, start: i64, n: i64) -> FakeChat {
    let mut c = FakeChat::new(id, "oneOnOne", None, vec![me(), FakeUser::new("u-ana", "Ana Lima")]);
    for i in 0..n {
        let at = start + i * MINUTE;
        let from = if i % 2 == 0 { "u-ana" } else { ME };
        c.messages.push(FakeMessage::new(&at.to_string(), Some(from), &rfc3339(at), &format!("<p>Message {i}</p>")));
    }
    c
}

fn teams_with(chats: Vec<FakeChat>) -> FakeTeams {
    FakeTeams { chats, ..FakeTeams::new(me(), TENANT) }
}

fn serve_teams(teams: FakeTeams) -> Server {
    let mut data = Data::default();
    data.teams = Some(teams);
    serve(data)
}

fn client(server: &Server) -> TeamsClient {
    TeamsClient::connect(&server.base, TOKEN).unwrap().0
}

fn hits(server: &Server, route: &str) -> u32 {
    server.data.lock().unwrap().teams.as_ref().unwrap().hits.get(route).copied().unwrap_or(0)
}

fn headers(server: &Server, route: &str) -> Vec<(String, String)> {
    server.data.lock().unwrap().teams.as_ref().unwrap().last_headers.get(route).cloned().unwrap_or_default()
}

/// A raw answer with a JSON body.
fn answer(status: u16, body: &str) -> Vec<u8> {
    format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

const ME_JSON: &str = r#"{"id": "u-me", "displayName": "Morgan Lee", "mail": null, "userPrincipalName": "morgan@contoso.example"}"#;

/// The ids of every message of a range, every page followed.
fn read_all(c: &TeamsClient, chat: &str, range: Range<'_>) -> Vec<i64> {
    let mut out = Vec::new();
    let mut next: Option<String> = None;
    for _ in 0..20 {
        let page = c.messages(chat, range, next.as_deref()).unwrap();
        out.extend(page.items.iter().map(|m| (m["id"].as_str().unwrap().parse::<i64>().unwrap() - T0) / MINUTE));
        match page.next {
            Some(n) => next = Some(n),
            None => return out,
        }
    }
    panic!("the pages never end");
}

#[test]
fn connect_reads_me() {
    let server = serve_teams(teams_with(Vec::new()));
    let (c, me) = TeamsClient::connect(&server.base, TOKEN).unwrap();
    assert_eq!(
        (me.id.as_str(), me.display_name.as_str(), me.mail.as_str(), me.upn.as_str()),
        (ME, "Morgan Lee", "morgan@contoso.example", "morgan@contoso.example")
    );
    assert_eq!(hits(&server, "me"), 1);
    assert!(headers(&server, "me").contains(&("authorization".into(), format!("Bearer {TOKEN}"))));
    assert_eq!((c.stats().requests, c.stats().throttled), (1, 0));
    assert!(matches!(TeamsClient::connect(&server.base, "wrong"), Err(NetError::Auth(m)) if m.contains("401")));
    assert!(TeamsClient::connect("http://graph.example.com/v1.0", TOKEN).is_err(), "https only, except on this computer");

    // A mail that is null is empty; an answer without an id is refused.
    let (base, h) = crate::http::tests::serve(vec![answer(200, ME_JSON), answer(200, r#"{"displayName": "x"}"#)]);
    let (_, me) = TeamsClient::connect(&base, "t").unwrap();
    assert_eq!((me.mail.as_str(), me.upn.as_str()), ("", "morgan@contoso.example"));
    assert!(matches!(TeamsClient::connect(&base, "t"), Err(NetError::Protocol(_))));
    let seen = h.join().unwrap();
    assert!(seen[0].starts_with("GET /me?$select=id,displayName,mail,userPrincipalName HTTP/1.1\r\n"), "{}", seen[0]);
}

#[test]
fn server_time_is_the_servers_date() {
    let dated = |date: &str| {
        format!("HTTP/1.1 200 OK\r\nDate: {date}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{ME_JSON}", ME_JSON.len())
            .into_bytes()
    };
    let (base, h) = crate::http::tests::serve(vec![dated("Thu, 29 Oct 2026 11:30:24 GMT"), answer(200, r#"{"value": []}"#)]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    assert_eq!(c.server_time(), Some(1_793_273_424));
    // An answer without a date leaves the last one known.
    c.chats(None).unwrap();
    assert_eq!(c.server_time(), Some(1_793_273_424));
    h.join().unwrap();
    // The fake's clock, as its Date says it: not before this computer's.
    let server = serve_teams(teams_with(Vec::new()));
    let clock = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let date = client(&server).server_time().unwrap();
    assert!((clock..clock + 5).contains(&date), "{date} vs {clock}");
}

#[test]
fn chats_follow_pages_to_the_end() {
    // Activity times shuffled against the order the chats are kept in.
    let chats = (0..120).map(|i| chat(&format!("19:c{i}@thread.v2"), T0 + (i * 37 % 120) * MINUTE * 10, 1)).collect();
    let server = serve_teams(teams_with(chats));
    let c = client(&server);
    let (mut ids, mut times, mut pages) = (Vec::new(), Vec::new(), 0);
    let mut next: Option<String> = None;
    loop {
        let page = c.chats(next.as_deref()).unwrap();
        pages += 1;
        assert!(page.items.len() <= 50);
        for chat in &page.items {
            ids.push(chat["id"].as_str().unwrap().to_string());
            times.push(chat["lastMessagePreview"]["createdDateTime"].as_str().unwrap().to_string());
        }
        match page.next {
            Some(n) => next = Some(n),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 120);
    assert!(times.windows(2).all(|w| w[0] > w[1]), "newest activity first");
    assert_eq!(hits(&server, "chats"), 3);
    // A next page somewhere else is refused before any request.
    for foreign in ["https://evil.example/v1.0/me/chats?$skiptoken=50", "http://127.0.0.1:1/v1.0/me/chats", "junk"] {
        assert!(c.chats(Some(foreign)).is_err(), "{foreign}");
    }
    assert_eq!(hits(&server, "chats"), 3);
    assert_eq!(c.stats().requests, 4, "/me and three pages");
}

#[test]
fn messages_after_a_watermark_only_newer() {
    let mut ch = chat(CHAT, T0, 60);
    // An old message edited after all the others: newest by lastModifiedDateTime.
    ch.messages[2].modified = rfc3339(T0 + 90 * MINUTE);
    let server = serve_teams(teams_with(vec![ch]));
    let c = client(&server);
    let mid = rfc3339(T0 + 4 * MINUTE + 30_000);
    let newer = read_all(&c, CHAT, Range::After(&mid));
    let mut want: Vec<i64> = vec![2];
    want.extend((5..60).rev());
    assert_eq!(newer, want, "the newer ones, last changed first, over two pages");
    assert_eq!(hits(&server, "messages"), 2);
    assert_eq!(read_all(&c, CHAT, Range::Before(&mid)), [4, 3, 1, 0], "the older ones, newest first");
    // A time in another zone, without milliseconds, is the same instant.
    assert_eq!(read_all(&c, CHAT, Range::After("2026-10-01T02:58:00+02:00")).len(), 2, "after 00:58 UTC: 59 and 2");
    for junk in ["", "yesterday", "2026-13-01T00:00:00Z", "2026-10-01 00:00:00Z", "2026-10-01T00:00:00Z' or 1 eq 1"] {
        let before = hits(&server, "messages");
        assert!(c.messages(CHAT, Range::After(junk), None).is_err(), "{junk}");
        assert_eq!(hits(&server, "messages"), before, "refused before any request: {junk}");
    }
}

#[test]
fn messages_send_prefer_and_range() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 3)]));
    let c = client(&server);
    c.messages(CHAT, Range::After("2026-10-01T00:00:00Z"), None).unwrap();
    assert_eq!(hits(&server, "messages"), 1);
    assert!(headers(&server, "messages").contains(&("prefer".into(), "include-unknown-enum-members".into())));

    // The request lines, exactly.
    let foreign = "https://evil.example/v1.0/chats/x/messages?$skiptoken=2";
    let page = format!(r#"{{"value": [], "@odata.nextLink": "{foreign}"}}"#);
    let (base, h) = crate::http::tests::serve(vec![answer(200, ME_JSON), answer(200, &page), answer(200, r#"{"value": []}"#)]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    let first = c.messages(CHAT, Range::After("2026-10-30T11:00:00.123+01:00"), None).unwrap();
    assert_eq!(first.next.as_deref(), Some(foreign), "handed back as it came");
    assert!(c.messages(CHAT, Range::After("2026-10-30T10:00:00Z"), first.next.as_deref()).is_err(), "but not followed elsewhere");
    c.messages(CHAT, Range::Before("2026-10-30T10:00:00Z"), None).unwrap();
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 3);
    let line = |i: usize| seen[i].lines().next().unwrap().to_string();
    assert_eq!(
        line(1),
        "GET /chats/19%3Aa1b2%40thread.v2/messages?$filter=lastModifiedDateTime%20gt%202026-10-30T10%3A00%3A00.123Z&$top=50&$orderby=lastModifiedDateTime%20desc HTTP/1.1"
    );
    assert_eq!(
        line(2),
        "GET /chats/19%3Aa1b2%40thread.v2/messages?$filter=lastModifiedDateTime%20lt%202026-10-30T10%3A00%3A00Z&$top=50&$orderby=lastModifiedDateTime%20desc HTTP/1.1"
    );
    for req in &seen[1..3] {
        assert!(req.contains("\r\nPrefer: include-unknown-enum-members\r\n"), "{req}");
        assert!(req.contains("\r\nAuthorization: Bearer t\r\n"), "{req}");
    }
}

#[test]
fn ids_with_colon_and_at_are_encoded() {
    let (base, h) = crate::http::tests::serve(vec![
        answer(200, ME_JSON),
        answer(200, r#"{"value": [{"userId": "u-ana"}]}"#),
        answer(204, ""),
        answer(204, ""),
        answer(204, ""),
        answer(204, ""),
        answer(201, r#"{"id": "2"}"#),
        answer(204, ""),
    ]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    assert_eq!(c.members("19:x/y@thread.v2").unwrap().len(), 1);
    c.set_reaction("19:x/y@thread.v2", "17 9", "👍").unwrap();
    c.soft_delete("19:x/y@thread.v2", "1", Some("u?1")).unwrap();
    c.soft_delete("19:x/y@thread.v2", "1", None).unwrap();
    c.mark_read("19:x/y@thread.v2", ME, TENANT).unwrap();
    assert_eq!(c.send_text("19:x/y@thread.v2", "a \"b\"").unwrap()["id"], "2");
    c.edit_text("19:x/y@thread.v2", "17 9", "c").unwrap();
    let seen = h.join().unwrap();
    let line = |i: usize| seen[i].lines().next().unwrap().to_string();
    assert_eq!(line(1), "GET /chats/19%3Ax%2Fy%40thread.v2/members HTTP/1.1");
    assert_eq!(line(2), "POST /chats/19%3Ax%2Fy%40thread.v2/messages/17%209/setReaction HTTP/1.1");
    assert!(seen[2].ends_with(r#"{"reactionType":"👍"}"#), "{}", seen[2]);
    assert_eq!(line(3), "POST /users/u%3F1/chats/19%3Ax%2Fy%40thread.v2/messages/1/softDelete HTTP/1.1");
    assert_eq!(line(4), "POST /me/chats/19%3Ax%2Fy%40thread.v2/messages/1/softDelete HTTP/1.1");
    assert_eq!(line(5), "POST /chats/19%3Ax%2Fy%40thread.v2/markChatReadForUser HTTP/1.1");
    assert!(seen[5].ends_with(r#"{"user":{"id":"u-me","tenantId":"tenant-1"}}"#), "{}", seen[5]);
    assert_eq!(line(6), "POST /chats/19%3Ax%2Fy%40thread.v2/messages HTTP/1.1");
    assert!(seen[6].ends_with(r#"{"body":{"content":"a \"b\"","contentType":"text"}}"#), "{}", seen[6]);
    assert_eq!(line(7), "PATCH /chats/19%3Ax%2Fy%40thread.v2/messages/17%209 HTTP/1.1");
    assert!(seen[7].ends_with(r#"{"body":{"content":"c","contentType":"text"}}"#), "{}", seen[7]);

    // And the fake, which decodes them, finds the chat.
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 2)]));
    let c = client(&server);
    let members = c.members(CHAT).unwrap();
    let ids: Vec<&str> = members.iter().filter_map(|m| m["userId"].as_str()).collect();
    assert_eq!(ids, [ME, "u-ana"]);
}

#[test]
fn writes_reach_the_chat() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 2)]));
    let c = client(&server);
    let made = c.send_text(CHAT, "Hello <b>").unwrap();
    let id = made["id"].as_str().unwrap().to_string();
    assert_eq!(made["body"]["content"], "Hello <b>");
    c.edit_text(CHAT, &id, "Hello again").unwrap();
    c.set_reaction(CHAT, &id, "like").unwrap();
    c.unset_reaction(CHAT, &id, "like").unwrap();
    c.set_reaction(CHAT, &id, "heart").unwrap();
    c.mark_read(CHAT, ME, TENANT).unwrap();
    c.soft_delete(CHAT, &id, Some(ME)).unwrap();
    let d = server.data.lock().unwrap();
    let t = d.teams.as_ref().unwrap();
    let m = t.chat(CHAT).unwrap().messages.iter().find(|m| m.id == id).unwrap();
    assert_eq!(m.body_html, "Hello again");
    assert_eq!(m.reactions, [("heart".to_string(), ME.to_string())]);
    assert!(m.edited.is_some() && m.deleted.is_some());
    assert_ne!(t.chat(CHAT).unwrap().read_time, "2026-01-01T00:00:00.000Z");
    for route in ["post", "edit", "setReaction", "unsetReaction", "markChatReadForUser", "softDelete"] {
        assert!(t.hits.get(route).is_some_and(|n| *n >= 1), "{route}");
    }
}

#[test]
fn a_429_waits_and_is_counted() {
    let mut t = teams_with(vec![chat(CHAT, T0, 3)]);
    t.throttle_every = Some(2);
    let server = serve_teams(t);
    let c = client(&server);
    c.chats(None).unwrap();
    c.members(CHAT).unwrap();
    c.messages(CHAT, Range::After("2026-01-01T00:00:00Z"), None).unwrap();
    c.send_text(CHAT, "hi").unwrap();
    let s = c.stats();
    assert!(s.throttled >= 1, "{}", s.throttled);
    assert_eq!(s.requests, 5 + s.throttled, "each 429 retried once");

    // An answer that stays 429 is given up after four rounds.
    let busy = || b"HTTP/1.1 429 X\r\nRetry-After: 0\r\nContent-Length: 0\r\n\r\n".to_vec();
    let (base, h) = crate::http::tests::serve(vec![answer(200, ME_JSON), busy(), busy(), busy(), busy()]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    assert!(matches!(c.chats(None), Err(NetError::Closed(m)) if m.contains("429")));
    assert_eq!((c.stats().requests, c.stats().throttled), (5, 4));
    assert_eq!(h.join().unwrap().len(), 5);
}

#[test]
fn a_post_is_not_retried_on_503() {
    // A 503 to a post may come after the message was made: posting again could make it twice.
    let busy = || b"HTTP/1.1 503 X\r\nRetry-After: 0\r\nContent-Length: 0\r\n\r\n".to_vec();
    let (base, h) = crate::http::tests::serve(vec![answer(200, ME_JSON), busy(), busy(), answer(204, "")]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    assert!(matches!(c.send_text(CHAT, "hi"), Err(NetError::Closed(m)) if m == "HTTP 503"));
    assert_eq!(c.stats().requests, 2, "sent once");
    // What can be repeated is: a 503 to a reaction is waited out.
    c.set_reaction(CHAT, "1", "like").unwrap();
    assert_eq!(c.stats().requests, 4);
    assert_eq!(h.join().unwrap().len(), 4);
}

#[test]
fn forbidden_post_is_a_protocol_error_with_403() {
    let closed = "19:closed@thread.v2";
    let mut t = teams_with(vec![chat(closed, T0, 1)]);
    t.forbid_post.push(closed.into());
    let server = serve_teams(t);
    let c = client(&server);
    match c.send_text(closed, "hi") {
        Err(NetError::Protocol(m)) => {
            assert!(m.contains("403") && m.contains("Forbidden"), "{m}");
            assert!(!m.contains("Posting"), "never Graph's message text: {m}");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(c.members("19:none@thread.v2"), Err(NetError::Protocol(m)) if m.contains("404")));
}

#[test]
fn errors_carry_the_status_and_code_only() {
    let body = br#"{"error": {"code": "BadRequest", "message": "Ana's secret topic"}}"#;
    assert!(matches!(graph::status_error(400, body), Err(NetError::Protocol(m)) if m == "HTTP 400 BadRequest"));
    assert!(matches!(graph::status_error(401, b"junk"), Err(NetError::Auth(m)) if m.contains("401")));
    assert!(matches!(graph::status_error(503, b""), Err(NetError::Closed(m)) if m == "HTTP 503"));
    assert!(graph::status_error(204, b"").is_ok());
    // A code that isn't a code (spaces, quotes, too long) isn't repeated.
    let odd = br#"{"error": {"code": "Ana Lima said \"no\""}}"#;
    assert!(matches!(graph::status_error(403, odd), Err(NetError::Protocol(m)) if m == "HTTP 403"));
    let long = format!(r#"{{"error": {{"code": "{}"}}}}"#, "A".repeat(200));
    assert!(matches!(graph::status_error(403, long.as_bytes()), Err(NetError::Protocol(m)) if m == "HTTP 403"));
}

#[test]
fn filter_times_are_utc_with_z() {
    assert_eq!(graph::filter_time("2026-10-30T10:00:00Z").as_deref(), Some("2026-10-30T10:00:00Z"));
    assert_eq!(graph::filter_time("2026-10-30T11:00:00.5+01:00").as_deref(), Some("2026-10-30T10:00:00.500Z"));
    assert_eq!(graph::filter_time("2026-10-30T09:12:34.5678912Z").as_deref(), Some("2026-10-30T09:12:34.567Z"));
    assert_eq!(graph::filter_time("2026-10-30t00:30:00-01:30").as_deref(), Some("2026-10-30T02:00:00Z"));
    for junk in [
        "",
        "2026-10-30",
        "2026-10-30T10:00:00",
        "2026-10-30T10:00:00+0100",
        "2026-10-30T10:00:00.Z",
        "2026-13-01T00:00:00Z",
        "2026-10-30T25:00:00Z",
        "2026-10-30T10:00:00Z and 1 eq 1",
        "1969-12-31T23:59:59Z",
        "２026-10-30T10:00:00Z",
    ] {
        assert_eq!(graph::filter_time(junk), None, "{junk}");
    }
}

/// A file shared in a chat, by the URL its attachment carries.
const SHARED: &str = "https://contoso.sharepoint.com/sites/team/Shared%20Documents/plan.pdf";

fn with_share(bytes: &[u8]) -> Server {
    let mut t = teams_with(vec![chat(CHAT, T0, 1)]);
    t.shares.insert(SHARED.into(), bytes.to_vec());
    serve_teams(t)
}

#[test]
fn download_shared_counts_bytes_and_drops_the_bearer_on_redirect() {
    let bytes = vec![7u8; 100_000];
    let server = with_share(&bytes);
    let c = client(&server);
    assert_eq!(c.download_shared(SHARED, 50 << 20).unwrap(), 100_000);
    assert_eq!((hits(&server, "shares"), hits(&server, "download")), (1, 1));
    assert!(headers(&server, "shares").iter().any(|(k, v)| k == "authorization" && v == &format!("Bearer {TOKEN}")));
    assert!(!headers(&server, "download").iter().any(|(k, _)| k == "authorization"), "no bearer past the redirect");
    // Nothing there: 404, said as such.
    assert!(matches!(c.download_shared("https://contoso.sharepoint.com/nothing", 1 << 20), Err(NetError::Protocol(m)) if m.contains("404")));

    // A redirect to plain HTTP elsewhere than this computer, or a second redirect, isn't followed.
    let to = |loc: &str| format!("HTTP/1.1 302 Found\r\nLocation: {loc}\r\nContent-Length: 0\r\n\r\n").into_bytes();
    let (base, h) = crate::http::tests::serve(vec![answer(200, ME_JSON), to("http://files.example.com/x")]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    assert!(c.download_shared(SHARED, 1 << 20).is_err());
    assert_eq!(h.join().unwrap().len(), 2);
    let (base, h) = crate::http::tests::serve(vec![answer(200, ME_JSON), to("/hop"), to("/again")]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    assert!(c.download_shared(SHARED, 1 << 20).is_err());
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 3);
    assert!(seen[1].contains("Authorization: Bearer t") && !seen[2].contains("Authorization"), "{}", seen[2]);
    let share = seen[1].lines().next().unwrap();
    assert!(
        share.starts_with(
            "GET /shares/u!aHR0cHM6Ly9jb250b3NvLnNoYXJlcG9pbnQuY29tL3NpdGVzL3RlYW0vU2hhcmVkJTIwRG9jdW1lbnRzL3BsYW4ucGRm/driveItem/content "
        ),
        "{share}"
    );
}

#[test]
fn download_shared_refuses_past_max() {
    let server = with_share(&[1u8; 1000]);
    let c = client(&server);
    assert_eq!(c.download_shared(SHARED, 1000).unwrap(), 1000);
    assert!(matches!(c.download_shared(SHARED, 999), Err(NetError::Protocol(m)) if m.contains("larger")));
    // Chunked, with no length said up front: counted as it comes, and cut at the limit.
    let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n400\r\n".to_vec();
    let mut chunked = chunked;
    chunked.extend_from_slice(&[0u8; 0x400]);
    chunked.extend_from_slice(b"\r\n0\r\n\r\n");
    let (base, h) = crate::http::tests::serve(vec![answer(200, ME_JSON), chunked.clone(), chunked]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    assert_eq!(c.download_shared(SHARED, 0x400).unwrap(), 0x400, "served by Graph itself, without a redirect");
    assert!(c.download_shared(SHARED, 0x3ff).is_err());
    h.join().unwrap();
    assert!(c.download_shared("", 10).is_err(), "no URL, no request");
}
