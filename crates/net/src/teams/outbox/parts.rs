// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! What goes with a message sent: its files and pasted pictures, taken with it, made ready, then
//! posted with it.
//!
//! * At the take, a file is looked at (a regular file of 1 byte to [`FILE_MAX`]) and kept by its
//!   path, size and modification time; a picture is read (a PNG or a JPEG by its first bytes, at
//!   most [`PICTURE_INLINE_MAX`]) and its bytes kept beside the record until the message goes. A
//!   file is taken only for a chat whose other members are known, at most [`SHARE_PEOPLE`], each
//!   of the organisation with an object id or with an address: nothing goes half shared, nor to
//!   no one. Else the message is "Not sent" at once.
//! * When the message's turn comes, its files are made ready one after the other, a request a
//!   step on [`Route::Drive`]: the file looked at again (as it was attached, else "Not sent"), an
//!   upload session asked for and handed to the upload lane, which sends the bytes without the
//!   token while the worker goes on with the rest; then the item OneDrive made, which must be on
//!   the user's own OneDrive; then the share with the chat's other members, to read, signed in,
//!   no mail sent. What OneDrive made is kept in the record at once: a retry, a restart or a
//!   check never uploads a file twice. Every file ready, the message is posted with its pictures
//!   inside it and its files by reference.
//! * A request answered 429 or 503 waits its `Retry-After`; one without an answer, or a server's
//!   error, is held back (`held.rs`) and made again, [`POSTS`] tries in all; a refusal (a 4xx, a
//!   507, an address that isn't Microsoft's) is "Not sent" with Teams' code. An upload's session
//!   gone is begun again, [`SESSIONS`] sessions in all.
//! * Nothing of an upload is kept on disk but what OneDrive made: after a restart, an upload
//!   under way begins again.
//!
//! What is hostile here: the files themselves (changed, gone, a folder, a device, too big, a
//! picture that isn't one), Graph's answers (an item elsewhere, a share half made, error codes)
//! and the lane's words. What it protects: the user's files (only as they were attached, shared
//! with the chat's members only, read-only), and the worker (never held by an upload). The log
//! says counts, sizes, statuses, keyed hashes and a host's last two labels: never a name, a path
//! or an address.

use std::io::Read;
use std::path::{Path, PathBuf};

use quatrelle_mime::teams_write_parts;
use quatrelle_model::pim::teams::{
    Attachment, FILE_MAX, Member, OutState, PARTS_MAX, PICTURE_INLINE_MAX, Part, SHARE_PEOPLE, is_internal, is_object_id,
};
use serde_json::Value;

use super::*;
use crate::http::Url;
use crate::teams::files::{Done, Source, UploadFail, code_of, modified_of};
use crate::teams::graph::{FileRef, Hosted, Recipient, address_ok, drive_name, host_tail, transfer_url};
use crate::teams::store::{PartRecord, Uploaded, path_ok};

/// The upload sessions a file's upload is begun in, in all, when OneDrive says them gone.
const SESSIONS: u32 = 2;

/// A message's files being made ready, in memory only: the part in hand, its bytes gone up so
/// far, its sessions gone, and the item OneDrive made of it until the record keeps it.
#[derive(Clone, Debug, Default)]
pub(super) struct Upload {
    part: usize,
    sent: u64,
    gone: u32,
    item: Option<String>,
}

/// Where a message with parts stands when its turn comes.
pub(super) enum Ready {
    /// Every file ready: it is posted.
    Post,
    /// A request went, or an upload runs, or it waits: nothing more this step.
    Busy,
    /// It is "Not sent", and said.
    Settled,
}

/// The `parts` of a message sent to `chat`, taken: each file looked at, each picture read (its
/// bytes apart, in order, for [`keep`]). Else why the message is "Not sent": a part that isn't a
/// file or is empty ([`Unsent::FileGone`]), a file past [`FILE_MAX`] ([`Unsent::FileTooBig`]), a
/// picture that isn't a PNG or a JPEG of at most [`PICTURE_INLINE_MAX`] or a path that can't be
/// one ([`Unsent::Refused`]), a file for a chat it can't be shared in ([`Unsent::NotShared`]).
pub(super) fn taken(w: &Worker, chat: &str, parts: Vec<Part>) -> Result<(Vec<PartRecord>, Vec<Vec<u8>>), Unsent> {
    if parts.len() > PARTS_MAX {
        return Err(Unsent::Refused);
    }
    let (mut records, mut pictures) = (Vec::new(), Vec::new());
    for part in parts {
        match part {
            Part::File { path } => records.push(file_at(path)?),
            Part::Picture { path } => {
                let (record, bytes) = picture_at(&path)?;
                records.push(record);
                pictures.push(bytes);
            }
        }
    }
    let members = w.copy.as_ref().and_then(|c| c.chat(chat)).map(|r| r.chat.members).unwrap_or_default();
    let files = records.iter().any(|p| matches!(p, PartRecord::File { .. }));
    if files && recipients(&members, &w.settings.object_id, &w.settings.tenant_id).is_none() {
        return Err(Unsent::NotShared);
    }
    Ok((records, pictures))
}

