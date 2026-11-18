// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Files through Graph: OneDrive's upload sessions and the ranges sent to them, a file's item,
//! its share with a chat's members, a post with pictures and files, a file shared in a chat (its
//! address, its bytes counted), and the readers of what Graph answers about them.
//!
//! An upload session's or a download's address carries its own authorisation: it is taken only
//! on Microsoft's file hosts ([`transfer_url`]), gets no `Authorization` header and is never
//! written out (an [`UploadSession`]'s `Debug` says its host's last two labels: [`host_tail`]).
//! What a range, an item or a share answers is a server's: it is read by the pure readers here
//! (`*_of`), which the fuzz target reads hostile answers with too.

use std::fmt;
use std::io::Read;
use std::net::IpAddr;

use quatrelle_mime::sanitize_filename;
use quatrelle_model::pim::teams::{ADDRESS_MAX, FILE_NAME_MAX, PARTS_MAX, PICTURE_INLINE_MAX, SHARE_PEOPLE, is_bidi, is_object_id};

use super::*;

/// The folder of the user's OneDrive Teams keeps the files sent in chats in.
pub const UPLOAD_FOLDER: &str = "Microsoft Teams Chat Files";
/// What an upload's or a download's host may end with, after a label of its own.
pub(crate) const TRANSFER_SUFFIXES: [&str; 1] = [".sharepoint.com"];
/// The bytes of one range sent to an upload session (5 MiB): a multiple of the 320 KiB OneDrive
/// wants of every range but the last.
pub(crate) const CHUNK: usize = 16 * 327_680;
/// The longest transfer address, or file's address, taken.
const MAX_TRANSFER_URL: usize = 8 << 10;
/// The longest name kept, in characters (`sanitize_filename`'s), and the longest extension.
const NAME_MAX: usize = 180;
const EXTENSION_MAX: usize = 16;
/// The most bytes of an eTag looked through for its GUID.
const ETAG_MAX: usize = 1 << 10;
/// The most bytes of an answer to a range, a status or a cancel.
const MAX_RANGE_ANSWER: usize = 1 << 20;

/// Whether `url` may be an upload's or a download's address for a client whose API is at
/// `base`, and the address: at most 8 KiB, without white space or a control character; `https` on
/// the default port, without user information, to a DNS name (never an IP address) ending with
/// one of [`TRANSFER_SUFFIXES`] after a label of its own; or plain HTTP to this computer when
/// the client's own API is there too (the tests' servers, as [`hop_allowed`]). Anything else is
/// refused, before any connection.
pub fn transfer_url(base: &str, url: &str) -> Result<Url> {
    let refused = not_microsofts;
    if url.is_empty() || url.len() > MAX_TRANSFER_URL || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(refused());
    }
    let to = Url::parse(url).map_err(|_| refused())?;
    let taken = if to.https { on_microsofts_hosts(&to) } else { hop_allowed(base, &to) };
    if taken { Ok(to) } else { Err(refused()) }
}

/// Whether an address is on Microsoft's file hosts: `https` on the default port, to a DNS name
/// (never an IP address, nor user information) ending with one of [`TRANSFER_SUFFIXES`] after a
/// label of its own. [`transfer_url`]'s rule, and the outbox's for a file uploaded.
pub(crate) fn on_microsofts_hosts(to: &Url) -> bool {
    to.https && to.port == 443 && microsoft_host(&to.host)
}

/// A transfer's address refused: the address itself is never said (it carries its own
/// authorisation).
fn not_microsofts() -> NetError {
    NetError::Refused("not one of Microsoft's file hosts".into())
}

/// A DNS name (labels of 1 to 63 lower-case letters, digits and `-`), not an IP address, ending
/// with one of [`TRANSFER_SUFFIXES`] after a label of its own.
fn microsoft_host(host: &str) -> bool {
    dns_name(host) && host.parse::<IpAddr>().is_err() && TRANSFER_SUFFIXES.iter().any(|s| host.strip_suffix(s).is_some_and(|own| !own.is_empty()))
}

