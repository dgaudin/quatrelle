// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The fake Graph server's OneDrive, as Graph v1.0 and OneDrive's upload sessions answer: a
//! session for a file of Teams' folder (`uploadSession`, 507 `quotaLimitReached` when the drive
//! is full); its ranges put, its status read and the session cancelled at its own address
//! (`upload`, which takes no token: one is answered 401); a file's item (`item`, its `webDavUrl`
//! only when `$select` names it), deleted (`deleteItem`) and shared (`invite`, 207 when a
//! recipient fails).
//!
//! The ranges follow OneDrive's rules: each the next expected (else 416), of one total, a multiple
//! of 320 KiB but the last, under 60 MiB (else 400); the last makes the file (201), a name taken
//! numbered as OneDrive does (`report 1.pdf`). A session expires after `expire_after` ranges; a
//! range mid-way can have its answer held back (`hold_range`) or its connection cut (`cut_range`).
//!
//! A file shared in a chat: its first hop (`shares`, with the token) redirects to its download
//! (`download`, without), which serves its bytes.

use super::*;

/// What OneDrive wants every range but the last to be a multiple of (320 KiB).
const RANGE_UNIT: u64 = 327_680;
/// The most bytes of one range.
const RANGE_MAX: u64 = 60 << 20;
/// The user's OneDrive when the fake has no `drive`.
const DRIVE: &str = "https://contoso-my.sharepoint.com/personal/me_contoso_example/Documents";
/// Teams' folder in it.
const FOLDER: &str = "Microsoft Teams Chat Files";

/// A file of the user's OneDrive.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeItem {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
    /// Its eTag as Graph writes it: `"{GUID},1"`.
    pub(crate) etag: String,
}

impl FakeItem {
    /// The GUID inside its eTag, lowercased.
    pub(super) fn guid(&self) -> String {
        self.etag.trim_matches('"').trim_start_matches('{').split('}').next().unwrap_or_default().to_ascii_lowercase()
    }
}

/// An upload session.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeSession {
    pub(crate) id: String,
    /// The name asked for.
    pub(crate) name: String,
    /// The request's path as it came: percent-encoded, without the API's `/v1.0`.
    pub(crate) path: String,
    /// The request's body.
    pub(crate) body: Value,
    /// The bytes received so far, and the total their ranges say.
    pub(crate) bytes: Vec<u8>,
    pub(crate) total: Option<u64>,
    /// The ranges it took.
    pub(crate) taken: u32,
    /// Finished, cancelled or expired.
    pub(crate) gone: bool,
}

/// The user's OneDrive, and what the tests make of it.
#[derive(Clone, Debug, Default)]
pub(crate) struct FakeDrive {
    pub(crate) files: Vec<FakeItem>,
    pub(crate) sessions: Vec<FakeSession>,
    /// Every range taken: (session id, first byte, last byte, total).
    pub(crate) ranges: Vec<(String, u64, u64, u64)>,
    /// Every share received, whatever its answer: (the item's id, the body).
    pub(crate) invites: Vec<(String, Value)>,
    /// The upload address a session gives, in place of the fake's own.
    pub(crate) upload_elsewhere: Option<String>,
    /// A session is gone (404) once it has taken this many ranges.
    pub(crate) expire_after: Option<u32>,
    /// The drive is full: a session is refused with 507 `quotaLimitReached`.
    pub(crate) full: bool,
    /// The recipients (object ids or addresses) a share fails for (207).
    pub(crate) fail_share: Vec<String>,
    /// Sharing is refused with 403.
    pub(crate) forbid_share: bool,
    /// The host the items' `webDavUrl`s are on, in place of the user's OneDrive's.
    pub(crate) item_elsewhere: Option<String>,
    /// Items are answered with an eTag without a GUID.
    pub(crate) no_guid: bool,
    /// The files made so far.
    pub(crate) made: u32,
    /// A shared file's first hop redirects there, in place of the fake's own download address.
    pub(crate) share_location: Option<String>,
    /// A download answers a redirect (to itself) in place of the file: a second hop.
    pub(crate) download_redirect: bool,
    /// A download's body stops after this many bytes, its connection left open, saying nothing.
    pub(crate) stall_after: Option<usize>,
    /// An upload's status says the next byte expected is the first, whatever came: going back.
    pub(crate) status_back: bool,
    /// Every range is refused with this status (Graph's code `accessDenied`).
    pub(crate) refuse_ranges: Option<u16>,
    /// The next this many ranges are answered 429 with this `Retry-After` (seconds).
    pub(crate) busy_ranges: Option<(u32, u64)>,
    /// The range put once this many were taken has its answer held back this long, once: an
    /// upload stalled mid-way.
    pub(crate) hold_range: Option<(usize, std::time::Duration)>,
    /// The range put once this many were taken is carried out, then its connection closed with
    /// no answer, once: an upload cut mid-way.
    pub(crate) cut_range: Option<usize>,
}