/// Whether a message's text and the places of its `parts` fit what Teams takes once written.
pub(super) fn fits(body: &ChatText, mentions: &[Mention], parts: &[Part]) -> bool {
    let pictures = parts.iter().filter(|p| matches!(p, Part::Picture { .. })).count();
    crate::teams::store::fits(body, mentions, pictures, parts.len().saturating_sub(pictures))
}

/// A file attached at `path`: a regular file of 1 byte to [`FILE_MAX`], kept as it is now, by its
/// name as OneDrive takes it.
fn file_at(path: String) -> Result<PartRecord, Unsent> {
    if !path_ok(&path) {
        return Err(Unsent::Refused);
    }
    let meta = std::fs::metadata(&path).ok().filter(std::fs::Metadata::is_file).ok_or(Unsent::FileGone)?;
    let size = match meta.len() {
        0 => return Err(Unsent::FileGone),
        n if n > FILE_MAX => return Err(Unsent::FileTooBig),
        n => n,
    };
    Ok(PartRecord::File { name: name_of(&path), size, modified: modified_of(&meta), up: None, path })
}

/// A picture pasted, put in the file at `path`: a regular file, a PNG or a JPEG by its first
/// bytes, of 1 byte to [`PICTURE_INLINE_MAX`] (never more read).
fn picture_at(path: &str) -> Result<(PartRecord, Vec<u8>), Unsent> {
    if !path_ok(path) {
        return Err(Unsent::Refused);
    }
    // Looked at before it is opened: a pipe or a device would hold the read.
    if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return Err(Unsent::FileGone);
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(PICTURE_INLINE_MAX.saturating_add(1)).read_to_end(&mut bytes)).map_err(|_| Unsent::FileGone)?;
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else {
        return Err(Unsent::Refused);
    };
    let len = u32::try_from(bytes.len()).ok().filter(|n| u64::from(*n) <= PICTURE_INLINE_MAX).ok_or(Unsent::Refused)?;
    Ok((PartRecord::Picture { name: name_of(path), mime: mime.into(), len }, bytes))
}

/// A part's name: its file's, as OneDrive takes it.
fn name_of(path: &str) -> String {
    drive_name(&Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

/// The pictures' bytes of `op`, kept once it is (any of a record once under its number gone
/// first). A failure lets the record go too: a message never goes without its pictures.
pub(super) fn keep(copy: &Copy, op: &OpRecord, pictures: Vec<Vec<u8>>) -> quatrelle_store::Result<()> {
    if pictures.is_empty() {
        return Ok(());
    }
    let places = op.parts.iter().enumerate().filter(|(_, p)| matches!(p, PartRecord::Picture { .. })).map(|(n, _)| n);
    let kept = copy.remove_pictures(op.seq).and_then(|()| places.zip(&pictures).try_for_each(|(n, bytes)| copy.put_picture(op.seq, n, bytes)));
    if kept.is_err() {
        let _ = copy.remove_op(op.seq);
    }
    kept
}

/// Who a file goes to: the chat's `members` but the user `me`, people of the organisation
/// `tenant` by their object id, the others by their address. `None` when one has neither, when
/// no other member is known, or past [`SHARE_PEOPLE`].
fn recipients<'a>(members: &'a [Member], me: &str, tenant: &str) -> Option<Vec<Recipient<'a>>> {
    let others: Vec<&Member> = members.iter().filter(|m| !m.user_id.eq_ignore_ascii_case(me)).collect();
    if me.is_empty() || others.is_empty() || others.len() > SHARE_PEOPLE {
        return None;
    }
    others
        .into_iter()
        .map(|m| match () {
            _ if is_internal(m, tenant) && is_object_id(&m.user_id) => Some(Recipient::Id(&m.user_id)),
            _ if address_ok(&m.address) => Some(Recipient::Address(&m.address)),
            _ => None,
        })
        .collect()
}

