// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! A message's attachments: the MIME parts that are files, and the files inside an Outlook
//! `winmail.dat` (issue #18), listed and read by one index.

use std::borrow::Cow;

use super::*;
use tnef::{Budget, is_tnef, read_with, shown_cids};
pub use tnef::{
    Tnef, TnefBody, TnefFile, byte_code_page, decode_code_page, decompress_rtf, known_code_page, read_tnef, rtf_body, rtf_wraps_html,
    unsupported_code_page, utf16_code_page,
};

mod index_text;
mod text_first;
mod tnef;
pub use index_text::{IndexDoc, MAX_INDEX_CHARS, index_doc};
#[doc(hidden)]
pub use index_text::{IndexStats, index_doc_stats};
pub use text_first::{MOST_PENDING_FILES, PendingFile, is_protected_file, parse_text_first, part_name, text_first};

/// A part's size: its bytes, or, with none yet (a message's text that came first), the size its
/// `Content-Disposition` gives (RFC 2183's `size`).
fn shown_size(p: &mail_parser::MessagePart<'_>) -> usize {
    if !p.is_empty() {
        return p.len();
    }
    p.content_disposition().and_then(|d| d.attribute("size")).and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(0)
}

/// One of a message's attachments, with its bytes.
pub(super) struct Attached<'a> {
    pub(super) info: AttachmentInfo,
    pub(super) data: Cow<'a, [u8]>,
    /// The MIME part it is (none for a file in a `winmail.dat`), for [`sent_bytes`].
    pub(super) part: Option<&'a mail_parser::MessagePart<'a>>,
}

/// A `winmail.dat` part as what it gives in its place: `None` when it stays one (it can't be read,
/// or holds nothing). Only the first body is read (`body`): a later one is noted, not read.
fn read_part(contents: &[u8], budget: &mut Budget, body: bool) -> Option<tnef::Tnef> {
    if !budget.files_left() {
        return None;
    }
    read_with(contents, budget, body).filter(|t| !t.attachments.is_empty() || !t.body.is_empty() || t.body_left)
}

fn part_mime(p: &mail_parser::MessagePart<'_>) -> String {
    p.content_type()
        .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")).to_ascii_lowercase())
        .unwrap_or_else(|| "application/octet-stream".into())
}

/// A message's attachments in order, a `winmail.dat` given as the files in it (it stays one
/// when it can't be read, or holds nothing, and is listed too when the reading's budget left
/// some of it out), and the body the first such file carries. One budget for all of them.
pub(super) fn attached<'a>(m: &'a Message<'a>) -> (Vec<Attached<'a>>, TnefBody) {
    // Each part, with the files a winmail.dat gives in its place (and whether it stays listed
    // too: files left out for want of budget), read before any is listed: the body's pictures
    // are then looked for once, among the files of every part.
    let mut parts = Vec::new();
    let mut body = TnefBody::None;
    let mut budget = Budget::new();
    for p in m.attachments() {
        let mime = part_mime(p);
        budget.cut = false;
        let files = match is_tnef(&mime, p.attachment_name()) {
            true => read_part(p.contents(), &mut budget, body.is_empty()),
            false => None,
        }
        .map(|mut t| {
            if body.is_empty() {
                body = std::mem::take(&mut t.body);
            }
            t.attachments
        });
        let listed = files.is_none() || budget.cut;
        parts.push((p, mime, files, listed));
    }
    // A picture the body shows is in it, whether or not Outlook marked it hidden.
    let shown = shown_cids(&body, parts.iter().flat_map(|(_, _, files, _)| files.iter().flatten().filter_map(|f| f.content_id.as_deref())));
    let mut out = Vec::new();
    for (p, mime, files, listed) in parts {
        for f in files.into_iter().flatten() {
            let index = out.len();
            let inline = f.content_id.as_deref().is_some_and(|c| f.hidden || shown.contains(c));
            let name = attachment_name(Some(&f.name), &f.mime, &f.data, index);
            let info = AttachmentInfo { index, name, mime: f.mime, size: f.data.len(), inline, content_id: f.content_id, content_location: None };
            out.push(Attached { info, data: Cow::Owned(f.data), part: None });
        }
        if !listed {
            continue;
        }
        let index = out.len();
        let name = attachment_name(p.attachment_name(), &mime, p.contents(), index);
        let info = AttachmentInfo {
            index,
            name,
            mime,
            size: shown_size(p),
            inline: p.is_inline_part(),
            content_id: p.content_id().map(|c| c.trim().trim_matches(['<', '>']).to_string()),
            content_location: p.content_location().map(|c| c.trim().trim_matches(['<', '>', '"']).to_string()).filter(|c| !c.is_empty()),
        };
        out.push(Attached { info, data: Cow::Borrowed(p.contents()), part: Some(p) });
    }
    (out, body)
}

