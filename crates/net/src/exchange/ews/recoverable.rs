// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! EWS: what the server keeps of the items deleted for good (Recoverable Items, its Deletions
//! folder, which Exchange 2010 SP1 and later let a client read): listed a page at a time,
//! brought back into Deleted Items or deleted for good. Read on the server each time, never kept.

use quatrelle_model::{RECOVERABLE_PAGE, Recoverable, RecoverableKind};

use super::*;

/// The most characters of a subject or sender kept from what the server says.
const TEXT_MAX: usize = 300;

/// Said when the server can't be asked for its Recoverable Items.
const NO_SUPPORT: &str =
    "this Exchange server does not let Quatrelle list the recoverable items (it needs Exchange 2010 SP1 or later, and the mailbox's permission)";

/// A page's size, which the server's own limit (1000) and ours bound.
fn clamp_page(n: usize) -> usize {
    n.clamp(1, RECOVERABLE_PAGE)
}

/// `text` cut to at most `max` characters, at a character boundary.
fn cut(text: &str, max: usize) -> String {
    text.trim().chars().take(max).collect()
}

/// The items of a `FindItem` answer's `Items`: what the server says is hostile, so the count, the
/// text and the numbers are bounded and what lacks an id is skipped.
pub(super) fn recoverable_items(items: &El, max: usize) -> Vec<Recoverable> {
    let mut out = Vec::new();
    for it in items.kids.iter().take(max) {
        let Some(id) = it.child("ItemId").and_then(|i| i.attr("Id")).filter(|i| !i.is_empty() && i.len() <= 2048) else { continue };
        let from = it
            .child("From")
            .or_else(|| it.child("Sender"))
            .map(|f| f.child("Mailbox").unwrap_or(f))
            .map(|f| match (f.text_of("Name"), f.text_of("EmailAddress")) {
                (Some(n), _) if !n.trim().is_empty() => cut(n, TEXT_MAX),
                (_, Some(a)) => cut(a, TEXT_MAX),
                _ => String::new(),
            })
            .unwrap_or_default();
        // `PidTagDeletedOnTime` when the server gives it; otherwise when the item last changed
        // (a deletion changes it), otherwise when it came.
        let mut deleted_on = None;
        let mut props = Vec::new();
        it.all("ExtendedProperty", &mut props);
        for p in props {
            if p.child("ExtendedFieldURI").and_then(|u| u.attr("PropertyTag")).is_some_and(|t| t.eq_ignore_ascii_case("0x668F")) {
                deleted_on = p.text_of("Value").and_then(jmap::utc_date);
            }
        }
        let deleted = deleted_on
            .or_else(|| it.text_of("LastModifiedTime").and_then(jmap::utc_date))
            .or_else(|| it.text_of("DateTimeReceived").and_then(jmap::utc_date))
            .unwrap_or(0);
        out.push(Recoverable {
            id: id.to_string(),
            kind: RecoverableKind::of_class(it.text_of("ItemClass").unwrap_or("")),
            subject: cut(it.text_of("Subject").unwrap_or(""), TEXT_MAX),
            from,
            deleted,
            size: it.text_of("Size").and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0),
        });
    }
    out
}

impl Client {
    /// A page of Recoverable Items, newest first from `offset`, and whether the server has more.
    pub fn recoverable_page(&self, offset: usize, page: usize) -> Result<(Vec<Recoverable>, bool)> {
        let page = clamp_page(page);
        let doc = self.call(&format!(
            "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
             <t:FieldURI FieldURI=\"item:Subject\"/><t:FieldURI FieldURI=\"item:ItemClass\"/><t:FieldURI FieldURI=\"item:Size\"/>\
             <t:FieldURI FieldURI=\"item:DateTimeReceived\"/><t:FieldURI FieldURI=\"item:LastModifiedTime\"/><t:FieldURI FieldURI=\"message:From\"/>\
             <t:ExtendedFieldURI PropertyTag=\"0x668F\" PropertyType=\"SystemTime\"/></t:AdditionalProperties></m:ItemShape>\
             <m:IndexedPageItemView MaxEntriesReturned=\"{page}\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>\
             <m:SortOrder><t:FieldOrder Order=\"Descending\"><t:FieldURI FieldURI=\"item:LastModifiedTime\"/></t:FieldOrder></m:SortOrder>\
             <m:ParentFolderIds>{}</m:ParentFolderIds></m:FindItem>",
            distinguished("recoverableitemsdeletions", None)
        ))?;
        let m = messages(&doc).first().map(|m| (*m).clone()).ok_or_else(|| NetError::Protocol("an empty answer".into()))?;
        if m.attr("ResponseClass") == Some("Error") {
            let code = m.text_of("ResponseCode").map(str::trim).unwrap_or("");
            return Err(match code {
                "ErrorInvalidServerVersion" | "ErrorFolderNotFound" | "ErrorInvalidRequest" | "ErrorInvalidOperation" | "ErrorAccessDenied" => {
                    NetError::No(NO_SUPPORT.into())
                }
                _ => check_message(&m).err().unwrap_or_else(|| NetError::No(NO_SUPPORT.into())),
            });
        }
        let more = m.find("RootFolder").and_then(|r| r.attr("IncludesLastItemInRange")) == Some("false");
        let items = m.find("Items").map(|i| recoverable_items(i, page)).unwrap_or_default();
        // A server that says "more" and gives nothing would have the window ask for ever.
        Ok((items.clone(), more && !items.is_empty()))
    }