/// A message whose files aren't all ready (gone up and shared): a step for the first of them, a
/// request at most. Its upload running, or the limiter not letting it go: it waits. Else, by
/// where that file stands: its upload begun ([`begin`]), the item OneDrive made asked for
/// ([`described`]), or its share ([`share`]). Every file ready: [`Ready::Post`].
pub(super) fn ready(w: &mut Worker, head: &Head, op: &mut OpRecord, now: Instant) -> Ready {
    let unready = |p: &PartRecord| matches!(p, PartRecord::File { up, .. } if !up.as_ref().is_some_and(|u| u.shared));
    let Some(at) = op.parts.iter().position(unready) else {
        w.outbox.uploads.remove(&op.seq);
        return Ready::Post;
    };
    if w.transfers.uploading(op.seq) || !w.may(Route::Drive, now) {
        return Ready::Busy;
    }
    let upload = w.outbox.uploads.entry(op.seq).or_default();
    if upload.part != at {
        *upload = Upload { part: at, ..Upload::default() };
    }
    let item = upload.item.clone();
    let up = match op.parts.get(at) {
        Some(PartRecord::File { up, .. }) => up.clone(),
        _ => return Ready::Busy,
    };
    match (up, item) {
        (Some(up), _) => share(w, op, at, &up.item, now),
        (None, Some(item)) => described(w, op, at, &item, now),
        (None, None) => begin(w, head, op, at, now),
    }
}

/// The file of part `at` looked at again (as it was attached, else "Not sent": too big past
/// [`FILE_MAX`], else gone), an upload session asked for, and handed to the upload lane.
fn begin(w: &mut Worker, head: &Head, op: &mut OpRecord, at: usize, now: Instant) -> Ready {
    let Some(PartRecord::File { path, name, size, modified, .. }) = op.parts.get(at).cloned() else { return Ready::Busy };
    let hash = w.hashed(&head.chat);
    let now_is = std::fs::metadata(&path).ok().filter(std::fs::Metadata::is_file);
    if !now_is.as_ref().is_some_and(|m| m.len() == size && modified_of(m) == modified) {
        // Grown past the cap (a Retry keeps the size it had): too big, as the take says it.
        let why = if now_is.is_some_and(|m| m.len() > FILE_MAX) { Unsent::FileTooBig } else { Unsent::FileGone };
        log::warn!("teams: a file of a message to conversation {hash} is no longer as it was attached ({why:?}): not sent");
        return not_sent(w, op, why, None);
    }
    let session = match w.request(Route::Drive, now, |c| c.upload_session(&name)) {
        Ok(session) => session,
        Err(e) => return failed(w, op, &e, Unsent::Refused, now),
    };
    answered(w, op);
    log::info!("teams: a file of {size} bytes of a message to conversation {hash} goes up to {}", host_tail(&session.url().host));
    let (base, floor) = (w.graph.clone(), w.floor());
    if !w.transfers.upload(op.seq, at, session, Source::File { path: PathBuf::from(path), size, modified }, &base, floor) {
        log::error!("teams: no lane to send a file up: a message to conversation {hash} isn't sent");
        return not_sent(w, op, Unsent::Refused, None);
    }
    Ready::Busy
}

/// The item OneDrive made of part `at`'s file asked for (its eTag's GUID, its address, the name it
/// got) and kept in the record, when it is on the user's own OneDrive; else "Not sent".
fn described(w: &mut Worker, op: &mut OpRecord, at: usize, item: &str, now: Instant) -> Ready {
    let made = match w.request(Route::Drive, now, |c| c.drive_item(item)) {
        Ok(made) => made,
        Err(e) => return failed(w, op, &e, Unsent::Refused, now),
    };
    answered(w, op);
    if !own_drive(w, &made.web_dav_url) {
        log::warn!("teams: a file gone up for conversation {} isn't on the user's own OneDrive: not sent", w.hashed(&op.chat));
        return not_sent(w, op, Unsent::Refused, None);
    }
    if let Some(PartRecord::File { up, .. }) = op.parts.get_mut(at) {
        *up = Some(Uploaded { item: made.id, guid: made.guid, url: made.web_dav_url, name: made.name, shared: false });
    }
    kept(w, op, now)
}

