//! The fake EWS server telling of changes as they happen (streaming notifications): `Subscribe`,
//! and `GetStreamingEvents` answered with a response kept open, a notification sent whenever a
//! message the subscription covers comes, goes, moves or changes, or a folder is made or deleted.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use super::server::respond;
use super::*;

/// The server the subscriptions live on, as it names it to the client.
pub const BACKEND: &str = "X-BackEndOverrideCookie=BE1~mbx";

/// What a subscription sees: each message's folder and marks, and the folders with their parent.
pub(super) type Seen = (BTreeMap<String, (String, bool, bool)>, BTreeSet<(String, String)>);

impl Data {
    /// `Subscribe`'s response message: refused as by an older server unless `streaming`.
    pub(super) fn subscribe(&mut self, op: &El) -> String {
        if !self.streaming {
            return "<m:R ResponseClass=\"Error\"><m:MessageText>The request is invalid.</m:MessageText><m:ResponseCode>ErrorInvalidRequest</m:ResponseCode></m:R>".into();
        }
        let req = op.find("StreamingSubscriptionRequest");
        let all = req.and_then(|r| r.attr("SubscribeToAllFolders")) == Some("true");
        let ids: Vec<String> = req
            .and_then(|r| r.child("FolderIds"))
            .map(|f| f.kids.iter().filter_map(|k| k.attr("Id").map(str::to_string)).collect())
            .unwrap_or_default();
        let id = self.id("SUB");
        let scope = if all { None } else { Some(ids) };
        let seen = self.seen(&scope);
        self.told.insert(id.clone(), seen);
        self.subscriptions.push((id.clone(), scope));
        format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:SubscriptionId>{id}</m:SubscriptionId></m:R>")
    }

    /// Whether a subscription covers a folder: all the account's own, or the folders it names.
    fn covers(&self, scope: &Option<Vec<String>>, folder: &str) -> bool {
        match scope {
            None => self.folders.iter().any(|f| f.id == folder && f.owner.is_empty()),
            Some(ids) => ids.iter().any(|i| i == folder),
        }
    }

    fn seen(&self, scope: &Option<Vec<String>>) -> Seen {
        // Mail, and the appointments and contacts of the folders it covers.
        let mail = self.items.iter().filter(|i| self.covers(scope, &i.folder)).map(|i| (i.id.clone(), (i.folder.clone(), i.read, i.flagged)));
        let pim = self.pim.iter().filter(|(_, f, _)| self.covers(scope, f)).map(|(id, f, _)| (id.clone(), (f.clone(), false, false)));
        let items = mail.chain(pim).collect();
        let folders = self.folders.iter().filter(|f| self.covers(scope, &f.id)).map(|f| (f.id.clone(), f.parent.clone())).collect();
        (items, folders)
    }
}

/// One envelope of the answer.
fn envelope(class: &str, inner: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body>\
         <m:GetStreamingEventsResponse xmlns:m=\"m\" xmlns:t=\"t\"><m:ResponseMessages><m:GetStreamingEventsResponseMessage ResponseClass=\"{class}\">\
         {inner}</m:GetStreamingEventsResponseMessage></m:ResponseMessages></m:GetStreamingEventsResponse></s:Body></s:Envelope>"
    )
}

fn status(s: &str, notifications: &str) -> String {
    envelope("Success", &format!("<m:ResponseCode>NoError</m:ResponseCode><m:ConnectionStatus>{s}</m:ConnectionStatus>{notifications}"))
}

fn gone() -> String {
    envelope("Error", "<m:MessageText>The subscription was not found.</m:MessageText><m:ResponseCode>ErrorSubscriptionNotFound</m:ResponseCode>")
}

/// The notifications for what changed between two looks.
fn changes(before: &Seen, now: &Seen) -> String {
    let item = |kind: &str, id: &str, folder: &str, old: Option<&str>| {
        let old = old.map(|o| format!("<t:OldItemId Id=\"{id}\"/><t:OldParentFolderId Id=\"{o}\"/>")).unwrap_or_default();
        format!("<t:{kind}><t:TimeStamp>2026-10-24T08:00:00Z</t:TimeStamp><t:ItemId Id=\"{id}\"/><t:ParentFolderId Id=\"{folder}\"/>{old}</t:{kind}>")
    };
    let folder = |kind: &str, id: &str, parent: &str| format!("<t:{kind}><t:FolderId Id=\"{id}\"/><t:ParentFolderId Id=\"{parent}\"/></t:{kind}>");
    let mut out = String::new();
    for (id, (f, read, flagged)) in &now.0 {
        match before.0.get(id) {
            None => out.push_str(&item("NewMailEvent", id, f, None)),
            Some((was, ..)) if was != f => out.push_str(&item("MovedEvent", id, f, Some(was))),
            Some((_, r, fl)) if r != read || fl != flagged => out.push_str(&item("ModifiedEvent", id, f, None)),
            Some(_) => {}
        }
    }
    for (id, (f, ..)) in &before.0 {
        if !now.0.contains_key(id) {
            out.push_str(&item("DeletedEvent", id, f, None));
        }
    }
    for (id, parent) in now.1.difference(&before.1) {
        out.push_str(&folder("CreatedEvent", id, parent));
    }
    for (id, parent) in before.1.difference(&now.1) {
        out.push_str(&folder("DeletedEvent", id, parent));
    }
    out
}

/// `GetStreamingEvents`: the answer kept open for `stream_for`, what changed looked at every few
/// milliseconds and told of, then the server's goodbye. A subscription it doesn't have (any
/// more) is answered so.
pub(super) fn stream_events(s: &mut TcpStream, shared: &Arc<Mutex<Data>>, op: &El, cookie: &str) {
    let sub = op.find("SubscriptionId").map(|e| e.text.trim().to_string()).unwrap_or_default();
    let found = {
        let mut d = shared.lock().unwrap();
        d.calls.push("GetStreamingEvents".into());
        d.stream_cookies.push(cookie.to_string());
        let scope = d.subscriptions.iter().find(|(id, _)| *id == sub).map(|(_, scope)| scope.clone());
        scope.map(|scope| (d.told.get(&sub).cloned().unwrap_or_else(|| d.seen(&scope)), scope, d.stream_for))
    };
    let Some((mut before, scope, stream_for)) = found else {
        respond(s, 200, &[], gone().as_bytes());
        return;
    };
    let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/xml; charset=utf-8\r\nTransfer-Encoding: chunked\r\n\r\n");
    let chunk = |s: &mut TcpStream, text: &str| s.write_all(format!("{:x}\r\n{text}\r\n", text.len()).as_bytes()).is_ok();
    if !chunk(s, &status("OK", "")) {
        return;
    }
    let end = Instant::now() + stream_for;
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
        let (now, still) = {
            let d = shared.lock().unwrap();
            (d.seen(&scope), d.subscriptions.iter().any(|(id, _)| *id == sub))
        };
        if !still {
            let _ = chunk(s, &gone());
            let _ = s.write_all(b"0\r\n\r\n");
            return;
        }
        let events = changes(&before, &now);
        if !events.is_empty() {
            let told =
                format!("<m:Notifications><m:Notification><t:SubscriptionId>{sub}</t:SubscriptionId>{events}</m:Notification></m:Notifications>");
            if !chunk(s, &status("OK", &told)) {
                return;
            }
        }
        shared.lock().unwrap().told.insert(sub.clone(), now.clone());
        before = now;
    }
    let _ = chunk(s, &status("Closed", ""));
    let _ = s.write_all(b"0\r\n\r\n");
}
