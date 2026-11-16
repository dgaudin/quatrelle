// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Tests: what the log says of the chats' links, so it tells why a chat's "open in Teams" isn't
//! offered: a chat opened, its kind, whether it's unreadable and its link as the window judges
//! it; a listing over, the figures of each kind and the hosts, and how many conversations in how
//! many pages, each said again only when it changes. Never an id, a name or a message's text.
//! And the link built for a chat Graph gives none (decision 21): Teams' own form, only from a Teams
//! thread's id and the organisation's GUID.

use quatrelle_model::ical::is_teams_chat_link;
use quatrelle_model::pim::teams::MAX_ID;

use super::*;
use crate::teams::sync::built_chat_link;

/// A host no other test's links are on: the lines it's in are this test's.
const HOST: &str = "links-7731.example";

/// Three chats whose ids and topics no other test has: a group chat with Teams' own link, a
/// one-on-one chat whose link is on another host, a meeting chat without one (refused, too).
#[test]
fn the_log_tells_the_chats_links_and_never_an_id() {
    const GROUP_CHAT: &str = "19:ocelot1@thread.v2";
    const ELSEWHERE: &str = "19:ocelot2@thread.v2";
    const MEETING: &str = "19:ocelot3@thread.v2";
    let a = group(GROUP_CHAT, Some("Ocelot plans"), T0);
    let mut b = chat(ELSEWHERE, T0 + MINUTE, 2);
    b.link = Some(Some(format!("https://{HOST}/l/chat/{ELSEWHERE}/0?tenantId={TENANT}")));
    let mut c = FakeChat::new(MEETING, "meeting", Some("Ocelot review"), vec![me(), ana()]);
    c.messages.push(FakeMessage::new(&(T0 + 2 * MINUTE).to_string(), Some("u-ana"), &rfc3339(T0 + 2 * MINUTE), "<p>Ocelot minutes</p>"));
    c.link = Some(None);
    let mut teams = teams_with(vec![a, b, c]);
    teams.forbid_reads.push(MEETING.into());
    let tap = crate::fakes::logs::tap(|l| l.starts_with("teams: "));
    let mut r = run(teams);
    r.turns(Duration::from_secs(25));
    for chat in [ELSEWHERE, MEETING, GROUP_CHAT] {
        r.w.job(Job::Open { chat: Some(chat.into()) });
        r.turns(Duration::from_secs(1));
    }
    let lines = tap.lines();
    let ours: Vec<&String> = lines.iter().filter(|l| l.contains(HOST)).collect();
    // The listing's figures: said once, the list being read again unchanged.
    let figures: Vec<&&String> = ours.iter().filter(|l| l.contains("links of the conversations listed")).collect();
    assert_eq!(figures.len(), 1, "said once while nothing changes: {lines:?}");
    let f = figures[0];
    for part in ["group 1 (1 with a link, 1 accepted)", "meeting 1 (0 with a link, 0 accepted)", "oneOnOne 1 (1 with a link, 0 accepted)"] {
        assert!(f.contains(part), "{part:?} in {f}");
    }
    assert!(f.contains(&format!("{HOST} 1")) && f.contains("teams.microsoft.com 1"), "{f}");
    // The chats opened.
    let opened = |what: &str| lines.iter().any(|l| *l == format!("teams: a conversation opened: {what}"));
    assert!(opened(&format!("oneOnOne; its link: refused (host {HOST}, path /l)")), "{lines:?}");
    assert!(opened("meeting, unreadable; its link: absent"), "{lines:?}");
    assert!(opened("group; its link: ok"), "{lines:?}");
    // A change in the list: said again.
    r.fake(|t| t.chat_mut(MEETING).unwrap().link = None);
    r.turns(Duration::from_secs(11));
    let again = tap.lines().into_iter().filter(|l| l.contains(HOST) && l.contains("links of the conversations listed")).count();
    assert_eq!(again, 2, "{:?}", tap.lines());
    // These lines, whichever test's: none carries an id, a name or a text.
    let these = |l: &String| l.contains("links of the conversations listed") || l.starts_with("teams: a conversation opened");
    for line in tap.lines().iter().filter(|l| these(l)) {
        let lower = line.to_lowercase();
        for private in ["ocelot", "@thread.v2", "19:", TENANT, "ana lima", "u-ana"] {
            assert!(!lower.contains(private), "{private:?} in the log: {line}");
        }
    }
}

/// A listing's count (conversations, pages): said once while the list, read every ten seconds,
/// doesn't change; said again when it does.
#[test]
fn the_listings_count_is_said_when_it_changes() {
    // Eleven chats, then twelve: counts no other test lists.
    let chats: Vec<FakeChat> = (0..11).map(|i| chat(&format!("19:lynx{i}@thread.v2"), T0 + i * MINUTE, 1)).collect();
    let tap = crate::fakes::logs::tap(|l| l.starts_with("teams: ") && l.contains(" conversations listed in "));
    let mut r = run(teams_with(chats));
    let pages = r.hits("chats");
    r.turns(Duration::from_secs(35));
    assert!(r.hits("chats") >= pages + 2, "listed again: {} pages", r.hits("chats") - pages);
    let said = |n: usize| tap.lines().iter().filter(|l| l.starts_with(&format!("teams: {n} conversations listed in "))).count();
    assert_eq!(said(11), 1, "said once while nothing changes: {:?}", tap.lines());
    r.fake(|t| t.chats.push(chat("19:lynx11@thread.v2", T0 + 11 * MINUTE, 1)));
    r.turns(Duration::from_secs(11));
    assert_eq!(said(12), 1, "said again once it changed: {:?}", tap.lines());
}