    /// Bring items back into Deleted Items (`MoveItem`), or with `purge` delete them for good
    /// (`HardDelete`). The ids it dealt with (one the server no longer has is as good as done),
    /// and the error that stopped it, when one did.
    pub fn restore_recoverable(&self, ids: &[String], purge: bool) -> (Vec<String>, Option<NetError>) {
        let mut done = Vec::new();
        for chunk in ids.chunks(BATCH) {
            let refs: String = chunk.iter().map(|id| format!("<t:ItemId Id=\"{}\"/>", esc(id))).collect();
            let body = if purge {
                format!("<m:DeleteItem DeleteType=\"HardDelete\"><m:ItemIds>{refs}</m:ItemIds></m:DeleteItem>")
            } else {
                format!("<m:MoveItem><m:ToFolderId>{}</m:ToFolderId><m:ItemIds>{refs}</m:ItemIds></m:MoveItem>", distinguished("deleteditems", None))
            };
            let doc = match self.call(&body) {
                Ok(d) => d,
                Err(e) => return (done, Some(e)),
            };
            let answers = messages(&doc);
            for (n, id) in chunk.iter().enumerate() {
                let Some(m) = answers.get(n) else { return (done, Some(NetError::Protocol("an answer missing".into()))) };
                match check_message(m) {
                    Ok(()) => done.push(id.clone()),
                    Err(_) if m.text_of("ResponseCode").map(str::trim) == Some("ErrorItemNotFound") => done.push(id.clone()),
                    Err(e) => return (done, Some(e)),
                }
            }
        }
        (done, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::fake_ews::{DOMAIN, Data, PASSWORD, USER, serve};

    fn mail(subject: &str) -> Vec<u8> {
        format!("From: Zoe Park <zoe@contoso.example>\r\nTo: riley@contoso.example\r\nSubject: {subject}\r\nMessage-ID: <{subject}@x>\r\n\r\nHi.\r\n")
            .into_bytes()
    }

    /// Mail deleted for good (`SoftDelete`) is listed, comes back into Deleted Items, or is purged.
    #[test]
    fn deleted_mail_is_listed_restored_and_purged() {
        let mut d = Data::default();
        let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        let trash = d.folder("Deleted Items", None, Some("deleteditems"), "IPF.Note");
        let one = d.deliver(&inbox, &mail("Budget"), true, "2026-10-01T10:00:00Z");
        let two = d.deliver(&inbox, &mail("Plans"), true, "2026-10-02T10:00:00Z");
        let server = serve(d);
        let c = Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        // Nothing deleted yet.
        assert!(c.recoverable_page(0, 50).unwrap().0.is_empty());
        c.call_all(&format!(
            "<m:DeleteItem DeleteType=\"SoftDelete\"><m:ItemIds><t:ItemId Id=\"{one}\"/><t:ItemId Id=\"{two}\"/></m:ItemIds></m:DeleteItem>"
        ))
        .unwrap();
        let (items, more) = c.recoverable_page(0, 50).unwrap();
        assert_eq!(items.iter().map(|i| i.subject.as_str()).collect::<Vec<_>>(), ["Plans", "Budget"], "newest first");
        assert!(!more);
        assert_eq!(items[0].from, "Zoe Park");
        assert_eq!(items[0].kind, RecoverableKind::Mail);
        assert!(items[0].deleted > 0 && items[0].size > 0);
        // Pages.
        let (first, more) = c.recoverable_page(0, 1).unwrap();
        assert_eq!((first.len(), more), (1, true));
        let (rest, more) = c.recoverable_page(1, 1).unwrap();
        assert_eq!((rest[0].subject.as_str(), more), ("Budget", false));
        // Restore one: back in Deleted Items.
        let (done, failed) = c.restore_recoverable(&[items[1].id.clone()], false);
        assert!(failed.is_none() && done.len() == 1, "{failed:?}");
        assert!(server.data.lock().unwrap().items.iter().any(|i| i.folder == trash));
        assert_eq!(c.recoverable_page(0, 50).unwrap().0.len(), 1);
        // Purge the other: gone for good.
        let (done, failed) = c.restore_recoverable(&[items[0].id.clone()], true);
        assert!(failed.is_none() && done.len() == 1, "{failed:?}");
        assert!(c.recoverable_page(0, 50).unwrap().0.is_empty());
        assert_eq!(server.data.lock().unwrap().purged.len(), 1);
        // One already gone counts as done.
        let (done, failed) = c.restore_recoverable(&[items[0].id.clone()], true);
        assert!(failed.is_none() && done.len() == 1);
    }

    /// A server too old to list them says so plainly.
    #[test]
    fn an_old_server_says_it_cannot() {
        let mut d = Data::default();
        d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.refuse_recoverable = true;
        let server = serve(d);
        let c = Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
        let said = c.recoverable_page(0, 50).unwrap_err().to_string();
        assert!(said.contains("Exchange 2010 SP1"), "{said}");
    }

    #[test]
    fn a_hostile_answer_is_bounded_and_never_panics() {
        let big = "é".repeat(5000);
        let xml = format!(
            "<Items><Message><ItemId Id=\"a\"/><Subject>{big}</Subject><Size>-4</Size><LastModifiedTime>x</LastModifiedTime></Message>\
             <Message><Subject>no id</Subject></Message><Foo/><CalendarItem><ItemId Id=\"b\"/><ItemClass>IPM.Appointment</ItemClass></CalendarItem></Items>"
        );
        let doc = xml::parse(&xml);
        let doc = doc.find("Items").unwrap();
        let items = recoverable_items(doc, 10);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].subject.chars().count(), TEXT_MAX);
        assert_eq!(items[0].size, 0);
        assert_eq!(items[1].kind, RecoverableKind::Appointment);
        assert_eq!(recoverable_items(doc, 1).len(), 1);
    }
}
