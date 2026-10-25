//! EWS: how much space each folder of the mailbox takes on the server, and the mailbox's limits,
//! as Outlook's Folder Size (its Server Data) and the bar on its File › Info show them.

use std::collections::HashSet;

use quatrelle_model::{FolderSize, MailboxQuota};

use super::*;

/// `PidTagMessageSizeExtended` ([MS-OXPROPS]): the bytes a folder's own items take.
const SIZE: &str = "<t:ExtendedFieldURI PropertyTag=\"0x0E08\" PropertyType=\"Long\"/>";
/// The mailbox's limits, in kilobytes ([MS-OXPROPS]): `PidTagStorageQuotaLimit` (the warning),
/// `PidTagProhibitSendQuota`, `PidTagProhibitReceiveQuota`.
const WARNING: &str = "0x3FF5";
const SEND: &str = "0x666E";
const RECEIVE: &str = "0x666A";
/// The deepest a folder is followed down (a loop in what the server says stops there).
const MAX_DEPTH: usize = 64;

/// A folder as listed, before it has its place.
struct Listed {
    id: String,
    parent: Option<String>,
    name: String,
    size: u64,
    items: u32,
    /// Exchange's own (hidden, or one of its folders at the top): left out with what's under it.
    own: bool,
}

impl Client {
    /// Every folder of the mailbox with the space its own items take, from the top down (each
    /// one's subfolders right after it, by name), and the mailbox's limits when the server
    /// tells them. Exchange's own hidden folders are left out, as Outlook leaves them out.
    pub fn folder_sizes(&self) -> Result<(Vec<FolderSize>, Option<MailboxQuota>)> {
        let mut listed: Vec<Listed> = Vec::new();
        let mut offset = 0;
        loop {
            let doc = self.call(&format!(
                "<m:FindFolder Traversal=\"Deep\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>\
                 <t:FieldURI FieldURI=\"folder:DisplayName\"/><t:FieldURI FieldURI=\"folder:ParentFolderId\"/>\
                 <t:FieldURI FieldURI=\"folder:TotalCount\"/>{HIDDEN}{SIZE}\
                 </t:AdditionalProperties></m:FolderShape>\
                 <m:IndexedPageFolderView MaxEntriesReturned=\"500\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>\
                 <m:ParentFolderIds>{}</m:ParentFolderIds></m:FindFolder>",
                distinguished("msgfolderroot", None)
            ))?;
            let Some(m) = messages(&doc).first().map(|m| (*m).clone()) else { break };
            check_message(&m)?;
            let Some(rf) = m.child("RootFolder") else { break };
            let folders = rf.folders();
            let n = folders.len();
            for f in folders {
                let Some(id) = f.child("FolderId").and_then(|i| i.attr("Id")).map(str::to_string) else { continue };
                if listed.len() >= MAX_FOLDERS {
                    break;
                }
                listed.push(Listed {
                    id,
                    parent: f.child("ParentFolderId").and_then(|p| p.attr("Id")).map(str::to_string),
                    name: f.text_of("DisplayName").unwrap_or_default().chars().take(255).collect(),
                    size: extended(f, "0x0E08").and_then(|v| u64::try_from(v).ok()).unwrap_or(0),
                    items: f.text_of("TotalCount").and_then(|v| v.trim().parse().ok()).unwrap_or(0),
                    own: is_hidden(f),
                });
            }
            let last = rf.attr("IncludesLastItemInRange").is_none_or(|v| v == "true");
            offset += n;
            if last || n == 0 || listed.len() >= MAX_FOLDERS {
                break;
            }
        }
        let quota = match self.limits() {
            Ok(q) => q,
            // The sizes are worth having without the limits.
            Err(e) if !e.is_fatal() && !matches!(e, NetError::Auth(_)) => {
                log::info!("the mailbox's limits couldn't be read: {e}");
                None
            }
            Err(e) => return Err(e),
        };
        Ok((placed(listed), quota))
    }