fn dns_name(host: &str) -> bool {
    host.len() <= MAX_HOST
        && host.split('.').all(|l| (1..=63).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
}

/// Whether a transfer's connection (an upload session's, a download's) must be to a public
/// address, by the client's API address `base`: always, but for the tests' servers on this
/// computer (a plain-HTTP API there), as [`hop_allowed`].
pub(crate) fn transfer_public_only(base: &str) -> bool {
    !Url::parse(base).is_ok_and(|b| !b.https && loopback(&b))
}

/// What the log says of a host: `…` and its last two labels (`…sharepoint.com`) when it is a
/// DNS name of `a-z 0-9 . -`; `…` alone otherwise (an IP address, one label, anything odd).
pub fn host_tail(host: &str) -> String {
    let host = host.to_ascii_lowercase();
    if !dns_name(&host) || host.parse::<IpAddr>().is_ok() {
        return "…".into();
    }
    match host.rsplit('.').take(2).collect::<Vec<_>>().as_slice() {
        [last, before] => format!("…{before}.{last}"),
        _ => "…".into(),
    }
}

/// A file's name as OneDrive takes it: [`sanitize_filename`]'s, its extension (up to 16 letters
/// and digits) kept within the 180 characters; `#` and `%` as `_`, no leading `~$`, `.lock` and
/// `desktop.ini` given a leading `_`, and `_vti_` written `-vti_`. A drive name is its own drive
/// name and its own safe name.
pub fn drive_name(name: &str) -> String {
    // Twice: a cut or a trim can leave the name with an extension it didn't have.
    drive_once(&drive_once(name))
}

fn drive_once(name: &str) -> String {
    let last: String = name.rsplit(['/', '\\']).next().unwrap_or_default().chars().map(|c| if matches!(c, '#' | '%') { '_' } else { c }).collect();
    let (stem, ext) = match last.rsplit_once('.') {
        Some((stem, ext))
            if !stem.trim().is_empty() && (1..=EXTENSION_MAX).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            (stem, Some(ext))
        }
        _ => (last.as_str(), None),
    };
    let trim = |s: &str| s.trim_matches(|c: char| c.is_whitespace() || c == '.').to_string();
    let mut s = trim(&sanitize_filename(stem));
    while let Some(rest) = s.strip_prefix("~$") {
        s = trim(rest);
    }
    // Trimmed, a name may be one Windows keeps (a leading `_`) or nothing (`attachment`); cut to
    // leave room for the extension, it may be again.
    let s = sanitize_filename(&s);
    let room = NAME_MAX.saturating_sub(ext.map_or(0, |e| e.len() + 1));
    let s = sanitize_filename(&trim(&s.chars().take(room).collect::<String>()));
    let mut whole = match ext {
        Some(ext) => format!("{s}.{ext}"),
        None => s,
    };
    if whole.eq_ignore_ascii_case("desktop.ini") || whole.eq_ignore_ascii_case(".lock") {
        whole.insert(0, '_');
    }
    while let Some(at) = whole.find("_vti_") {
        whole.replace_range(at..at + 1, "-");
    }
    whole
}

/// An upload session: where a file's ranges go. Its address carries its own authorisation, so it
/// is never written out: its `Debug` says its host's last two labels only.
pub struct UploadSession {
    url: Url,
}

impl UploadSession {
    /// Where the ranges go.
    pub(crate) fn url(&self) -> &Url {
        &self.url
    }
}

impl fmt::Debug for UploadSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadSession").field("host", &host_tail(&self.url.host)).finish()
    }
}

/// A file of the user's OneDrive, as a message needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriveItem {
    pub id: String,
    /// Its name, made safe ([`drive_name`]).
    pub name: String,
    pub size: u64,
    /// The GUID inside its eTag: the attachment's id in a message.
    pub guid: String,
    /// Its WebDAV address: the attachment's `contentUrl`.
    pub web_dav_url: String,
}

/// What a share did: the recipients granted, and those it failed for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shared {
    pub granted: usize,
    pub failed: usize,
}

/// Someone a file is shared with: a person of the organisation by their object id, anyone else by
/// their address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recipient<'a> {
    Id(&'a str),
    Address(&'a str),
}

/// A picture sent inside its message (a hosted content): PNG or JPEG.
#[derive(Clone, Copy, Debug)]
pub struct Hosted<'a> {
    pub bytes: &'a [u8],
    pub mime: &'a str,
}

/// A file of the user's OneDrive a message carries: its eTag's GUID, its WebDAV address, its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileRef<'a> {
    pub guid: &'a str,
    pub url: &'a str,
    pub name: &'a str,
}

