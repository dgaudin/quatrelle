// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The fake Graph server's Teams routes, asked over plain HTTP: what the Teams client and the
//! check of the next tasks are tested against.

use std::io::{Read, Write};
use std::net::TcpStream;

use super::*;

/// 2026-10-01T00:00:00Z in milliseconds.
const T0: i64 = 1_790_812_800_000;
const MINUTE: i64 = 60_000;
const ME: &str = "u-me";
const TENANT: &str = "tenant-1";
const BEARER: (&str, &str) = ("Authorization", "Bearer test-token");

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// One request to an absolute `http://` URL, read to the end (the fake closes each connection).
fn send(url: &str, method: &str, headers: &[(&str, &str)], body: &str) -> Resp {
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_at(rest.find('/').unwrap());
    let mut s = TcpStream::connect(host).unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).unwrap();
    s.write_all(body.as_bytes()).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let text = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut lines = text.split("\r\n");
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
    Resp { status, headers, body: raw[split + 4..].to_vec() }
}

fn get(server: &Server, path: &str) -> Resp {
    send(&format!("{}{path}", server.base), "GET", &[BEARER], "")
}

fn post(server: &Server, path: &str, body: &str) -> Resp {
    send(&format!("{}{path}", server.base), "POST", &[BEARER, ("Content-Type", "application/json")], body)
}

/// A chat id as it appears in a path.
fn enc(id: &str) -> String {
    id.replace(':', "%3A").replace('@', "%40")
}

fn ms(t: &str) -> i64 {
    // Only the fake's own spelling is read here: `YYYY-MM-DDTHH:MM:SS.mmmZ`.
    let (date, time) = t.split_once('T').unwrap();
    let d: Vec<u32> = date.split('-').map(|x| x.parse().unwrap()).collect();
    let days = quatrelle_model::dates::days_from_civil(i64::from(d[0]), d[1], d[2]);
    let h: Vec<f64> = time.trim_end_matches('Z').split(':').map(|x| x.parse().unwrap()).collect();
    days * 86_400_000 + (h[0] * 3_600_000.0 + h[1] * 60_000.0 + h[2] * 1000.0).round() as i64
}

fn me() -> FakeUser {
    FakeUser::new(ME, "Morgan Lee")
}

/// A chat with `n` messages a minute apart from `start`, alternately from Ana and from me.
fn chat(id: &str, start: i64, n: i64) -> FakeChat {
    let ana = FakeUser::new("u-ana", "Ana Lima");
    let mut c = FakeChat::new(id, "oneOnOne", None, vec![me(), ana]);
    for i in 0..n {
        let at = start + i * MINUTE;
        let from = if i % 2 == 0 { "u-ana" } else { ME };
        c.messages.push(FakeMessage::new(&at.to_string(), Some(from), &rfc3339(at), &format!("<p>Message {i}</p>")));
    }
    c
}

fn serve_teams(teams: FakeTeams) -> Server {
    serve(Data { teams: Some(teams), ..Data::default() })
}

fn teams_with(chats: Vec<FakeChat>) -> FakeTeams {
    FakeTeams { chats, ..FakeTeams::new(me(), TENANT) }
}

/// The messages of a chat read with this query, every page followed: (id, lastModifiedDateTime).
fn read_all(server: &Server, chat: &str, query: &str) -> Vec<(String, String)> {
    let mut url = format!("{}/chats/{}/messages?{query}", server.base, enc(chat));
    let mut out = Vec::new();
    for _ in 0..20 {
        let r = send(&url, "GET", &[BEARER], "");
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let v = r.json();
        for m in v["value"].as_array().unwrap() {
            out.push((m["id"].as_str().unwrap().to_string(), m["lastModifiedDateTime"].as_str().unwrap().to_string()));
        }
        match v["@odata.nextLink"].as_str() {
            Some(next) => url = next.to_string(),
            None => return out,
        }
    }
    panic!("the pages never end");
}