/// Whether a message carries a file: the paperclip, "Has files" and `has:attachment`, which agree
/// with search's count ([`index_doc`]) by the same rule: a part is a file unless it is S/MIME's
/// own, or a part the HTML shows in its place ([`may_be_shown`], [`HtmlRefs::shows`]), whatever
/// its name says (Outlook names its pictures `image001.png`; Apple Mail's inline PDF has no
/// `Content-ID` and is a file). A winmail.dat counts by what it gives in its place, as
/// `attached` lists it ([`tnef_files`]): its files, or itself when it stays one; one holding only
/// the body gives none (no paperclip over an empty list). No body of it is read to count them.
/// A data file's import says the same of a message before it is built ([`draft_carries_files`],
/// by the same [`carries_file`]).
pub(super) fn has_real_attachments(m: &Message<'_>) -> bool {
    let refs = HtmlRefs::new(|| message_html(m));
    let mut budget = Budget::new();
    m.attachments().any(|p| {
        let shown = || may_be_shown(p).is_some_and(|(cid, location)| refs.shows(cid, location));
        !is_smime(p) && carries_file(is_tnef(&part_mime(p), p.attachment_name()), p.contents(), &shown, &mut budget)
    })
}

/// Whether one of a message's non-text parts that isn't S/MIME's own is a file it carries: a
/// winmail.dat (`tnef`) only when it gives a file in its place (read within `budget`), anything
/// else unless it is a picture the HTML shows (`shown`).
fn carries_file(tnef: bool, data: &[u8], shown: &dyn Fn() -> bool, budget: &mut Budget) -> bool {
    if tnef {
        budget.cut = false;
        return !matches!(read_part(data, budget, false), Some(t) if !budget.cut && tnef_files(&t).next().is_none());
    }
    !shown()
}

/// Whether the message [`build`](crate::build) makes of `d` carries a file, as the paperclip,
/// "Has files" and `has:attachment` read it back from the source ([`has_real_attachments`], by
/// the same [`carries_file`]): a data file's import knows it before it writes the source, and
/// doesn't parse it again. Its files are files unless they are S/MIME's own part or a
/// `winmail.dat` that gives none; its pictures go in only beside HTML, and are files when the
/// HTML doesn't show them.
pub fn draft_carries_files(d: &Draft) -> bool {
    let refs = HtmlRefs::new(|| d.html.clone().unwrap_or_default());
    let mut budget = Budget::new();
    // A part as `build` writes it: its type (from its name when it has none), the name it is
    // read back with, and whether it is S/MIME's own or a winmail.dat.
    let kind = |mime: &str, name: Option<&str>| {
        let mime = if mime.is_empty() { mime_from_name(name.unwrap_or_default()).to_ascii_lowercase() } else { mime.to_ascii_lowercase() };
        (smime::is_smime_part(&mime, name.unwrap_or_default()), is_tnef(&mime, name))
    };
    let file = |mime: &str, name: Option<&str>, data: &[u8], shown: &dyn Fn() -> bool, budget: &mut Budget| {
        let (smime, tnef) = kind(mime, name);
        !smime && carries_file(tnef, data, shown, budget)
    };
    d.attachments.iter().any(|a| file(&a.mime, Some(&sanitize_filename(&a.name)), &a.data, &|| false, &mut budget))
        || (d.html.is_some() && d.inline.iter().any(|p| file(&p.mime, None, &p.data, &|| refs.shows(Some(&p.cid), None), &mut budget)))
}

/// The files of a `winmail.dat` that count as files: not the pictures Outlook marks as its
/// body's (hidden, with a Content-ID), as a MIME part the HTML shows isn't one.
pub(super) fn tnef_files(t: &Tnef) -> impl Iterator<Item = &TnefFile> {
    t.attachments.iter().filter(|f| !(f.hidden && f.content_id.is_some()))
}