/// What a range's answer says: the next byte OneDrive expects, or the file made (its item's id).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeAnswer {
    Next(u64),
    Done(String),
}

/// A request at an upload session's address that failed: why, and the `Retry-After` (in seconds)
/// of an answer asking to wait (429, 503) that said one.
#[derive(Debug)]
pub(crate) struct TransferFail(pub(crate) NetError, pub(crate) Option<u64>);

impl From<NetError> for TransferFail {
    fn from(e: NetError) -> Self {
        TransferFail(e, None)
    }
}

impl TeamsClient {
    /// An upload session for a file of this name in [`UPLOAD_FOLDER`] (a name taken renamed by
    /// OneDrive). The name must be its own [`drive_name`] (the caller names the file first), else
    /// it is refused before any request; the session's address must be on Microsoft's file hosts
    /// ([`transfer_url`]).
    pub fn upload_session(&self, name: &str) -> Result<UploadSession> {
        if drive_name(name) != name {
            return Err(NetError::Refused("not a name OneDrive takes".into()));
        }
        let url = self.at(&format!("/me/drive/root:/{}/{}:/createUploadSession", seg(UPLOAD_FOLDER), seg(name)));
        let v = self.json("POST", &url, Some(&json!({"item": {"@microsoft.graph.conflictBehavior": "rename"}})), false)?;
        let to = upload_url_of(&v).ok_or_else(|| NetError::Protocol("Graph: an upload session without its address".into()))?;
        Ok(UploadSession { url: transfer_url(&self.base, to)? })
    }

    /// A file of the user's OneDrive, with what a message needs of it: its name, size, eTag's GUID
    /// and WebDAV address (asked for by name: Graph gives it only then). The id must be one
    /// segment of the address ([`own_segment`]), else it is refused before any request.
    pub fn drive_item(&self, id: &str) -> Result<DriveItem> {
        if !own_segment(id) {
            return Err(NetError::Refused("not a file's id".into()));
        }
        let v = self.json("GET", &self.at(&format!("/me/drive/items/{}?$select=id,eTag,webDavUrl,webUrl,name,size", seg(id))), None, false)?;
        drive_item_of(&v).ok_or_else(|| NetError::Protocol("Graph: a file's item without what a message needs".into()))
    }

    /// Share a file of the user's OneDrive with these people: to read it, signed in, without a
    /// mail. The item's id must be one segment of the address, the recipients 1 to
    /// [`SHARE_PEOPLE`], each an object id or an address, else it is refused before any request.
    /// A 207 says some failed: those its answer doesn't list as granted ([`shared_of`]).
    pub fn invite(&self, item: &str, to: &[Recipient<'_>]) -> Result<Shared> {
        let fits = |r: &Recipient<'_>| match r {
            Recipient::Id(id) => is_object_id(id),
            Recipient::Address(a) => address_ok(a),
        };
        if !own_segment(item) || to.is_empty() || to.len() > SHARE_PEOPLE || !to.iter().all(fits) {
            return Err(NetError::Refused("not people a file can be shared with".into()));
        }
        let recipients: Vec<Value> = to
            .iter()
            .map(|r| match r {
                Recipient::Id(id) => json!({"objectId": id}),
                Recipient::Address(a) => json!({"email": a}),
            })
            .collect();
        let body = json!({"recipients": recipients, "roles": ["read"], "requireSignIn": true, "sendInvitation": false});
        let req = self.prepared("POST", &self.at(&format!("/me/drive/items/{}/invite", seg(item))), Some(&body), false)?;
        let (status, v) = self.answered(&req, true)?;
        Ok(if status == 207 { shared_of(&v, to.len()) } else { Shared { granted: to.len(), failed: 0 } })
    }

    /// Delete a file of the user's OneDrive (a check's own test file): the id must be one segment
    /// of the address, else it is refused before any request.
    pub fn delete_item(&self, id: &str) -> Result<()> {
        if !own_segment(id) {
            return Err(NetError::Refused("not a file's id".into()));
        }
        self.json("DELETE", &self.at(&format!("/me/drive/items/{}", seg(id))), None, false).map(drop)
    }

