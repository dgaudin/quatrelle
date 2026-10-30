//! Servers of the tests' own, in memory: IMAP, CalDAV/CardDAV, EWS, Graph and JMAP.
//!
//! The fake Graph server is also built for other crates' tests with the `fakes` feature (a
//! dev-dependency only, never in a build that ships): [`teams_sample`] starts it with a Teams
//! account.

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
mod real_servers;

use fake_graph::{Data, FakeChat, FakeMessage, FakeTeams, FakeUser, rfc3339};

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
}

/// Start a fake Graph server with a Teams account: Quixote Morgan, two chats (one with a
/// topic), messages with reactions in both forms and a shared file. It runs until the test
/// process ends.
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
    let mut teams = FakeTeams { chats: vec![group, one], ..FakeTeams::new(me, "tenant-quixote") };
    teams.shares.insert(SHARED.into(), b"marmalade".to_vec());
    let mut data = Data::default();
    data.teams = Some(teams);
    let server = fake_graph::serve(data);
    TeamsSample {
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
        ],
    }
}
