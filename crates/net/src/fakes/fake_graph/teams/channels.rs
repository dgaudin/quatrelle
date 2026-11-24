// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fake Graph server's Teams routes about teams and channels, read only, as Graph v1.0
//! answers them: the teams the user joined (`/me/joinedTeams`), a team's channels (a `shared`
//! one said as such only to a request that prefers unknown enum members, as Graph does), a
//! channel's root messages (by the thread's last change, the newest first; `$top` from 1 to 50,
//! `$expand=replies` with the replies cut past [`ChannelRules::expand_cap`]), a thread's replies,
//! a channel's members, and the old `…/messages/delta` (its status the test's). Nothing here
//! writes: a write to a channel is a Teams path no route answers (`unknown`, 404), counted with
//! the rest in `hits` and, by its method, in `methods`.

use super::chats::{Paging, page, top};
use super::*;

/// A team the signed-in user is a member of.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeTeam {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) archived: bool,
    /// Its organisation (`tenantId`), when not the signed-in user's.
    pub(crate) tenant: Option<String>,
    pub(crate) channels: Vec<FakeChannel>,
    /// Its channel list is refused with 403.
    pub(crate) forbid_channels: bool,
}

impl FakeTeam {
    pub(crate) fn new(id: &str, name: &str, channels: Vec<FakeChannel>) -> Self {
        Self { id: id.into(), name: name.into(), channels, ..Self::default() }
    }
}

/// A channel of a team.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeChannel {
    pub(crate) id: String,
    pub(crate) name: String,
    /// `membershipType`: `standard`, `private`, `shared`, or another of Graph's.
    pub(crate) membership: &'static str,
    /// `layoutType`: `post` (threads), `chat`; `None`: not said.
    pub(crate) layout: Option<&'static str>,
    /// Its organisation (`tenantId`), when not its team's.
    pub(crate) tenant: Option<String>,
    /// Its `webUrl` as the list gives it: `None`, Teams' own link to it; `Some(None)`: none.
    pub(crate) link: Option<Option<String>>,
    pub(crate) threads: Vec<FakeThread>,
    pub(crate) members: Vec<FakeMember>,
    /// Its messages (and their replies, and the old delta) are refused with 403.
    pub(crate) forbid_messages: bool,
    /// Its members are refused with 403.
    pub(crate) forbid_members: bool,
}

impl FakeChannel {
    /// A standard channel, in threads, with these members.
    pub(crate) fn new(id: &str, name: &str, members: Vec<FakeMember>) -> Self {
        Self { id: id.into(), name: name.into(), membership: "standard", layout: Some("post"), members, ..Self::default() }
    }
}

/// A thread of a channel: its root message and the replies to it.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeThread {
    pub(crate) root: FakeMessage,
    pub(crate) subject: Option<String>,
    /// `high`, `urgent`; empty: `normal`.
    pub(crate) importance: &'static str,
    pub(crate) replies: Vec<FakeMessage>,
    /// What the root mentions besides people (its `mentions[].mentioned`, as Graph writes them):
    /// a channel, a team, a tag.
    pub(crate) other_mentions: Vec<Value>,
}

impl FakeThread {
    pub(crate) fn new(root: FakeMessage, replies: Vec<FakeMessage>) -> Self {
        Self { root, replies, ..Self::default() }
    }

    /// When it last changed: its root's or one of its replies' last change, the latest.
    fn activity(&self) -> i64 {
        std::iter::once(&self.root).chain(&self.replies).filter_map(|m| ms_of(&m.modified)).max().unwrap_or_default()
    }
}

/// A member of a channel.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeMember {
    pub(crate) user: FakeUser,
    /// A guest (`roles` says `guest`).
    pub(crate) guest: bool,
    /// Its organisation (`tenantId`), when not the signed-in user's.
    pub(crate) tenant: Option<String>,
    /// Its `@odata.type`, when not an Entra user's (`aadUserConversationMember`).
    pub(crate) kind: Option<&'static str>,
}

impl FakeMember {
    pub(crate) fn new(user: FakeUser) -> Self {
        Self { user, ..Self::default() }
    }
}

/// How the channels' routes behave, as the step 0 check measures it.
#[derive(Clone, Debug)]
pub(crate) struct ChannelRules {
    /// A reply moves its root's `lastModifiedDateTime` (else the root keeps its own).
    pub(crate) root_moves: bool,
    /// Replies expanded per root at most; past it the root carries `replies@odata.nextLink`.
    pub(crate) expand_cap: usize,
    /// `/replies` lists the oldest first (else the newest first).
    pub(crate) replies_oldest_first: bool,
    /// The old `…/messages/delta`'s status (200: an empty page with a delta link).
    pub(crate) delta_status: u16,
    /// A channel's members, and the joined teams, given this many a page.
    pub(crate) members_page: usize,
    pub(crate) teams_page: usize,
    /// The joined teams are refused with 403 (no `Team.ReadBasic.All`).
    pub(crate) forbid_teams: bool,
}