    /// Post a message written in HTML with pictures inside it (`hostedContents`, numbered from 1
    /// in order, as the body's `../hostedContents/n/$value` places say) and files of the user's
    /// OneDrive (`attachments`, by reference, as the body's `<attachment id>` places say), each
    /// list only when it has some: the message Graph made. As [`Self::send_html`], and refused
    /// before any request past [`PARTS_MAX`] parts, a picture that isn't PNG or JPEG of 1 byte to
    /// [`PICTURE_INLINE_MAX`], or a file without a GUID, an `https` address or a plain name.
    pub fn send_parts(&self, chat: &str, html: &str, mentions: &[Mention], pictures: &[Hosted<'_>], files: &[FileRef<'_>]) -> Result<Value> {
        if !own_segment(chat) {
            return Err(NetError::Refused("not a chat's id".into()));
        }
        if html.len() > SEND_MAX {
            return Err(NetError::Refused(format!("a message longer than Teams takes ({SEND_MAX} bytes)")));
        }
        if pictures.len().saturating_add(files.len()) > PARTS_MAX || !pictures.iter().all(picture_ok) || !files.iter().all(file_ok) {
            return Err(NetError::Refused("not pictures and files a message can carry".into()));
        }
        let mut body = json!({"body": {"contentType": "html", "content": html}});
        let mentioned = mentions_json(mentions);
        if mentioned.as_array().is_some_and(|a| !a.is_empty()) {
            body["mentions"] = mentioned;
        }
        if !pictures.is_empty() {
            body["hostedContents"] = pictures
                .iter()
                .enumerate()
                .map(|(i, p)| json!({"@microsoft.graph.temporaryId": (i + 1).to_string(), "contentBytes": crate::imap::base64(p.bytes), "contentType": p.mime}))
                .collect();
        }
        if !files.is_empty() {
            body["attachments"] =
                files.iter().map(|f| json!({"id": f.guid, "contentType": "reference", "contentUrl": f.url, "name": f.name})).collect();
        }
        self.request("POST", &self.at(&format!("/chats/{}/messages", seg(chat))), Some(&body), false, false)
    }

    /// A shared file's download address: the first hop of [`Self::download_shared`] (Graph's
    /// `/shares/u!…/driveItem/content`, with the token), its redirect taken through
    /// [`transfer_url`] and its body never read. A file Graph serves itself has no address: an
    /// error, as any other answer is.
    pub fn shared_location(&self, content_url: &str) -> Result<Url> {
        match self.share_hop(content_url, None)? {
            Hop::To(url) => Ok(url),
            Hop::Served(_) => Err(NetError::Protocol("Graph: a shared file without its address".into())),
        }
    }

    /// Download a file shared in a chat, by the `contentUrl` its attachment carries, counting
    /// its bytes and keeping none: [`Self::shared_location`]'s first hop, then its one redirect,
    /// on Microsoft's file hosts, followed without the token. More than `max` bytes is refused.
    pub fn download_shared(&self, content_url: &str, max: usize) -> Result<usize> {
        let to = match self.share_hop(content_url, Some(max))? {
            // Graph served the file itself.
            Hop::Served(bytes) => return Ok(bytes),
            Hop::To(to) => to,
        };
        self.download_at(&to, max)
    }

    /// The bytes of a file at the address [`Self::shared_location`] gave, counted and not kept
    /// (at most `max`), on a connection of its own to a public address ([`Self::transfer_stream`])
    /// and without the token. A second redirect or any other answer is an error with its status.
    pub fn download_at(&self, to: &Url, max: usize) -> Result<usize> {
        // A request of its own: the token stays with Graph.
        let (head, tally) = self.download(&Request::get(&address(to), max), false, true)?;
        if redirect(&head).is_some() {
            return Err(NetError::Protocol("the shared file's download redirected again".into()));
        }
        if !head.ok() {
            return Err(NetError::Protocol(format!("HTTP {}", head.status)));
        }
        Ok(tally.bytes)
    }

    /// A connection to the server of an upload session or a download (Microsoft's file hosts), to
    /// a public address only ([`stream::tcp_public`]: it logs no address tried), but for the tests'
    /// servers on this computer ([`transfer_public_only`]). Its errors name the host by
    /// [`host_tail`] alone: the address carries its own authorisation, and a name a log keeps.
    pub(crate) fn transfer_stream(&self, url: &Url) -> Result<Stream> {
        transfer_stream_at(&self.base, url)
    }

