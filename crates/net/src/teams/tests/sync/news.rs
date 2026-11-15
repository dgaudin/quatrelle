// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The news the worker says of new messages: who sent them as the members name them, External
//! and App, mentions of the user, the chat's name; never an edit, a reaction, the user's own
//! message (read back by a read or by the outbox's check), a system line, the first copy or the
//! history; a chat Graph won't serve by its new last message, once; and nothing a name or a text
//! carries that could pass for something else.

use quatrelle_mime::teams_compose;
use quatrelle_model::pim::teams::{News, Op, is_bidi};
use serde_json::json;

use super::super::ME;
use super::super::worker::{AtDesk, desk, emit_into, settings};
use super::*;

/// The news said from event number `from` on.
fn news(r: &Run, from: usize) -> Vec<News> {
    r.events().into_iter().skip(from).flat_map(|e| if let model::Event::News { news } = e { news } else { Vec::new() }).collect()
}

/// A message of `from` added to a chat of the fake at `at` (its clock moved past it), as `html`,
/// `made` changing what it says of itself (who sent it, its mentions, a system line…).
fn write(r: &Run, chat: &str, from: &str, at: i64, html: &str, made: impl FnOnce(&mut FakeMessage)) {
    r.fake(|t| {
        t.clock = Some(t.clock.unwrap_or(NOW).max(at + 1000));
        let mut m = FakeMessage::new(&at.to_string(), Some(from), &rfc3339(at), html);
        made(&mut m);
        t.chat_mut(chat).unwrap().messages.push(m);
    });
}

/// Turns until `chat` has news (fifteen seconds at most).
fn until_news(r: &mut Run, from: usize, chat: &str) {
    let end = r.at + Duration::from_secs(15);
    while !news(r, from).iter().any(|n| n.chat == chat) {
        assert!(r.at < end, "no news of {chat}: {:?}", r.events());
        r.turns(STEP);
    }
}

#[test]
fn a_new_message_is_news_with_who_said_it() {
    let mut r = run(teams_with(vec![group(GROUP, Some("Agenda"), T0), chat(OTHER, T0, 2)]));
    let (from, unread) = (r.events().len(), r.row(GROUP).unread);
    // The list's round reads the group first (its message is the newer), then the other chat.
    r.post(OTHER, "u-ana", NOW + MINUTE, "Later");
    let at = NOW + 2 * MINUTE;
    write(&r, GROUP, "u-ana", at, "<p>Lunch?</p><p>at noon</p>", |_| {});
    until_news(&mut r, from, GROUP);
    let want = News {
        chat: GROUP.into(),
        message: at.to_string(),
        sender: "Ana Lima".into(),
        external: false,
        app: false,
        mentions_me: false,
        chat_name: Some("Agenda".into()),
        direct: false,
        hidden: false,
        text: "Lunch?".into(),
        at,
    };
    assert_eq!(news(&r, from), [want]);
    assert_eq!(r.row(GROUP).unread, unread + 1, "its row in the same turn");
    // The other's, a one-to-one chat: sent to the user alone, no chat's name.
    until_news(&mut r, from, OTHER);
    let other = news(&r, from).into_iter().filter(|n| n.chat == OTHER).collect::<Vec<_>>();
    assert!(matches!(other.as_slice(), [n] if n.direct && n.chat_name.is_none() && n.sender == "Ana Lima" && n.text == "Later"), "{other:?}");
}

