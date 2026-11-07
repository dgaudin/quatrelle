// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Graph: the mail an Exchange Online mailbox keeps of what was deleted for good (its
//! `recoverableitemsdeletions` folder): listed a page at a time, moved back into Deleted Items or
//! purged (`permanentDelete`). Graph shows mail only: appointments, contacts and tasks kept there
//! are not listed. Read on the server each time, never kept.

use quatrelle_model::{Recoverable, RecoverableKind};

use super::*;

/// The most characters of a subject or sender kept from what the server says.
const TEXT_MAX: usize = 300;

/// The size property, `PidTagMessageSize`, asked for with the page.
const SIZE_ID: &str = "Integer 0xe08";

fn cut(text: &str) -> String {
    text.trim().chars().take(TEXT_MAX).collect()
}

/// The items of a page's `value` array: bounded, and whatever lacks an id is skipped.
pub(super) fn recoverable_items(list: &Value, max: usize) -> Vec<Recoverable> {
    let Some(array) = list.get("value").and_then(Value::as_array) else { return Vec::new() };
    let mut out = Vec::new();
    for m in array.iter().take(max) {
        let Some(id) = m.get("id").and_then(Value::as_str).filter(|i| !i.is_empty() && i.len() <= 2048) else { continue };
        let who = m.get("from").and_then(|f| f.get("emailAddress"));
        let named = |k: &str| who.and_then(|w| w.get(k)).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        let date = |k: &str| m.get(k).and_then(Value::as_str).and_then(jmap::utc_date);
        let size = m
            .get("singleValueExtendedProperties")
            .and_then(Value::as_array)
            .and_then(|p| p.iter().find(|p| p.get("id").and_then(Value::as_str).is_some_and(|i| i.eq_ignore_ascii_case(SIZE_ID))))
            .and_then(|p| p.get("value").and_then(Value::as_str))
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(0);
        out.push(Recoverable {
            id: id.to_string(),
            kind: RecoverableKind::Mail,
            subject: cut(m.get("subject").and_then(Value::as_str).unwrap_or("")),
            from: named("name").or_else(|| named("address")).map(cut).unwrap_or_default(),
            deleted: date("lastModifiedDateTime").or_else(|| date("receivedDateTime")).unwrap_or(0),
            size,
        });
    }
    out
}

impl Client {
    /// A page of the deleted mail, newest first from `offset`, and whether there is more.
    pub fn recoverable_page(&self, offset: usize, page: usize) -> Result<(Vec<Recoverable>, bool)> {
        let page = page.clamp(1, quatrelle_model::RECOVERABLE_PAGE);
        let v = self
            .get(&format!(
                "/me/mailFolders/recoverableitemsdeletions/messages?$top={page}&$skip={offset}&$orderby=lastModifiedDateTime%20desc\
                 &$select=id,subject,from,receivedDateTime,lastModifiedDateTime\
                 &$expand=singleValueExtendedProperties($filter=id%20eq%20%27{}%27)",
                SIZE_ID.replace(' ', "%20")
            ))
            .map_err(|e| match e {
                NetError::Gone(_) | NetError::No(_) => {
                    NetError::No("Microsoft 365 did not let Quatrelle list the recoverable items (the mailbox may not allow it)".into())
                }
                e => e,
            })?;
        let items = recoverable_items(&v, page);
        let more = v.get("@odata.nextLink").is_some() && !items.is_empty();
        Ok((items, more))
    }

    /// Move items back into Deleted Items, or with `purge` delete them for good
    /// (`permanentDelete`). The ids dealt with (one already gone is as good as done), and the
    /// error that stopped it, when one did.
    pub fn restore_recoverable(&self, ids: &[String], purge: bool) -> (Vec<String>, Option<NetError>) {
        let mut done = Vec::new();
        for id in ids {
            let r = if purge {
                self.send_raw("POST", &format!("/me/messages/{}/permanentDelete", seg(id)), "application/json", Vec::new())
            } else {
                self.post(&format!("/me/messages/{}/move", seg(id)), &json!({"destinationId": "deleteditems"}))
            };
            match r {
                Ok(_) | Err(NetError::Gone(_)) => done.push(id.clone()),
                Err(e) => return (done, Some(e)),
            }
        }
        (done, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::fake_graph::{Data, TOKEN, serve};

    /// A deleted message (`DELETE`) is listed, moved back into Deleted Items, or purged.
    #[test]
    fn deleted_mail_is_listed_restored_and_purged() {
        let mut d = Data::default();
        let inbox = d.folder("Inbox", None, Some("inbox"));
        let trash = d.folder("Deleted Items", None, Some("deleteditems"));
        let raw = |s: &str| {
            format!("From: Zoe Park <zoe@contoso.example>\r\nTo: me@contoso.example\r\nSubject: {s}\r\nMessage-ID: <{s}@x>\r\n\r\nHi.\r\n")
                .into_bytes()
        };
        let a = d.deliver(&inbox, &raw("Budget"), true, "2026-10-01T10:00:00Z");
        let b = d.deliver(&inbox, &raw("Plans"), true, "2026-10-02T10:00:00Z");
        let server = serve(d);
        let c = Client::connect(&server.base, TOKEN).unwrap();
        assert!(c.recoverable_page(0, 50).unwrap().0.is_empty());
        for id in [&a, &b] {
            c.send_raw("DELETE", &format!("/me/messages/{id}"), "application/json", Vec::new()).unwrap();
        }
        let (items, more) = c.recoverable_page(0, 50).unwrap();
        assert_eq!(items.iter().map(|i| i.subject.as_str()).collect::<Vec<_>>(), ["Plans", "Budget"]);
        assert!(!more && items[0].from == "Zoe Park" && items[0].deleted > 0);
        let (first, more) = c.recoverable_page(0, 1).unwrap();
        assert_eq!((first.len(), more), (1, true));
        let (done, failed) = c.restore_recoverable(&[items[1].id.clone()], false);
        assert!(failed.is_none() && done.len() == 1, "{failed:?}");
        assert!(server.data.lock().unwrap().messages.iter().any(|m| m.folder == trash));
        let (done, failed) = c.restore_recoverable(&[items[0].id.clone()], true);
        assert!(failed.is_none() && done.len() == 1, "{failed:?}");
        assert!(c.recoverable_page(0, 50).unwrap().0.is_empty());
    }

    #[test]
    fn a_hostile_page_is_bounded_and_never_panics() {
        let page = json!({"value": [
            {"id": "a", "subject": "é".repeat(5000), "from": {"emailAddress": {"name": 3}}, "lastModifiedDateTime": "nope"},
            {"subject": "no id"}, 7, {"id": ""},
            {"id": "b", "from": {"emailAddress": {"address": "x@y.example"}}, "singleValueExtendedProperties": [{"id": "Integer 0xe08", "value": "-1"}]}
        ]});
        let items = recoverable_items(&page, 10);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].subject.chars().count(), TEXT_MAX);
        assert_eq!(items[1].from, "x@y.example");
        assert_eq!(items[1].size, 0);
        assert!(recoverable_items(&json!("x"), 5).is_empty());
        assert_eq!(recoverable_items(&page, 1).len(), 1);
    }
}
