// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fake Graph server's Teams routes about chats: the chat list, and one chat's members,
//! messages (listed in a `lastModifiedDateTime` range, posted, edited, reacted to,
//! soft-deleted), hosted images and read mark.

use super::*;

impl FakeTeams {
    fn message_json(&self, chat: &FakeChat, m: &FakeMessage) -> Value {
        json!({
            "id": m.id, "replyToId": null, "etag": ms_of(&m.modified).unwrap_or_default().to_string(), "messageType": "message",
            "createdDateTime": m.created, "lastModifiedDateTime": m.modified, "lastEditedDateTime": m.edited, "deletedDateTime": m.deleted,
            "subject": null, "summary": null, "chatId": chat.id, "importance": "normal", "locale": "en-us", "webUrl": null,
            "channelIdentity": null, "policyViolation": null, "eventDetail": null,
            "from": m.from.as_deref().map(|f| self.identity(f)),
            "body": {"contentType": "html", "content": if m.deleted.is_some() { "" } else { m.body_html.as_str() }},
            "attachments": m.attachments.iter().enumerate().map(|(i, (kind, url))| json!({
                "id": format!("{}-{i}", m.id), "contentType": kind, "contentUrl": url, "content": null,
                "name": url.rsplit('/').next(), "thumbnailUrl": null, "teamsAppId": null,
            })).collect::<Vec<_>>(),
            "mentions": [],
            "reactions": m.reactions.iter().map(|(kind, user)| json!({
                "reactionType": kind, "displayName": null, "reactionContentUrl": null, "createdDateTime": m.modified, "user": self.identity(user),
            })).collect::<Vec<_>>(),
        })
    }

    /// The newest message not deleted.
    fn newest(chat: &FakeChat) -> Option<&FakeMessage> {
        chat.messages.iter().filter(|m| m.deleted.is_none()).max_by_key(|m| ms_of(&m.created))
    }

    fn chat_json(&self, c: &FakeChat, preview: bool) -> Value {
        let created = c.messages.iter().map(|m| m.created.as_str()).min_by_key(|t| ms_of(t)).unwrap_or("2026-01-01T00:00:00.000Z");
        let updated = c.updated.as_deref().unwrap_or(created);
        let mut v = json!({
            "id": c.id, "topic": c.topic, "createdDateTime": created, "lastUpdatedDateTime": updated, "chatType": c.kind,
            "webUrl": c.link.clone().unwrap_or_else(|| Some(format!("https://teams.microsoft.com/l/chat/{}/0?tenantId={}", c.id, self.tenant))),
            "tenantId": c.tenant.as_deref().unwrap_or(&self.tenant),
            "onlineMeetingInfo": null, "viewpoint": {"isHidden": c.hidden, "lastMessageReadDateTime": c.read_time},
        });
        if preview {
            v["lastMessagePreview"] = Self::newest(c).map_or(Value::Null, |m| {
                json!({
                    "id": m.id, "createdDateTime": m.created, "isDeleted": false, "messageType": "message", "eventDetail": null,
                    "body": {"contentType": "html", "content": m.body_html}, "from": m.from.as_deref().map(|f| self.identity(f)),
                })
            });
        }
        v
    }

    pub(super) fn chats(&self, q: &HashMap<String, String>, path: &str, query: &str, base: &str) -> Reply {
        let top = match top(q, 20) {
            Ok(t) => t,
            Err(e) => return e,
        };
        let preview = q.get("$expand").is_some_and(|e| e.split(',').any(|x| x.trim() == "lastMessagePreview"));
        let mut chats: Vec<&FakeChat> = self.chats.iter().collect();
        if q.get("$orderby").is_some_and(|o| o.starts_with("lastMessagePreview/createdDateTime")) {
            chats.sort_by_key(|c| std::cmp::Reverse(Self::newest(c).and_then(|m| ms_of(&m.created))));
        }
        let top = self.list_page.unwrap_or(top);
        let mut reply = page(chats.into_iter().map(|c| self.chat_json(c, preview)).collect(), top, q, path, query, base, self.paging("chats"));
        if self.list_next_elsewhere
            && let Ok(mut v) = serde_json::from_slice::<Value>(&reply.body)
            && let Some(next) = v.get("@odata.nextLink").and_then(Value::as_str).map(|n| n.replacen(base, "https://elsewhere.example", 1))
        {
            v["@odata.nextLink"] = json!(next);
            reply = Reply::json(200, &v);
        }
        reply
    }

