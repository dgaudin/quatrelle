// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! 7. A file sent and opened, in the chat the user named: one small file goes up to their
//! OneDrive, is shared with the others of the chat, posted in a message, read back as a file,
//! downloaded as someone else's Teams would, and removed.
//!
//! Each step says one line: yes, or no with the HTTP status and Graph's error code, or `not
//! tried` when an earlier step failed; never a name, an address or an id. The upload's and the
//! download's addresses carry their own authorisation: they are never said, and the host only by
//! its last two labels ([`host_tail`]). What was made (the file, the message) is deleted whatever
//! failed, and the last line says whether it was.

use quatrelle_model::pim::teams::{Attachment, SHARE_PEOPLE, is_object_id};

use super::*;
use crate::http::Url;
use crate::teams::graph::{DriveItem, FileRef, RangeAnswer, Recipient, UploadSession, address_ok, host_tail, message_of, put_range_on};

/// The file's name in OneDrive (its own drive name), and its 30 bytes.
const NAME: &str = "Quatrelle check.txt";
const CONTENT: &[u8] = b"Quatrelle check, please ignore";
/// The message's text, before the file.
const TEXT: &str = "Quatrelle check, please ignore (file)";
/// The most bytes of the file's download taken (it has 30; a hostile answer has more).
const DOWNLOAD_MAX: usize = 1 << 10;
/// How far before the message's time the read back looks (ms).
const READ_SLACK: i64 = 1000;

/// What the check made, to remove.
#[derive(Default)]
struct Made {
    /// The file's item id.
    item: Option<String>,
    /// The message's id.
    message: Option<String>,
    /// A message may have been posted whose id isn't known.
    lost: bool,
    /// A file may have been made whose item isn't known (the last range's answer was lost).
    lost_file: bool,
}