    /// The first hop of a shared file's download, with the token, waited out while busy: Graph's
    /// `/shares/u!…/driveItem/content`. A file Graph serves itself is counted, at most `max` bytes
    /// (`None`: its body left unread); a redirect's address is taken through [`transfer_url`];
    /// any other answer is an error with its status.
    fn share_hop(&self, content_url: &str, max: Option<usize>) -> Result<Hop> {
        let content_url = content_url.trim();
        if content_url.is_empty() || content_url.len() > MAX_SHARE_URL {
            return Err(NetError::Refused("not a shared file's address".into()));
        }
        let share = crate::imap::base64(content_url.as_bytes()).trim_end_matches('=').replace('/', "_").replace('+', "-");
        let first = self.at(&format!("/shares/u!{share}/driveItem/content"));
        let req = Request::get(&first, max.unwrap_or(KEEP_ERROR)).header("Authorization", &self.bearer());
        let mut round = 0;
        let (head, tally) = loop {
            round += 1;
            let (head, tally) = self.download(&req, true, max.is_some())?;
            if !busy(head.status) || !self.waiting.get() || round >= BUSY_ROUNDS {
                break (head, tally);
            }
            pause(backoff_secs(head.header("Retry-After"), round));
        };
        if head.ok() {
            return Ok(Hop::Served(tally.bytes));
        }
        let Some(location) = redirect(&head) else {
            status_error(head.status, &tally.kept)?;
            return Err(NetError::Protocol(format!("HTTP {}", head.status)));
        };
        // An address as it came, or a path joined to Graph's (the tests' servers); one that isn't
        // an address is refused as any other, never quoted.
        let to = match Url::parse(location) {
            Ok(_) => location.to_string(),
            Err(_) => Url::parse(&first).and_then(|u| u.join(location)).map(|u| address(&u)).map_err(|_| not_microsofts())?,
        };
        transfer_url(&self.base, &to).map(Hop::To)
    }
}

/// Where the first hop of a shared file's download ends.
enum Hop {
    /// Graph served the file itself: its bytes, counted.
    Served(usize),
    /// Graph's redirect, on Microsoft's file hosts.
    To(Url),
}

/// Send `bytes`, the file's from byte `from` on, to an upload session's address on the connection
/// `s`, without the token: the next byte OneDrive expects (past these bytes, before `total`), or
/// the file made once the last range is in ([`range_answer_of`]); the file made before its last
/// range, or not made after it, is an error. A range already there is `Protocol("HTTP 416")` (ask
/// where the upload stands: [`upload_status_on`]), a session gone
/// `Refused("the upload session is gone")`, an answer asking to wait with its `Retry-After`
/// ([`TransferFail`]); what isn't a range of `total` is refused before anything is written.
pub(crate) fn put_range_on<S: Read + Write>(
    s: &mut S,
    url: &Url,
    bytes: &[u8],
    from: u64,
    total: u64,
) -> std::result::Result<RangeAnswer, TransferFail> {
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let end = from.checked_add(len).filter(|end| len > 0 && *end <= total).ok_or_else(|| NetError::Refused("not a range of the file".into()))?;
    let mut req = Request::get(&address(url), MAX_RANGE_ANSWER).header("Content-Range", &format!("bytes {from}-{}/{total}", end - 1));
    req.method = "PUT".into();
    req.body = bytes.to_vec();
    let resp = http::exchange(s, &req, url)?;
    let done = match (resp.status, end == total) {
        (202, false) => false,
        (200 | 201, true) => true,
        (202, true) => return Err(NetError::Protocol("Graph: an upload not finished after its last range".into()).into()),
        (200 | 201, false) => return Err(NetError::Protocol("Graph: an upload finished early".into()).into()),
        _ => return Err(transfer_fail(&resp)),
    };
    let v = serde_json::from_slice::<Value>(&resp.body).unwrap_or(Value::Null);
    range_answer_of(&v, done, end, total).ok_or_else(|| NetError::Protocol("OneDrive: an answer to a range that isn't one".into()).into())
}

