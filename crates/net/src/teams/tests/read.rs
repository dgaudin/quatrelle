// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Graph's JSON read into the model (hostile input: its caps, its refusals, its guard), and the
//! client's calls for hosted images, photos, presence and the user's drive, and its no-wait mode.

use std::time::{Duration, Instant};

use quatrelle_model::pim::teams::{Attachment, MAX_BODY, MAX_ID, MessageKind, NAME_CHARS, Presence, Sender};
use serde_json::{Value, json};

use super::*;
use crate::teams::graph::{MAX_ROLES, PANIC_ON_BODY, chat_of, member_of, message_of, presence_of};

/// A chat as the list of chats gives it, with a preview whose body is `preview`.
fn chat_json(id: &str, preview: &str) -> Value {
    json!({
        "id": id, "topic": null, "chatType": "oneOnOne", "tenantId": TENANT,
        "createdDateTime": "2026-09-01T08:00:00Z", "lastUpdatedDateTime": "2026-10-01T09:00:00.250Z",
        "webUrl": "https://teams.microsoft.com/l/chat/19%3Aa1b2%40thread.v2/0",
        "viewpoint": {"isHidden": false, "lastMessageReadDateTime": "2026-10-01T09:30:00Z"},
        "lastMessagePreview": {
            "id": "1790845200000", "createdDateTime": "2026-10-01T09:00:00Z", "isDeleted": false, "messageType": "message",
            "eventDetail": null, "body": {"contentType": "html", "content": preview},
            "from": {"user": {"id": "u-paul", "displayName": "Paul Martin", "userIdentityType": "aadUser", "tenantId": TENANT}},
        },
    })
}

/// A message from Ana, created at T0 and changed a minute later.
fn message_json(id: &str) -> Value {
    json!({
        "id": id, "messageType": "message", "eventDetail": null,
        "createdDateTime": rfc3339(T0), "lastModifiedDateTime": rfc3339(T0 + MINUTE), "lastEditedDateTime": null, "deletedDateTime": null,
        "from": {"application": null, "device": null, "user": {"id": "u-ana", "displayName": "Ana Lima", "userIdentityType": "aadUser", "tenantId": TENANT}},
        "body": {"contentType": "html", "content": "<p>Hello <b>there</b></p>"},
        "attachments": [], "mentions": [], "reactions": [],
    })
}

fn ana() -> Sender {
    Sender::User { id: "u-ana".into(), name: "Ana Lima".into(), identity_type: Some("aadUser".into()), tenant_id: Some(TENANT.into()) }
}

#[test]
fn a_preview_is_read_like_a_message_and_kept_on_one_line() {
    let chat = chat_of(&chat_json(CHAT, "<p>Paul:\n<b>it's</b> fixed\u{202E}</p>")).unwrap();
    let preview = chat.preview.unwrap();
    assert_eq!(preview.text, "Paul: it's fixed");
    assert_eq!(preview.at, 1_790_845_200_000);
    assert!(matches!(&preview.sender, Sender::User { id, .. } if id == "u-paul"));
    assert_eq!((chat.last_activity, chat.updated, chat.last_read), (Some(1_790_845_200_000), Some(1_790_845_200_250), Some(1_790_847_000_000)));
    assert_eq!(chat.tenant_id.as_deref(), Some(TENANT));

    // A long preview is cut on its line; a deleted one keeps no text; a plain-text one is read as text.
    let long = chat_of(&chat_json(CHAT, &format!("<p>{}</p>", "word ".repeat(10_000)))).unwrap();
    assert_eq!(long.preview.unwrap().text.chars().count(), NAME_CHARS);
    let mut gone = chat_json(CHAT, "<p>secret</p>");
    gone["lastMessagePreview"]["isDeleted"] = json!(true);
    assert_eq!(chat_of(&gone).unwrap().preview.unwrap().text, "");
    let mut plain = chat_json(CHAT, "a <b>not bold</b>\nline");
    plain["lastMessagePreview"]["body"]["contentType"] = json!("text");
    assert_eq!(chat_of(&plain).unwrap().preview.unwrap().text, "a <b>not bold</b> line");
}

#[test]
fn never_read_is_none() {
    let mut v = chat_json(CHAT, "<p>hi</p>");
    v["viewpoint"]["lastMessageReadDateTime"] = json!("0001-01-01T00:00:00Z");
    assert_eq!(chat_of(&v).unwrap().last_read, None);
    v["viewpoint"] = json!({"isHidden": true});
    let chat = chat_of(&v).unwrap();
    assert_eq!((chat.last_read, chat.hidden), (None, true));
}

