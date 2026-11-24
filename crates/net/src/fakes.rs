// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Servers of the tests' own, in memory: IMAP, CalDAV/CardDAV, EWS, Graph, JMAP and Webex; and
//! the log as the tests read it (`logs`).
//!
//! The fake Graph server is also built for other crates' tests with the `fakes` feature (a
//! dev-dependency only, never in a build that ships): [`teams_sample`] starts it with a Teams
//! account.

use std::sync::{Arc, Mutex};

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
pub(crate) mod fake_dav;
#[cfg(test)]
pub(crate) mod fake_ews;
pub(crate) mod fake_graph;
#[cfg(test)]
pub(crate) mod fake_jmap;
#[cfg(test)]
pub(crate) mod fake_llm;
#[cfg(test)]
pub(crate) mod fake_webex;
#[cfg(test)]
pub(crate) mod logs;
#[cfg(test)]
mod real_layout;
#[cfg(test)]
mod real_pgp;
#[cfg(test)]
mod real_servers;

use fake_graph::{Data, FakeChannel, FakeChat, FakeMember, FakeMessage, FakeTeam, FakeTeams, FakeThread, FakeUser, rfc3339};

/// A fake Graph server holding a Teams account, for other crates' tests.
pub struct TeamsSample {
    /// The API address (`…/v1.0`).
    pub base: String,
    /// The one bearer token it takes.
    pub token: &'static str,
    /// The chat to write in (its id).
    pub chat: &'static str,
    /// Its names, addresses, topics, texts and ids: what nothing said about it may carry.
    pub private: Vec<&'static str>,
    data: Arc<Mutex<Data>>,
}

impl TeamsSample {
    /// The Teams requests it was sent other than reads (`GET`), whatever their answer.
    pub fn writes_asked(&self) -> u32 {
        let data = self.data.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        data.teams.as_ref().map_or(0, |t| t.methods.iter().filter(|(m, _)| *m != "GET").map(|(_, n)| *n).sum())
    }
}

/// Start a fake Graph server with a Teams account: Quixote Morgan, two chats (one with a
/// topic), messages with reactions in both forms and a shared file; and a team of two channels,
/// with threads, replies, mentions, a picture and a guest. It runs until the test process ends.
pub fn teams_sample() -> TeamsSample {
    const T0: i64 = 1_790_812_800_000;
    const CHAT: &str = "19:quixote-group@thread.v2";
    let me = FakeUser::new("u-quixote-oid", "Quixote Morgan");
    let ana = FakeUser::new("u-banana-oid", "Banana Lima");
    let bruno = FakeUser::new("u-pudding-oid", "Pudding Rossi");
    let mut group = FakeChat::new(CHAT, "group", Some("Secret marmalade plan"), vec![me.clone(), ana.clone(), bruno.clone()]);
    let mut one = FakeChat::new("19:quixote-one@unq.gbl.spaces", "oneOnOne", None, vec![me.clone(), ana.clone()]);
    for i in 0..4_i64 {
        let at = T0 + i * 60_000;
        let from = if i % 2 == 0 { &ana.id } else { &me.id };
        group.messages.push(FakeMessage::new(&at.to_string(), Some(from), &rfc3339(at + 60_000), &format!("<p>Marmalade secret {i}</p>")));
        one.messages.push(FakeMessage::new(&at.to_string(), Some(from), &rfc3339(at), "<p>Marmalade aside</p>"));
    }
    const SHARED: &str = "https://contoso.sharepoint.com/sites/marmalade/plan.docx";
    if let Some(m) = group.messages.get_mut(1) {
        m.reactions = vec![("like".into(), ana.id.clone()), ("\u{1f600}".into(), bruno.id.clone())];
        m.attachments = vec![("reference".into(), SHARED.into())];
    }
    let mut teams = FakeTeams { chats: vec![group, one], teams: vec![sample_team(&me, &ana, &bruno)], ..FakeTeams::new(me, "tenant-quixote") };
    teams.shares.insert(SHARED.into(), b"marmalade".to_vec());
    let mut data = Data::default();
    data.teams = Some(teams);
    let server = fake_graph::serve(data);
    TeamsSample {
        data: server.data,
        base: server.base,
        token: fake_graph::TOKEN,
        chat: CHAT,
        private: vec![
            "Quixote",
            "quixote",
            "Banana",
            "banana",
            "Pudding",
            "pudding",
            "contoso",
            "marmalade",
            "Marmalade",
            "oid",
            "19:",
            "19%3A",
            "tenant-quixote",
            "sharepoint",
            "Guild",
            "guild",
            "Jam",
            "jam",
            "Crumble",
        ],
    }
}

/// The sample's team: two channels (the second private), a thread of two replies with a picture
/// and mentions of a person and of the channel, a thread without replies, and a guest.
fn sample_team(me: &FakeUser, ana: &FakeUser, bruno: &FakeUser) -> FakeTeam {
    const T0: i64 = 1_790_812_800_000;
    const GENERAL: &str = "19:quixote-guild-jam@thread.tacv2";
    let members = || vec![FakeMember::new(me.clone()), FakeMember::new(ana.clone()), FakeMember { guest: true, ..FakeMember::new(bruno.clone()) }];
    let at = |minutes: i64| rfc3339(T0 + minutes * 60_000);
    let picture = format!(
        "<p>Jam secret <img src=\"https://graph.microsoft.com/v1.0/teams/t-quixote-guild/channels/{}/messages/{}/hostedContents/aG9zdGVk/$value\"></p>",
        "19%3Aquixote-guild-jam%40thread.tacv2", T0
    );
    let mut root = FakeMessage::new(&T0.to_string(), Some(&ana.id), &at(0), &picture);
    root.mentions = vec![me.id.clone()];
    let replies = (1..3_i64)
        .map(|i| FakeMessage::new(&(T0 + i * 60_000).to_string(), Some(if i == 1 { &me.id } else { &bruno.id }), &at(i), "<p>Jam reply</p>"))
        .collect();
    let mut thread = FakeThread::new(root, replies);
    thread.subject = Some("Crumble plans".into());
    thread.other_mentions = vec![serde_json::json!({"conversation": {"id": GENERAL, "displayName": "Jam", "conversationIdentityType": "channel"}})];
    let mut general = FakeChannel::new(GENERAL, "Jam", members());
    general.threads =
        vec![thread, FakeThread::new(FakeMessage::new(&(T0 + 600_000).to_string(), Some(&me.id), &at(10), "<p>Jam aside</p>"), Vec::new())];
    let private = FakeChannel { membership: "private", ..FakeChannel::new("19:quixote-guild-crumble@thread.tacv2", "Crumble", members()) };
    FakeTeam::new("t-quixote-guild", "Marmalade Guild", vec![general, private])
}
