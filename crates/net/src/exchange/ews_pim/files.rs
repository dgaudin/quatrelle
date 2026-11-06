// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An appointment's files (R25), as Outlook's Cached Exchange Mode has them: each appointment read
//! lists its files and attached items (`item:Attachments`, names, types and sizes, never their
//! bytes); those of the account's own calendar are kept with it, their bytes asked of the server
//! a few at a time (`GetAttachment`), as the event's `ATTACH` lines a `.pst` import writes; the
//! others are listed (`X-QUATRELLE-SERVER-ATTACH`, [`quatrelle_model::pim::files`]) and fetched
//! when opened ([`attachment`]), as Outlook fetches a file not there yet.
//!
//! Outlook keeps every file in its cache, a database read by pieces; here an account's calendar
//! is one document, held in memory and written whole at each change, so what's kept is held to
//! [`MAX_FILE`] a file and [`MAX_EVENT_FILES`] an appointment: an agenda, minutes, a plan or a
//! picture are kept, a presentation or a video comes when it's opened. Other people's calendars
//! and the public folders, read whole and set aside away from the calendar, only list theirs.
//!
//! An appointment changed on the server is read again whole, its files with it: one taken off
//! there goes from here, one added comes.

use std::collections::HashMap;

use quatrelle_model::pim::files::server_file_line;

use super::*;

/// A file of the account's own appointment kept with it at most, in bytes…
pub(crate) const MAX_FILE: usize = 1 << 20;
/// …and all of one appointment's together.
pub(crate) const MAX_EVENT_FILES: usize = 2 << 20;
/// Bytes and attachments asked of the server in one `GetAttachment`, at most.
const ASK_BYTES: usize = 8 << 20;
const ASK_IDS: usize = 20;

/// Asks the server for an attached item's source, as well as a file's bytes.
const WITH_SOURCE: &str = "<m:AttachmentShape><t:IncludeMimeContent>true</t:IncludeMimeContent></m:AttachmentShape>";

/// A file an appointment lists: its id on the server, name, type and size, and whether it's an
/// attached item (a message, an appointment), kept as its source (`.eml`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Listed {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: usize,
    pub item: bool,
}

/// The files an item lists: its files and attached items, not the pictures inside its text (with
/// a content id, marked inline) nor a link to a file elsewhere (`ReferenceAttachment`). At most as
/// many as an event lists.
pub(super) fn listed(it: &El) -> Vec<Listed> {
    let Some(list) = it.child("Attachments") else { return Vec::new() };
    list.kids
        .iter()
        .filter_map(|a| {
            let item = match a.name.as_str() {
                "FileAttachment" => false,
                "ItemAttachment" => true,
                _ => return None,
            };
            let inline = a.text_of("IsInline").is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
                && a.text_of("ContentId").is_some_and(|c| !c.trim().is_empty());
            if inline {
                return None;
            }
            let id = a.child("AttachmentId")?.attr("Id")?.trim();
            if id.is_empty() {
                return None;
            }
            let name = a.text_of("Name").unwrap_or_default().trim();
            let (name, mime) = if item {
                (item_name(name), "message/rfc822".to_string())
            } else {
                (name.to_string(), a.text_of("ContentType").unwrap_or_default().trim().to_string())
            };
            let size = a.text_of("Size").and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(0);
            Some(Listed { id: id.to_string(), name, mime, size, item })
        })
        .take(quatrelle_model::pim::files::MAX_FILES)
        .collect()
}

/// An attached item's file name: its subject, as a message saved (`.eml`).
fn item_name(subject: &str) -> String {
    let base = if subject.is_empty() { "message" } else { subject };
    if base.to_ascii_lowercase().ends_with(".eml") { base.to_string() } else { format!("{base}.eml") }
}