#[test]
fn an_event_detail_makes_a_system_message() {
    let mut v = message_json("1");
    v["eventDetail"] = json!({"@odata.type": "#microsoft.graph.membersAddedEventMessageDetail", "members": []});
    v["from"] = Value::Null;
    let m = message_of(CHAT, &v);
    assert_eq!((m.kind, &m.sender, m.unshowable), (MessageKind::System, &Sender::Nobody, false));
    assert_eq!(message_of(CHAT, &message_json("2")).kind, MessageKind::Message);
}

#[test]
fn a_message_that_panics_is_unshowable_and_the_read_goes_on() {
    let page = [message_json("1"), message_json("2")];
    let read: Vec<_> = page
        .iter()
        .enumerate()
        .map(|(i, v)| {
            PANIC_ON_BODY.with(|p| p.set(i == 0));
            message_of(CHAT, v)
        })
        .collect();
    PANIC_ON_BODY.with(|p| p.set(false));
    let (failed, next) = (&read[0], &read[1]);
    assert!(failed.unshowable);
    assert_eq!((failed.id.as_str(), failed.chat.as_str(), failed.created, failed.modified), ("1", CHAT, T0, T0 + MINUTE));
    assert_eq!(failed.sender, ana());
    assert!(failed.body.blocks.is_empty());
    assert!(!next.unshowable);
    assert_eq!(next.body.plain(), "Hello there");

    // A chat whose preview panics is kept, without its preview.
    PANIC_ON_BODY.with(|p| p.set(true));
    let chat = chat_of(&chat_json(CHAT, "<p>hi</p>"));
    PANIC_ON_BODY.with(|p| p.set(false));
    let chat = chat.unwrap();
    assert_eq!((chat.id.as_str(), chat.preview.is_none()), (CHAT, true));
}

#[test]
fn caps_hold() {
    let mut v = message_json("1");
    v["attachments"] =
        (0..30).map(|i| json!({"id": format!("a{i}"), "contentType": "reference", "contentUrl": "https://x.example/f", "name": "f"})).collect();
    v["mentions"] = (0..60).map(|i| json!({"id": i, "mentionText": "Ana", "mentioned": {"user": {"id": "u-ana"}}})).collect();
    v["reactions"] =
        (0..150).flat_map(|e| (0..300).map(move |u| json!({"reactionType": format!("r{e}"), "user": {"user": {"id": format!("u{u}")}}}))).collect();
    let m = message_of(CHAT, &v);
    assert_eq!((m.attachments.len(), m.mentions.len(), m.reactions.len()), (20, 50, 100));
    assert!(m.reactions.iter().all(|r| r.users.len() == 200));
    // The same person reacting twice the same way counts once; a reaction without a person isn't kept.
    v["reactions"] = json!([
        {"reactionType": "like", "user": {"user": {"id": "u-ana"}}},
        {"reactionType": "like", "user": {"user": {"id": "u-ana"}}},
        {"reactionType": "heart", "user": {"user": null}},
    ]);
    let m = message_of(CHAT, &v);
    assert_eq!(m.reactions.len(), 1);
    assert_eq!(m.reactions[0].users, ["u-ana"]);
    // A body past the cap is cut to it.
    v["body"] = json!({"contentType": "text", "content": "x".repeat(MAX_BODY + 10)});
    assert_eq!(message_of(CHAT, &v).body.plain().len(), MAX_BODY);
}

#[test]
fn a_reference_attachment_keeps_no_thumbnail() {
    let url = "https://contoso-my.sharepoint.com/personal/ana/Documents/Microsoft%20Teams%20Chat%20Files/plan.docx";
    let mut v = message_json("1");
    v["attachments"] = json!([
        {"id": "f1", "contentType": "reference", "contentUrl": url, "name": "plan\u{202E}xcod.docx",
         "thumbnailUrl": "https://thumbs.example/track.png", "content": null},
        {"id": "c1", "contentType": "application/vnd.microsoft.card.adaptive", "content": "{\"body\":[{\"url\":\"https://cards.example/\"}]}"},
    ]);
    let m = message_of(CHAT, &v);
    assert_eq!(m.attachments.len(), 2);
    assert!(matches!(&m.attachments[0], Attachment::File { id, name, url: u, size: None } if id == "f1" && name == "planxcod.docx" && u == url));
    assert_eq!(m.attachments[1], Attachment::Card);
    let kept = serde_json::to_string(&m).unwrap();
    assert!(!kept.contains("thumbs.example") && !kept.contains("cards.example"), "{kept}");
}