/// Part `at`'s file, the item `item`, shared with the chat's other members as they are now: with
/// every one of them, or the message is "Not sent".
fn share(w: &mut Worker, op: &mut OpRecord, at: usize, item: &str, now: Instant) -> Ready {
    let hash = w.hashed(&op.chat);
    let members = w.copy.as_ref().and_then(|c| c.chat(&op.chat)).map(|r| r.chat.members).unwrap_or_default();
    let Some(to) = recipients(&members, &w.settings.object_id, &w.settings.tenant_id) else {
        log::warn!("teams: a file for conversation {hash} can't be shared with everyone in it: not sent");
        return not_sent(w, op, Unsent::NotShared, None);
    };
    let shared = match w.request(Route::Drive, now, |c| c.invite(item, &to)) {
        Ok(shared) => shared,
        Err(e) => return failed(w, op, &e, Unsent::NotShared, now),
    };
    answered(w, op);
    if shared.failed > 0 {
        log::warn!("teams: a file for conversation {hash} was shared with {} of {}: not sent", shared.granted, to.len());
        return not_sent(w, op, Unsent::NotShared, None);
    }
    log::info!("teams: a file for conversation {hash} shared with {} people", to.len());
    if let Some(PartRecord::File { up: Some(up), .. }) = op.parts.get_mut(at) {
        up.shared = true;
    }
    kept(w, op, now)
}

/// Whether a file's address is on the user's own OneDrive: its host, when known (read at the
/// sign-in), else one of Microsoft's file hosts ([`transfer_url`]).
fn own_drive(w: &Worker, url: &str) -> bool {
    match &w.settings.drive_host {
        Some(host) => Url::parse(url).is_ok_and(|u| u.https && u.port == 443 && u.host.eq_ignore_ascii_case(host)),
        None => transfer_url(&w.graph, url).is_ok(),
    }
}

/// A request of a file's was answered: its holds forgotten and its tries counted from none, in the
/// copy too.
fn answered(w: &mut Worker, op: &mut OpRecord) {
    w.outbox.held.forget(op.seq);
    if op.tries > 0 {
        op.tries = 0;
        if let Some(Err(e)) = w.copy.as_ref().map(|c| c.put_op(op)) {
            log::error!("teams: a message's state couldn't be kept: {}", kind(&NetError::Store(e)));
        }
    }
}

/// What a file's step learnt kept in the record, and the outbox said; what OneDrive made, then
/// forgotten in memory. The copy failing, it is asked again after the worker's pause.
fn kept(w: &mut Worker, op: &OpRecord, now: Instant) -> Ready {
    if let Some(Err(e)) = w.copy.as_ref().map(|c| c.put_op(op)) {
        let e = NetError::Store(e);
        log::error!("teams: a message's file couldn't be kept as it went: {}", kind(&e));
        w.pause(&e, now);
        return Ready::Busy;
    }
    if let Some(upload) = w.outbox.uploads.get_mut(&op.seq) {
        upload.item = None;
    }
    changed(w, &op.chat);
    Ready::Busy
}

/// A request of a file's that failed. Graph busy: its `Retry-After` waited. The sign-in refused:
/// it waits for it. No answer, or Graph's server failing (500, 502, 504: these requests can be
/// made again): held back ([`Held::unanswered`]), a try when the network answers (a server's
/// error is one), the [`POSTS`]th "Not sent" (couldn't be confirmed). Anything else refused: "Not
/// sent" for `why`, with Teams' code.
fn failed(w: &mut Worker, op: &mut OpRecord, e: &NetError, why: Unsent, now: Instant) -> Ready {
    let hash = w.hashed(&op.chat);
    if busy(e) || matches!(e, NetError::Auth(_)) {
        log::warn!("teams: a file of a message to conversation {hash} waits ({})", kind(e));
        if busy(e) {
            w.busy_wait(now);
        }
        return Ready::Busy;
    }
    let status = match e {
        NetError::Closed(m) => http_code(&NetError::Protocol(m.clone())),
        e => http_code(e),
    };
    if !of_the_network(e) && !matches!(status, Some(500 | 502 | 504)) {
        log::warn!("teams: a file of a message to conversation {hash} was refused ({}): not sent", kind(e));
        return not_sent(w, op, why, code_of(e));
    }
    let counts = w.outbox.held.unanswered(op.seq, w.answered, now) || status.is_some();
    log::warn!("teams: a file of a message to conversation {hash} failed ({}); asked again later", kind(e));
    if counts {
        tried(w, op)
    } else {
        w.pause(e, now);
        Ready::Busy
    }
}