/// `bytes a-b/total`.
fn content_range(v: &str) -> Option<(u64, u64, u64)> {
    let (range, total) = v.trim().strip_prefix("bytes ")?.split_once('/')?;
    let (first, last) = range.split_once('-')?;
    Some((first.parse().ok()?, last.parse().ok()?, total.parse().ok()?))
}

/// `name`, or OneDrive's numbered rename of it when a file has it already (`report 1.pdf`).
fn free_name(files: &[FakeItem], name: &str) -> String {
    let taken = |n: &str| files.iter().any(|f| f.name.eq_ignore_ascii_case(n));
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    std::iter::once(name.to_string()).chain((1..).map(|i| format!("{stem} {i}{ext}"))).find(|n| !taken(n)).unwrap_or_default()
}

/// A session's answer while ranges are still expected: 202 to a range, 200 to its status.
fn expecting(status: u16, next: u64) -> Reply {
    Reply::json(status, &json!({"expirationDateTime": "2026-12-31T00:00:00.000Z", "nextExpectedRanges": [format!("{next}-")]}))
}

impl FakeTeams {
    /// A file's `webDavUrl`: in Teams' folder of the user's OneDrive (or on `item_elsewhere`).
    pub(super) fn web_dav_url(&self, item: &FakeItem) -> String {
        let drive = match &self.drive_files.item_elsewhere {
            Some(host) => format!("https://{host}/personal/me_contoso_example/Documents"),
            None => self.drive.clone().unwrap_or_else(|| DRIVE.to_string()),
        };
        format!("{drive}/{}/{}", crate::http::pct(FOLDER), crate::http::pct(&item.name))
    }

    /// The routes of the user's OneDrive but the uploads themselves: `None` for the others.
    pub(super) fn drive_route(
        &mut self,
        name: &str,
        p: &[&str],
        q: &HashMap<String, String>,
        raw_path: &str,
        input: &Value,
        base: &str,
    ) -> Option<Reply> {
        let not_found = || Reply::error(404, "itemNotFound", "The item wasn't found.");
        let at = p.get(3).and_then(|id| self.drive_files.files.iter().position(|f| f.id == *id));
        Some(match (name, at) {
            ("uploadSession", _) => self.upload_session(p, raw_path, input, base),
            ("item", Some(at)) => self.item(at, q),
            ("deleteItem", Some(at)) => {
                self.drive_files.files.remove(at);
                Reply::empty(204)
            }
            ("invite", Some(_)) => self.invite(input),
            ("item" | "deleteItem" | "invite", None) => not_found(),
            _ => return None,
        })
    }