impl Check<'_> {
    /// The eight steps, then the clean-up.
    pub(super) fn files(&mut self, chat: &str) {
        let mut made = Made::default();
        let file = self.send_up(&mut made);
        let message = match &file {
            Some(item) => self.share_and_post(chat, item, &mut made),
            None => {
                self.not_tried(&["shared with the others:", "message with the file:"]);
                None
            }
        };
        match message {
            Some(m) => self.read_and_open(chat, &m),
            None => self.not_tried(&["listed as a file:", "download:"]),
        }
        self.pace();
        self.clean_up(chat, &made);
    }

    /// 1 to 3. The session, the 30 bytes in one range, and the file's item.
    fn send_up(&mut self, made: &mut Made) -> Option<DriveItem> {
        let session = match self.client.upload_session(NAME) {
            Ok(s) => {
                self.say(format!("upload session: yes (host: {})", host_tail(&s.url().host)));
                s
            }
            Err(e) => {
                self.said_no("upload session:", &e);
                self.not_tried(&["upload:", "item:"]);
                return None;
            }
        };
        let Some(id) = self.upload(&session, made) else {
            self.not_tried(&["item:"]);
            return None;
        };
        made.item = Some(id.clone());
        match self.client.drive_item(&id) {
            Ok(item) => {
                let on = self.on_their_drive(&item);
                self.say(format!("item: yes (attachment id: yes, on your OneDrive: {on})"));
                Some(item)
            }
            Err(e) => {
                self.said_no("item:", &e);
                None
            }
        }
    }

    /// 2. The bytes, on a connection of their own to the upload's server, without the token.
    fn upload(&mut self, session: &UploadSession, made: &mut Made) -> Option<String> {
        let total = u64::try_from(CONTENT.len()).unwrap_or(0);
        let sent =
            self.client.transfer_stream(session.url()).and_then(|mut s| put_range_on(&mut s, session.url(), CONTENT, 0, total).map_err(|f| f.0));
        match sent {
            Ok(RangeAnswer::Done(id)) => {
                self.say("upload: yes".into());
                Some(id)
            }
            Ok(RangeAnswer::Next(_)) => {
                self.say("upload: no (unexpected answer)".into());
                None
            }
            Err(e) => {
                made.lost_file = uncertain(&e);
                self.said_no("upload:", &e);
                None
            }
        }
    }

    /// Whether the item is on the user's own OneDrive: `yes`, `no`, or `not known`.
    fn on_their_drive(&self, item: &DriveItem) -> &'static str {
        let Ok(Some(mine)) = self.client.drive_host() else { return "not known" };
        if Url::parse(&item.web_dav_url).is_ok_and(|u| u.host.eq_ignore_ascii_case(&mine)) { "yes" } else { "no" }
    }

    /// 4 and 5. The share with the others of the chat, then the message with the file: the
    /// message made (its id and time), whether or not the share worked.
    fn share_and_post(&mut self, chat: &str, item: &DriveItem, made: &mut Made) -> Option<(String, Option<i64>)> {
        let shared = self.share(chat, &item.id);
        self.say(shared);
        self.pace();
        let html = format!("<p>{TEXT}</p><attachment id=\"{}\"></attachment>", item.guid);
        let file = FileRef { guid: &item.guid, url: &item.web_dav_url, name: &item.name };
        match self.client.send_parts(chat, &html, &[], &[], &[file]) {
            Ok(m) => {
                let Some(id) = id_of(&m) else {
                    made.lost = true;
                    self.say("message with the file: no (unexpected answer)".into());
                    self.not_tried(&["listed as a file:", "download:"]);
                    return None;
                };
                self.say("message with the file: yes".into());
                made.message = Some(id.clone());
                let at = ["lastModifiedDateTime", "createdDateTime"].iter().find_map(|k| m.get(*k).and_then(Value::as_str).and_then(ms_of));
                Some((id, at))
            }
            Err(e) => {
                made.lost = uncertain(&e);
                self.said_no("message with the file:", &e);
                None
            }
        }
    }

    /// The line for the share (step 4): the file given to the chat's members but the user, by
    /// object id when they have one, else by address.
    fn share(&mut self, chat: &str, item: &str) -> String {
        const SAID: &str = "shared with the others:";
        let members = match self.client.members_all(chat) {
            Ok(m) => m,
            Err(e) => return format!("{SAID} no ({})", said(&e)),
        };
        let others: Vec<&Value> = members.iter().filter(|m| m.get("userId").and_then(Value::as_str) != Some(self.me.id.as_str())).collect();
        let text = |m: &Value, key: &str| m.get(key).and_then(Value::as_str).map(str::to_string);
        let named: Vec<(Option<String>, Option<String>)> = others.iter().map(|m| (text(m, "userId"), text(m, "email"))).collect();
        let to: Vec<Recipient<'_>> = named
            .iter()
            .filter_map(|(id, mail)| match (id, mail) {
                (Some(id), _) if is_object_id(id) => Some(Recipient::Id(id)),
                (_, Some(mail)) if address_ok(mail) => Some(Recipient::Address(mail)),
                _ => None,
            })
            .take(SHARE_PEOPLE)
            .collect();
        if to.is_empty() {
            return format!("{SAID} 0 of {}", others.len());
        }
        match self.client.invite(item, &to) {
            Ok(s) => format!("{SAID} {} of {}", s.granted, others.len()),
            Err(e) => format!("{SAID} no ({})", said(&e)),
        }
    }

    /// 6 and 7. The message read back (it lists the file as one), then the file's bytes fetched
    /// the way the others' Teams would: its address from Graph with the token, the bytes from
    /// the file host without it.
    fn read_and_open(&mut self, chat: &str, (id, at): &(String, Option<i64>)) {
        self.pace();
        let url = match self.listed(chat, id, *at) {
            Ok(url) => {
                self.say("listed as a file: yes".into());
                url
            }
            Err(said) => {
                self.say(format!("listed as a file: no ({said})"));
                self.not_tried(&["download:"]);
                return;
            }
        };
        let line = match self.client.shared_location(&url) {
            Err(e) => format!("download: no ({})", said(&e)),
            Ok(to) => match self.client.download_at(&to, DOWNLOAD_MAX) {
                Ok(n) if n == CONTENT.len() => format!("download: yes (host: {})", host_tail(&to.host)),
                Ok(n) => format!("download: no ({n} bytes came, {} were sent)", CONTENT.len()),
                Err(e) => format!("download: no ({})", said(&e)),
            },
        };
        self.say(line);
    }

    /// The file's address, from the message read back from the chat: `Err` says why it isn't there.
    fn listed(&mut self, chat: &str, id: &str, at: Option<i64>) -> std::result::Result<String, String> {
        let from = at.or_else(|| self.now().map(|n| n.saturating_sub(60_000))).ok_or("no time")?;
        let after = rfc3339(from.saturating_sub(READ_SLACK));
        let mut next: Option<String> = None;
        for _ in 0..PAGES {
            let page = self.client.messages(chat, Range::After(&after), next.as_deref()).map_err(|e| said(&e))?;
            if let Some(m) = page.items.iter().find(|m| id_of(m).as_deref() == Some(id)) {
                let read = message_of(chat, m);
                return read
                    .attachments
                    .into_iter()
                    .find_map(|a| match a {
                        Attachment::File { url, .. } => Some(url),
                        _ => None,
                    })
                    .ok_or_else(|| "not as a file".to_string());
            }
            match page.next {
                Some(n) => next = Some(n),
                None => break,
            }
        }
        Err("message not found".into())
    }

    /// 8. The message soft-deleted and the file deleted, whatever failed before.
    fn clean_up(&mut self, chat: &str, made: &Made) {
        let mut left: Vec<&str> = Vec::new();
        if let Some(id) = &made.message
            && !self.delete(chat, id)
        {
            left.push("the message is left");
        }
        if made.lost {
            left.push("a message may be left");
        }
        if made.lost_file {
            left.push("a file may be left");
        }
        if let Some(id) = &made.item
            && self.client.delete_item(id).is_err()
        {
            left.push("the file is left");
        }
        if left.is_empty() {
            self.say("cleaned up: yes".into());
        } else {
            self.say(format!("cleaned up: no ({})", left.join(", ")));
        }
    }

    /// A step that failed: `no (HTTP <status> <code>)`, or what kind of failure.
    fn said_no(&mut self, step: &str, e: &NetError) {
        self.say(format!("{step} no ({})", said(e)));
    }
}

/// An error said without its text: `HTTP 403 Forbidden`, or the kind of failure.
fn said(e: &NetError) -> String {
    let w = why(e);
    if w.starts_with(|c: char| c.is_ascii_digit()) { format!("HTTP {w}") } else { w }
}
