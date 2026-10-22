//! Graph: the mailbox settings — automatic replies, and the Inbox rules Microsoft 365 runs.

use super::*;

impl Client {
    /// The automatic replies.
    pub fn vacation(&self) -> Result<quatrelle_model::job::Vacation> {
        let v = self.get("/me/mailboxSettings/automaticRepliesSetting")?;
        let status = v.get("status").and_then(Value::as_str).unwrap_or("disabled").to_ascii_lowercase();
        let day = |k: &str| {
            v.get(k).and_then(|d| d.get("dateTime")).and_then(Value::as_str).and_then(|d| d.get(..10)).filter(|d| quatrelle_model::job::is_day(d))
        };
        let scheduled = status == "scheduled";
        let reply = |k: &str| {
            let html = v.get(k).and_then(Value::as_str).unwrap_or_default();
            if html.contains('<') { mime::html_to_text(html).trim().to_string() } else { html.trim().to_string() }
        };
        let (text, external) = (reply("internalReplyMessage"), reply("externalReplyMessage"));
        let outside = match v.get("externalAudience").and_then(Value::as_str).unwrap_or("all").to_ascii_lowercase().as_str() {
            "none" => "none",
            "contactsonly" => "known",
            _ => "",
        };
        Ok(quatrelle_model::job::Vacation {
            enabled: status != "disabled",
            subject: String::new(),
            external: (!external.is_empty() && external != text).then_some(external),
            outside: outside.to_string(),
            text,
            from: day("scheduledStartDateTime").filter(|_| scheduled).map(str::to_string),
            until: day("scheduledEndDateTime").filter(|_| scheduled).map(str::to_string),
        })
    }

    pub fn set_vacation(&self, v: &quatrelle_model::job::Vacation) -> Result<()> {
        let html = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\n', "<br>");
        let dated = v.from.is_some() || v.until.is_some();
        let audience = match v.outside.as_str() {
            "none" => "none",
            "known" => "contactsOnly",
            _ => "all",
        };
        let mut s = json!({
            "status": if !v.enabled { "disabled" } else if dated { "scheduled" } else { "alwaysEnabled" },
            "externalAudience": audience,
            "internalReplyMessage": html(&v.text),
            "externalReplyMessage": html(v.external_text()),
        });
        if v.enabled && dated {
            // Whole days: from the start of the first to the end of the last (open ends far off).
            let from = v.from.clone().unwrap_or_else(|| "2000-01-01".into());
            let until = v.until.clone().unwrap_or_else(|| "2099-12-31".into());
            s["scheduledStartDateTime"] = json!({"dateTime": format!("{from}T00:00:00"), "timeZone": "UTC"});
            s["scheduledEndDateTime"] = json!({"dateTime": format!("{until}T23:59:59"), "timeZone": "UTC"});
        }
        let body = serde_json::to_vec(&json!({ "automaticRepliesSetting": s })).map_err(|e| NetError::Protocol(e.to_string()))?;
        self.send_raw("PATCH", "/me/mailboxSettings", "application/json", body).map(|_| ())
    }