#[test]
fn teams_chats_page_and_order() {
    // Activity times shuffled against the order the chats are kept in.
    let chats = (0..120).map(|i| chat(&format!("19:c{i}@thread.v2"), T0 + (i * 37 % 120) * MINUTE * 10, 1)).collect();
    let server = serve_teams(teams_with(chats));
    let mut url = format!("{}/me/chats?$top=50&$expand=lastMessagePreview&$orderby=lastMessagePreview/createdDateTime%20desc", server.base);
    let (mut ids, mut times, mut pages) = (Vec::new(), Vec::new(), 0);
    loop {
        let r = send(&url, "GET", &[BEARER], "");
        assert_eq!(r.status, 200);
        let v = r.json();
        let page = v["value"].as_array().unwrap();
        assert!(page.len() <= 50);
        pages += 1;
        for c in page {
            ids.push(c["id"].as_str().unwrap().to_string());
            assert_eq!(c["chatType"], "oneOnOne");
            assert_eq!(c["lastMessagePreview"]["body"]["content"], "<p>Message 0</p>");
            assert_eq!(c["lastMessagePreview"]["from"]["user"]["id"], "u-ana");
            times.push(ms(c["lastMessagePreview"]["createdDateTime"].as_str().unwrap()));
        }
        match v["@odata.nextLink"].as_str() {
            Some(next) => url = next.to_string(),
            None => break,
        }
    }
    assert_eq!(pages, 3);
    let distinct: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(distinct.len(), 120);
    assert!(times.windows(2).all(|w| w[0] > w[1]), "newest activity first");
    assert_eq!(get(&server, "/me/chats").json()["value"].as_array().unwrap().len(), 20, "20 a page unless asked");
    assert_eq!(get(&server, "/me/chats?$top=51").status, 400);
}

#[test]
fn teams_me_select_and_members() {
    let server = serve_teams(teams_with(vec![chat("19:a@thread.v2", T0, 1)]));
    let v = get(&server, "/me").json();
    assert_eq!((v["id"].as_str(), v["displayName"].as_str(), v["mail"].as_str()), (Some(ME), Some("Morgan Lee"), Some("morgan@contoso.example")));
    let v = get(&server, "/me?$select=id,displayName").json();
    assert_eq!(v["id"], ME);
    assert!(v.get("mail").is_none() && v.get("userPrincipalName").is_none(), "only what is selected");
    let v = get(&server, &format!("/chats/{}/members", enc("19:a@thread.v2"))).json();
    let users: Vec<&str> = v["value"].as_array().unwrap().iter().map(|m| m["userId"].as_str().unwrap()).collect();
    assert_eq!(users, [ME, "u-ana"]);
    assert_eq!(get(&server, "/chats/19%3Anone%40thread.v2/members").status, 404);
}

#[test]
fn teams_messages_need_a_range_and_honour_it() {
    let id = "19:a@thread.v2";
    let mut c = chat(id, T0, 10);
    // An old message edited after all the others: newest by lastModifiedDateTime.
    c.messages[2].modified = rfc3339(T0 + 30 * MINUTE);
    c.messages[2].edited = Some(c.messages[2].modified.clone());
    let server = serve_teams(teams_with(vec![c]));
    assert_eq!(get(&server, &format!("/chats/{}/messages", enc(id))).status, 400, "no range");
    assert_eq!(get(&server, &format!("/chats/{}/messages?$filter=createdDateTime%20gt%202026-10-01T00:00:00Z", enc(id))).status, 400);
    assert_eq!(get(&server, &format!("/chats/{}/messages?$top=51&$filter=lastModifiedDateTime%20gt%202026-10-01T00:00:00Z", enc(id))).status, 400);
    let mid = rfc3339(T0 + 4 * MINUTE + 30_000);
    let newer = read_all(&server, id, &format!("$top=2&$orderby=lastModifiedDateTime%20desc&$filter=lastModifiedDateTime%20gt%20{mid}"));
    let ids: Vec<i64> = newer.iter().map(|(i, _)| (i.parse::<i64>().unwrap() - T0) / MINUTE).collect();
    assert_eq!(ids, [2, 9, 8, 7, 6, 5], "the newer ones, last changed first");
    let older = read_all(&server, id, &format!("$filter=lastModifiedDateTime%20lt%20{mid}"));
    let ids: Vec<i64> = older.iter().map(|(i, _)| (i.parse::<i64>().unwrap() - T0) / MINUTE).collect();
    assert_eq!(ids, [4, 3, 1, 0], "the older ones, newest first");
    // A time without milliseconds, in another zone, is read too.
    let shifted = read_all(&server, id, "$filter=lastModifiedDateTime%20gt%202026-10-01T02:08:00%2B02:00");
    assert_eq!(shifted.len(), 2, "after 00:08 UTC: messages 9 and 2");
}