/// Where an upload stands: the next byte OneDrive expects (at most `total`), asked at the
/// session's address without the token; a session gone is `Refused("the upload session is gone")`,
/// an answer asking to wait has its `Retry-After` ([`TransferFail`]).
pub(crate) fn upload_status_on<S: Read + Write>(s: &mut S, url: &Url, total: u64) -> std::result::Result<u64, TransferFail> {
    let resp = http::exchange(s, &Request::get(&address(url), MAX_RANGE_ANSWER), url)?;
    if resp.status != 200 {
        return Err(transfer_fail(&resp));
    }
    let v = serde_json::from_slice::<Value>(&resp.body).unwrap_or(Value::Null);
    next_expected_of(&v, total).ok_or_else(|| NetError::Protocol("OneDrive: an upload without its next range".into()).into())
}

/// A transfer's failed answer ([`transfer_error`]), with its `Retry-After` when it asks to wait
/// (429, 503) and says how long, in seconds.
fn transfer_fail(resp: &Response) -> TransferFail {
    let wait = if busy(resp.status) { resp.header("Retry-After").and_then(|h| h.trim().parse::<u64>().ok()) } else { None };
    TransferFail(transfer_error(resp.status, &resp.body), wait)
}

/// Cancel an upload session at its address, without the token (one already gone is fine).
pub(crate) fn cancel_upload_on<S: Read + Write>(s: &mut S, url: &Url) -> Result<()> {
    let mut req = Request::get(&address(url), MAX_RANGE_ANSWER);
    req.method = "DELETE".into();
    let resp = http::exchange(s, &req, url)?;
    if resp.ok() || resp.status == 404 { Ok(()) } else { Err(transfer_error(resp.status, &resp.body)) }
}

/// A transfer's failure: a session gone (404) said so, a range already there (416) by its status,
/// any other as [`status_error`] says it, but never as a sign-in's (the address carries its own
/// authorisation, not the token).
fn transfer_error(status: u16, body: &[u8]) -> NetError {
    match (status, status_error(status, body)) {
        (404, _) => NetError::Refused("the upload session is gone".into()),
        (416, _) => NetError::Protocol("HTTP 416".into()),
        (_, Err(NetError::Auth(_)) | Ok(())) => NetError::Protocol(format!("HTTP {status}")),
        (_, Err(e)) => e,
    }
}

/// [`TeamsClient::transfer_stream`] for a client whose API is at `base`, without the client (it
/// holds the token): what the worker's transfer lanes connect with.
pub(crate) fn transfer_stream_at(base: &str, url: &Url) -> Result<Stream> {
    if !transfer_public_only(base) {
        return connect(url);
    }
    let tail = host_tail(&url.host);
    let sock = stream::tcp_public(&url.host, url.port).map_err(|e| without_host(e, &tail))?;
    if url.https { stream::tls(sock, &url.host).map_err(|e| without_host(e, &tail)) } else { Ok(Stream::Plain(sock)) }
}

/// A connection's error, said without the host's name (it can carry the address's own
/// authorisation, and a log keeps what an error says): its kind and `tail` ([`host_tail`]) only.
pub(crate) fn without_host(e: NetError, tail: &str) -> NetError {
    match e {
        NetError::Io(io) => NetError::Io(std::io::Error::new(io.kind(), format!("{tail}: no connection"))),
        NetError::Tls(_) => NetError::Tls(format!("{tail}: no secure connection")),
        NetError::Protocol(_) => NetError::Protocol(format!("{tail} is on this computer or its network: only addresses on the internet are used")),
        other => other,
    }
}

/// A URL as a request names it.
pub(crate) fn address(url: &Url) -> String {
    format!("{}{}", url.origin(), url.path)
}

/// An address: 1 to [`ADDRESS_MAX`] characters with an `@`, no white space, control or bidi
/// character.
pub(crate) fn address_ok(a: &str) -> bool {
    !a.is_empty() && a.contains('@') && a.chars().count() <= ADDRESS_MAX && !a.chars().any(|c| c.is_whitespace() || c.is_control() || is_bidi(c))
}

fn picture_ok(p: &Hosted<'_>) -> bool {
    matches!(p.mime, "image/png" | "image/jpeg") && !p.bytes.is_empty() && u64::try_from(p.bytes.len()).is_ok_and(|n| n <= PICTURE_INLINE_MAX)
}

fn file_ok(f: &FileRef<'_>) -> bool {
    let named = !f.name.is_empty() && f.name.chars().count() <= FILE_NAME_MAX && !f.name.chars().any(|c| c.is_control() || is_bidi(c));
    is_object_id(f.guid) && web_address(f.url) && named
}

