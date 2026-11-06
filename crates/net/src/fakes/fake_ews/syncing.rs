// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The fake EWS server's `SyncFolderItems`: what changed in a folder since a sync state. Here the
//! state is the folder as it was (its messages' ids and marks), and the changes are what differs
//! now — as Exchange answers them.

use super::*;

/// A message as a sync state remembers it: its id and marks (read, flagged, draft, categories).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Marks {
    id: String,
    read: bool,
    flagged: bool,
    draft: bool,
    categories: Vec<String>,
    /// Its follow-up marked complete, and when.
    complete: bool,
    complete_time: Option<String>,
    /// The last thing done with it (replied, forwarded), when, and its icon.
    verb: Verb,
}

impl Marks {
    /// The item as a change carries it.
    fn xml(&self) -> String {
        let flag = flag_xml(self.flagged, self.complete, self.complete_time.as_deref(), &self.verb);
        format!(
            "<t:Message><t:ItemId Id=\"{}\" ChangeKey=\"ck\"/><t:IsRead>{}</t:IsRead>{flag}<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x0E07\" PropertyType=\"Integer\"/><t:Value>{}</t:Value></t:ExtendedProperty>{}</t:Message>",
            self.id,
            self.read,
            if self.draft { 8 } else { 1 },
            categories_xml(&self.categories)
        )
    }
}

impl Data {
    /// `SyncFolderItems`' response message.
    pub(super) fn sync_folder_items(&self, op: &El) -> Vec<String> {
        let ok = |inner: String| format!("<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>{inner}</m:R>");
        let err =
            |code: &str| format!("<m:R ResponseClass=\"Error\"><m:MessageText>{code}</m:MessageText><m:ResponseCode>{code}</m:ResponseCode></m:R>");
        let Some(folder) = op.child("SyncFolderId").and_then(|f| f.kids.first()).and_then(|f| self.folder_id(f)) else {
            return vec![err("ErrorFolderNotFound")];
        };
        // A public folder asked for without being routed to the mailbox holding it, or (some
        // servers) at all.
        if self.unrouted(&folder) {
            return vec![err("ErrorFolderNotFound")];
        }
        if self.public_changes_refused && self.public_ids().contains(&folder) {
            return vec![err("ErrorInvalidOperation")];
        }
        let given = op.child("SyncState").map(|s| s.text.trim().to_string()).unwrap_or_default();
        if self.pim_folder(op.child("SyncFolderId").and_then(|f| f.kids.first())) {
            return vec![self.sync_pim_items(&folder, &given, op)];
        }
        let Some(mut was) = sync_snapshot(&given) else {
            return vec![err("ErrorInvalidSyncStateData")];
        };
        let max: usize =
            op.child("MaxChangesReturned").and_then(|m| m.text.trim().parse().ok()).unwrap_or(512).min(self.sync_page.unwrap_or(usize::MAX));
        let now: Vec<Marks> = self
            .items
            .iter()
            .filter(|i| i.folder == folder)
            .map(|i| Marks {
                id: i.id.clone(),
                read: i.read,
                flagged: i.flagged,
                draft: i.draft,
                categories: i.categories.clone(),
                complete: i.complete,
                complete_time: i.complete_time.clone(),
                verb: i.verb.clone(),
            })
            .collect();
        let mut changes = Vec::new();
        for n in &now {
            match was.iter().find(|w| w.id == n.id) {
                None => changes.push((format!("<t:Create>{}</t:Create>", n.xml()), n.clone(), false)),
                Some(w)
                    if (w.flagged, w.draft, &w.categories, w.complete, &w.complete_time, &w.verb)
                        != (n.flagged, n.draft, &n.categories, n.complete, &n.complete_time, &n.verb) =>
                {
                    changes.push((format!("<t:Update>{}</t:Update>", n.xml()), n.clone(), false))
                }
                Some(w) if w.read != n.read => changes.push((
                    format!("<t:ReadFlagChange><t:ItemId Id=\"{}\" ChangeKey=\"ck\"/><t:IsRead>{}</t:IsRead></t:ReadFlagChange>", n.id, n.read),
                    n.clone(),
                    false,
                )),
                Some(_) => {}
            }
        }
        for w in &was {
            if !now.iter().any(|n| n.id == w.id) {
                changes.push((format!("<t:Delete><t:ItemId Id=\"{}\" ChangeKey=\"ck\"/></t:Delete>", w.id), w.clone(), true));
            }
        }
        let last = changes.len() <= max;
        let mut xml = String::new();
        for (x, item, gone) in changes.into_iter().take(max) {
            xml.push_str(&x);
            was.retain(|w| w.id != item.id);
            if !gone {
                was.push(item);
            }
        }
        vec![ok(format!(
            "<m:SyncState>{}</m:SyncState><m:IncludesLastItemInRange>{last}</m:IncludesLastItemInRange><m:Changes>{xml}</m:Changes>",
            sync_state(&was)
        ))]
    }
}