#[test]
fn edits_reactions_own_and_system_messages_are_not_news() {
    let mut g = group(GROUP, Some("Agenda"), T0);
    g.messages.push(FakeMessage::new(&(T0 + MINUTE).to_string(), Some("u-bruno"), &rfc3339(T0 + MINUTE), "<p>Second</p>"));
    let mut r = run(teams_with(vec![g]));
    let from = r.events().len();
    r.fake(|t| {
        t.clock = Some(NOW + 10 * MINUTE);
        let c = t.chat_mut(GROUP).unwrap();
        // Two messages made long ago: one edited, one reacted to (their times move, not their making).
        c.messages[0].edited = Some(rfc3339(NOW + MINUTE));
        c.messages[0].modified = rfc3339(NOW + MINUTE);
        c.messages[0].body_html = "<p>Agenda, edited</p>".into();
        c.messages[1].reactions.push(("like".into(), "u-ana".into()));
        c.messages[1].modified = rfc3339(NOW + 2 * MINUTE);
    });
    write(&r, GROUP, ME, NOW + 3 * MINUTE, "<p>Mine</p>", |_| {});
    write(&r, GROUP, "u-ana", NOW + 4 * MINUTE, "<p>Bruno joined</p>", |m| m.system = true);
    write(&r, GROUP, "u-ana", NOW + 5 * MINUTE, "<p>Oops</p>", |m| m.deleted = Some(rfc3339(NOW + 5 * MINUTE + 1000)));
    r.w.job(Job::Open { chat: Some(GROUP.into()) });
    r.turns(Duration::from_secs(15));
    // Read, and said to the window as fresh: only not news.
    let said = r.messages(GROUP, from);
    let fresh: Vec<String> = said.iter().filter(|(_, fresh, _)| *fresh).flat_map(|(m, _, _)| m.iter().map(|m| m.id.clone())).collect();
    for at in [T0, T0 + MINUTE, NOW + 3 * MINUTE, NOW + 4 * MINUTE, NOW + 5 * MINUTE] {
        assert!(fresh.contains(&at.to_string()), "{at} read: {said:?}");
    }
    assert_eq!(news(&r, from), [], "{:?}", r.events());
    // Someone's new message after them all is.
    write(&r, GROUP, "u-ana", NOW + 6 * MINUTE, "<p>Now?</p>", |_| {});
    r.turns(Duration::from_secs(4));
    assert_eq!(news(&r, from).iter().map(|n| n.at).collect::<Vec<_>>(), [NOW + 6 * MINUTE]);
}

#[test]
fn the_first_copy_and_history_are_not_news() {
    // A recent chat, its last message read by its first read (within its overlap), and one with a
    // long history, from Ana mostly.
    let mut recent = chat(CHAT, NOW - 10 * MINUTE, 6);
    recent.messages.push(FakeMessage::new(&(NOW - 1000).to_string(), Some("u-ana"), &rfc3339(NOW - 1000), "<p>Just now</p>"));
    let mut r = run(teams_with(vec![recent, chat(OTHER, T0, 120)]));
    r.turns(Duration::from_secs(30));
    assert!(r.record(OTHER).done && r.record(CHAT).done, "the history is copied");
    assert!(r.w.copy.as_ref().unwrap().last_pages(CHAT, 1).iter().any(|m| m.id == (NOW - 1000).to_string()));
    assert_eq!(news(&r, 0), []);
    // A chat new since the first copy: its first message is news.
    let from = r.events().len();
    r.fake(|t| {
        t.clock = Some(NOW + 2 * MINUTE);
        let mut c = chat(GROUP, NOW + MINUTE, 1);
        c.kind = "group";
        c.topic = Some("New team".into());
        t.chats.push(c);
    });
    until_news(&mut r, from, GROUP);
    assert_eq!(news(&r, from).iter().map(|n| (n.at, n.chat_name.clone())).collect::<Vec<_>>(), [(NOW + MINUTE, Some("New team".into()))]);
}

#[test]
fn a_sender_is_named_as_the_members_name_them() {
    let mut g = group(GROUP, Some("Agenda"), T0);
    g.members.push(FakeUser::new("u-eve", "Eve Stone"));
    let mut r = run(teams_with(vec![g]));
    let from = r.events().len();
    // The message says another name, under Eve's id (in another case once).
    for (i, id) in ["u-eve", "U-EVE"].iter().enumerate() {
        let from_json = json!({"user": {"id": id, "displayName": "Ana Lima", "userIdentityType": "aadUser", "tenantId": TENANT}});
        write(&r, GROUP, "u-eve", NOW + (i as i64 + 1) * MINUTE, "<p>Hello</p>", |m| m.from_json = Some(from_json));
    }
    r.turns(Duration::from_secs(11));
    let said = news(&r, from);
    assert_eq!(said.len(), 2, "{said:?}");
    assert!(said.iter().all(|n| n.sender == "Eve Stone" && !n.external), "{said:?}");
}