#[test]
fn teams_reaction_moves_modified_and_soft_delete_both_forms() {
    let id = "19:a@thread.v2";
    let server = serve_teams(teams_with(vec![chat(id, T0, 6)]));
    let (m0, m1, m3) = (T0.to_string(), (T0 + MINUTE).to_string(), (T0 + 3 * MINUTE).to_string());
    let path = format!("/chats/{}/messages/{m0}", enc(id));
    let modified =
        |m: &str| server.data.lock().unwrap().teams.as_ref().unwrap().chat(id).unwrap().messages.iter().find(|x| x.id == m).unwrap().clone();
    let before = ms(&modified(&m0).modified);
    assert_eq!(post(&server, &format!("{path}/setReaction"), r#"{"reactionType": "👍"}"#).status, 204);
    let after = modified(&m0);
    assert_eq!(after.reactions, [("👍".to_string(), ME.to_string())]);
    assert!(ms(&after.modified) > before, "a reaction moves lastModifiedDateTime");
    let listed = get(
        &server,
        &format!("/chats/{}/messages?$orderby=lastModifiedDateTime%20desc&$filter=lastModifiedDateTime%20gt%20{}", enc(id), rfc3339(before)),
    )
    .json();
    let r = &listed["value"][0];
    assert_eq!((r["id"].as_str(), r["reactions"][0]["reactionType"].as_str()), (Some(m0.as_str()), Some("👍")));
    assert_eq!(r["reactions"][0]["user"]["user"]["id"], ME);
    assert_eq!(post(&server, &format!("{path}/unsetReaction"), r#"{"reactionType": "👍"}"#).status, 204);
    let unset = modified(&m0);
    assert!(unset.reactions.is_empty() && unset.modified > after.modified);

    let gone = post(&server, &format!("/users/{ME}/chats/{}/messages/{m1}/softDelete", enc(id)), "");
    assert_eq!(gone.status, 204);
    let d = modified(&m1);
    assert!(d.deleted.is_some() && d.modified > unset.modified);
    assert_eq!(post(&server, &format!("/me/chats/{}/messages/{m3}/softDelete", enc(id)), "").status, 204);
    assert!(modified(&m3).deleted.is_some());
    assert_eq!(post(&server, &format!("/me/chats/{}/messages/{m0}/softDelete", enc(id)), "").status, 403, "not another's message");
    assert_eq!(post(&server, &format!("/users/u-ana/chats/{}/messages/{m1}/softDelete", enc(id)), "").status, 403, "not as someone else");
    assert_eq!(post(&server, &format!("/chats/{}/messages/123/softDelete", enc(id)), "").status, 404);
    let listed = get(&server, &format!("/chats/{}/messages?$filter=lastModifiedDateTime%20gt%20{}", enc(id), rfc3339(T0))).json();
    let deleted = listed["value"].as_array().unwrap().iter().find(|m| m["id"] == m1.as_str()).unwrap().clone();
    assert!(deleted["deletedDateTime"].is_string(), "a deleted message is still listed, with its time");

    // An edit: edited and modified move together.
    let edit = send(
        &format!("{}/chats/{}/messages/{}", server.base, enc(id), T0 + 5 * MINUTE),
        "PATCH",
        &[BEARER],
        r#"{"body": {"contentType": "html", "content": "<p>Fixed</p>"}}"#,
    );
    assert_eq!(edit.status, 204);
    let e = modified(&(T0 + 5 * MINUTE).to_string());
    assert_eq!((e.body_html.as_str(), e.edited.as_deref()), ("<p>Fixed</p>", Some(e.modified.as_str())));
}

#[test]
fn teams_mark_read_wants_the_right_user() {
    let id = "19:a@thread.v2";
    let server = serve_teams(teams_with(vec![chat(id, T0, 2)]));
    let path = format!("/chats/{}/markChatReadForUser", enc(id));
    let read_time = || server.data.lock().unwrap().teams.as_ref().unwrap().chat(id).unwrap().read_time.clone();
    let before = read_time();
    assert_eq!(post(&server, &path, &format!(r#"{{"user": {{"id": "u-ana", "tenantId": "{TENANT}"}}}}"#)).status, 400);
    assert_eq!(post(&server, &path, &format!(r#"{{"user": {{"id": "{ME}", "tenantId": "other"}}}}"#)).status, 400);
    assert_eq!(post(&server, &path, "junk").status, 400);
    assert_eq!(read_time(), before);
    assert_eq!(post(&server, &path, &format!(r#"{{"user": {{"id": "{ME}", "tenantId": "{TENANT}"}}}}"#)).status, 204);
    let after = read_time();
    assert!(ms(&after) > ms(&before));
    let v = get(&server, "/me/chats").json();
    assert_eq!(v["value"][0]["viewpoint"]["lastMessageReadDateTime"].as_str(), Some(after.as_str()));
}

#[test]
fn teams_post_forbidden_and_throttle() {
    let (open, closed) = ("19:open@thread.v2", "19:closed@thread.v2");
    let mut t = teams_with(vec![chat(open, T0, 2), chat(closed, T0, 2)]);
    t.forbid_post.push(closed.into());
    let server = serve_teams(t);
    let body = r#"{"body": {"contentType": "html", "content": "<p>Hello</p>"}}"#;
    assert_eq!(post(&server, &format!("/chats/{}/messages", enc(closed)), body).status, 403);
    let made = post(&server, &format!("/chats/{}/messages", enc(open)), body);
    assert_eq!(made.status, 201);
    let v = made.json();
    assert_eq!((v["from"]["user"]["id"].as_str(), v["body"]["content"].as_str()), (Some(ME), Some("<p>Hello</p>")));
    assert!(v["id"].as_str().unwrap().parse::<i64>().unwrap() > T0 + MINUTE, "ids increase within a chat");
    {
        let d = server.data.lock().unwrap();
        let t = d.teams.as_ref().unwrap();
        assert_eq!((t.chat(closed).unwrap().messages.len(), t.chat(open).unwrap().messages.len()), (2, 3));
    }
    assert_eq!(send(&format!("{}/me/chats", server.base), "GET", &[], "").status, 401, "a token is wanted");

    server.data.lock().unwrap().teams.as_mut().unwrap().throttle_every = Some(3);
    let statuses: Vec<u16> = (0..6).map(|_| get(&server, "/me").status).collect();
    let hits = server.data.lock().unwrap().teams.as_ref().unwrap().hits.clone();
    // Three requests before these (the 401 one counted): the 6th and the 9th are refused.
    assert_eq!(statuses, [200, 200, 429, 200, 200, 429]);
    let r = (0..3).map(|_| get(&server, "/me")).find(|r| r.status == 429).unwrap();
    assert_eq!(r.header("Retry-After"), Some("1"));
    assert_eq!((hits.get("post"), hits.get("chats"), hits.get("me")), (Some(&2), Some(&1), Some(&6)));
}

#[test]
fn teams_posts_are_kept_bounded_and_of_a_kind() {
    let id = "19:a@thread.v2";
    let mut t = teams_with(vec![chat(id, T0, 2)]);
    t.post_limit = Some(5);
    t.forbid_mark.push(id.into());
    let server = serve_teams(t);
    let fake = |f: &mut dyn FnMut(&mut FakeTeams)| f(server.data.lock().unwrap().teams.as_mut().unwrap());
    let path = format!("/chats/{}/messages", enc(id));
    let body = |kind: &str, content: &str| format!(r#"{{"body": {{"contentType": "{kind}", "content": "{content}"}}}}"#);
    assert_eq!(post(&server, &path, &body("html", "12345")).status, 201);
    let long = post(&server, &path, &body("html", "123456"));
    assert_eq!((long.status, long.json()["error"]["code"].as_str()), (413, Some("RequestEntityTooLarge")));
    assert_eq!(post(&server, &path, &body("xml", "1")).status, 400);
    assert_eq!(post(&server, &path, r#"{"body": {"content": "1"}}"#).status, 400, "no kind said");
    assert_eq!(post(&server, &path, "junk").status, 400);
    fake(&mut |t| {
        // Every post kept as it came, whatever its answer; only the first made a message.
        assert_eq!(t.posts.len(), 5);
        assert_eq!(t.posts[1], (id.to_string(), json!({"body": {"contentType": "html", "content": "123456"}})));
        assert_eq!(t.posts[4].1, Value::Null);
        assert_eq!(t.chat(id).unwrap().messages.len(), 3);
    });
    // Marking that chat read is refused, and changes nothing.
    let read_time = || server.data.lock().unwrap().teams.as_ref().unwrap().chat(id).unwrap().read_time.clone();
    let before = read_time();
    let mark = format!(r#"{{"user": {{"id": "{ME}", "tenantId": "{TENANT}"}}}}"#);
    assert_eq!(post(&server, &format!("/chats/{}/markChatReadForUser", enc(id)), &mark).status, 403);
    assert_eq!(read_time(), before);
    // Without a limit set: 100 KiB.
    fake(&mut |t| t.post_limit = None);
    assert_eq!(post(&server, &path, &body("text", &"a".repeat(100 << 10))).status, 201);
    assert_eq!(post(&server, &path, &body("text", &"a".repeat((100 << 10) + 1))).status, 413);
    // A lost answer's 503 says a Retry-After only when one is set.
    fake(&mut |t| _ = t.answer_lost.insert("post".into(), 2));
    let lost = post(&server, &path, &body("html", "1"));
    assert_eq!((lost.status, lost.header("Retry-After")), (503, None));
    fake(&mut |t| t.lost_retry_after = Some(7));
    let lost = post(&server, &path, &body("html", "2"));
    assert_eq!((lost.status, lost.header("Retry-After")), (503, Some("7")));
    fake(&mut |t| assert_eq!(t.chat(id).unwrap().messages.len(), 6, "both made"));
}

#[test]
fn teams_shares_redirects_then_serves() {
    let url = "https://contoso.sharepoint.com/sites/team/Shared%20Documents/plan.pdf";
    let mut t = teams_with(vec![chat("19:a@thread.v2", T0, 1)]);
    t.shares.insert(url.into(), b"%PDF-1.7 plan".to_vec());
    let server = serve_teams(t);
    let share = format!("u!{}", crate::imap::base64(url.as_bytes()).trim_end_matches('=').replace('/', "_").replace('+', "-"));
    let r = get(&server, &format!("/shares/{share}/driveItem/content"));
    assert_eq!(r.status, 302);
    let to = r.header("Location").unwrap().to_string();
    assert!(to.starts_with(&format!("{}/download/", server.base)));
    let file = send(&to, "GET", &[], "");
    assert_eq!((file.status, file.body.as_slice()), (200, b"%PDF-1.7 plan".as_slice()), "served without a token");
    assert_eq!(get(&server, "/shares/u!aHR0cHM6Ly9ub3doZXJl/driveItem/content").status, 404);
    assert_eq!(send(&format!("{}/download/u!aHR0cHM6Ly9ub3doZXJl", server.base), "GET", &[], "").status, 404);
}

#[test]
fn teams_headers_are_recorded() {
    let id = "19:a@thread.v2";
    let server = serve_teams(teams_with(vec![chat(id, T0, 1)]));
    let url = format!("{}/chats/{}/messages?$filter=lastModifiedDateTime%20lt%202030-01-01T00:00:00Z", server.base, enc(id));
    assert_eq!(send(&url, "GET", &[BEARER, ("Prefer", "include-unknown-enum-members")], "").status, 200);
    assert_eq!(send(&format!("{}/download/x", server.base), "GET", &[], "").status, 404);
    let d = server.data.lock().unwrap();
    let t = d.teams.as_ref().unwrap();
    let messages = &t.last_headers["messages"];
    assert!(messages.contains(&("prefer".to_string(), "include-unknown-enum-members".to_string())));
    assert!(messages.contains(&("authorization".to_string(), "Bearer test-token".to_string())), "names lowercased");
    assert!(!t.last_headers["download"].iter().any(|(k, _)| k == "authorization"));
}

fn send_meeting(server: &Server, method: &str, path: &str, body: &str) -> Resp {
    send(&format!("{}{path}", server.base), method, &[BEARER, ("Content-Type", "application/json")], body)
}

const MEETING: &str = r#"{"subject": "Weekly", "startDateTime": "2026-11-20T09:00:00Z", "endDateTime": "2026-11-20T10:00:00Z"}"#;

fn with_fake(server: &Server, f: impl FnOnce(&mut FakeTeams)) {
    f(server.data.lock().unwrap().teams.as_mut().unwrap());
}

#[test]
fn a_meeting_is_created_changed_and_deleted() {
    let server = serve_teams(teams_with(vec![]));
    let made = send_meeting(&server, "POST", "/me/onlineMeetings", MEETING);
    assert_eq!(made.status, 201);
    let v = made.json();
    let id = v["id"].as_str().unwrap().to_string();
    assert!(id.contains('/') && id.contains('+') && id.contains('='), "{id}");
    assert_eq!((v["subject"].as_str(), v["startDateTime"].as_str()), (Some("Weekly"), Some("2026-11-20T09:00:00Z")));
    let join = v["joinWebUrl"].as_str().unwrap();
    assert!(join.starts_with("https://teams.microsoft.com/l/meetup-join/19%3ameeting_1%40thread.v2/0?context="), "{join}");
    assert_eq!(v["joinMeetingIdSettings"]["joinMeetingId"], "1234567890");
    assert_eq!(v["joinMeetingIdSettings"]["passcode"], "aB3xY9");
    assert_eq!(v["joinMeetingIdSettings"]["isPasscodeRequired"], true);
    assert!(v["meetingOptionsWebUrl"].as_str().unwrap().ends_with("&threadId=19_meeting_1@thread.v2"));
    assert_eq!(v["joinInformation"]["content"], "<div>Microsoft's own invitation text</div>");
    assert!(v["chatInfo"].is_object());
    assert!(v["audioConferencing"].is_null());

    let path = format!("/me/onlineMeetings/{}", id.replace('/', "%2F").replace('+', "%2B").replace('=', "%3D"));
    let changed = send_meeting(&server, "PATCH", &path, r#"{"subject": "Daily"}"#);
    assert_eq!(changed.status, 200);
    assert_eq!(changed.json()["subject"], "Daily");
    assert_eq!(changed.json()["startDateTime"], "2026-11-20T09:00:00Z");
    assert_eq!(send_meeting(&server, "DELETE", &path, "").status, 204);
    let gone = send_meeting(&server, "DELETE", &path, "");
    assert_eq!(gone.status, 404);
    assert_eq!(gone.json()["error"]["code"], "NotFound");
    assert_eq!(send_meeting(&server, "PATCH", &path, "{}").status, 404);
    assert_eq!(send_meeting(&server, "GET", "/me/onlineMeetings/x", "").status, 404);
}

#[test]
fn a_meeting_answers_audio_conferencing_only_when_set() {
    let server = serve_teams(teams_with(vec![]));
    with_fake(&server, |t| t.audio_conferencing = true);
    let v = send_meeting(&server, "POST", "/me/onlineMeetings", MEETING).json();
    assert_eq!(v["audioConferencing"]["tollNumber"], "+1 323-555-0166");
    assert!(v["audioConferencing"]["tollFreeNumber"].is_null());
    assert_eq!(v["audioConferencing"]["conferenceId"], "9999999");
    assert!(v["audioConferencing"]["dialinUrl"].is_string());
}

#[test]
fn a_meeting_without_its_times_is_refused() {
    let server = serve_teams(teams_with(vec![]));
    for body in [
        r#"{"startDateTime": "2026-11-20T09:00:00Z", "endDateTime": "2026-11-20T10:00:00Z"}"#,
        r#"{"subject": "A", "endDateTime": "2026-11-20T10:00:00Z"}"#,
        r#"{"subject": "A", "startDateTime": "2026-11-20T09:00:00Z"}"#,
        r#"{"subject": "A", "startDateTime": 5, "endDateTime": "2026-11-20T10:00:00Z"}"#,
        r#"{"subject": "A", "startDateTime": "2026-11-20T09:00:00", "endDateTime": "2026-11-20T10:00:00Z"}"#,
        r#"{"subject": "A", "startDateTime": "2026-11-20T09:00:00+01:00", "endDateTime": "2026-11-20T10:00:00Z"}"#,
        r#"{"subject": "A", "startDateTime": "soon Z", "endDateTime": "2026-11-20T10:00:00Z"}"#,
        r#"{"subject": "A", "startDateTime": "2026-13-45T09:00:00Z", "endDateTime": "2026-11-20T10:00:00Z"}"#,
        "not json",
    ] {
        let r = send_meeting(&server, "POST", "/me/onlineMeetings", body);
        assert_eq!((r.status, r.json()["error"]["code"].as_str().map(str::to_string)), (400, Some("BadRequest".into())), "{body}");
    }
    with_fake(&server, |t| assert!(t.meetings.is_empty()));
    let ok = r#"{"subject": "A", "startDateTime": "2026-11-20T09:00:00.5Z", "endDateTime": "2026-11-20T10:00:00Z"}"#;
    assert_eq!(send_meeting(&server, "POST", "/me/onlineMeetings", ok).status, 201);
}

#[test]
fn a_scripted_refusal_answers_its_status_and_code() {
    let server = serve_teams(teams_with(vec![]));
    for (status, code) in [(403, "InsufficientPrivileges"), (403, "Forbidden"), (403, "AccessDenied")] {
        with_fake(&server, |t| t.meeting_refusal = Some((status, code)));
        for _ in 0..2 {
            let r = send_meeting(&server, "POST", "/me/onlineMeetings", MEETING);
            assert_eq!((r.status, r.json()["error"]["code"].as_str().map(str::to_string)), (status, Some(code.into())));
        }
    }
    with_fake(&server, |t| assert!(t.meetings.is_empty()));
    with_fake(&server, |t| t.meeting_refusal = None);
    assert_eq!(send_meeting(&server, "POST", "/me/onlineMeetings", MEETING).status, 201);
}

#[test]
fn a_hostile_answer_is_laid_over_once() {
    let server = serve_teams(teams_with(vec![]));
    with_fake(&server, |t| t.meeting_answer = Some(json!({"joinWebUrl": "javascript:alert(1)", "id": 7})));
    let v = send_meeting(&server, "POST", "/me/onlineMeetings", MEETING).json();
    assert_eq!((v["joinWebUrl"].as_str(), v["id"].as_i64()), (Some("javascript:alert(1)"), Some(7)));
    assert_eq!(v["subject"], "Weekly");
    let v = send_meeting(&server, "POST", "/me/onlineMeetings", MEETING).json();
    assert!(v["joinWebUrl"].as_str().unwrap().starts_with("https://teams.microsoft.com/"));
    with_fake(&server, |t| assert!(t.meeting_answer.is_none()));
}

#[test]
fn a_patch_may_move_the_join_link() {
    let server = serve_teams(teams_with(vec![]));
    let v = send_meeting(&server, "POST", "/me/onlineMeetings", MEETING).json();
    let (id, join) = (v["id"].as_str().unwrap().replace('/', "%2F").replace('+', "%2B").replace('=', "%3D"), v["joinWebUrl"].clone());
    with_fake(&server, |t| t.meeting_moves = true);
    let moved = send_meeting(&server, "PATCH", &format!("/me/onlineMeetings/{id}"), r#"{"subject": "B"}"#).json();
    assert_ne!(moved["joinWebUrl"], join);
}

#[test]
fn every_meeting_request_is_kept_and_counted() {
    let server = serve_teams(teams_with(vec![]));
    send_meeting(&server, "POST", "/me/onlineMeetings", MEETING);
    assert_eq!(send_meeting(&server, "PATCH", "/me/onlineMeetings/a%2Fb?x=1", r#"{"subject": "S"}"#).status, 404);
    assert_eq!(send_meeting(&server, "GET", "/me/onlineMeetings/a/attendanceReports", "").status, 404);
    assert_eq!(send_meeting(&server, "DELETE", "/me/onlineMeetings/zz", "").status, 404);
    // Scripted refusals work by route name, as on the others.
    with_fake(&server, |t| {
        t.throttle_once = Some("createMeeting");
        t.refuse_once = Some("updateMeeting");
        t.fail_next.insert("deleteMeeting".into(), 1);
    });
    assert_eq!(send_meeting(&server, "POST", "/me/onlineMeetings", MEETING).status, 429);
    assert_eq!(send_meeting(&server, "PATCH", "/me/onlineMeetings/q", "{}").status, 401);
    assert_eq!(send_meeting(&server, "DELETE", "/me/onlineMeetings/q", "").status, 500);
    with_fake(&server, |t| {
        t.not_found_next.insert("createMeeting".into(), 1);
    });
    assert_eq!(send_meeting(&server, "POST", "/me/onlineMeetings", MEETING).status, 404);
    with_fake(&server, |t| t.cut_all = true);
    let cut = TcpStream::connect(server.base.trim_start_matches("http://").split('/').next().unwrap()).and_then(|mut s| {
        s.write_all(b"POST /me/onlineMeetings HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\n")?;
        let mut raw = Vec::new();
        s.read_to_end(&mut raw)?;
        Ok(raw)
    });
    assert!(cut.is_ok_and(|raw| raw.is_empty()), "no answer at all");
    // The server may still be counting it when the connection closes.
    for _ in 0..100 {
        if server.data.lock().unwrap().teams.as_ref().unwrap().meeting_paths.len() == 9 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let d = server.data.lock().unwrap();
    let t = d.teams.as_ref().unwrap();
    assert_eq!(t.meeting_paths[1], "/me/onlineMeetings/a%2Fb");
    assert_eq!(t.meeting_paths[2], "/me/onlineMeetings/a/attendanceReports");
    assert_eq!(t.meeting_paths.len(), 9, "{:?}", t.meeting_paths);
    let writes: Vec<(&str, &str)> = t.meeting_writes.iter().map(|(r, id, _)| (r.as_str(), id.as_str())).collect();
    assert_eq!(&writes[..4], [("createMeeting", ""), ("updateMeeting", "a/b"), ("deleteMeeting", "zz"), ("createMeeting", "")]);
    assert_eq!(t.meeting_writes[1].2["subject"], "S");
    assert_eq!(t.meeting_writes[0].2["subject"], "Weekly");
    assert_eq!(t.meeting_writes.len(), 7, "the cut request is counted in hits only");
    assert_eq!((t.hits["createMeeting"], t.hits["updateMeeting"], t.hits["deleteMeeting"], t.hits["unknown"]), (4, 2, 2, 1));
}