/// The files of the appointments just read (`lists`: each event's place in `out.events`, and what
/// its item lists) written into them: kept (`keep`, the account's own calendar), in their order,
/// those within [`MAX_FILE`] and [`MAX_EVENT_FILES`], their bytes asked of the server; listed, the
/// others, and any the server won't give (refused, gone, bigger than it said).
pub(super) fn fill(c: &Client, out: &mut DavSync, lists: Vec<(usize, Vec<Listed>)>, keep: bool) {
    if lists.is_empty() {
        return;
    }
    let mut wanted: Vec<&Listed> = Vec::new();
    if keep {
        for (_, list) in &lists {
            let mut room = MAX_EVENT_FILES;
            for f in list.iter().filter(|f| f.size <= MAX_FILE) {
                if f.size <= room {
                    room -= f.size;
                    wanted.push(f);
                }
            }
        }
    }
    let got = download(c, &wanted);
    let (mut kept, mut listed) = (0usize, 0usize);
    for (i, list) in &lists {
        let Some(e) = out.events.get_mut(*i) else { continue };
        let mut room = MAX_EVENT_FILES;
        let mut lines = Vec::new();
        for f in list {
            // What the server gave, when it fits (it may give more than it said).
            match got.get(&f.id).filter(|d| d.len() <= MAX_FILE && d.len() <= room) {
                Some(data) => {
                    room -= data.len();
                    kept += 1;
                    lines.push(crate::pst_pim::event_file(&f.name, &f.mime, data));
                }
                None => {
                    listed += 1;
                    lines.extend(server_file_line(&f.name, &f.mime, u64::try_from(f.size).unwrap_or(u64::MAX), &f.id));
                }
            }
        }
        // Before any part of the event (an alarm), as an import puts them.
        let at = e
            .event
            .extra
            .iter()
            .position(|l| l.trim_start().get(..6).is_some_and(|b| b.eq_ignore_ascii_case("BEGIN:")))
            .unwrap_or(e.event.extra.len());
        e.event.extra.splice(at..at, lines);
    }
    log::info!("appointments' files: {kept} kept with them, {listed} left on the server until opened");
}

/// The bytes of `wanted`, asked a few at a time; those the server gives, by id. One that fails
/// leaves its files on the server (fetched when opened); the connection lost ends it.
fn download(c: &Client, wanted: &[&Listed]) -> HashMap<String, Vec<u8>> {
    let mut got = HashMap::new();
    let mut rest = wanted;
    while !rest.is_empty() {
        let (mut n, mut bytes) = (0usize, 0usize);
        while let Some(f) = rest.get(n) {
            if n > 0 && (n >= ASK_IDS || bytes.saturating_add(f.size) > ASK_BYTES) {
                break;
            }
            bytes = bytes.saturating_add(f.size);
            n += 1;
        }
        let (group, after) = rest.split_at(n.min(rest.len()));
        rest = after;
        match ask(c, group) {
            Ok(list) => got.extend(list),
            Err(e) => {
                log::info!("{} appointments' files not given ({e}): left on the server until opened", group.len());
                if e.is_fatal() {
                    break;
                }
            }
        }
    }
    got
}

/// One `GetAttachment` for `group`: the bytes of each the server gives, by id (its answers come in
/// the order asked).
fn ask(c: &Client, group: &[&Listed]) -> Result<Vec<(String, Vec<u8>)>> {
    let ids: String = group.iter().map(|f| format!("<t:AttachmentId Id=\"{}\"/>", esc(&f.id))).collect();
    let shape = if group.iter().any(|f| f.item) { WITH_SOURCE } else { "" };
    let doc = c.call(&format!("<m:GetAttachment>{shape}<m:AttachmentIds>{ids}</m:AttachmentIds></m:GetAttachment>"))?;
    let mut out = Vec::new();
    for (m, f) in crate::ews::messages(&doc).into_iter().zip(group) {
        if m.attr("ResponseClass") == Some("Error") {
            log::info!("{}: not given ({})", f.name, m.text_of("ResponseCode").unwrap_or("no code").trim());
            continue;
        }
        if let Some(data) = bytes_of(m) {
            out.push((f.id.clone(), data));
        }
    }
    Ok(out)
}

/// An attachment's bytes in an answer of `GetAttachment`: a file's content, an attached item's
/// source.
fn bytes_of(m: &El) -> Option<Vec<u8>> {
    let a = m.child("Attachments")?.kids.first()?;
    let b64 = match a.name.as_str() {
        "FileAttachment" => a.text_of("Content")?,
        "ItemAttachment" => a.find("MimeContent").map(|c| c.text.as_str())?,
        _ => return None,
    };
    let b64: String = b64.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    crate::ews::unb64(&b64)
}

