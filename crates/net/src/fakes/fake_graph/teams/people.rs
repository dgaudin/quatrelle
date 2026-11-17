// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fake Graph server's directory and the chats it creates, as Graph v1.0 answers them:
//! `GET /users` searched by `$search` (which wants `$count=true` and `ConsistencyLevel: eventual`,
//! else 400; its `$search` kept in `last_search`), and `POST /chats` (`createChat`), which gives
//! the one-to-one chat already there or makes the chat (201), once its members are bound as Graph
//! documents: each an `aadUserConversationMember` at `{API}/users('{id}')`, a guest as `guest`,
//! anyone else as `owner`, the signed-in user among them.

use super::*;

impl FakeTeams {
    /// `GET /users`: the people of `directory` one of the `$search`'s quoted `field:value` terms
    /// finds (`displayName` or `mail` holding the value, case aside), `$top` of them.
    pub(super) fn users(&mut self, query: &str, headers: &[(String, String)]) -> Reply {
        let q = params(query);
        self.last_search = q.get("$search").cloned().unwrap_or_default();
        let eventual = headers.iter().any(|(k, v)| k == "consistencylevel" && v.eq_ignore_ascii_case("eventual"));
        if q.get("$count").map(String::as_str) != Some("true") || !eventual {
            return Reply::error(400, "Request_UnsupportedQuery", "$search wants $count=true and ConsistencyLevel: eventual.");
        }
        let Some(top) = q.get("$top").map_or(Some(100), |t| t.parse::<usize>().ok().filter(|n| (1..=999).contains(n))) else {
            return Reply::error(400, "Request_UnsupportedQuery", "$top must be from 1 to 999.");
        };
        let terms: Vec<(String, String)> = self
            .last_search
            .split('"')
            .skip(1)
            .step_by(2)
            .filter_map(|t| t.split_once(':').map(|(field, value)| (field.to_string(), value.to_lowercase())))
            .collect();
        let found = |u: &FakeUser| {
            terms.iter().any(|(field, value)| match field.as_str() {
                "displayName" => u.display_name.to_lowercase().contains(value.as_str()),
                "mail" => u.mail.to_lowercase().contains(value.as_str()),
                _ => false,
            })
        };
        let value: Vec<Value> = self
            .directory
            .iter()
            .filter(|u| found(u))
            .take(top)
            .map(|u| {
                json!({
                    "id": u.id, "displayName": u.display_name, "mail": if u.mail.is_empty() { Value::Null } else { json!(u.mail) },
                    "userPrincipalName": u.upn, "userType": if self.guests.contains(&u.id) { "Guest" } else { "Member" },
                })
            })
            .collect();
        Reply::json(200, &json!({"@odata.count": value.len(), "value": value}))
    }

    /// `POST /chats`: the module's rules.
    pub(super) fn create_chat(&mut self, input: &Value, base: &str) -> Reply {
        let bad = |why: &str| Reply::error(400, "BadRequest", why);
        if self.forbid_create {
            return Reply::error(403, "Forbidden", "Creating chats is not allowed.");
        }
        let kind = match input.get("chatType").and_then(Value::as_str) {
            Some("oneOnOne") => "oneOnOne",
            Some("group") => "group",
            _ => return bad("chatType must be oneOnOne or group."),
        };
        let bind = format!("{base}/users('");
        let mut ids: Vec<String> = Vec::new();
        for m in input.get("members").and_then(Value::as_array).into_iter().flatten() {
            let typed = m.get("@odata.type").and_then(Value::as_str) == Some("#microsoft.graph.aadUserConversationMember");
            let roles: Vec<&str> = m.get("roles").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
            let id = m.get("user@odata.bind").and_then(Value::as_str).and_then(|b| b.strip_prefix(&bind)?.strip_suffix("')"));
            let Some(id) = id.filter(|_| typed) else { return bad("A member isn't bound as Graph documents.") };
            let role = if self.guests.iter().any(|g| g == id) { "guest" } else { "owner" };
            if roles != [role] || ids.iter().any(|i| i == id) {
                return bad("A member's roles are wrong, or a member is there twice.");
            }
            if id != self.me.id && !self.directory.iter().any(|u| u.id == id) && self.user(id).is_none() {
                return Reply::error(404, "NotFound", "No such user.");
            }
            ids.push(id.to_string());
        }
        let fits = match kind {
            "oneOnOne" => ids.len() == 2,
            _ => ids.len() >= 3,
        };
        if !ids.contains(&self.me.id) || !fits {
            return bad("Not the members this kind of chat takes.");
        }
        let mut sorted = ids.clone();
        sorted.sort();
        let same = |c: &FakeChat| {
            let mut theirs: Vec<String> = c.members.iter().map(|u| u.id.clone()).collect();
            theirs.sort();
            c.kind == "oneOnOne" && theirs == sorted
        };
        if kind == "oneOnOne"
            && let Some(c) = self.chats.iter().find(|c| same(c))
        {
            return Reply::json(201, &self.chat_json(c, false));
        }
        let members: Vec<FakeUser> = ids
            .iter()
            .map(|id| self.directory.iter().find(|u| u.id == *id).or_else(|| self.user(id)).cloned().unwrap_or_else(|| FakeUser::new(id, id)))
            .collect();
        let id =
            if kind == "oneOnOne" { format!("19:{}@unq.gbl.spaces", ids.join("_")) } else { format!("19:{:032x}@thread.v2", self.chats.len() + 1) };
        let topic = input.get("topic").and_then(Value::as_str).filter(|t| kind == "group" && !t.is_empty());
        let chat = FakeChat::new(&id, kind, topic, members);
        let made = self.chat_json(&chat, false);
        self.chats.push(chat);
        Reply::json(201, &made)
    }
}