    fn upload_session(&mut self, p: &[&str], raw_path: &str, input: &Value, base: &str) -> Reply {
        if self.drive_files.full {
            return Reply::error(507, "quotaLimitReached", "The drive is full.");
        }
        let name = match p {
            [_, _, _, folder, name, _] if *folder == FOLDER => name.strip_suffix(':').filter(|n| !n.is_empty()),
            _ => None,
        };
        let Some(name) = name else { return Reply::error(400, "invalidRequest", "Not a file of Teams' folder.") };
        let behaviour = input.pointer("/item/@microsoft.graph.conflictBehavior").and_then(Value::as_str);
        if !matches!(behaviour, None | Some("rename" | "replace" | "fail")) {
            return Reply::error(400, "invalidRequest", "Not a conflict behaviour.");
        }
        let id = format!("s{}", self.drive_files.sessions.len() + 1);
        let url = self.drive_files.upload_elsewhere.clone().unwrap_or_else(|| format!("{base}/upload/{id}"));
        let session = FakeSession { id, name: name.to_string(), path: raw_path.to_string(), body: input.clone(), ..FakeSession::default() };
        self.drive_files.sessions.push(session);
        Reply::json(200, &json!({"uploadUrl": url, "expirationDateTime": "2026-12-31T00:00:00.000Z"}))
    }

    /// A request at an upload session's own address (`/upload/{id}`): a range put, its status
    /// read, the session cancelled.
    pub(super) fn upload(&mut self, method: &str, p: &[&str], headers: &[(String, String)], body: &[u8]) -> Reply {
        let gone = || Reply::error(404, "itemNotFound", "The upload session is gone.");
        let expire_after = self.drive_files.expire_after;
        let Some(at) = p.get(1).and_then(|id| self.drive_files.sessions.iter().position(|s| s.id == *id)) else { return gone() };
        let Some(s) = self.drive_files.sessions.get_mut(at) else { return gone() };
        s.gone |= expire_after.is_some_and(|n| s.taken >= n);
        if s.gone {
            return gone();
        }
        match method {
            "DELETE" => {
                s.gone = true;
                Reply::empty(204)
            }
            "GET" => expecting(200, if self.drive_files.status_back { 0 } else { s.bytes.len() as u64 }),
            _ if self.drive_files.busy_ranges.is_some_and(|(n, _)| n > 0) => {
                let (n, wait) = self.drive_files.busy_ranges.unwrap_or_default();
                self.drive_files.busy_ranges = Some((n - 1, wait));
                Reply { retry_after: Some(wait), ..Reply::error(429, "activityLimitReached", "Busy, as the test asked.") }
            }
            _ => match self.drive_files.refuse_ranges {
                Some(status) => Reply::error(status, "accessDenied", "Refused, as the test asked."),
                None => {
                    self.mid_way();
                    self.put_range(at, headers, body)
                }
            },
        }
    }

    /// What `hold_range` and `cut_range` make of the range being put.
    fn mid_way(&mut self) {
        let taken = self.drive_files.ranges.len();
        if let Some((_, wait)) = self.drive_files.hold_range.take_if(|(n, _)| *n == taken) {
            self.holding = Some(wait);
        }
        if self.drive_files.cut_range.take_if(|n| *n == taken).is_some() {
            *self.cut_after.entry("upload".into()).or_default() += 1;
        }
    }

    /// A shared file's first hop (`shares`: a redirect to its download, or to `share_location`)
    /// and its download (`download`: its bytes, a redirect again, or a body that stalls); `None`
    /// for the other routes.
    pub(super) fn share_route(&self, name: &str, p: &[&str], base: &str) -> Option<Reply> {
        if !matches!(name, "shares" | "download") {
            return None;
        }
        let Some(bytes) = p.get(1).and_then(|id| self.shared_bytes(id)) else { return Some(Reply::error(404, "NotFound", "Not found.")) };
        let token = p.get(1).copied().unwrap_or_default();
        let to = |at: String| Reply { location: Some(at), ..Reply::bytes(302, Vec::new(), "text/plain") };
        let d = &self.drive_files;
        Some(match name {
            "shares" => to(d.share_location.clone().unwrap_or_else(|| format!("{base}/download/{token}"))),
            _ if d.download_redirect => to(format!("{base}/download/{token}")),
            _ => Reply { stall: d.stall_after, ..Reply::bytes(200, bytes, "application/octet-stream") },
        })
    }