impl Default for ChannelRules {
    fn default() -> Self {
        Self {
            root_moves: true,
            expand_cap: 1000,
            replies_oldest_first: false,
            delta_status: 400,
            members_page: 100,
            teams_page: 100,
            forbid_teams: false,
        }
    }
}

impl FakeTeams {
    fn team(&self, id: Option<&&str>) -> Option<&FakeTeam> {
        self.teams.iter().find(|t| Some(&t.id.as_str()) == id)
    }

    /// A team's channel (by the ids of the path `teams/{t}/channels/{c}/…`), and its team.
    fn channel_at(&self, p: &[&str]) -> Option<(&FakeTeam, &FakeChannel)> {
        let team = self.team(p.get(1))?;
        Some((team, team.channels.iter().find(|c| Some(&c.id.as_str()) == p.get(3))?))
    }

    fn team_json(&self, t: &FakeTeam) -> Value {
        json!({
            "id": t.id, "displayName": t.name, "description": t.description, "isArchived": t.archived,
            "tenantId": t.tenant.as_deref().unwrap_or(&self.tenant), "internalId": null, "classification": null, "specialization": null,
            "visibility": null, "webUrl": null,
        })
    }

    fn channel_json(&self, t: &FakeTeam, c: &FakeChannel) -> Value {
        let tenant = c.tenant.as_deref().or(t.tenant.as_deref()).unwrap_or(&self.tenant);
        let membership = if c.membership == "shared" && !self.asked_enums { "unknownFutureValue" } else { c.membership };
        let own = format!(
            "https://teams.microsoft.com/l/channel/{}/{}?groupId={}&tenantId={tenant}",
            crate::http::pct(&c.id),
            crate::http::pct(&c.name),
            t.id
        );
        json!({
            "id": c.id, "displayName": c.name, "description": null, "createdDateTime": "2026-01-01T00:00:00Z",
            "email": format!("{}@contoso.example", c.name.replace(' ', ".")), "membershipType": membership, "isFavoriteByDefault": null,
            "layoutType": c.layout, "tenantId": tenant, "webUrl": c.link.clone().unwrap_or(Some(own)),
        })
    }

    /// A message of a channel's thread, as Graph lists it: the root, or with `reply` that reply.
    fn post_json(&self, t: &FakeTeam, c: &FakeChannel, th: &FakeThread, reply: Option<&FakeMessage>) -> Value {
        let m = reply.unwrap_or(&th.root);
        let mut v = self.message_json(&FakeChat::default(), m);
        v["chatId"] = Value::Null;
        v["replyToId"] = reply.map_or(Value::Null, |_| json!(th.root.id));
        v["channelIdentity"] = json!({"teamId": t.id, "channelId": c.id});
        v["webUrl"] = json!(format!(
            "https://teams.microsoft.com/l/message/{}/{}?groupId={}&parentMessageId={}",
            crate::http::pct(&c.id),
            m.id,
            t.id,
            th.root.id
        ));
        if reply.is_none() {
            v["subject"] = json!(th.subject);
            v["importance"] = json!(if th.importance.is_empty() { "normal" } else { th.importance });
            if self.channel_rules.root_moves {
                v["lastModifiedDateTime"] = json!(rfc3339(th.activity()));
            }
            if let Some(list) = v.get_mut("mentions").and_then(Value::as_array_mut) {
                let first = list.len();
                list.extend(th.other_mentions.iter().enumerate().map(|(i, m)| {
                    json!({"id": first + i, "mentionText": m.pointer("/conversation/displayName").or(m.pointer("/tag/displayName")), "mentioned": m})
                }));
            }
        }
        v
    }

