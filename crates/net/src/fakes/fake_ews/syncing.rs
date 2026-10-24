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
}

impl Marks {
    /// The item as a change carries it.
    fn xml(&self) -> String {
        let flag = if self.flagged {
            "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty>"
        } else {
            ""
        };
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
        let given = op.child("SyncState").map(|s| s.text.trim().to_string()).unwrap_or_default();
        let Some(mut was) = sync_snapshot(&given) else {
            return vec![err("ErrorInvalidSyncStateData")];
        };
        let max: usize =
            op.child("MaxChangesReturned").and_then(|m| m.text.trim().parse().ok()).unwrap_or(512).min(self.sync_page.unwrap_or(usize::MAX));
        let now: Vec<Marks> = self
            .items
            .iter()
            .filter(|i| i.folder == folder)
            .map(|i| Marks { id: i.id.clone(), read: i.read, flagged: i.flagged, draft: i.draft, categories: i.categories.clone() })
            .collect();
        let mut changes = Vec::new();
        for n in &now {
            match was.iter().find(|w| w.id == n.id) {
                None => changes.push((format!("<t:Create>{}</t:Create>", n.xml()), n.clone(), false)),
                Some(w) if (w.flagged, w.draft, &w.categories) != (n.flagged, n.draft, &n.categories) => {
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

/// The separator of a message's categories in a sync state.
const SEP: char = '\u{1f}';

/// A sync state for a snapshot (what the client keeps and sends back, opaque to it).
fn sync_state(s: &[Marks]) -> String {
    let lines: String = s
        .iter()
        .map(|m| {
            let cats: Vec<&str> = m.categories.iter().map(String::as_str).collect();
            format!("{}\t{}{}{}\t{}\n", m.id, u8::from(m.read), u8::from(m.flagged), u8::from(m.draft), cats.join(&SEP.to_string()))
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
            let mut parts = l.splitn(3, '\t');
            let id = parts.next()?.to_string();
            let b: Vec<bool> = parts.next()?.chars().map(|c| c == '1').collect();
            let categories = parts.next().filter(|c| !c.is_empty()).map(|c| c.split(SEP).map(str::to_string).collect()).unwrap_or_default();
            Some(Marks { id, read: *b.first()?, flagged: *b.get(1)?, draft: *b.get(2)?, categories })
        })
        .collect()
}