    fn put_range(&mut self, at: usize, headers: &[(String, String)], body: &[u8]) -> Reply {
        let bad = |why: &str| Reply::error(400, "invalidRange", why);
        let range = headers.iter().find(|(k, _)| k == "content-range").and_then(|(_, v)| content_range(v));
        let Some((first, last, total)) = range else { return bad("A Content-Range is wanted.") };
        let len = body.len() as u64;
        if last < first || last - first + 1 != len || last >= total || len > RANGE_MAX {
            return bad("The range and its bytes don't agree.");
        }
        let Some(s) = self.drive_files.sessions.get_mut(at) else { return bad("No such session.") };
        if s.total.is_some_and(|t| t != total) {
            return bad("Another total.");
        }
        if first != s.bytes.len() as u64 {
            return Reply::error(416, "invalidRange", "Not the next expected range.");
        }
        let end = last + 1 == total;
        if !end && !len.is_multiple_of(RANGE_UNIT) {
            return bad("A range but the last is a multiple of 320 KiB.");
        }
        s.total = Some(total);
        s.bytes.extend_from_slice(body);
        s.taken += 1;
        self.drive_files.ranges.push((s.id.clone(), first, last, total));
        if !end {
            return expecting(202, last + 1);
        }
        s.gone = true;
        let (name, bytes) = (s.name.clone(), std::mem::take(&mut s.bytes));
        let d = &mut self.drive_files;
        d.made += 1;
        let n = d.made;
        let item = FakeItem {
            id: format!("01ITEM{n:04}"),
            name: free_name(&d.files, &name),
            bytes,
            etag: format!("\"{{5B8C7D0E-{n:04X}-4000-8000-{n:012X}}},1\""),
        };
        let reply = Reply::json(201, &json!({"id": item.id, "name": item.name, "size": item.bytes.len()}));
        d.files.push(item);
        reply
    }

    /// A file's item; `webDavUrl` only when `$select` names it, as Graph does.
    fn item(&self, at: usize, q: &HashMap<String, String>) -> Reply {
        let Some(f) = self.drive_files.files.get(at) else { return Reply::error(404, "itemNotFound", "The item wasn't found.") };
        let etag = if self.drive_files.no_guid { "\"{not-a-guid},1\"".to_string() } else { f.etag.clone() };
        let dav = self.web_dav_url(f);
        let full = json!({"id": f.id, "eTag": etag, "name": f.name, "size": f.bytes.len(), "webUrl": format!("{dav}?web=1"), "webDavUrl": dav});
        let selected: Vec<&str> =
            q.get("$select").map(|s| s.split(',').map(str::trim).collect()).unwrap_or_else(|| vec!["id", "eTag", "name", "size", "webUrl"]);
        let v: serde_json::Map<String, Value> = selected.iter().filter_map(|k| full.get(*k).map(|x| (k.to_string(), x.clone()))).collect();
        Reply::json(200, &Value::Object(v))
    }

    /// A share: 403 when refused; each recipient in `fail_share` listed with its error (207).
    fn invite(&self, input: &Value) -> Reply {
        if self.drive_files.forbid_share {
            return Reply::error(403, "Forbidden", "Sharing this item is not allowed.");
        }
        let roles = input.get("roles").and_then(Value::as_array).map(Vec::as_slice);
        let recipients = input.get("recipients").and_then(Value::as_array).filter(|r| !r.is_empty());
        let (Some(recipients), Some([role])) = (recipients, roles) else {
            return Reply::error(400, "invalidRequest", "Recipients and a role are wanted.");
        };
        if !matches!(role.as_str(), Some("read" | "write")) {
            return Reply::error(400, "invalidRequest", "Not a role.");
        }
        let mut failed = false;
        let value: Vec<Value> = recipients
            .iter()
            .map(|r| {
                let who = r.get("objectId").or_else(|| r.get("email")).and_then(Value::as_str).unwrap_or_default();
                if who.is_empty() || self.drive_files.fail_share.iter().any(|f| f == who) {
                    failed = true;
                    json!({"error": {"code": "notAllowed", "message": "This recipient can't be invited."}})
                } else {
                    json!({"id": format!("perm-{who}"), "roles": [role], "grantedToV2": {"user": {"id": who}}})
                }
            })
            .collect();
        Reply::json(if failed { 207 } else { 200 }, &json!({ "value": value }))
    }
}