#[test]
fn a_stranger_a_guest_and_an_app_are_said_so() {
    let mut g = group(GROUP, Some("Agenda"), T0);
    g.members.push(FakeUser::new("u-gale", "Gale Stone"));
    let mut teams = teams_with(vec![g]);
    teams.guests.push("u-gale".into());
    let mut r = run(teams);
    let from = r.events().len();
    let outside = json!({"user": {"id": "u-zed", "displayName": "Zed Black", "userIdentityType": "aadUser", "tenantId": "tenant-2"}});
    write(&r, GROUP, "u-zed", NOW + MINUTE, "<p>From outside</p>", |m| m.from_json = Some(outside));
    write(&r, GROUP, "u-gale", NOW + 2 * MINUTE, "<p>A guest</p>", |_| {});
    let bot = json!({"application": {"id": "app-1", "displayName": "Build Bot"}, "user": null});
    write(&r, GROUP, "app-1", NOW + 3 * MINUTE, "<p>Build passed</p>", |m| m.from_json = Some(bot));
    r.turns(Duration::from_secs(15));
    let said = news(&r, from);
    assert_eq!(said.len(), 3, "{said:?}");
    let by = |name: &str| said.iter().find(|n| n.sender == name).cloned().unwrap_or_else(|| panic!("no news from {name}: {said:?}"));
    let (zed, gale, bot) = (by("Zed Black"), by("Gale Stone"), by("Build Bot"));
    assert!(zed.external && !zed.app, "another organisation's, not among the members: {zed:?}");
    assert!(gale.external && !gale.app, "a guest: {gale:?}");
    assert!(bot.app && !bot.external, "an application: {bot:?}");
}

#[test]
fn a_mention_of_me_is_said() {
    let mut r = run(teams_with(vec![group(GROUP, Some("Agenda"), T0)]));
    let from = r.events().len();
    for (i, who) in [ME.to_string(), "u-bruno".to_string(), ME.to_uppercase()].into_iter().enumerate() {
        write(&r, GROUP, "u-ana", NOW + (i as i64 + 1) * MINUTE, "<p><at id=\"0\">You</at> can you look?</p>", |m| m.mentions = vec![who]);
    }
    r.turns(Duration::from_secs(11));
    let mut said: Vec<(i64, bool)> = news(&r, from).iter().map(|n| (n.at, n.mentions_me)).collect();
    said.sort();
    assert_eq!(said, [(NOW + MINUTE, true), (NOW + 2 * MINUTE, false), (NOW + 3 * MINUTE, true)]);
}

#[test]
fn names_and_text_carry_no_bidi_or_control() {
    let mut r = run(teams_with(vec![group(GROUP, Some("Agenda\u{202E}"), T0)]));
    // A member's name as a damaged or hostile copy may hold it.
    let mut kept = r.record(GROUP);
    for m in kept.chat.members.iter_mut().filter(|m| m.user_id == "u-ana") {
        m.name = "Ana\u{202E} Lima\u{7}".into();
    }
    r.w.copy.as_ref().unwrap().put_chat(&kept).unwrap();
    let from = r.events().len();
    write(&r, GROUP, "u-ana", NOW + MINUTE, "<p>Lunch\u{7}?\u{202E}</p>", |_| {});
    r.turns(Duration::from_secs(11));
    let said = news(&r, from);
    assert!(matches!(said.as_slice(), [n] if n.sender == "Ana Lima" && n.text == "Lunch?" && n.chat_name.as_deref() == Some("Agenda")), "{said:?}");
    let all = said.iter().map(|n| format!("{}{}{}", n.sender, n.text, n.chat_name.clone().unwrap_or_default())).collect::<String>();
    assert!(!all.chars().any(|c| c.is_control() || is_bidi(c)), "{all:?}");
}