/// A try of a file's counted: the [`POSTS`]th, "Not sent" (couldn't be confirmed); else kept.
fn tried(w: &mut Worker, op: &mut OpRecord) -> Ready {
    op.tries = op.tries.saturating_add(1);
    if op.tries >= POSTS {
        log::warn!("teams: a file of a message to conversation {} failed {POSTS} times: not sent", w.hashed(&op.chat));
        return not_sent(w, op, Unsent::Unconfirmed, None);
    }
    settle(w, op);
    Ready::Busy
}

/// The message "Not sent" for `why`, what was held of its files forgotten, and said.
fn not_sent(w: &mut Worker, op: &mut OpRecord, why: Unsent, code: Option<String>) -> Ready {
    w.outbox.uploads.remove(&op.seq);
    w.outbox.held.forget(op.seq);
    op.state = OpState::NotSent { reason: why, code };
    settle(w, op);
    Ready::Settled
}

/// A word of the upload lane about part `part` of message `seq` ([`Done::Sent`],
/// [`Done::Uploaded`], [`Done::NotUploaded`]): the bytes gone up so far, said with the chat's
/// outbox; the item OneDrive made, kept for the next step; or why the upload stopped. The file
/// gone or changed: "Not sent". The session gone: a new one at the next step, [`SESSIONS`] in all,
/// then "Not sent" (couldn't be confirmed). Stalled or cut: as a request without an answer, and
/// the upload begun again. Refused: "Not sent" with Teams' code. A word for a message no longer
/// going out is dropped.
pub(crate) fn uploaded(w: &mut Worker, done: Done, now: Instant) {
    let (seq, part) = match &done {
        Done::Sent { seq, part, .. } | Done::Uploaded { seq, part, .. } | Done::NotUploaded { seq, part, .. } => (*seq, *part),
        Done::Saved { .. } | Done::NotSaved { .. } => return,
    };
    let going = |o: &OpRecord| o.state == OpState::Waiting && matches!(o.parts.get(part), Some(PartRecord::File { up: None, .. }));
    let Some(mut op) = w.copy.as_ref().and_then(|c| c.op_at(seq)).filter(going) else {
        w.outbox.uploads.remove(&seq);
        return log::info!("teams: an upload's word for a message no longer going out was dropped");
    };
    let (size, hash) = (op.parts.get(part).map_or(0, |p| p.shown().size), w.hashed(&op.chat));
    let upload = w.outbox.uploads.entry(seq).or_default();
    if upload.part != part {
        *upload = Upload { part, ..Upload::default() };
    }
    match done {
        Done::Sent { sent, .. } => {
            upload.sent = sent.min(size);
            say_chat(w, &op.chat);
        }
        Done::Uploaded { item, .. } => {
            (upload.sent, upload.item) = (size, Some(item));
            w.outbox.held.forget(seq);
            log::info!("teams: a file of a message to conversation {hash} went up");
            say_chat(w, &op.chat);
        }
        Done::NotUploaded { why, .. } => {
            upload.sent = 0;
            let gone = match why {
                UploadFail::SessionGone => {
                    upload.gone = upload.gone.saturating_add(1);
                    upload.gone
                }
                _ => 0,
            };
            log::warn!("teams: a file of a message to conversation {hash} didn't go up: {why:?}");
            match why {
                UploadFail::Gone => {
                    not_sent(w, &mut op, Unsent::FileGone, None);
                }
                UploadFail::SessionGone if gone >= SESSIONS => {
                    not_sent(w, &mut op, Unsent::Unconfirmed, None);
                }
                UploadFail::Stalled | UploadFail::Network => {
                    if w.outbox.held.unanswered(seq, w.answered, now) {
                        tried(w, &mut op);
                    } else {
                        say_chat(w, &op.chat);
                    }
                }
                UploadFail::Refused(code) => {
                    not_sent(w, &mut op, Unsent::Refused, code);
                }
                UploadFail::SessionGone => say_chat(w, &op.chat),
                UploadFail::Cancelled => {}
            }
        }
        Done::Saved { .. } | Done::NotSaved { .. } => {}
    }
}