/// Most of a message's HTML looked through for the pictures it shows.
const SHOWN_HTML_MAX: usize = 4 << 20;

/// A message's real `text/html` bodies, the first [`SHOWN_HTML_MAX`] bytes of them.
fn message_html(m: &Message<'_>) -> String {
    let mut out = String::new();
    for t in m.html_bodies().filter(|b| b.is_text_html()).filter_map(|b| b.text_contents()) {
        push_capped(&mut out, t);
    }
    out
}

/// `t` joins `out` on a line of its own, `out` kept to [`SHOWN_HTML_MAX`] bytes.
pub(super) fn push_capped(out: &mut String, t: &str) {
    if !out.is_empty() {
        out.push('\n');
    }
    let room = SHOWN_HTML_MAX.saturating_sub(out.len());
    let end = (0..=room.min(t.len())).rev().find(|i| t.is_char_boundary(*i)).unwrap_or(0);
    out.push_str(t.get(..end).unwrap_or_default());
}

/// Most references kept from one message's HTML.
const MAX_REFS: usize = 10_000;
/// Longest reference read (a `cid:` or an address; the reading pane matches none longer).
const REF_MAX: usize = 4096;

/// The pictures a message's HTML shows, by `cid:` or by the address a part's `Content-Location`
/// gives it (RFC 2557): read from the HTML only when a part asks, once.
pub struct HtmlRefs<F: Fn() -> String> {
    refs: std::cell::OnceCell<Refs>,
    make: F,
}

/// What an HTML body refers to, lower case: every `cid:` (up to its closing quote, bracket or
/// space), and every `src`, `href` and `background` address, whole and by its last segment.
#[derive(Default)]
struct Refs {
    cids: std::collections::HashSet<String>,
    paths: std::collections::HashSet<String>,
    files: std::collections::HashSet<String>,
}

impl Refs {
    /// One pass over the HTML: each reference is read at most [`REF_MAX`] bytes and the search
    /// goes on after it, so the time stays linear however the HTML is made.
    fn read(html: &str) -> Refs {
        let lower = html.as_bytes().to_ascii_lowercase();
        let mut r = Refs::default();
        let stop = |b: &u8| b.is_ascii_whitespace() || b"\"'()<>&;,".contains(b);
        let window = |from: usize| {
            let rest = lower.get(from..).unwrap_or_default();
            rest.get(..rest.len().min(REF_MAX + 1)).unwrap_or_default()
        };
        let mut at = 0;
        while let Some(i) = memchr::memmem::find(lower.get(at..).unwrap_or_default(), b"cid:") {
            let mut start = at + i + 4;
            while lower.get(start) == Some(&b'<') && start < at + i + 4 + REF_MAX {
                start += 1;
            }
            let w = window(start);
            let end = w.iter().position(stop).unwrap_or(w.len());
            if (1..=REF_MAX).contains(&end) && r.cids.len() < MAX_REFS {
                let cid = String::from_utf8_lossy(w.get(..end).unwrap_or_default()).into_owned();
                r.cids.insert(percent_decoded(&cid));
                r.cids.insert(cid);
            }
            at = start + end;
        }
        let space = |from: usize| from + lower.get(from..).unwrap_or_default().iter().take_while(|b| b.is_ascii_whitespace()).count();
        for attr in [&b"src"[..], b"href", b"background"] {
            let mut from = 0;
            while let Some(i) = memchr::memmem::find(lower.get(from..).unwrap_or_default(), attr) {
                let at = from + i;
                from = at + attr.len();
                if at == 0 || !lower.get(at - 1).is_some_and(|b| b.is_ascii_whitespace()) {
                    continue;
                }
                let eq = space(from);
                if lower.get(eq) != Some(&b'=') {
                    from = eq;
                    continue;
                }
                let start = space(eq + 1);
                let (value, next) = match lower.get(start) {
                    Some(&q @ (b'"' | b'\'')) => {
                        let w = window(start + 1);
                        let end = w.iter().position(|b| *b == q).unwrap_or(w.len());
                        (w.get(..end).unwrap_or_default(), start + 1 + end)
                    }
                    _ => {
                        let w = window(start);
                        let end = w.iter().position(|b| b.is_ascii_whitespace() || *b == b'>').unwrap_or(w.len());
                        (w.get(..end).unwrap_or_default(), start + end)
                    }
                };
                from = next.max(from);
                if value.len() > REF_MAX || r.paths.len() >= MAX_REFS {
                    continue;
                }
                let value = String::from_utf8_lossy(value);
                let path = percent_decoded(value.split(['?', '#']).next().unwrap_or(&value).trim());
                if !path.is_empty() {
                    if !last_segment(&path).is_empty() {
                        r.files.insert(last_segment(&path).to_string());
                    }
                    r.paths.insert(path);
                }
            }
        }
        r
    }
}