#[test]
fn ids_too_long_are_refused() {
    let long = format!("19:{}@thread.v2", "a".repeat(600));
    assert!(chat_of(&chat_json(&long, "<p>hi</p>")).is_none());
    assert!(chat_of(&json!({"chatType": "group"})).is_none(), "no id");
    let m = message_of(CHAT, &message_json(&"1".repeat(600)));
    assert!(m.unshowable);
    assert_eq!((m.id.as_str(), m.body.blocks.len()), ("", 0));
    // Ids of a sender, a member, a mention or an attachment past the cap are dropped.
    let mut v = message_json("1");
    v["from"]["user"]["id"] = json!("u".repeat(600));
    v["attachments"] = json!([{"id": "a".repeat(600), "contentType": "reference", "contentUrl": "https://x.example/f"}]);
    v["mentions"] = json!([{"id": 0, "mentionText": "Ana", "mentioned": {"user": {"id": "u".repeat(600)}}}]);
    let m = message_of(CHAT, &v);
    assert_eq!((&m.sender, m.attachments.len(), m.mentions[0].user_id.as_deref()), (&Sender::Nobody, 0, None));
    assert!(member_of(&json!({"userId": "u".repeat(600), "displayName": "X"})).is_none());
    assert!(presence_of(&json!({"id": "", "availability": "Busy"})).is_none());
}

#[test]
fn a_deleted_message_keeps_only_its_id_times_and_sender() {
    let mut v = message_json("1");
    v["deletedDateTime"] = json!(rfc3339(T0 + 2 * MINUTE));
    v["mentions"] = json!([{"id": 0, "mentionText": "Ana", "mentioned": {"user": {"id": "u-ana"}}}]);
    let m = message_of(CHAT, &v);
    assert_eq!((m.id.as_str(), m.deleted, &m.sender), ("1", Some(T0 + 2 * MINUTE), &ana()));
    assert!(m.body.blocks.is_empty() && m.mentions.is_empty() && m.attachments.is_empty() && m.reactions.is_empty());
}

#[test]
fn members_say_their_kind_tenant_and_roles() {
    let m = member_of(&json!({
        "@odata.type": "#microsoft.graph.aadUserConversationMember", "userId": "u-ana", "displayName": "Ana\n\u{202E}Lima",
        "email": "ana@contoso.example", "tenantId": TENANT, "roles": ["owner"],
    }))
    .unwrap();
    assert_eq!(
        (m.kind.as_str(), m.name.as_str(), m.address.as_str(), m.tenant_id.as_deref()),
        ("aadUserConversationMember", "Ana Lima", "ana@contoso.example", Some(TENANT))
    );
    // A guest past the cap on roles stays a guest; an anonymous guest has its own id; an odd type is none.
    let roles: Vec<String> = (0..20).map(|i| format!("r{i}")).chain(["guest".to_string()]).collect();
    let g = member_of(&json!({"@odata.type": "#microsoft.graph.aadUserConversationMember", "userId": "u-g", "roles": roles})).unwrap();
    assert_eq!(g.roles.len(), MAX_ROLES);
    assert_eq!(g.roles.last().map(String::as_str), Some("guest"));
    let anon = member_of(&json!({"@odata.type": "#microsoft.graph.anonymousGuestConversationMember", "anonymousGuestId": "anon-1"})).unwrap();
    assert_eq!((anon.user_id.as_str(), anon.kind.as_str(), anon.tenant_id), ("anon-1", "anonymousGuestConversationMember", None));
    let odd = member_of(&json!({"@odata.type": "#microsoft.graph.aad User", "userId": "u-x"})).unwrap();
    assert_eq!(odd.kind, "");
}