    /// A thread's replies in the order `/replies` lists them.
    fn replies_listed<'a>(&self, th: &'a FakeThread) -> Vec<&'a FakeMessage> {
        let mut list: Vec<&FakeMessage> = th.replies.iter().collect();
        list.sort_by_key(|m| ms_of(&m.created));
        if !self.channel_rules.replies_oldest_first {
            list.reverse();
        }
        list
    }

    /// The routes about teams and channels; `None` for the others.
    pub(super) fn channel_route(&self, name: &str, p: &[&str], q: &HashMap<String, String>, path: &str, query: &str, base: &str) -> Option<Reply> {
        let not_found = || Reply::error(404, "NotFound", "Not found.");
        let rules = &self.channel_rules;
        Some(match name {
            "joinedTeams" if rules.forbid_teams => Reply::error(403, "Forbidden", "Reading the joined teams is not allowed."),
            "joinedTeams" => page(self.teams.iter().map(|t| self.team_json(t)).collect(), rules.teams_page, q, path, query, base, Paging::Plain),
            "channels" => match self.team(p.get(1)) {
                None => not_found(),
                Some(t) if t.forbid_channels => Reply::error(403, "Forbidden", "Reading this team's channels is not allowed."),
                Some(t) => Reply::json(200, &json!({ "value": t.channels.iter().map(|c| self.channel_json(t, c)).collect::<Vec<_>>() })),
            },
            "channelMessages" | "channelReplies" | "channelDelta" | "channelMembers" => {
                let Some((t, c)) = self.channel_at(p) else { return Some(not_found()) };
                match name {
                    "channelMembers" if c.forbid_members => Reply::error(403, "Forbidden", "Reading this channel's members is not allowed."),
                    "channelMembers" => {
                        let members = c.members.iter().map(|m| self.member_json(t, c, m)).collect();
                        page(members, rules.members_page, q, path, query, base, Paging::Plain)
                    }
                    _ if c.forbid_messages => Reply::error(403, "Forbidden", "Reading this channel's messages is not allowed."),
                    "channelDelta" => match rules.delta_status {
                        200 => Reply::json(200, &json!({"value": [], "@odata.deltaLink": format!("{base}{path}?$deltatoken=1")})),
                        403 => Reply::error(403, "Forbidden", "Not allowed."),
                        404 => Reply::error(404, "NotFound", "Not found."),
                        s => Reply::error(s, "BadRequest", "Resource not found for the segment 'delta'."),
                    },
                    "channelMessages" => self.roots(t, c, q, path, query, base),
                    _ => match c.threads.iter().find(|th| Some(&th.root.id.as_str()) == p.get(5)) {
                        None => not_found(),
                        Some(th) => match top(q, 20) {
                            Err(e) => e,
                            Ok(n) => {
                                let replies = self.replies_listed(th).into_iter().map(|r| self.post_json(t, c, th, Some(r))).collect();
                                page(replies, n, q, path, query, base, Paging::Plain)
                            }
                        },
                    },
                }
            }
            _ => return None,
        })
    }

    /// A channel's root messages, the thread changed last first, with their replies when asked.
    fn roots(&self, t: &FakeTeam, c: &FakeChannel, q: &HashMap<String, String>, path: &str, query: &str, base: &str) -> Reply {
        let n = match top(q, 20) {
            Ok(n) => n,
            Err(e) => return e,
        };
        let expand = q.get("$expand").is_some_and(|e| e.split(',').any(|x| x.trim() == "replies"));
        let mut threads: Vec<&FakeThread> = c.threads.iter().collect();
        threads.sort_by_key(|th| std::cmp::Reverse(th.activity()));
        let cap = self.channel_rules.expand_cap;
        let roots = threads
            .into_iter()
            .map(|th| {
                let mut v = self.post_json(t, c, th, None);
                if expand {
                    let replies: Vec<Value> = self.replies_listed(th).into_iter().take(cap).map(|r| self.post_json(t, c, th, Some(r))).collect();
                    v["replies"] = json!(replies);
                    if th.replies.len() > cap {
                        let rest = format!("/teams/{}/channels/{}/messages/{}/replies", t.id, crate::http::pct(&c.id), th.root.id);
                        v["replies@odata.nextLink"] = json!(format!("{base}{rest}?$skiptoken={cap}"));
                    }
                }
                v
            })
            .collect();
        page(roots, n, q, path, query, base, Paging::Plain)
    }

    fn member_json(&self, t: &FakeTeam, c: &FakeChannel, m: &FakeMember) -> Value {
        json!({
            "@odata.type": m.kind.unwrap_or("#microsoft.graph.aadUserConversationMember"), "id": format!("{}#{}", c.id, m.user.id),
            "roles": if m.guest { json!(["guest"]) } else { json!([]) }, "displayName": m.user.display_name, "userId": m.user.id,
            "email": m.user.mail, "tenantId": m.tenant.as_deref().or(t.tenant.as_deref()).unwrap_or(&self.tenant),
            "visibleHistoryStartDateTime": "0001-01-01T00:00:00Z",
        })
    }
}