    /// A chat's messages in a `lastModifiedDateTime` range (`gt` and/or `lt`, joined by `and`).
    fn messages(&self, chat: &FakeChat, q: &HashMap<String, String>, path: &str, query: &str, base: &str) -> Reply {
        let top = match top(q, 20) {
            Ok(t) => t,
            Err(e) => return e,
        };
        let refused = || Reply::error(400, "BadRequest", "A lastModifiedDateTime range is wanted.");
        let Some(filter) = q.get("$filter") else { return refused() };
        let mut range: Vec<(bool, i64)> = Vec::new();
        for cond in filter.split(" and ") {
            match cond.split_whitespace().collect::<Vec<_>>().as_slice() {
                [field, op, at] if field.eq_ignore_ascii_case("lastModifiedDateTime") && ["gt", "lt"].contains(op) => match ms_of(at) {
                    Some(t) => range.push((*op == "gt", t)),
                    None => return refused(),
                },
                _ => return refused(),
            }
        }
        let by_modified = q.get("$orderby").is_some_and(|o| o.starts_with("lastModifiedDateTime"));
        let key = |m: &FakeMessage| ms_of(if by_modified { &m.modified } else { &m.created });
        let mut list: Vec<&FakeMessage> = chat
            .messages
            .iter()
            .filter(|m| ms_of(&m.modified).is_some_and(|t| range.iter().all(|(gt, at)| if *gt { t > *at } else { t < *at })))
            .collect();
        list.sort_by_key(|m| std::cmp::Reverse(key(m)));
        page(list.into_iter().map(|m| self.message_json(chat, m)).collect(), top, q, path, query, base, self.paging("messages"))
    }

    /// A request about one chat (`/chats/{id}/…`), by route name: 404 when the fake has no such
    /// chat.
    pub(super) fn chat_route(&mut self, name: &str, p: &[&str], query: &str, raw_path: &str, input: &Value, base: &str) -> Reply {
        let not_found = || Reply::error(404, "NotFound", "Not found.");
        let q = params(query);
        let Some(ci) = p.get(1).and_then(|id| self.chats.iter().position(|c| c.id == *id)) else { return not_found() };
        let now = rfc3339(self.now());
        let mine = self.me.id.clone();
        let tenant = self.tenant.clone();
        let forbidden = self.forbid_post.iter().any(|f| Some(f.as_str()) == p.get(1).copied());
        let members_refused = self.forbid_members.iter().any(|f| Some(f.as_str()) == p.get(1).copied());
        let guests = self.guests.clone();
        let Some(chat) = self.chats.get_mut(ci) else { return not_found() };
        match name {
            "members" if members_refused => Reply::error(403, "Forbidden", "Reading this chat's members is not allowed."),
            "hosted" => {
                let Some(m) = p.get(3).and_then(|mid| chat.messages.iter().find(|m| m.id == *mid)) else { return not_found() };
                match p.get(5).and_then(|h| m.hosted.iter().find(|(id, _)| id == h)) {
                    Some((_, bytes)) => Reply { status: 200, body: bytes.clone(), kind: "image/png", location: None, date: None },
                    None => not_found(),
                }
            }
            "members" => {
                let members: Vec<Value> = chat
                    .members
                    .iter()
                    .map(|u| {
                        let roles = if guests.contains(&u.id) { json!(["guest"]) } else { json!(["owner"]) };
                        json!({
                            "@odata.type": "#microsoft.graph.aadUserConversationMember", "id": format!("{}#{}", chat.id, u.id), "roles": roles,
                            "displayName": u.display_name, "userId": u.id, "email": u.mail, "tenantId": tenant,
                            "visibleHistoryStartDateTime": "0001-01-01T00:00:00Z",
                        })
                    })
                    .collect();
                Reply::json(200, &json!({ "value": members }))
            }
            "messages" => {
                let filter = q.get("$filter").cloned().unwrap_or_default();
                self.message_reads.push((chat.id.clone(), filter));
                if self.fail_reads.contains(&chat.id) {
                    return Reply::error(500, "InternalServerError", "This chat's messages fail, as the test asked.");
                }
                if self.forbid_reads.contains(&chat.id) {
                    return Reply::error(403, "Forbidden", "Reading this chat is not allowed.");
                }
                if self.refuse_links > 0 && q.contains_key("$skiptoken") {
                    self.refuse_links -= 1;
                    if self.links_not_found {
                        return Reply::error(404, "NotFound", "The skip token was not found.");
                    }
                    return Reply::error(400, "BadRequest", "The skip token is no longer valid.");
                }
                let Some(chat) = self.chats.get(ci).cloned() else { return not_found() };
                self.messages(&chat, &q, raw_path, query, base)
            }
            "post" if forbidden => Reply::error(403, "Forbidden", "Posting in this chat is not allowed."),
            "post" => {
                let after = chat.messages.iter().filter_map(|m| m.id.parse::<i64>().ok()).max().map_or(0, |m| m + 1);
                let id = ms_of(&now).unwrap_or_default().max(after).to_string();
                let html = input.pointer("/body/content").and_then(Value::as_str).unwrap_or_default();
                chat.messages.push(FakeMessage::new(&id, Some(&mine), &now, html));
                let chat = chat.clone();
                let made = chat.messages.last().map(|m| self.message_json(&chat, m)).unwrap_or(Value::Null);
                Reply::json(201, &made)
            }
            "markChatReadForUser" => {
                let user = &input["user"];
                if user["id"].as_str() != Some(mine.as_str()) || user["tenantId"].as_str() != Some(tenant.as_str()) {
                    return Reply::error(400, "BadRequest", "The user is not the signed-in user.");
                }
                chat.read_time = now;
                Reply::empty(204)
            }
            _ => {
                let Some(m) = p.get(3).and_then(|mid| chat.messages.iter_mut().find(|m| m.id == *mid)) else { return not_found() };
                let kind = input["reactionType"].as_str().unwrap_or_default().to_string();
                match name {
                    "edit" => {
                        if let Some(html) = input.pointer("/body/content").and_then(Value::as_str) {
                            m.body_html = html.to_string();
                        }
                        m.edited = Some(now.clone());
                    }
                    "setReaction" | "unsetReaction" if kind.is_empty() => return Reply::error(400, "BadRequest", "A reactionType is wanted."),
                    "setReaction" => {
                        if !m.reactions.iter().any(|(k, u)| *k == kind && *u == mine) {
                            m.reactions.push((kind, mine));
                        }
                    }
                    "unsetReaction" => m.reactions.retain(|(k, u)| !(*k == kind && *u == mine)),
                    "softDelete" => m.deleted = Some(now.clone()),
                    _ => return not_found(),
                }
                m.modified = now;
                Reply::empty(204)
            }
        }
    }
}