/// A path's last segment (`chart.png` of `http://studio.example/a/chart.png`).
fn last_segment(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// `%xx` escapes decoded.
fn percent_decoded(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match (c, b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl<F: Fn() -> String> HtmlRefs<F> {
    /// The references of the HTML `make` gives, read the first time a part asks.
    pub fn new(make: F) -> HtmlRefs<F> {
        HtmlRefs { refs: std::cell::OnceCell::new(), make }
    }

    /// Whether the HTML refers to this `Content-ID` by a `cid:` (whatever its case). A file a
    /// message is built with goes in its body only then: an address whose last segment is the
    /// same (a link to `plan.pdf`, and a file whose Content-ID is `plan.pdf`) doesn't show it.
    pub fn cites(&self, cid: &str) -> bool {
        let r = self.refs.get_or_init(|| Refs::read(&(self.make)()));
        let cid = cid.trim().trim_matches(['<', '>']).to_ascii_lowercase();
        !cid.is_empty() && r.cids.contains(&cid)
    }

    /// Whether the HTML shows the part with this `Content-ID` or `Content-Location`, matched as
    /// the reading pane finds a picture ([`picture_attachment`]): the whole `cid:` (whatever its
    /// case; `cid:image10` doesn't show `image1`), the address exactly or by its last segment.
    pub fn shows(&self, cid: Option<&str>, location: Option<&str>) -> bool {
        let r = self.refs.get_or_init(|| Refs::read(&(self.make)()));
        let cid = cid.map(|c| c.trim().trim_matches(['<', '>']).to_ascii_lowercase()).filter(|c| !c.is_empty());
        let location = location.map(|l| l.trim().trim_matches(['<', '>', '"']).to_ascii_lowercase()).filter(|l| !l.is_empty());
        cid.is_some_and(|c| r.cids.contains(&c) || r.files.contains(&c))
            || location.is_some_and(|l| r.paths.contains(&l) || r.files.contains(last_segment(&l)) || r.cids.contains(&l))
    }
}

/// A part's type as `type/subtype`, lower case.
pub(super) fn mime_of(p: &mail_parser::MessagePart<'_>) -> Option<String> {
    p.content_type().map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or_default()).to_ascii_lowercase())
}

/// S/MIME's signature and encrypted content are the message itself, not files in it.
pub(super) fn is_smime(p: &mail_parser::MessagePart<'_>) -> bool {
    smime::is_smime_part(&mime_of(p).unwrap_or_default(), p.attachment_name().unwrap_or_default())
}

/// The `Content-ID` and `Content-Location` of a part the HTML may show in its place: one inline
/// or without a disposition, that has either.
pub(super) fn may_be_shown<'p>(p: &'p mail_parser::MessagePart<'_>) -> Option<(Option<&'p str>, Option<&'p str>)> {
    let inline = p.content_disposition().is_none_or(|d| d.ctype().eq_ignore_ascii_case("inline"));
    let (cid, location) = (p.content_id(), p.content_location());
    (inline && (cid.is_some() || location.is_some())).then_some((cid, location))
}

pub(super) trait InlinePart {
    fn is_inline_part(&self) -> bool;
}

impl InlinePart for mail_parser::MessagePart<'_> {
    /// Inline images referenced from the HTML (`Content-ID`) don't count as attachments.
    fn is_inline_part(&self) -> bool {
        self.content_id().is_some() && self.content_disposition().is_none_or(|d| d.ctype().eq_ignore_ascii_case("inline"))
    }
}

/// An attachment as the reading pane lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentInfo {
    /// Position among the message's attachments (for [`attachment_data`]).
    pub index: usize,
    pub name: String,
    pub mime: String,
    pub size: usize,
    pub inline: bool,
    /// `Content-ID` without angle brackets, for pictures the HTML shows (`cid:`).
    #[serde(default)]
    pub content_id: Option<String>,
    /// `Content-Location` (RFC 2557): the address the HTML may show it by instead
    /// (`<img src="logo.png">`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_location: Option<String>,
}