#[test]
fn an_unreadable_chats_new_preview_is_news_once() {
    let mut teams = teams_with(vec![group(GROUP, Some("Ops"), T0)]);
    teams.forbid_reads.push(GROUP.into());
    let mut r = run(teams);
    assert!(r.row(GROUP).unreadable);
    assert_eq!(news(&r, 0), [], "its last message, known before");
    let from = r.events().len();
    let at = NOW + MINUTE;
    r.post(GROUP, "u-ana", at, "Are you there?");
    r.turns(Duration::from_secs(11));
    let want = News {
        chat: GROUP.into(),
        message: String::new(),
        sender: "Ana Lima".into(),
        external: false,
        app: false,
        mentions_me: false,
        chat_name: Some("Ops".into()),
        direct: false,
        hidden: false,
        text: "Are you there?".into(),
        at,
    };
    assert_eq!(news(&r, from), std::slice::from_ref(&want));
    r.turns(Duration::from_secs(31));
    assert_eq!(news(&r, from), [want], "once");
    // The user's own, from another device: none.
    r.post(GROUP, ME, NOW + 2 * MINUTE, "Yes");
    r.turns(Duration::from_secs(11));
    assert_eq!(news(&r, from).len(), 1);
    assert!(r.row(GROUP).last_line.contains("Yes"), "{:?}", r.row(GROUP));
}

/// A chat Graph won't serve, whose new last message is from someone the members don't have: the
/// members are read again before it's said, and they say who that is (a guest, though the
/// message says they're of the organisation).
#[test]
fn an_unreadable_chats_stranger_is_judged_by_the_members_read_again() {
    let mut teams = teams_with(vec![group(GROUP, Some("Ops"), T0)]);
    teams.forbid_reads.push(GROUP.into());
    let mut r = run(teams);
    r.fake(|t| {
        t.guests.push("u-gale".into());
        t.chat_mut(GROUP).unwrap().members.push(FakeUser::new("u-gale", "Gale Stone"));
    });
    let (from, members) = (r.events().len(), r.hits("members"));
    r.post(GROUP, "u-gale", NOW + MINUTE, "Hi all");
    until_news(&mut r, from, GROUP);
    assert_eq!(r.hits("members"), members + 1, "the members read before it was said");
    let said = news(&r, from);
    assert!(matches!(said.as_slice(), [n] if n.sender == "Gale Stone" && n.external && n.message.is_empty()), "{said:?}");
    r.turns(Duration::from_secs(21));
    assert_eq!(news(&r, from).len(), 1, "once");
}

/// P5: the user's own messages, read back by the outbox's check (its post's answer lost), by its
/// post's answer, or by the chat's read (sent from another device), are never news.
#[test]
fn the_users_own_message_read_back_is_never_news() {
    let mut r = run(teams_with(vec![chat(CHAT, T0, 2)]));
    let from = r.events().len();
    r.fake(|t| {
        t.answer_lost.insert("post".into(), 1);
    });
    for (local, text) in [("1-1", "On my way"), ("1-2", "Coming too")] {
        r.w.job(Job::Op { op: Op::Send { chat: CHAT.into(), local: local.into(), body: teams_compose(text, &[], &[]) } });
    }
    r.post(CHAT, ME, NOW + MINUTE, "From my phone");
    let end = r.at + Duration::from_secs(40);
    while !r.w.copy.as_ref().unwrap().ops().is_empty() {
        assert!(r.at < end, "the outbox never emptied: {:?}", r.w.copy.as_ref().unwrap().ops());
        r.turns(STEP);
    }
    r.turns(Duration::from_secs(11));
    assert_eq!(r.fake(|t| t.posts.len()), 2, "each posted once");
    let copied = r.w.copy.as_ref().unwrap().last_pages(CHAT, 1);
    for text in ["On my way", "Coming too", "From my phone"] {
        assert!(copied.iter().any(|m| m.body.plain() == text), "{text} copied");
    }
    assert_eq!(news(&r, from), [], "{:?}", r.events());
    // Someone else's, after them: news.
    write(&r, CHAT, "u-ana", NOW + 5 * MINUTE, "<p>Great</p>", |_| {});
    r.turns(Duration::from_secs(11));
    assert_eq!(news(&r, from).iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["Great"]);
}