impl Data {
    /// `SyncFolderItems` in a calendar or a contacts folder: its items as Exchange lists them (a
    /// series, not its occurrences that became items of their own), each changed when anything of
    /// it changed (an occurrence of a series moved changes the series, as Exchange keeps the
    /// occurrence inside it). The state is each item's id and a digest of it.
    fn sync_pim_items(&self, folder: &str, given: &str, op: &El) -> String {
        use std::hash::{Hash, Hasher};
        let Some(mut was) = pim_snapshot(given) else {
            return "<m:R ResponseClass=\"Error\"><m:MessageText>The synchronization state data is corrupt or otherwise invalid.</m:MessageText>\
                    <m:ResponseCode>ErrorInvalidSyncStateData</m:ResponseCode></m:R>"
                .into();
        };
        let max: usize =
            op.child("MaxChangesReturned").and_then(|m| m.text.trim().parse().ok()).unwrap_or(512).min(self.sync_page.unwrap_or(usize::MAX));
        let own = |id: &str, e: &El| !id.contains(":occ") && !matches!(e.text_of("CalendarItemType"), Some("Occurrence" | "Exception"));
        let now: Vec<(String, String, u64)> = self
            .pim
            .iter()
            .filter(|(id, f, e)| *f == folder && own(id, e))
            .map(|(id, _, e)| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                super::xml(e).hash(&mut h);
                let prefix = format!("{id}:occ");
                for (_, _, o) in self.pim.iter().filter(|(i, _, _)| i.starts_with(&prefix)) {
                    super::xml(o).hash(&mut h);
                }
                (id.clone(), e.name.clone(), h.finish())
            })
            .collect();
        // Each change, the item's id, and its digest (`None`: deleted).
        let mut changes: Vec<(String, String, Option<u64>)> = Vec::new();
        for (id, kind, digest) in &now {
            let item = format!("<t:{kind}><t:ItemId Id=\"{id}\" ChangeKey=\"ck\"/></t:{kind}>");
            match was.iter().find(|(w, _)| w == id) {
                None => changes.push((format!("<t:Create>{item}</t:Create>"), id.clone(), Some(*digest))),
                Some((_, d)) if d != digest => changes.push((format!("<t:Update>{item}</t:Update>"), id.clone(), Some(*digest))),
                Some(_) => {}
            }
        }
        for (w, _) in &was {
            if !now.iter().any(|(id, _, _)| id == w) {
                changes.push((format!("<t:Delete><t:ItemId Id=\"{w}\" ChangeKey=\"ck\"/></t:Delete>"), w.clone(), None));
            }
        }
        let last = changes.len() <= max;
        let mut xml = String::new();
        for (x, id, digest) in changes.into_iter().take(max) {
            xml.push_str(&x);
            was.retain(|(w, _)| *w != id);
            if let Some(d) = digest {
                was.push((id, d));
            }
        }
        let lines: String = was.iter().map(|(id, d)| format!("{id}\t{d:x}\n")).collect();
        format!(
            "<m:R ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:SyncState>p1{}</m:SyncState>\
             <m:IncludesLastItemInRange>{last}</m:IncludesLastItemInRange><m:Changes>{xml}</m:Changes></m:R>",
            crate::imap::base64(lines.as_bytes())
        )
    }
}

/// The items a calendar's or a contacts folder's sync state stands for, with their digests
/// (empty: none); `None` for one this server didn't make.
fn pim_snapshot(state: &str) -> Option<Vec<(String, u64)>> {
    if state.is_empty() {
        return Some(Vec::new());
    }
    let text = String::from_utf8(unb64(state.strip_prefix("p1")?)?).ok()?;
    text.lines()
        .map(|l| {
            let (id, d) = l.split_once('\t')?;
            Some((id.to_string(), u64::from_str_radix(d, 16).ok()?))
        })
        .collect()
}

/// The separator of a message's categories in a sync state.
const SEP: char = '\u{1f}';

/// A sync state for a snapshot (what the client keeps and sends back, opaque to it).
fn sync_state(s: &[Marks]) -> String {
    let lines: String = s
        .iter()
        .map(|m| {
            let cats: Vec<&str> = m.categories.iter().map(String::as_str).collect();
            format!(
                "{}\t{}{}{}{}\t{}\t{}\t{}\t{}\t{}\n",
                m.id,
                u8::from(m.read),
                u8::from(m.flagged),
                u8::from(m.draft),
                u8::from(m.complete),
                cats.join(&SEP.to_string()),
                m.complete_time.as_deref().unwrap_or(""),
                m.verb.last.as_deref().unwrap_or(""),
                m.verb.time.as_deref().unwrap_or(""),
                m.verb.icon.as_deref().unwrap_or("")
            )
        })
        .collect();
    format!("v2{}", crate::imap::base64(lines.as_bytes()))
}

/// The snapshot a sync state stands for (empty: none); `None` for one this server didn't make.
fn sync_snapshot(state: &str) -> Option<Vec<Marks>> {
    if state.is_empty() {
        return Some(Vec::new());
    }
    let bytes = unb64(state.strip_prefix("v2")?)?;
    let text = String::from_utf8(bytes).ok()?;
    text.lines()
        .map(|l| {
            let mut parts = l.splitn(7, '\t');
            let id = parts.next()?.to_string();
            let b: Vec<bool> = parts.next()?.chars().map(|c| c == '1').collect();
            let categories = parts.next().filter(|c| !c.is_empty()).map(|c| c.split(SEP).map(str::to_string).collect()).unwrap_or_default();
            let mut text = || parts.next().filter(|t| !t.is_empty()).map(str::to_string);
            let complete_time = text();
            let verb = Verb { last: text(), time: text(), icon: text() };
            Some(Marks {
                id,
                read: *b.first()?,
                flagged: *b.get(1)?,
                draft: *b.get(2)?,
                categories,
                complete: b.get(3).copied().unwrap_or(false),
                complete_time,
                verb,
            })
        })
        .collect()
}