    /// The ids of the Inbox rules Graph keeps, by name.
    pub fn message_rule_ids(&self) -> Result<Vec<(String, String)>> {
        let v = self.get("/me/mailFolders/inbox/messageRules?$select=id,displayName&$top=500")?;
        Ok(v.get("value")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(|r| Some((r.get("displayName")?.as_str()?.to_string(), r.get("id")?.as_str()?.to_string()))).collect())
            .unwrap_or_default())
    }

    /// The Inbox's rules, as Graph keeps them (made in Outlook on the web, or here).
    pub fn message_rules(&self) -> Result<Vec<quatrelle_model::organize::ForeignRule>> {
        let v = self.get("/me/mailFolders/inbox/messageRules?$select=id,displayName,isEnabled&$top=500")?;
        Ok(v.get("value")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|r| {
                        Some(quatrelle_model::organize::ForeignRule {
                            account: String::new(),
                            id: r.get("id")?.as_str()?.to_string(),
                            name: r.get("displayName").and_then(Value::as_str).unwrap_or("").to_string(),
                            enabled: r.get("isEnabled").and_then(Value::as_bool).unwrap_or(true),
                            runnable: false,
                            rule: None,
                            summary: "a rule Microsoft 365 runs".into(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Turn one of the Inbox's rules on or off.
    pub fn set_rule_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let body = serde_json::to_vec(&json!({ "isEnabled": enabled })).map_err(|e| NetError::Protocol(e.to_string()))?;
        self.send_raw("PATCH", &format!("/me/mailFolders/inbox/messageRules/{}", seg(id)), "application/json", body).map(drop)
    }

    /// Remove one of the Inbox's rules.
    pub fn delete_rule(&self, id: &str) -> Result<()> {
        self.send_raw("DELETE", &format!("/me/mailFolders/inbox/messageRules/{}", seg(id)), "application/json", Vec::new()).map(drop)
    }

    /// Create, change and remove the Inbox rules Graph runs, from Quatrelle's rules, as
    /// [`ews::Client::set_inbox_rules`](crate::ews::Client::set_inbox_rules) does.
    pub fn set_message_rules(&self, wanted: &[(Rule, Option<String>)], delete: &[String]) -> Result<Vec<(String, String)>> {
        if wanted.is_empty() && delete.is_empty() {
            return Ok(Vec::new());
        }
        let before: Vec<String> = self.message_rule_ids()?.into_iter().map(|(_, id)| id).collect();
        for (i, (r, folder)) in wanted.iter().enumerate() {
            let body = message_rule_json(r, folder.as_deref(), i + 1);
            match r.remote_id.as_deref().filter(|id| !id.is_empty()) {
                Some(id) => {
                    let path = format!("/me/mailFolders/inbox/messageRules/{}", seg(id));
                    self.send_raw("PATCH", &path, "application/json", serde_json::to_vec(&body).map_err(|e| NetError::Protocol(e.to_string()))?)?;
                }
                None => {
                    self.post("/me/mailFolders/inbox/messageRules", &body)?;
                }
            }
        }
        for id in delete.iter().filter(|id| !id.is_empty()) {
            let path = format!("/me/mailFolders/inbox/messageRules/{}", seg(id));
            self.send_raw("DELETE", &path, "application/json", Vec::new())?;
        }
        let after = self.message_rule_ids()?;
        let mut out = Vec::new();
        for (r, _) in wanted {
            let id = match r.remote_id.as_deref().filter(|id| !id.is_empty()) {
                Some(id) => Some(id.to_string()),
                None => after.iter().find(|(name, id)| name == &r.name && !before.contains(id)).map(|(_, id)| id.clone()),
            };
            match id {
                Some(id) => out.push((r.name.clone(), id)),
                None => log::warn!("the rule {} was sent to Graph but isn't listed there", r.name),
            }
        }
        Ok(out)
    }
}

/// A rule as Graph's `messageRules` hold it. Only the parts Exchange can express are written;
/// the engine never sends the others (`Rule::server_blockers`).
fn message_rule_json(r: &Rule, folder: Option<&str>, sequence: usize) -> Value {
    let mut conditions = serde_json::Map::new();
    let mut subjects: Vec<String> = Vec::new();
    let mut senders: Vec<String> = Vec::new();
    let mut recipients: Vec<String> = Vec::new();
    for c in &r.conditions {
        match c {
            Condition::Subject(t) => subjects.push(t.clone()),
            Condition::From(t) => senders.push(t.clone()),
            Condition::To(t) => recipients.push(t.clone()),
            Condition::SubjectAny(v) => subjects.extend(v.iter().cloned()),
            Condition::FromAny(v) => senders.extend(v.iter().cloned()),
            Condition::ToAny(v) => recipients.extend(v.iter().cloned()),
            Condition::HasAttachment => {
                conditions.insert("hasAttachments".into(), Value::Bool(true));
            }
            Condition::AnyText(_) | Condition::Bulk => {}
        }
    }
    if !subjects.is_empty() {
        conditions.insert("subjectContains".into(), json!(subjects));
    }
    if !senders.is_empty() {
        conditions.insert("senderContains".into(), json!(senders));
    }
    if !recipients.is_empty() {
        conditions.insert("recipientContains".into(), json!(recipients));
    }
    let mut actions = serde_json::Map::new();
    for a in &r.actions {
        match a {
            RuleAction::MoveTo(_) | RuleAction::MoveToRole(_) => {
                if let Some(folder) = folder {
                    actions.insert("moveToFolder".into(), Value::String(folder.to_string()));
                }
            }
            RuleAction::MarkRead => {
                actions.insert("markAsRead".into(), Value::Bool(true));
            }
            RuleAction::Categorize(c) => {
                let list = actions.entry("assignCategories").or_insert_with(|| json!([]));
                if let Some(list) = list.as_array_mut() {
                    list.push(Value::String(c.clone()));
                }
            }
            RuleAction::Delete => {
                actions.insert("delete".into(), Value::Bool(true));
            }
            RuleAction::Flag | RuleAction::Pin => {}
        }
    }
    if r.stop {
        actions.insert("stopProcessingRules".into(), Value::Bool(true));
    }
    json!({
        "displayName": r.name,
        "sequence": sequence,
        "isEnabled": r.enabled,
        "conditions": Value::Object(conditions),
        "actions": Value::Object(actions),
    })
}