/// One of an appointment's files, fetched when it's opened (`id`: what the server knows it by): a
/// file's bytes, or an attached item's source.
pub fn attachment(c: &Client, id: &str) -> Result<Vec<u8>> {
    let doc =
        c.call(&format!("<m:GetAttachment>{WITH_SOURCE}<m:AttachmentIds><t:AttachmentId Id=\"{}\"/></m:AttachmentIds></m:GetAttachment>", esc(id)))?;
    let m = crate::ews::messages(&doc).first().copied().ok_or_else(|| NetError::Protocol("an empty answer".into()))?;
    crate::ews::check_message(m)?;
    bytes_of(m).ok_or_else(|| NetError::No("the server gave nothing for this file".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(list: &str) -> El {
        crate::ews::parse(&format!("<t:CalendarItem><t:Attachments>{list}</t:Attachments></t:CalendarItem>")).kids.remove(0)
    }

    #[test]
    fn an_items_list_gives_its_files_not_its_pictures_nor_links() {
        let it = item(
            "<t:FileAttachment><t:AttachmentId Id=\"A1\"/><t:Name>Agenda.pdf</t:Name><t:ContentType>application/pdf</t:ContentType><t:Size>3000</t:Size><t:IsInline>false</t:IsInline></t:FileAttachment>\
             <t:FileAttachment><t:AttachmentId Id=\"A2\"/><t:Name>logo.png</t:Name><t:ContentType>image/png</t:ContentType><t:ContentId>logo@x</t:ContentId><t:Size>20</t:Size><t:IsInline>true</t:IsInline></t:FileAttachment>\
             <t:ReferenceAttachment><t:AttachmentId Id=\"A3\"/><t:Name>On the share</t:Name></t:ReferenceAttachment>\
             <t:ItemAttachment><t:AttachmentId Id=\"A4\"/><t:Name>Minutes</t:Name><t:Size>4096</t:Size></t:ItemAttachment>\
             <t:ItemAttachment><t:AttachmentId Id=\"A5\"/><t:Size>lots</t:Size></t:ItemAttachment>\
             <t:FileAttachment><t:Name>no id</t:Name></t:FileAttachment>\
             <t:FileAttachment><t:AttachmentId Id=\"A6\"/><t:Name>marked inline.docx</t:Name><t:IsInline>true</t:IsInline></t:FileAttachment>",
        );
        let got: Vec<(String, String, usize, bool)> = listed(&it).into_iter().map(|f| (f.name, f.mime, f.size, f.item)).collect();
        assert_eq!(
            got,
            [
                ("Agenda.pdf".to_string(), "application/pdf".to_string(), 3000, false),
                ("Minutes.eml".to_string(), "message/rfc822".to_string(), 4096, true),
                ("message.eml".to_string(), "message/rfc822".to_string(), 0, true),
                ("marked inline.docx".to_string(), String::new(), 0, false),
            ]
        );
        let many: String = (0..100).map(|i| format!("<t:FileAttachment><t:AttachmentId Id=\"F{i}\"/></t:FileAttachment>")).collect();
        assert_eq!(listed(&item(&many)).len(), quatrelle_model::pim::files::MAX_FILES);
    }

    #[test]
    fn an_answer_gives_a_files_bytes_or_an_items_source() {
        let answer = |inner: &str| crate::ews::parse(&format!("<m:R><m:Attachments>{inner}</m:Attachments></m:R>")).kids.remove(0);
        assert_eq!(bytes_of(&answer("<t:FileAttachment><t:Content>aGVs\r\nbG8=</t:Content></t:FileAttachment>")).as_deref(), Some(&b"hello"[..]));
        let source = "<t:ItemAttachment><t:Message><t:MimeContent CharacterSet=\"UTF-8\">aGk=</t:MimeContent></t:Message></t:ItemAttachment>";
        assert_eq!(bytes_of(&answer(source)).as_deref(), Some(&b"hi"[..]));
        assert_eq!(bytes_of(&answer("<t:ItemAttachment><t:Message/></t:ItemAttachment>")), None);
        assert_eq!(bytes_of(&answer("<t:FileAttachment><t:Content>not base64!</t:Content></t:FileAttachment>")), None);
    }
}