/// An organisation's id no real one has.
const GUID: &str = "00000000-0000-0000-0000-000000000000";

/// The link built for a chat Graph gives none: Teams' own form, the id encoded as Graph's
/// addresses encode a segment, the organisation's GUID after it; one the window opens. Only from
/// an id one segment can carry that is a Teams thread's, and a GUID: nothing else is built upon.
#[test]
fn a_chats_link_is_built_only_from_a_teams_thread_and_a_guid() {
    let built = built_chat_link("19:quokka1@thread.v2", GUID);
    assert_eq!(built.as_deref(), Some("https://teams.microsoft.com/l/chat/19%3Aquokka1%40thread.v2/0?tenantId=00000000-0000-0000-0000-000000000000"));
    for id in
        ["19:quokka1@thread.v2", "19:meeting_Mjdh@thread.v2", "19:a_b@unq.gbl.spaces", "19:a@thread.skype", "19:a@thread.tacv2", "19:a@THREAD.V2"]
    {
        assert!(built_chat_link(id, GUID).is_some_and(|u| is_teams_chat_link(&u)), "{id}");
    }
    let too_long = format!("19:{}@thread.v2", "a".repeat(MAX_ID));
    for id in [
        "../x",
        "..",
        "19:a@evil.example",
        "19:a?b@thread.v2",
        "19:a#b@thread.v2",
        "19:a/b@thread.v2",
        "19:a\\b@thread.v2",
        "19:a@thread.v2?x",
        "19:a@thread.v2#x",
        "19:a@thread.v2/x",
        "19:@thread.v2",
        "19:a@thread.v2\u{0}",
        "x@thread.v2",
        "20:a@thread.v2",
        "",
        too_long.as_str(),
    ] {
        assert_eq!(built_chat_link(id, GUID), None, "{id:?}");
    }
    for tenant in [
        "",
        "tenant-1",
        "contoso.example",
        "organizations",
        "00000000-0000-0000-0000-00000000000",
        "00000000-0000-0000-0000-0000000000000",
        "0000000g-0000-0000-0000-000000000000",
        "00000000_0000-0000-0000-000000000000",
        "00000000-0000-0000-0000-000000000000&x=1",
    ] {
        assert_eq!(built_chat_link("19:quokka1@thread.v2", tenant), None, "{tenant:?}");
    }
}

/// A chat Graph won't serve and gives no link: opened, its link goes to the engine built, and the
/// log says so; one Graph gives a link keeps Graph's. Signed in to an organisation named by its
/// GUID (the other tests' isn't one: they get no link built).
#[test]
fn a_chat_graph_gives_no_link_gets_one_built() {
    const UNREADABLE: &str = "19:quokka1@thread.v2";
    const LINKED: &str = "19:quokka2@thread.v2";
    let mut unknown = FakeChat::new(UNREADABLE, "unknownFutureValue", None, vec![me(), ana()]);
    unknown.link = Some(None);
    let linked = chat(LINKED, T0, 2);
    let mut teams = FakeTeams { chats: vec![unknown, linked], ..FakeTeams::new(me(), GUID) };
    teams.forbid_reads.push(UNREADABLE.into());
    let tap = crate::fakes::logs::tap(|l| l.starts_with("teams: a conversation opened: "));
    let mut r = run_as(teams, GUID, |_| {});
    r.turns(Duration::from_secs(5));
    let link = |r: &mut Run, chat: &str| {
        let from = r.events().len();
        r.w.job(Job::Open { chat: Some(chat.into()) });
        r.turns(Duration::from_secs(1));
        let said: Vec<Option<String>> = r
            .events()
            .into_iter()
            .skip(from)
            .filter_map(|e| match e {
                model::Event::Members { chat: c, web_url, first: true, .. } if c == chat => Some(web_url),
                _ => None,
            })
            .collect();
        assert_eq!(said.len(), 1, "{chat}: {said:?}");
        said.into_iter().next().flatten()
    };
    let built = format!("https://teams.microsoft.com/l/chat/19%3Aquokka1%40thread.v2/0?tenantId={GUID}");
    assert_eq!(link(&mut r, UNREADABLE).as_deref(), Some(built.as_str()));
    let graphs = format!("https://teams.microsoft.com/l/chat/{LINKED}/0?tenantId={GUID}");
    assert_eq!(link(&mut r, LINKED).as_deref(), Some(graphs.as_str()), "Graph's own link is kept");
    let lines = tap.lines();
    assert!(lines.iter().any(|l| l == "teams: a conversation opened: unknown, unreadable; its link: built"), "{lines:?}");
}