/// `$top`: `None` when absent, `Err` when it isn't a number from 1 to 50.
fn top(q: &HashMap<String, String>, default: usize) -> Result<usize, Reply> {
    match q.get("$top") {
        None => Ok(default),
        Some(t) => {
            t.parse::<usize>().ok().filter(|n| (1..=50).contains(n)).ok_or_else(|| Reply::error(400, "BadRequest", "$top must be from 1 to 50."))
        }
    }
}

/// How a listing's pages follow one another.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Paging {
    /// As Graph's do.
    Plain,
    /// Every page's link points at the second page.
    Second,
    /// Every link brings the first page again, under a link never given before (this number).
    Same(u32),
}

/// One page of a listing at `$skiptoken`, with `@odata.nextLink` for the rest.
fn page(list: Vec<Value>, top: usize, q: &HashMap<String, String>, path: &str, query: &str, base: &str, paging: Paging) -> Reply {
    let skip: usize = if matches!(paging, Paging::Same(_)) { 0 } else { q.get("$skiptoken").and_then(|s| s.parse().ok()).unwrap_or(0) };
    let mut v = json!({ "value": list.iter().skip(skip).take(top).cloned().collect::<Vec<_>>() });
    let token = match paging {
        Paging::Plain => (skip.saturating_add(top) < list.len()).then(|| (skip + top).to_string()),
        Paging::Second => Some(top.to_string()),
        Paging::Same(n) => Some(format!("again{n}")),
    };
    if let Some(token) = token {
        let rest: Vec<&str> =
            query.split('&').filter(|kv| !kv.is_empty() && !kv.starts_with("$skiptoken=") && !kv.starts_with("%24skiptoken=")).collect();
        let sep = if rest.is_empty() { "" } else { "&" };
        v["@odata.nextLink"] = json!(format!("{base}{path}?{}{sep}$skiptoken={token}", rest.join("&")));
    }
    Reply::json(200, &v)
}

impl FakeTeams {
    /// How the listing of `route` pages.
    fn paging(&self, route: &str) -> Paging {
        match () {
            _ if self.loop_next == Some(route) => Paging::Second,
            _ if self.repeat_page == Some(route) => Paging::Same(self.hits.get(route).copied().unwrap_or(0)),
            _ => Paging::Plain,
        }
    }
}
