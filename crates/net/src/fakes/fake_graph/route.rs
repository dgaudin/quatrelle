//! The fake Graph server's requests: each path and method answered from the data, as Graph would.

use super::*;

impl Data {
    /// One request: (status, body, content type).
    pub(super) fn route(&mut self, method: &str, target: &str, body: &[u8], base: &str) -> (u16, Vec<u8>, &'static str) {
        self.calls.push(format!("{method} {target}"));
        let now = std::time::Instant::now();
        for (_, raw) in self.late.extract_if(.., |(at, _)| *at <= now).collect::<Vec<_>>() {
            let sent = self.resolve("sentitems").unwrap_or_default();
            self.deliver(&sent, &raw, true, "2030-01-01T00:00:00Z");
        }
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let q = |k: &str| query.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}=")).map(str::to_string));
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let ok = |v: Value| (200, v.to_string().into_bytes(), "application/json");
        let not_found =
            || (404, json!({"error": {"code": "ErrorItemNotFound", "message": "Not found."}}).to_string().into_bytes(), "application/json");
        let input: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        // As Graph answers a Microsoft account with no mailbox (one made with a Gmail address).
        if self.no_mailbox && matches!(parts.as_slice(), ["me", "mailFolders" | "messages" | "sendMail" | "mailboxSettings", ..]) {
            let e = json!({"error": {"code": "MailboxNotEnabledForRESTAPI", "message": "The mailbox is either inactive, soft-deleted, or is hosted on-premise."}});
            return (404, e.to_string().into_bytes(), "application/json");
        }
        // Another mailbox's mail, as `/me`'s but for that mailbox; one not shared with this
        // account is refused.
        let mut parts = parts;
        self.ctx = None;
        if let ["users", who, "mailFolders" | "messages", ..] = parts.as_slice() {
            let who = who.replace("%40", "@").to_ascii_lowercase();
            if !self.shared.contains(&who) {
                return (
                    403,
                    json!({"error": {"code": "ErrorAccessDenied", "message": "Access is denied."}}).to_string().into_bytes(),
                    "application/json",
                );
            }
            self.ctx = Some(who);
            parts = std::iter::once("me").chain(parts.iter().skip(2).copied()).collect();
        }
        // A message or folder asked for through another mailbox than its own isn't found there.
        if let ["me", "messages", id, ..] = parts.as_slice()
            && let Some(m) = self.message(id)
            && self.owner_of(&m.folder) != self.ctx
        {
            return not_found();
        }
        match (method, parts.as_slice()) {
            ("POST", ["$batch"]) => {
                let mut out = Vec::new();
                for (i, r) in input["requests"].as_array().cloned().unwrap_or_default().into_iter().enumerate() {
                    if self.at_once.is_some_and(|n| i >= n) {
                        let mut refused = serde_json::Map::new();
                        refused.insert("id".into(), r["id"].clone());
                        refused.insert("status".into(), json!(429));
                        refused.insert("headers".into(), json!({ "retry-after": 1 }));
                        refused.insert("body".into(), json!({"error": {"code": "TooManyRequests"}}));
                        out.push(Value::Object(refused));
                        continue;
                    }
                    // Graph reads Prefer from each batch entry's own headers.
                    let prefer = r.pointer("/headers/Prefer").and_then(Value::as_str).unwrap_or_default();
                    self.asked_immutable = prefer.split(',').any(|part| part.trim() == "IdType=\"ImmutableId\"");
                    let b = serde_json::to_vec(&r["body"]).unwrap_or_default();
                    let (status, rb, _) = self.route(r["method"].as_str().unwrap_or("GET"), r["url"].as_str().unwrap_or("/"), &b, base);
                    self.asked_immutable = true;
                    let rbody: Value = serde_json::from_slice(&rb).unwrap_or(Value::Null);
                    out.push(json!({"id": r["id"], "status": status, "body": rbody}));
                }
                ok(json!({ "responses": out }))
            }
            ("GET", ["me", "mailFolders"]) => {
                let top = self.top();
                self.page(self.folders.iter().filter(|f| f.parent == top).map(|f| self.folder_json(f)).collect(), path, query, base)
            }
            ("POST", ["me", "mailFolders"]) => {
                let id = self.folder(input["displayName"].as_str().unwrap_or("?"), None, None);
                ok(json!({ "id": id }))
            }
            ("GET", ["me", "mailFolders", id]) => match self.resolve(id) {
                Some(id) => {
                    let f = self.folders.iter().find(|f| f.id == id).cloned();
                    f.map_or_else(not_found, |f| ok(self.folder_json(&f)))
                }
                None => not_found(),
            },
            ("PATCH", ["me", "mailFolders", id]) => match self.folders.iter_mut().find(|f| f.id == *id) {
                Some(f) => {
                    f.name = input["displayName"].as_str().unwrap_or(&f.name).to_string();
                    ok(json!({ "id": id }))
                }
                None => not_found(),
            },
            ("DELETE", ["me", "mailFolders", id]) => {
                let id = id.to_string();
                self.folders.retain(|f| f.id != id);
                self.messages.retain(|m| m.folder != id);
                (204, Vec::new(), "text/plain")
            }
            ("GET", ["me", "mailFolders", id, "childFolders"]) => {
                let list = self.folders.iter().filter(|f| f.parent == *id).map(|f| self.folder_json(f)).collect();
                self.page(list, path, query, base)
            }
            ("POST", ["me", "mailFolders", id, "childFolders"]) => {
                let id = self.folder(input["displayName"].as_str().unwrap_or("?"), Some(id), None);
                ok(json!({ "id": id }))
            }
            ("POST", ["me", "mailFolders", id, "move"]) => {
                let dest = input["destinationId"].as_str().unwrap_or_default().to_string();
                let dest = if dest == "msgfolderroot" { ROOT.to_string() } else { dest };
                match self.folders.iter_mut().find(|f| f.id == *id) {
                    Some(f) => {
                        f.parent = dest;
                        ok(json!({ "id": id }))
                    }
                    None => not_found(),
                }
            }
            ("GET", ["me", "mailFolders", id, "messages"]) => {
                let Some(id) = self.resolve(id) else { return not_found() };
                let mut list: Vec<&Msg> = self.messages.iter().filter(|m| m.folder == id).collect();
                if let Some(o) = q("$orderby") {
                    assert_eq!(o, "receivedDateTime%20desc");
                }
                // A search: its words (KQL, the whole in quotes), or read and flag conditions.
                if let Some(s) = q("$search") {
                    let s = crate::oauth::unpct(&s);
                    let inner = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(&s).replace("\\\"", "\"");
                    list.retain(|m| words_match(&inner, &m.raw));
                }
                if let Some(f) = q("$filter") {
                    let f = crate::oauth::unpct(&f);
                    for cond in f.split(" and ") {
                        match cond {
                            "isRead eq false" => list.retain(|m| !m.read),
                            "isRead eq true" => list.retain(|m| m.read),
                            "flag/flagStatus eq 'flagged'" => list.retain(|m| m.flagged),
                            "flag/flagStatus ne 'flagged'" => list.retain(|m| !m.flagged),
                            "hasAttachments eq true" => list.clear(),
                            _ => {}
                        }
                    }
                }
                list.sort_by(|a, b| b.received.cmp(&a.received));
                let top: usize = q("$top").and_then(|t| t.parse().ok()).unwrap_or(10);
                let list: Vec<Value> = list.into_iter().take(top).map(|m| self.message_json(m)).collect();
                ok(json!({ "value": list }))
            }
            ("POST", ["me", "messages"]) => {
                let raw = unbase64(body);
                let drafts = self.resolve("drafts").unwrap_or_default();
                let id = self.deliver(&drafts, &raw, true, "2030-01-01T00:00:00Z");
                if let Some(m) = self.messages.iter_mut().find(|m| m.id == id) {
                    m.draft = true;
                }
                ok(json!({ "id": id, "parentFolderId": drafts }))
            }
            ("GET", ["me", "messages", id]) => self.message(id).map_or_else(not_found, |m| ok(self.message_json(m))),
            ("PATCH", ["me", "messages", id]) => match self.messages.iter_mut().find(|m| m.id == plain_id(id)) {
                Some(m) => {
                    if let Some(r) = input["isRead"].as_bool() {
                        m.read = r;
                    }
                    if let Some(f) = input["flag"]["flagStatus"].as_str() {
                        m.flagged = f == "flagged";
                    }
                    if let Some(subject) = input["subject"].as_str() {
                        let text = String::from_utf8_lossy(&m.raw).into_owned();
                        let kept: String = text.split_inclusive("\r\n").filter(|l| !l.starts_with("Subject:")).collect();
                        m.raw = format!("Subject: {subject}\r\n{kept}").into_bytes();
                    }
                    for p in input["singleValueExtendedProperties"].as_array().cloned().unwrap_or_default() {
                        if p["id"].as_str().is_some_and(|id| id.to_ascii_lowercase().ends_with("id 0x8524")) {
                            m.vote = p["value"].as_str().unwrap_or_default().to_string();
                        }
                    }
                    ok(json!({ "id": id }))
                }
                None => not_found(),
            },
            ("DELETE", ["me", "messages", id]) => {
                let id = plain_id(id).to_string();
                self.messages.retain(|m| m.id != id);
                (204, Vec::new(), "text/plain")
            }
            ("POST", ["me", "messages", id, "createReply"]) => {
                let Some(orig) = self.message(id).map(|m| quatrelle_mime::parse(&m.raw).unwrap_or_default()) else { return not_found() };
                let raw = format!(
                    "From: riley@contoso.example\r\nTo: {}\r\nSubject: RE: {}\r\nIn-Reply-To: <{}>\r\n\r\n",
                    orig.from.email, orig.subject, orig.message_id
                );
                let drafts = self.resolve("drafts").unwrap_or_default();
                let rid = self.deliver(&drafts, raw.as_bytes(), true, "2030-01-01T00:00:00Z");
                if let Some(m) = self.messages.iter_mut().find(|m| m.id == rid) {
                    m.draft = true;
                }
                (201, json!({"id": rid, "parentFolderId": drafts}).to_string().into_bytes(), "application/json")
            }
            ("POST", ["me", "messages", id, "send"]) => {
                let sent = self.resolve("sentitems").unwrap_or_default();
                match self.messages.iter_mut().find(|m| m.id == *id && m.draft) {
                    Some(m) => {
                        m.folder = sent;
                        m.draft = false;
                        let subject = quatrelle_mime::parse(&m.raw).map(|p| p.subject).unwrap_or_default();
                        let (raw, vote) = (m.raw.clone(), m.vote.clone());
                        self.sent.push(raw);
                        self.votes.push((subject, vote));
                        (202, Vec::new(), "text/plain")
                    }
                    None => not_found(),
                }
            }
            ("POST", ["me", "messages", id, "move"]) => {
                let dest = input["destinationId"].as_str().and_then(|d| self.resolve(d));
                match (dest, self.messages.iter_mut().find(|m| m.id == plain_id(id))) {
                    (Some(d), Some(m)) => {
                        m.folder = d;
                        ok(json!({ "id": id }))
                    }
                    _ => not_found(),
                }
            }
            ("GET", ["me", "messages", id, "$value"]) if self.unreadable.iter().any(|u| u == plain_id(id)) => {
                (500, json!({"error": {"code": "ErrorInternalServerError", "message": "can't read it"}}).to_string().into_bytes(), "application/json")
            }
            ("GET", ["me", "messages", id, "$value"]) => match self.message(id) {
                Some(m) => (200, m.raw.clone(), "text/plain"),
                None => not_found(),
            },
            ("POST", ["me", "sendMail"]) => {
                let raw = unbase64(body);
                if String::from_utf8_lossy(&raw).contains("nobody@") {
                    return (
                        400,
                        json!({"error": {"code": "ErrorInvalidRecipients", "message": "no such recipient"}}).to_string().into_bytes(),
                        "application/json",
                    );
                }
                if self.file_late {
                    self.late.push((std::time::Instant::now() + std::time::Duration::from_secs(1), raw.clone()));
                } else {
                    let sent = self.resolve("sentitems").unwrap_or_default();
                    self.deliver(&sent, &raw, true, "2030-01-01T00:00:00Z");
                }
                self.sent.push(raw);
                (202, Vec::new(), "text/plain")
            }
            ("GET", ["me", "mailboxSettings", "automaticRepliesSetting"]) => ok(self.replies.clone()),
            ("PATCH", ["me", "mailboxSettings"]) => {
                let r = input["automaticRepliesSetting"].clone();
                if r["internalReplyMessage"].as_str() == Some("refuse") {
                    return (400, json!({"error": {"message": "refused"}}).to_string().into_bytes(), "application/json");
                }
                self.replies = r;
                ok(json!({}))
            }
            ("GET", ["me", "calendars"]) => {
                let list: Vec<Value> = self
                    .calendars
                    .iter()
                    .enumerate()
                    .map(|(i, (id, name))| json!({"id": id, "name": name, "canEdit": true, "isDefaultCalendar": i == 0}))
                    .collect();
                ok(json!({ "value": list }))
            }
            ("GET", ["me", "calendarGroups"]) => {
                let list: Vec<Value> = self.groups.iter().map(|(id, name, _)| json!({"id": id, "name": name})).collect();
                ok(json!({ "value": list }))
            }
            ("GET", ["me", "calendarGroups", gid, "calendars"]) => {
                let Some((_, _, ids)) = self.groups.iter().find(|(g, _, _)| g == gid) else { return not_found() };
                let list: Vec<Value> = ids
                    .iter()
                    .filter_map(|id| match self.calendars.iter().find(|(c, _)| c == id) {
                        Some((c, name)) => Some(json!({"id": c, "name": name, "canEdit": true})),
                        None => self.group_calendars.iter().find(|(c, _, _, _)| c == id).map(|(c, name, owner, edit)| {
                            json!({"id": c, "name": name, "canEdit": edit, "owner": {"name": owner, "address": format!("{}@contoso.example", owner.to_lowercase())}})
                        }),
                    })
                    .collect();
                ok(json!({ "value": list }))
            }
            ("GET", ["me", "calendarGroups", gid, "calendars", cal, "events"]) => {
                if !self.groups.iter().any(|(g, _, ids)| g == gid && ids.iter().any(|c| c == cal)) {
                    return not_found();
                }
                let list: Vec<Value> = self.events.iter().filter(|e| e["calendar"] == *cal).cloned().collect();
                ok(json!({ "value": list }))
            }
            ("GET", ["me", "calendars", cal, "events"]) => {
                let list: Vec<Value> = self.events.iter().filter(|e| e["calendar"] == *cal).cloned().collect();
                ok(json!({ "value": list }))
            }
            ("POST", ["me", "calendars", cal, "events"]) => {
                let id = self.id("EV");
                let mut e = input.clone();
                e["id"] = json!(id);
                e["iCalUId"] = json!(format!("ical-{id}"));
                e["changeKey"] = json!("ck1");
                e["calendar"] = json!(cal);
                self.events.push(e.clone());
                (201, e.to_string().into_bytes(), "application/json")
            }
            // Someone else's calendar, by their address: read when shared, changed when they allow it.
            ("GET" | "POST" | "PATCH" | "DELETE", ["users", who, ..]) => {
                let who = who.replace("%40", "@").to_ascii_lowercase();
                let Some(edit) = self.others.iter().find(|(e, _)| *e == who).map(|(_, e)| *e) else {
                    return (404, json!({"error": {"code": "ErrorItemNotFound"}}).to_string().into_bytes(), "application/json");
                };
                let denied = || {
                    (
                        403,
                        json!({"error": {"code": "ErrorAccessDenied", "message": "Access is denied."}}).to_string().into_bytes(),
                        "application/json",
                    )
                };
                let cal = format!("user:{who}");
                match (method, parts.get(2..).unwrap_or_default()) {
                    ("GET", ["calendar"]) => ok(json!({"id": cal, "canEdit": edit})),
                    ("GET", ["calendar", "events"]) => {
                        ok(json!({"value": self.events.iter().filter(|e| e["calendar"] == cal.as_str()).cloned().collect::<Vec<_>>()}))
                    }
                    (_, _) if !edit => denied(),
                    ("POST", ["calendar", "events"]) => {
                        let id = self.id("EV");
                        let mut e = input.clone();
                        e["id"] = json!(id);
                        e["iCalUId"] = json!(format!("ical-{id}"));
                        e["changeKey"] = json!("ck1");
                        e["calendar"] = json!(cal);
                        self.events.push(e.clone());
                        (201, e.to_string().into_bytes(), "application/json")
                    }
                    ("PATCH", ["events", id]) => match self.events.iter_mut().find(|e| e["id"] == *id && e["calendar"] == cal.as_str()) {
                        Some(e) => {
                            for (k, v) in input.as_object().cloned().unwrap_or_default() {
                                e[k.as_str()] = v;
                            }
                            ok(e.clone())
                        }
                        None => not_found(),
                    },
                    ("DELETE", ["events", id]) => {
                        self.events.retain(|e| !(e["id"] == *id && e["calendar"] == cal.as_str()));
                        (204, Vec::new(), "text/plain")
                    }
                    _ => not_found(),
                }
            }
            // `/me/events` reaches only this mailbox's own events, as Graph's does.
            ("PATCH", ["me", "events", id]) => {
                match self.events.iter_mut().find(|e| e["id"] == *id && !e["calendar"].as_str().unwrap_or("").starts_with("user:")) {
                    Some(e) => {
                        for (k, v) in input.as_object().cloned().unwrap_or_default() {
                            e[k.as_str()] = v;
                        }
                        e["changeKey"] = json!("ck2");
                        ok(e.clone())
                    }
                    None => not_found(),
                }
            }
            ("DELETE", ["me", "events", id]) => {
                self.events.retain(|e| e["id"] != *id || e["calendar"].as_str().unwrap_or("").starts_with("user:"));
                (204, Vec::new(), "text/plain")
            }
            // The organisation's rooms: the Places API (when the sign-in may), and the older
            // functions.
            ("GET", ["places", "microsoft.graph.roomList"]) if self.places_refused => (
                403,
                json!({"error": {"code": "ErrorAccessDenied", "message": "Access is denied. Check credentials and try again."}})
                    .to_string()
                    .into_bytes(),
                "application/json",
            ),
            ("GET", ["places", "microsoft.graph.roomList"]) => {
                let list: Vec<Value> =
                    self.room_lists.iter().map(|(name, email)| json!({"id": email, "displayName": name, "emailAddress": email})).collect();
                ok(json!({ "value": list }))
            }
            ("GET", ["places", list, "microsoft.graph.roomList", "rooms"]) => {
                let list = list.replace("%40", "@").to_ascii_lowercase();
                if self.places_refused || !self.room_lists.iter().any(|(_, e)| e.eq_ignore_ascii_case(&list)) {
                    return not_found();
                }
                let rooms: Vec<Value> = self
                    .rooms
                    .iter()
                    .filter(|(_, _, l)| l.eq_ignore_ascii_case(&list))
                    .map(|(name, email, _)| json!({"id": email, "displayName": name, "emailAddress": email, "capacity": 8}))
                    .collect();
                ok(json!({ "value": rooms }))
            }
            ("GET", ["me", "findRoomLists"]) => {
                let list: Vec<Value> = self.room_lists.iter().map(|(name, email)| json!({"name": name, "address": email})).collect();
                ok(json!({ "value": list }))
            }
            ("GET", ["me", rooms]) if rooms.starts_with("findRooms") => {
                let wanted = rooms.strip_prefix("findRooms(RoomList=").map(|w| crate::oauth::unpct(w).trim_matches(['\'', ')']).to_ascii_lowercase());
                let list: Vec<Value> = self
                    .rooms
                    .iter()
                    .filter(|(_, _, l)| wanted.as_deref().is_none_or(|w| l.eq_ignore_ascii_case(w)))
                    .map(|(name, email, _)| json!({"name": name, "address": email}))
                    .collect();
                ok(json!({ "value": list }))
            }
            ("POST", ["me", "calendar", "getSchedule"]) => {
                self.schedule_asked.push(input.clone());
                let at = |k: &str| input.get(k).and_then(|t| t.get("dateTime")).and_then(Value::as_str).and_then(quatrelle_jmap::utc_date);
                let (Some(from), Some(to)) = (at("startTime"), at("endTime")) else {
                    return (
                        400,
                        json!({"error": {"code": "ErrorInvalidRequest", "message": "the window is invalid"}}).to_string().into_bytes(),
                        "application/json",
                    );
                };
                let mut out = Vec::new();
                for email in input["schedules"].as_array().cloned().unwrap_or_default().iter().filter_map(Value::as_str) {
                    let email = email.trim().to_ascii_lowercase();
                    if self.no_freebusy.iter().any(|e| e.eq_ignore_ascii_case(&email)) {
                        out.push(json!({"scheduleId": email, "error": {"message": "No free/busy access.", "responseCode": "ErrorNoFreeBusyAccess"}}));
                        continue;
                    }
                    // The account's own calendars, or another's shared here, between the dates.
                    let own = email == "riley@contoso.example";
                    let theirs = format!("user:{email}");
                    let when =
                        |e: &Value, k: &str| e.get(k).and_then(|t| t.get("dateTime")).and_then(Value::as_str).and_then(quatrelle_jmap::utc_date);
                    let mut items: Vec<Value> = self
                        .events
                        .iter()
                        .filter(|e| {
                            let cal = e["calendar"].as_str().unwrap_or("");
                            if own { self.calendars.iter().any(|(c, _)| c == cal) } else { cal == theirs }
                        })
                        .filter(|e| when(e, "end").is_some_and(|x| x > from) && when(e, "start").is_some_and(|x| x < to))
                        .map(|e| json!({"status": e.get("showAs").and_then(Value::as_str).unwrap_or("busy"), "start": e["start"], "end": e["end"], "subject": e["subject"]}))
                        .collect();
                    items.extend(self.busy.iter().filter(|(e, ..)| e.eq_ignore_ascii_case(&email)).map(|(_, s, e, status, subject)| {
                        json!({"status": status, "start": {"dateTime": s, "timeZone": "UTC"}, "end": {"dateTime": e, "timeZone": "UTC"}, "subject": subject})
                    }));
                    out.push(json!({"scheduleId": email, "availabilityView": "", "scheduleItems": items}));
                }
                ok(json!({ "value": out }))
            }
            ("GET", ["me", "contacts"]) => ok(json!({ "value": self.contacts })),
            ("POST", ["me", "contacts"]) => {
                let id = self.id("CT");
                let mut c = input.clone();
                c["id"] = json!(id);
                c["changeKey"] = json!("ck1");
                self.contacts.push(c.clone());
                (201, c.to_string().into_bytes(), "application/json")
            }
            ("DELETE", ["me", "contacts", id]) => {
                self.contacts.retain(|c| c["id"] != *id);
                (204, Vec::new(), "text/plain")
            }
            ("GET", ["me", "contacts", id, "photo", "$value"]) => {
                if self.contact_photos.iter().any(|p| p == id) {
                    // A JPEG marker: enough to be read as a picture.
                    (200, vec![0xFF, 0xD8, 0xFF], "image/jpeg")
                } else {
                    not_found()
                }
            }
            _ => not_found(),
        }
    }

    /// A listing two items a page, with `@odata.nextLink` for the rest.
    fn page(&self, list: Vec<Value>, path: &str, query: &str, base: &str) -> (u16, Vec<u8>, &'static str) {
        let skip: usize = query.split('&').find_map(|kv| kv.strip_prefix("$skip=")).and_then(|s| s.parse().ok()).unwrap_or(0);
        let page: Vec<Value> = list.iter().skip(skip).take(2).cloned().collect();
        let mut v = json!({ "value": page });
        if skip + 2 < list.len() {
            let rest: String = query.split('&').filter(|kv| !kv.starts_with("$skip=")).collect::<Vec<_>>().join("&");
            v["@odata.nextLink"] = json!(format!("{base}{path}?{rest}&$skip={}", skip + 2));
        }
        (200, v.to_string().into_bytes(), "application/json")
    }
}