/// An `https` address of at most 8 KiB, without white space, control or bidi character.
fn web_address(u: &str) -> bool {
    u.len() <= MAX_TRANSFER_URL && !u.chars().any(|c| c.is_whitespace() || c.is_control() || is_bidi(c)) && Url::parse(u).is_ok_and(|p| p.https)
}

/// The address of an upload session (Graph's `uploadUrl`), as it came.
pub fn upload_url_of(v: &Value) -> Option<&str> {
    v.get("uploadUrl").and_then(Value::as_str).filter(|u| !u.is_empty() && u.len() <= MAX_TRANSFER_URL)
}

/// The first byte OneDrive expects next (the start of the first of `nextExpectedRanges`), when it
/// is at most `total`.
pub fn next_expected_of(v: &Value, total: u64) -> Option<u64> {
    let first = v.get("nextExpectedRanges")?.as_array()?.first()?.as_str()?;
    let start = first.split('-').next()?;
    if start.is_empty() || start.len() > 20 || !start.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    start.parse::<u64>().ok().filter(|n| *n <= total)
}

/// What the answer to a range ending at `end` says: for a 202 (`done` false) on a range that isn't
/// the last, the next byte OneDrive expects, which must be `end` or later and before `total`; for a
/// 200 or a 201 on the last range (`end` is `total`), the file made, by its item's id (one segment
/// of an address). Anything else says nothing.
pub fn range_answer_of(v: &Value, done: bool, end: u64, total: u64) -> Option<RangeAnswer> {
    match (done, end == total) {
        (true, true) => v.get("id").and_then(Value::as_str).filter(|id| own_segment(id)).map(|id| RangeAnswer::Done(id.to_string())),
        (false, false) => next_expected_of(v, total).filter(|n| *n >= end && *n < total).map(RangeAnswer::Next),
        _ => None,
    }
}

/// The GUID inside an eTag (`"{5B8C7D0E-…},2"`): its first run of 8-4-4-4-12 hexadecimal digits
/// standing alone, lowercased.
pub fn etag_guid(etag: &str) -> Option<String> {
    let b = etag.as_bytes();
    let b = b.get(..b.len().min(ETAG_MAX)).unwrap_or_default();
    let joined = |i: Option<usize>| i.and_then(|i| b.get(i)).is_some_and(|c| c.is_ascii_hexdigit() || *c == b'-');
    (0..b.len().saturating_sub(35)).find_map(|i| {
        let guid = std::str::from_utf8(b.get(i..i + 36)?).ok()?;
        (is_object_id(guid) && !joined(i.checked_sub(1)) && !joined(Some(i + 36))).then(|| guid.to_ascii_lowercase())
    })
}

/// A file's item (Graph's `driveItem`) as a message needs it: an id that is one segment of an
/// address, a name (made a [`drive_name`]), its eTag's GUID and an `https` WebDAV address; its
/// size, 0 when not said.
pub fn drive_item_of(v: &Value) -> Option<DriveItem> {
    let text = |k: &str| v.get(k).and_then(Value::as_str);
    Some(DriveItem {
        id: text("id").filter(|id| own_segment(id))?.to_string(),
        name: text("name").filter(|n| !n.trim().is_empty()).map(drive_name)?,
        size: v.get("size").and_then(Value::as_u64).unwrap_or(0),
        guid: text("eTag").and_then(etag_guid)?,
        web_dav_url: text("webDavUrl").filter(|u| web_address(u))?.to_string(),
    })
}

/// A share's answer when some of the `asked` recipients failed (207): those it lists as granted
/// (an entry without an `error`, among its first [`SHARE_PEOPLE`]), at most `asked`; the others
/// failed, so what it doesn't say was granted is counted as failed.
pub fn shared_of(v: &Value, asked: usize) -> Shared {
    let listed = v.get("value").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let listed = listed.get(..listed.len().min(SHARE_PEOPLE)).unwrap_or_default();
    let said = listed.iter().filter(|e| e.as_object().is_some_and(|o| o.get("error").is_none_or(Value::is_null))).count();
    let granted = said.min(asked);
    Shared { granted, failed: asked.saturating_sub(granted) }
}