/// Quatrelle started again on `r`'s cache and keychain, the worker before it gone: whatever the
/// engine remembered of the news went with it.
fn restart(r: Run) -> Run {
    let Run { w, server, creds, .. } = r;
    let store = w.store.clone();
    drop(w);
    let seen = Seen::default();
    let desk = AtDesk(desk(grant(Some(TENANT))));
    let mut w = Worker::new(store, creds.clone(), emit_into(&seen), server.base.clone(), Box::new(desk)).unwrap();
    w.job(Job::Start { settings: settings(ME) });
    Run { w, server, seen, t0: Instant::now(), at: Duration::ZERO, creds }
}

/// Review of Task 6: a reaction to, or an edit of, a chat's newest message (the one its watermark
/// stands on) brings it back in a read. It was news once, and isn't again: in the same run, nor
/// after a restart.
#[test]
fn a_reaction_or_an_edit_to_the_newest_message_is_not_news_again() {
    let mut r = run(teams_with(vec![group(GROUP, Some("Agenda"), T0)]));
    let from = r.events().len();
    r.w.job(Job::Open { chat: Some(GROUP.into()) });
    // Message `id` reacted to, or edited, at `when`.
    let change = |r: &Run, id: i64, when: i64, edit: bool| {
        r.fake(|t| {
            t.clock = Some(when + 1000);
            let m = t.chat_mut(GROUP).unwrap().messages.iter_mut().find(|m| m.id == id.to_string()).unwrap();
            if edit {
                m.body_html = "<p>Lunch at one?</p>".into();
                m.edited = Some(rfc3339(when));
            } else {
                m.reactions.push(("like".into(), "u-bruno".into()));
            }
            m.modified = rfc3339(when);
        })
    };
    // Message `id` as the reads said it, fresh, from event number `from` on.
    let read = |r: &Run, id: i64, from: usize| -> Vec<ChatMessage> {
        r.messages(GROUP, from).into_iter().filter(|(_, fresh, _)| *fresh).flat_map(|(m, _, _)| m).filter(|m| m.id == id.to_string()).collect()
    };
    let (first, second) = (NOW + MINUTE, NOW + 3 * MINUTE);
    write(&r, GROUP, "u-ana", first, "<p>Lunch?</p>", |_| {});
    until_news(&mut r, from, GROUP);
    let said = r.events().len();
    change(&r, first, NOW + 2 * MINUTE, false);
    r.turns(Duration::from_secs(5));
    assert!(read(&r, first, said).iter().any(|m| !m.reactions.is_empty()), "the reaction read: {:?}", r.events());
    assert_eq!(news(&r, from).len(), 1, "not news again: {:?}", news(&r, from));

    // Another newest message, said; then Quatrelle started again, and it is edited.
    write(&r, GROUP, "u-ana", second, "<p>Lunch at noon?</p>", |_| {});
    r.turns(Duration::from_secs(5));
    assert_eq!(news(&r, from).iter().map(|n| n.at).collect::<Vec<_>>(), [first, second]);
    let mut r = restart(r);
    r.turns(Duration::from_secs(5));
    r.w.job(Job::Open { chat: Some(GROUP.into()) });
    change(&r, second, NOW + 4 * MINUTE, true);
    r.turns(Duration::from_secs(11));
    assert!(read(&r, second, 0).iter().any(|m| m.edited.is_some() && m.body.plain() == "Lunch at one?"), "the edit read: {:?}", r.events());
    assert_eq!(news(&r, 0), [], "nor after a restart");
}