/// The message posted with its parts: its pictures inside it, their bytes as the copy keeps them
/// (one gone or not of its length: refused), and its files, all ready, by reference under the
/// names OneDrive gave them, each at an address on the user's own OneDrive as it is read back
/// (else refused: the copy is never taken at its word).
pub(super) fn post(w: &mut Worker, head: &Head, op: &OpRecord, now: Instant) -> crate::Result<Value> {
    let lost = || NetError::Refused("a part of the message is no longer there".into());
    let (mut pictures, mut files): (Vec<(Vec<u8>, &str)>, Vec<FileRef<'_>>) = (Vec::new(), Vec::new());
    for (n, part) in op.parts.iter().enumerate() {
        match part {
            PartRecord::Picture { mime, len, .. } => {
                let bytes =
                    w.copy.as_ref().and_then(|c| c.picture(op.seq, n)).filter(|b| u32::try_from(b.len()).ok() == Some(*len)).ok_or_else(lost)?;
                pictures.push((bytes, mime.as_str()));
            }
            PartRecord::File { up: Some(up), .. } if up.shared && own_drive(w, &up.url) => {
                files.push(FileRef { guid: &up.guid, url: &up.url, name: &up.name })
            }
            PartRecord::File { .. } => return Err(lost()),
        }
    }
    let guids: Vec<&str> = files.iter().map(|f| f.guid).collect();
    let html = teams_write_parts(&op.body, &op.mentions, pictures.len(), &guids)
        .map_err(|_| NetError::Refused("a message longer than Teams takes".into()))?;
    let hosted: Vec<Hosted<'_>> = pictures.iter().map(|(bytes, mime)| Hosted { bytes, mime }).collect();
    let chat = head.chat.clone();
    w.request(Route::Send(head.key), now, |c| c.send_parts(&chat, &html, &op.mentions, &hosted, &files))
}

/// Whether a message a check read is `op`'s by its parts: it carries files of the same names in
/// the same order (those OneDrive gave them), and at least as many pictures as `op` has.
pub(super) fn same_files(op: &OpRecord, m: &ChatMessage) -> bool {
    let ours = op.parts.iter().filter_map(|p| match p {
        PartRecord::File { name, up, .. } => Some(up.as_ref().map_or(name, |u| &u.name)),
        PartRecord::Picture { .. } => None,
    });
    let theirs = m.attachments.iter().filter_map(|a| if let Attachment::File { name, .. } = a { Some(name) } else { None });
    let pictures = op.parts.iter().filter(|p| matches!(p, PartRecord::Picture { .. })).count();
    if !ours.eq(theirs) {
        return false;
    }
    let mut hosted: Vec<&str> =
        m.attachments.iter().filter_map(|a| if let Attachment::Hosted { id } = a { Some(id.as_str()) } else { None }).collect();
    hosted.extend(m.body.blocks.iter().flat_map(|b| &b.runs).filter_map(|r| r.hosted.as_deref()));
    hosted.sort_unstable();
    hosted.dedup();
    hosted.len() >= pictures
}

/// An item of the outbox as the conversation shows it: a message whose files are being made ready
/// (an upload of it running, or begun, or its files not all gone up while it is its chat's first)
/// is "Uploading", the part going up said as far as it is.
pub(super) fn shown(w: &Worker, mut o: Outgoing) -> Outgoing {
    if o.state != OutState::Waiting || o.parts.iter().all(|p| p.picture) {
        return o;
    }
    let upload = w.outbox.uploads.get(&o.seq);
    let first = w.outbox.heads.iter().any(|h| h.seq == o.seq);
    let unready = o.parts.iter().any(|p| !p.picture && p.sent < p.size);
    if !(w.transfers.uploading(o.seq) || upload.is_some() || (first && unready)) {
        return o;
    }
    o.state = OutState::Uploading;
    if let Some(u) = upload
        && let Some(p) = o.parts.get_mut(u.part)
    {
        p.sent = p.sent.max(u.sent.min(p.size));
    }
    o
}

/// [`Job::Retry`](quatrelle_model::pim::teams::Job::Retry): each file not gone up yet taken as it
/// is now (its size and time), so that a file put back or saved again goes; what went up stays,
/// never uploaded again.
pub(super) fn again(op: &mut OpRecord) {
    for part in &mut op.parts {
        if let PartRecord::File { path, size, modified, up: None, .. } = part
            && let Ok(m) = std::fs::metadata(&*path)
            && m.is_file()
            && (1..=FILE_MAX).contains(&m.len())
        {
            (*size, *modified) = (m.len(), modified_of(&m));
        }
    }
}

/// A message let go (deleted, or its chat left): its upload cancelled, what was held of its files
/// forgotten, its pictures removed.
pub(super) fn let_go(w: &mut Worker, seq: u64) {
    w.transfers.cancel_upload(seq);
    w.outbox.uploads.remove(&seq);
    if let Some(copy) = &w.copy
        && let Err(e) = copy.remove_pictures(seq)
    {
        log::error!("teams: a message's pictures couldn't be let go: {}", kind(&NetError::Store(e)));
    }
}