    /// The mailbox's limits as the server keeps them on its root folders, `None` when it tells
    /// none (Exchange may keep them on the mailbox alone, where EWS doesn't reach).
    pub(crate) fn limits(&self) -> Result<Option<MailboxQuota>> {
        let prop = |tag: &str| format!("<t:ExtendedFieldURI PropertyTag=\"{tag}\" PropertyType=\"Integer\"/>");
        let doc = self.call(&format!(
            "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{}{}{}</t:AdditionalProperties>\
             </m:FolderShape><m:FolderIds>{}{}</m:FolderIds></m:GetFolder>",
            prop(WARNING),
            prop(SEND),
            prop(RECEIVE),
            distinguished("root", None),
            distinguished("msgfolderroot", None)
        ))?;
        let kb = |f: &El, tag: &str| extended(f, tag).and_then(|v| u64::try_from(v).ok()).map_or(0, |v| v.saturating_mul(1024));
        // Either folder may carry them (and one that isn't there just answers an error).
        for m in messages(&doc) {
            if m.attr("ResponseClass") == Some("Error") {
                continue;
            }
            if let Some(f) = m.folders().into_iter().next() {
                let q = MailboxQuota { warning: kb(f, WARNING), send: kb(f, SEND), receive: kb(f, RECEIVE) };
                if q.limit() > 0 {
                    return Ok(Some(q));
                }
            }
        }
        Ok(None)
    }
}