impl AttachmentInfo {
    /// What kind of preview the UI can offer.
    pub fn kind(&self) -> AttachmentKind {
        kind_of(&self.mime, &self.name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentKind {
    Image,
    Video,
    Audio,
    Pdf,
    Text,
    Archive,
    Document,
    Other,
}

pub fn kind_of(mime: &str, name: &str) -> AttachmentKind {
    let mime = mime.to_ascii_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    // Sent as `application/octet-stream`, a picture this can show is still one (by its name).
    if mime.starts_with("image/") || ["png", "jpg", "jpeg", "gif", "webp", "bmp"].contains(&ext.as_str()) {
        AttachmentKind::Image
    } else if mime.starts_with("video/") {
        AttachmentKind::Video
    } else if mime.starts_with("audio/") {
        AttachmentKind::Audio
    } else if mime == "application/pdf" || ext == "pdf" {
        AttachmentKind::Pdf
    } else if mime.starts_with("text/") {
        AttachmentKind::Text
    } else if ["zip", "7z", "rar", "gz", "tar"].contains(&ext.as_str()) {
        AttachmentKind::Archive
    } else if ["docx", "doc", "odt", "xlsx", "xls", "ods", "pptx", "ppt", "odp", "rtf", "craft"].contains(&ext.as_str()) {
        AttachmentKind::Document
    } else {
        AttachmentKind::Other
    }
}

/// The Craft app that edits files of this kind (by its command name), when there is one.
pub fn craft_app_for(mime: &str, name: &str) -> Option<(&'static str, &'static str)> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let app = match kind_of(mime, name) {
        AttachmentKind::Pdf => ("pdfcraft", "PdfCraft"),
        AttachmentKind::Image => ("photocraft", "PhotoCraft"),
        AttachmentKind::Video => ("filmcraft", "FilmCraft"),
        AttachmentKind::Audio => ("soundcraft", "SoundCraft"),
        _ => match ext.as_str() {
            "docx" | "doc" | "odt" | "rtf" | "dotx" => ("wordcraft", "WordCraft"),
            "xlsx" | "xls" | "ods" | "csv" | "xlsm" => ("gridcraft", "GridCraft"),
            "pptx" | "ppt" | "odp" | "ppsx" => ("deckcraft", "DeckCraft"),
            _ => return None,
        },
    };
    Some(app)
}

/// Whether an attachment can be shown as text (and how: `message` for an attached email).
pub fn text_preview_kind(mime: &str, name: &str) -> Option<&'static str> {
    let m = mime.to_ascii_lowercase();
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    if m == "message/rfc822" || ext == "eml" {
        return Some("message");
    }
    let texty = m.starts_with("text/")
        || ["application/json", "application/xml", "application/x-yaml", "application/javascript"].contains(&m.as_str())
        || [
            "txt", "md", "csv", "tsv", "json", "xml", "yaml", "yml", "log", "ics", "vcf", "ini", "toml", "rs", "py", "js", "ts", "html", "css", "sh",
            "srt", "vtt",
        ]
        .contains(&ext.as_str());
    texty.then_some("text")
}

/// Most pictures a reply or forward takes from the message it quotes, and bytes each.
const QUOTED_PICTURES: usize = 100;
const QUOTED_PICTURE_MAX: usize = 25 << 20;

/// The pictures of a message as a reply or forward quotes it (issue #10: they came out as
/// "[picture]"): each one in the message (by `cid:`, by its `Content-Location` or name, or written
/// into the HTML as `data:`) given by `cid:`, with its bytes for the reply to send; one on the web
/// kept by its address, as Outlook keeps it (never loaded here); tracking pictures left out.
/// Also the indexes of the attachments the quote shows.
pub fn quoted_pictures(p: &mut Parsed, raw: &[u8]) -> (Vec<InlinePicture>, Vec<usize>) {
    let mut out: Vec<InlinePicture> = Vec::new();
    let mut used: Vec<usize> = Vec::new();
    let found: Vec<(String, Option<usize>)> = p
        .rich
        .iter()
        .flatten()
        .filter_map(|s| s.image.clone())
        .map(|src| {
            let at = picture_attachment(p, &src);
            (src, at)
        })
        .collect();
    let trackers = p.trackers.pictures.clone();
    // The message read once for all its pictures (a winmail.dat's files are made each time it's
    // read), and only when one of them is among its attachments.
    let message = found.iter().any(|(_, at)| at.is_some()).then(|| MessageParser::default().parse(raw)).flatten();
    let files = message.as_ref().map(|m| attached(m).0).unwrap_or_default();
    let mut given: Vec<(String, String)> = Vec::new();
    for (src, at) in found {
        if given.iter().any(|(s, _)| *s == src) || out.len() >= QUOTED_PICTURES {
            continue;
        }
        let picture = match at.and_then(|i| p.attachments.get(i)) {
            Some(a) if a.size <= QUOTED_PICTURE_MAX => files.get(a.index).map(|f| f.data.to_vec()).map(|data| {
                let cid = a
                    .content_id
                    .clone()
                    .filter(|c| !c.is_empty())
                    .unwrap_or_else(|| format!("quoted-{}-{:012x}@quatrelle", a.index, fnv(&src) & 0xFFFF_FFFF_FFFF));
                InlinePicture { cid, name: a.name.clone(), mime: a.mime.clone(), data }
            }),
            Some(_) => None,
            None => data_url(&src).map(|(mime, data)| {
                let n = out.len() + 1;
                let ext = Some(files::ext_for_mime(&mime)).filter(|e| !e.is_empty()).unwrap_or(".png");
                InlinePicture {
                    cid: format!("quoted{n}-{:012x}@quatrelle", fnv(&src) & 0xFFFF_FFFF_FFFF),
                    name: format!("image{n:03}{ext}"),
                    mime,
                    data,
                }
            }),
        };
        if let Some(pic) = picture {
            used.extend(at);
            given.push((src, pic.cid.clone()));
            if !out.iter().any(|q| q.cid == pic.cid) {
                out.push(pic);
            }
        }
    }
    for span in p.rich.iter_mut().flatten() {
        let Some(src) = span.image.clone() else { continue };
        if let Some((_, cid)) = given.iter().find(|(s, _)| *s == src) {
            span.image = Some(format!("cid:{cid}"));
        } else if trackers.contains(&src) {
            span.image = None;
            span.text.clear();
        }
    }
    (out, used)
}

/// A picture written into HTML as `data:image/…;base64,…`: its type and bytes.
fn data_url(src: &str) -> Option<(String, Vec<u8>)> {
    let rest = src.trim().get(..11).filter(|p| p.eq_ignore_ascii_case("data:image/")).and_then(|_| src.trim().get(5..))?;
    let (kind, b64) = rest.split_once(',')?;
    let mime = kind.strip_suffix(";base64").or_else(|| kind.strip_suffix(";BASE64"))?.to_ascii_lowercase();
    if !["image/png", "image/jpeg", "image/jpg", "image/gif", "image/webp", "image/bmp"].contains(&mime.as_str())
        || b64.len() > QUOTED_PICTURE_MAX / 3 * 4 + 8
    {
        return None;
    }
    let clean: Vec<u8> = b64.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let data = mail_parser::decoders::base64::base64_decode(&clean).filter(|d| !d.is_empty())?;
    Some((if mime == "image/jpg" { "image/jpeg".into() } else { mime }, data))
}

/// The bytes of attachment `index`.
pub fn attachment_data(raw: &[u8], index: usize) -> Option<Vec<u8>> {
    attachments_data(raw, &[index], usize::MAX, usize::MAX).pop().map(|(_, d)| d)
}

/// Attachment `index` as text to show: a text file read in the charset it names (its bytes,
/// saved as they came, are [`attachment_data`]'s).
pub fn attachment_text(raw: &[u8], index: usize) -> Option<String> {
    let m = MessageParser::default().parse(raw)?;
    let a = attached(&m).0.into_iter().find(|a| a.info.index == index)?;
    Some(String::from_utf8_lossy(&a.data).into_owned())
}

/// Attachment `index` to send again (a forward, an Outbox message taken back to change): its
/// bytes as they came ([`attachment_data`]'s), and a text file's type with the charset it names
/// (`text/plain; charset=windows-1252`), so the copy is read as the original was; without it, a
/// file in Latin-1 went out as bare `text/plain` and was read as UTF-8.
pub fn attachment_out(raw: &[u8], index: usize) -> Option<crate::OutAttachment> {
    let m = MessageParser::default().parse(raw)?;
    let a = attached(&m).0.into_iter().find(|a| a.info.index == index)?;
    let sent = a.part.and_then(|p| sent_bytes(&m, p));
    let charset =
        a.part.filter(|_| sent.is_some() && a.info.mime.starts_with("text/")).and_then(|p| charset_token(p.content_type()?.attribute("charset")?));
    let mime = match charset {
        Some(c) => format!("{}; charset={c}", a.info.mime),
        None => a.info.mime.clone(),
    };
    let data = sent.unwrap_or_else(|| a.data.into_owned());
    Some(crate::OutAttachment { name: a.info.name, mime, data })
}

/// A charset's name as it can go into a `Content-Type`: a token (RFC 2045 §5.1) of at most 64
/// bytes, so it can't add to the header it goes into.
fn charset_token(c: &str) -> Option<&str> {
    let c = c.trim().trim_matches('"');
    let ok = |b: u8| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`{|}~".contains(&b);
    (!c.is_empty() && c.len() <= 64 && c.bytes().all(ok)).then_some(c)
}

/// A text file's bytes as they were sent. The parser hands every `text/*` part out as text, read
/// in its charset or, with none, as UTF-8 with any other byte replaced, so a file in Latin-1 or
/// Windows-1252 was saved changed. They're taken again from the message's source, undoing only
/// the transfer encoding, with the parser's own decoders (a part it could not decode is its
/// source as it is). `None` for other parts, whose bytes are already these, and when the source
/// has no such range.
fn sent_bytes(m: &Message<'_>, p: &mail_parser::MessagePart<'_>) -> Option<Vec<u8>> {
    if !matches!(p.body, mail_parser::PartType::Text(_) | mail_parser::PartType::Html(_)) {
        return None;
    }
    let body = m.raw_message.get(p.offset_body as usize..p.offset_end as usize)?;
    let mut s = mail_parser::parsers::MessageStream::new(body);
    let (end, bytes) = match p.encoding {
        mail_parser::Encoding::Base64 => s.decode_base64_mime(b""),
        mail_parser::Encoding::QuotedPrintable => s.decode_quoted_printable_mime(b""),
        mail_parser::Encoding::None => return Some(body.to_vec()),
    };
    (end != usize::MAX).then(|| bytes.into_owned())
}

/// Most attachments [`attachments_data`] looks for at once.
pub const MAX_WANTED: usize = 256;

/// The bytes of several attachments from one parse (a message with 36 pictures was read and
/// parsed 36 times to show their thumbnails). Indexes are [`attachment_data`]'s; they're taken
/// in the order `wanted` lists them (the first [`MAX_WANTED`], each once), each at most
/// `max_each` bytes and all together at most `max_total`: one past either cap is left out, and
/// so is an index the message doesn't have.
pub fn attachments_data(raw: &[u8], wanted: &[usize], max_each: usize, max_total: usize) -> Vec<(usize, Vec<u8>)> {
    let mut order: Vec<usize> = Vec::new();
    for i in wanted {
        if order.len() >= MAX_WANTED {
            break;
        }
        if !order.contains(i) {
            order.push(*i);
        }
    }
    if order.is_empty() {
        return Vec::new();
    }
    let Some(m) = MessageParser::default().parse(raw) else { return Vec::new() };
    let mut sorted = order.clone();
    sorted.sort_unstable();
    let mut found: Vec<(usize, Cow<'_, [u8]>)> = attached(&m)
        .0
        .into_iter()
        .filter(|a| sorted.binary_search(&a.info.index).is_ok())
        .map(|a| (a.info.index, a.part.and_then(|p| sent_bytes(&m, p)).map_or(a.data, Cow::Owned)))
        .collect();
    let mut out = Vec::new();
    let mut total = 0usize;
    for i in order {
        let Some(at) = found.iter().position(|(j, _)| *j == i) else { continue };
        let (_, data) = found.swap_remove(at);
        if data.len() > max_each {
            continue;
        }
        match total.checked_add(data.len()) {
            Some(t) if t <= max_total => total = t,
            _ => continue,
        }
        out.push((i, data.into_owned()));
    }
    out
}