#[test]
fn hosted_content_address_is_built_from_the_message() {
    let mut c = chat(CHAT, T0, 1);
    c.messages[0].hosted.push(("aWQ9eA==".into(), b"PNG bytes".to_vec()));
    let msg = c.messages[0].id.clone();
    let server = serve_teams(teams_with(vec![c]));
    let bytes = client(&server).hosted(CHAT, &msg, "aWQ9eA==", 1 << 20).unwrap();
    assert_eq!((bytes.as_slice(), hits(&server, "hosted")), (b"PNG bytes".as_slice(), 1));
    assert!(headers(&server, "hosted").contains(&("consistencylevel".into(), "eventual".into())));
    assert!(matches!(client(&server).hosted(CHAT, &msg, "other", 1 << 20), Err(NetError::Protocol(m)) if m.contains("404")));
    // An id that would climb the path is refused before any request.
    let c = client(&server);
    for (chat, message, id) in [(CHAT, msg.as_str(), ".."), (CHAT, ".", "x"), ("", msg.as_str(), "x")] {
        assert!(matches!(c.hosted(chat, message, id, 1 << 20), Err(NetError::Refused(_))));
    }
    assert_eq!(hits(&server, "hosted"), 2);

    // The exact request line: each part encoded as one segment.
    let (base, h) = crate::http::tests::serve(vec![answer(200, ME_JSON), answer(200, "x")]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    c.hosted("19:a/b@thread.v2", "17/9", "h=", 16).unwrap();
    let seen = h.join().unwrap();
    assert!(seen[1].starts_with("GET /chats/19%3Aa%2Fb%40thread.v2/messages/17%2F9/hostedContents/h%3D/$value HTTP/1.1\r\n"), "{}", seen[1]);
}

#[test]
fn a_429_is_returned_at_once_when_not_waiting() {
    let server = serve_teams(teams_with(vec![chat(CHAT, T0, 1)]));
    let c = client(&server);
    server.data.lock().unwrap().teams.as_mut().unwrap().throttle_every = Some(1);
    c.set_waiting(false);
    let start = Instant::now();
    assert!(matches!(c.chats(None), Err(NetError::Closed(m)) if m.starts_with("HTTP 429")));
    assert!(start.elapsed() < Duration::from_millis(200), "{:?}", start.elapsed());
    assert_eq!((c.retry_after(), hits(&server, "chats"), c.stats().throttled), (Some(1), 1, 1));
    // The same for a picture.
    assert!(matches!(c.photo("u-ana", 1 << 20), Err(NetError::Closed(_))));
    assert!(start.elapsed() < Duration::from_millis(400));

    // Retry-After is taken in seconds, up to an hour; a date isn't.
    let busy = |after: &str| format!("HTTP/1.1 429 X\r\nRetry-After: {after}\r\nContent-Length: 0\r\n\r\n").into_bytes();
    let (base, h) = crate::http::tests::serve(vec![answer(200, ME_JSON), busy("86400"), busy("Wed, 21 Oct 2026 07:28:00 GMT")]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    c.set_waiting(false);
    assert!(c.chats(None).is_err());
    assert_eq!(c.retry_after(), Some(3600));
    assert!(c.chats(None).is_err());
    assert_eq!(c.retry_after(), None);
    h.join().unwrap();
}

#[test]
fn a_photo_404_is_none() {
    let mut t = teams_with(Vec::new());
    t.photos.insert("u-ana".into(), b"JPEG".to_vec());
    let server = serve_teams(t);
    let c = client(&server);
    assert_eq!(c.photo("u-ana", 1 << 20).unwrap().as_deref(), Some(b"JPEG".as_slice()));
    assert_eq!(c.photo("u-nobody", 1 << 20).unwrap(), None);
    assert!(matches!(c.photo("..", 1 << 20), Err(NetError::Refused(_))));
    assert_eq!(hits(&server, "photo"), 2);
}

#[test]
fn presence_maps_graph_names() {
    let names = ["Available", "Busy", "Away", "BeRightBack", "DoNotDisturb", "Offline", "PresenceUnknown"];
    let mut t = teams_with(Vec::new());
    for (i, n) in names.iter().enumerate() {
        t.presences.insert(format!("u{i}"), n.to_string());
    }
    let server = serve_teams(t);
    let ids: Vec<String> = (0..700).map(|i| format!("u{i}")).collect();
    let got = client(&server).presences(&ids).unwrap();
    assert_eq!(got.len(), 650, "at most 650 ids are asked at once");
    let read: Vec<(String, Presence)> = got.iter().filter_map(presence_of).take(names.len()).collect();
    let want = [Presence::Available, Presence::Busy, Presence::Away, Presence::Away, Presence::DoNotDisturb, Presence::Offline, Presence::Unknown];
    assert_eq!(read, (0..names.len()).map(|i| format!("u{i}")).zip(want).collect::<Vec<_>>());
    assert!(client(&server).presences(&[]).unwrap().is_empty());
    assert_eq!(hits(&server, "presence"), 1);
}

#[test]
fn drive_host_is_the_host_only() {
    let mut t = teams_with(Vec::new());
    t.drive = Some("https://Contoso-my.sharepoint.com/personal/morgan_contoso_example/Documents".into());
    let server = serve_teams(t);
    assert_eq!(client(&server).drive_host().unwrap().as_deref(), Some("contoso-my.sharepoint.com"));
    server.data.lock().unwrap().teams.as_mut().unwrap().drive = Some("http://contoso-my.sharepoint.com/x".into());
    assert_eq!(client(&server).drive_host().unwrap(), None, "https only");
    server.data.lock().unwrap().teams.as_mut().unwrap().drive = None;
    assert_eq!(client(&server).drive_host().unwrap(), None, "no drive");
}

#[test]
fn members_all_stops_at_a_page_already_followed_and_members_refused_is_403() {
    let server = serve_teams(FakeTeams { forbid_members: vec![CHAT.into()], ..teams_with(vec![chat(CHAT, T0, 1)]) });
    assert!(matches!(client(&server).members_all(CHAT), Err(NetError::Protocol(m)) if m.starts_with("HTTP 403")));

    // Pages that point back at the second one forever: its link isn't followed twice.
    let (l, base) = crate::http::tests::bind();
    let looping = answer(200, &format!(r#"{{"value": [{{"userId": "u-1"}}], "@odata.nextLink": "{base}/chats/x/members?$skiptoken=1"}}"#));
    // Answers for me, the first page and the next one: a fourth request would find no server.
    let h = crate::http::tests::serve_on(l, vec![answer(200, ME_JSON), looping.clone(), looping]);
    let (c, _) = TeamsClient::connect(&base, "t").unwrap();
    assert_eq!(c.members_all("x").unwrap().len(), 2);
    assert_eq!(h.join().unwrap().len(), 3);
}

/// Every field of a chat, a member, a message and a presence, at every depth, replaced by values
/// of every shape: the readers, without their guards, never panic.
#[test]
fn every_field_of_every_shape_never_panics() {
    use crate::teams::graph::{chat_of_unguarded, message_of_unguarded};
    let shapes = [
        Value::Null,
        json!(true),
        json!(-1),
        json!(u64::MAX),
        json!(1e300),
        json!(""),
        json!("x".repeat(MAX_ID + 1)),
        json!("0001-01-01T00:00:00Z"),
        json!("9999-12-31T23:59:60.999999999+23:59"),
        json!("\u{202E}\n\u{0}"),
        json!([]),
        json!({}),
        json!([{}, null, 1]),
    ];
    let mut message = message_json("1");
    message["mentions"] = json!([{"id": 0, "mentionText": "Ana", "mentioned": {"user": {"id": "u-ana"}}}]);
    message["reactions"] = json!([{"reactionType": "like", "user": {"user": {"id": "u-ana"}}}]);
    message["attachments"] = json!([{"id": "f1", "contentType": "reference", "contentUrl": "https://x.example/f", "name": "f"}]);
    let mut chat = chat_json(CHAT, "<p>hi</p>");
    chat["members"] = json!([{"@odata.type": "#microsoft.graph.aadUserConversationMember", "userId": "u-ana", "roles": ["owner"]}]);
    let presence = json!({"id": "u-ana", "availability": "Busy"});
    let mut tried = 0;
    for base in [message, chat, presence] {
        let mut paths = Vec::new();
        pointers(&base, String::new(), &mut paths);
        for path in paths {
            for shape in &shapes {
                let mut v = base.clone();
                if let Some(at) = v.pointer_mut(&path) {
                    *at = shape.clone();
                }
                let _ = (chat_of_unguarded(&v), member_of(&v), message_of_unguarded(CHAT, &v), presence_of(&v));
                for item in ["members", "mentions", "reactions", "attachments"].iter().filter_map(|k| v.get(*k)).filter_map(Value::as_array).flatten()
                {
                    let _ = member_of(item);
                }
                tried += 1;
            }
        }
    }
    assert!(tried > 500, "{tried}");
}

/// The JSON pointer of every value inside `v`, `v` itself included.
fn pointers(v: &Value, at: String, out: &mut Vec<String>) {
    match v {
        Value::Object(o) => o.iter().for_each(|(k, x)| pointers(x, format!("{at}/{}", k.replace('~', "~0").replace('/', "~1")), out)),
        Value::Array(a) => a.iter().enumerate().for_each(|(i, x)| pointers(x, format!("{at}/{i}"), out)),
        _ => {}
    }
    out.push(at);
}