/// The folders listed, each with its place from the top down: tops first, each folder's own
/// right after it, by name; Exchange's own (and anything under one) left out.
fn placed(listed: Vec<Listed>) -> Vec<FolderSize> {
    let ids: HashSet<&str> = listed.iter().map(|l| l.id.as_str()).collect();
    // At the top: under the mailbox's root, which isn't listed itself.
    let top = |l: &Listed| l.parent.as_deref().is_none_or(|p| !ids.contains(p));
    let mut kids: HashMap<Option<&str>, Vec<usize>> = HashMap::new();
    for (i, l) in listed.iter().enumerate() {
        let parent = if top(l) { None } else { l.parent.as_deref() };
        kids.entry(parent).or_default().push(i);
    }
    let by_name = |v: &mut Vec<usize>| v.sort_by_cached_key(|i| listed.get(*i).map(|l| l.name.to_lowercase()).unwrap_or_default());
    for v in kids.values_mut() {
        by_name(v);
    }
    let mut out = Vec::new();
    // Depth first, from the top: (index, the names above it).
    let mut stack: Vec<(usize, Vec<String>)> = kids.get(&None).map(|v| v.iter().rev().map(|i| (*i, Vec::new())).collect()).unwrap_or_default();
    let mut seen: HashSet<usize> = HashSet::new();
    while let Some((i, above)) = stack.pop() {
        let Some(l) = listed.get(i) else { continue };
        if !seen.insert(i) || above.len() >= MAX_DEPTH || l.own || (above.is_empty() && mailbox::SYSTEM_FOLDERS.contains(&l.name.trim())) {
            continue;
        }
        let mut path = above;
        path.push(l.name.clone());
        if let Some(v) = kids.get(&Some(l.id.as_str())) {
            for k in v.iter().rev() {
                stack.push((*k, path.clone()));
            }
        }
        out.push(FolderSize { path, size: l.size, items: l.items, folder: None });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(id: &str, parent: &str, name: &str, size: u64) -> Listed {
        Listed { id: id.into(), parent: Some(parent.into()), name: name.into(), size, items: 1, own: false }
    }

    #[test]
    fn folders_come_from_the_top_down_by_name_without_exchanges_own() {
        let mut hidden = l("h", "ROOT", "Conversation Action Settings", 5);
        hidden.own = true;
        let listed = vec![
            l("sub", "inbox", "Projets", 30),
            l("inbox", "ROOT", "Boîte de réception", 100),
            l("cal", "ROOT", "Calendrier", 20),
            hidden,
            l("under-hidden", "h", "Anything", 7),
            l("yammer", "ROOT", "Yammer Root", 9),
            l("a", "inbox", "Archives", 10),
            // A loop in what the server says: never followed for ever.
            l("x", "y", "X", 1),
            l("y", "x", "Y", 1),
        ];
        let paths: Vec<Vec<String>> = placed(listed).into_iter().map(|f| f.path).collect();
        let want: Vec<Vec<String>> = vec![
            vec!["Boîte de réception".into()],
            vec!["Boîte de réception".into(), "Archives".into()],
            vec!["Boîte de réception".into(), "Projets".into()],
            vec!["Calendrier".into()],
        ];
        assert_eq!(paths, want);
    }

    #[test]
    fn exchange_tells_each_folders_size_and_the_mailboxs_limits() {
        let mut d = crate::fakes::fake_ews::Data::default();
        let inbox = d.folder("Boîte de réception", None, Some("inbox"), "IPF.Note");
        let sub = d.folder("Projets", Some(&inbox), None, "IPF.Note");
        d.folder("Calendrier", None, Some("calendar"), "IPF.Appointment");
        let hidden = d.folder("Conversation Action Settings", None, None, "IPF.Configuration");
        d.hidden.push(hidden);
        for i in 0..4 {
            d.deliver(&inbox, format!("Subject: m{i}\r\n\r\nbody\r\n").as_bytes(), false, "2024-05-01T09:00:00Z");
        }
        d.deliver(&sub, b"Subject: s\r\n\r\na longer body\r\n", false, "2024-05-01T09:00:00Z");
        // Warned at 1 GB, sending stops at 1.5, receiving at 2.
        d.limits = Some((1 << 20, 3 << 19, 2 << 20));
        let server = crate::fakes::fake_ews::serve(d);
        let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
        let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
        let (folders, quota) = c.folder_sizes().unwrap();
        let names: Vec<String> = folders.iter().map(|f| f.path.join("/")).collect();
        assert_eq!(names, ["Boîte de réception", "Boîte de réception/Projets", "Calendrier"], "{folders:?}");
        let inbox_bytes = 4 * "Subject: m0\r\n\r\nbody\r\n".len() as u64;
        assert_eq!((folders[0].size, folders[0].items), (inbox_bytes, 4));
        assert_eq!(folders[1].size, "Subject: s\r\n\r\na longer body\r\n".len() as u64);
        assert_eq!(quota, Some(MailboxQuota { warning: 1 << 30, send: 3 << 29, receive: 2 << 30 }));
        assert_eq!(quota.map(|q| q.limit()), Some(2 << 30), "measured against where mail stops coming in");
    }

    #[test]
    fn a_server_without_limits_still_tells_the_sizes() {
        let mut d = crate::fakes::fake_ews::Data::default();
        let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
        d.deliver(&inbox, b"Subject: m\r\n\r\nbody\r\n", false, "2024-05-01T09:00:00Z");
        let server = crate::fakes::fake_ews::serve(d);
        let user = format!("{}\\{}", crate::fakes::fake_ews::DOMAIN, crate::fakes::fake_ews::USER);
        let c = Client::connect(&server.url, &user, crate::fakes::fake_ews::PASSWORD, "riley@contoso.example").unwrap();
        let (folders, quota) = c.folder_sizes().unwrap();
        assert_eq!((folders.len(), quota), (1, None));
        assert!(folders[0].size > 0);
    }

    #[test]
    fn a_tag_is_known_however_the_server_spells_it() {
        assert!(is_tag("0xe08", "0x0E08"));
        assert!(is_tag("0x6698", "26264"));
        assert!(is_tag("0x10F4", "0x10f4"));
        assert!(!is_tag("0xe07", "0x0E08"));
    }
}
