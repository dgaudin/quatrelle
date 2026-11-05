// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! EWS: changes made here applied on the server (flags, moves, deletions, folders), drafts stored and mail sent.

use super::*;

fn item_ids(ids: &[String]) -> String {
    ids.iter().map(|id| format!("<t:ItemId Id=\"{}\"/>", esc(id))).collect()
}

fn folder_ref(id: Option<&str>) -> String {
    match id {
        Some(id) => format!("<t:FolderId Id=\"{}\"/>", esc(id)),
        None => "<t:DistinguishedFolderId Id=\"msgfolderroot\"/>".into(),
    }
}

/// Put a category on messages or take it off, as Outlook does: Exchange keeps an item's
/// categories as a list of names, so each one's list is read and written back with the name in
/// it, or without.
fn set_category(c: &Client, ids: &[(String, String)], name: &str, on: bool) -> Result<()> {
    for chunk in ids.chunks(BATCH) {
        let wanted: Vec<String> = chunk.iter().map(|(id, _)| id.clone()).collect();
        let doc = c.call(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties><t:FieldURI FieldURI=\"item:Categories\"/>\
             </t:AdditionalProperties></m:ItemShape><m:ItemIds>{}</m:ItemIds></m:GetItem>",
            item_ids(&wanted)
        ))?;
        let mut changes = String::new();
        // One that's gone meanwhile is passed over.
        for it in messages(&doc).iter().filter(|m| m.attr("ResponseClass") != Some("Error")).filter_map(|m| m.find("Items")?.kids.first()) {
            let Some(id) = it.child("ItemId").and_then(|i| i.attr("Id")) else { continue };
            let mut names: Vec<String> =
                it.child("Categories").map(|l| l.children("String").map(|s| s.text.trim().to_string()).collect()).unwrap_or_default();
            if names.iter().any(|n| n.eq_ignore_ascii_case(name)) == on {
                continue;
            }
            if on {
                names.push(name.to_string());
            } else {
                names.retain(|n| !n.eq_ignore_ascii_case(name));
            }
            let update = if names.is_empty() {
                "<t:DeleteItemField><t:FieldURI FieldURI=\"item:Categories\"/></t:DeleteItemField>".to_string()
            } else {
                // The item as the server named its kind (a meeting request isn't a message).
                let list: String = names.iter().map(|n| format!("<t:String>{}</t:String>", esc(n))).collect();
                format!(
                    "<t:SetItemField><t:FieldURI FieldURI=\"item:Categories\"/><t:{kind}><t:Categories>{list}</t:Categories></t:{kind}></t:SetItemField>",
                    kind = it.name
                )
            };
            changes.push_str(&format!("<t:ItemChange><t:ItemId Id=\"{}\"/><t:Updates>{update}</t:Updates></t:ItemChange>", esc(id)));
        }
        if !changes.is_empty() {
            c.call_all(&format!(
                "<m:UpdateItem MessageDisposition=\"SaveOnly\" ConflictResolution=\"AlwaysOverwrite\"><m:ItemChanges>{changes}</m:ItemChanges></m:UpdateItem>"
            ))?;
        }
    }
    Ok(())
}

/// `PidTagFlagStatus`: 2 flagged, 1 its follow-up complete.
const FLAG_STATUS: &str = "<t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/>";
/// `PidTagFlagCompleteTime`: when the follow-up was marked complete.
const COMPLETE_TIME: &str = "<t:ExtendedFieldURI PropertyTag=\"0x1091\" PropertyType=\"SystemTime\"/>";
/// `PidTagFollowupIcon`: the colour of a flag Outlook set.
const FOLLOW_UP_ICON: &str = "<t:ExtendedFieldURI PropertyTag=\"0x1095\" PropertyType=\"Integer\"/>";

/// `PidTagLastVerbExecuted`: the last thing done with a message (replied, forwarded).
const LAST_VERB: &str = "<t:ExtendedFieldURI PropertyTag=\"0x1081\" PropertyType=\"Integer\"/>";
/// `PidTagLastVerbExecutionTime`: when.
const LAST_VERB_TIME: &str = "<t:ExtendedFieldURI PropertyTag=\"0x1082\" PropertyType=\"SystemTime\"/>";
/// `PidTagIconIndex`: the icon Outlook's list draws for it.
const ICON_INDEX: &str = "<t:ExtendedFieldURI PropertyTag=\"0x1080\" PropertyType=\"Integer\"/>";

/// An update setting a property to a value.
fn set(field: &str, value: &str) -> String {
    format!(
        "<t:SetItemField>{field}<t:Message><t:ExtendedProperty>{field}<t:Value>{value}</t:Value></t:ExtendedProperty></t:Message></t:SetItemField>"
    )
}

/// Replied to or forwarded, as Outlook marks the original when it sends its reply or forward
/// ([MS-OXOMSG]: `PidTagLastVerbExecuted` the verb, `PidTagLastVerbExecutionTime` now,
/// `PidTagIconIndex` the icon, [`super::replied_verb`]): Quatrelle sends its own message
/// (`CreateItem` with its source), so Exchange marks nothing by itself.
fn replied(forward: bool) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let (verb, icon) = super::replied_verb(forward);
    format!(
        "{}{}{}",
        set(LAST_VERB, &verb.to_string()),
        set(LAST_VERB_TIME, &quatrelle_jmap::utc_date_string(now)),
        set(ICON_INDEX, &icon.to_string())
    )
}

/// An update setting the flag status.
fn flag_status(v: u8) -> String {
    format!(
        "<t:SetItemField>{FLAG_STATUS}<t:Message><t:ExtendedProperty>{FLAG_STATUS}<t:Value>{v}</t:Value></t:ExtendedProperty></t:Message></t:SetItemField>"
    )
}

/// An update deleting a property.
fn delete(field: &str) -> String {
    format!("<t:DeleteItemField>{field}</t:DeleteItemField>")
}

/// Vote on a message (see [`Client::vote`]).
pub fn vote(c: &Client, st: &State, key: &MessageKey, choice: &str, subject: &str) -> Result<()> {
    let id = st.id(key.uid).filter(|_| key.validity == VALIDITY).ok_or_else(|| NetError::No("the message is no longer on the server".into()))?;
    c.vote(&id, choice, subject)
}

/// Apply one change on the server.
pub fn apply_remote(c: &Client, st: &State, op: &Op) -> Result<()> {
    let ids = |keys: &[MessageKey]| -> Vec<(String, String)> {
        let found: Vec<(String, String)> =
            keys.iter().filter(|k| k.validity == VALIDITY).filter_map(|k| Some((st.id(k.uid)?, k.folder.clone()))).collect();
        // Said, not passed over in silence: such a change is made here only, and the server's
        // state comes back over it at the next full read of the folder.
        if found.len() < keys.len() {
            let account = keys.first().map_or("", |k| k.account.as_str());
            log::warn!("{account}: {} of {} messages have no server id: the change is made here only", keys.len() - found.len(), keys.len());
        }
        found
    };
    let update = |keys: &[MessageKey], change: &str| -> Result<()> {
        for chunk in ids(keys).chunks(BATCH) {
            let changes: String = chunk
                .iter()
                .map(|(id, _)| format!("<t:ItemChange><t:ItemId Id=\"{}\"/><t:Updates>{change}</t:Updates></t:ItemChange>", esc(id)))
                .collect();
            c.call_all(&format!(
                "<m:UpdateItem MessageDisposition=\"SaveOnly\" ConflictResolution=\"AlwaysOverwrite\"><m:ItemChanges>{changes}</m:ItemChanges></m:UpdateItem>"
            ))?;
        }
        Ok(())
    };
    match op {
        Op::SetFlag { keys, flag: Flag::Seen, on } => update(
            keys,
            &format!("<t:SetItemField><t:FieldURI FieldURI=\"message:IsRead\"/><t:Message><t:IsRead>{on}</t:IsRead></t:Message></t:SetItemField>"),
        ),
        // Flagged: status 2, and no time done (a follow-up completed and flagged again).
        Op::SetFlag { keys, flag: Flag::Flagged, on: true } => update(keys, &format!("{}{}", flag_status(2), delete(COMPLETE_TIME))),
        // Cleared, or marked complete (the engine takes the flag off first): neither.
        Op::SetFlag { keys, flag: Flag::Flagged, on: false } => update(keys, &format!("{}{}", delete(FLAG_STATUS), delete(COMPLETE_TIME))),
        // Outlook's Mark Complete: status 1 and the time it was done, the flag's icon gone
        // ([MS-OXOFLAG] 3.1.4.2); taken off (Undo, or flagged again): none of it.
        Op::SetKeyword { keys, keyword, on, .. } if keyword.eq_ignore_ascii_case(quatrelle_model::FOLLOW_UP_COMPLETE) => {
            if *on {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
                let time = format!(
                    "<t:SetItemField>{COMPLETE_TIME}<t:Message><t:ExtendedProperty>{COMPLETE_TIME}<t:Value>{}</t:Value></t:ExtendedProperty></t:Message></t:SetItemField>",
                    quatrelle_jmap::utc_date_string(now)
                );
                update(keys, &format!("{}{time}{}", flag_status(1), delete(FOLLOW_UP_ICON)))
            } else {
                update(keys, &format!("{}{}", delete(FLAG_STATUS), delete(COMPLETE_TIME)))
            }
        }
        Op::SetKeyword { keys, on, name: Some(name), .. } => set_category(c, &ids(keys), name, *on),
        // Replied to or forwarded here: the original marked as Outlook marks it.
        Op::SetFlag { keys, flag: flag @ (Flag::Answered | Flag::Forwarded), on: true } => update(keys, &replied(*flag == Flag::Forwarded)),
        // Exchange keeps the last thing done, which the next one replaces (the engine takes one
        // arrow off before it puts the other on); other keywords (`$Pinned`) are this computer's.
        Op::SetFlag { .. } | Op::SetKeyword { .. } => Ok(()),
        Op::Move { keys, to } => {
            let dest = st.mailbox(to)?.to_string();
            let moving: Vec<String> = ids(keys).into_iter().filter(|(_, from)| from != to).map(|(id, _)| id).collect();
            for chunk in moving.chunks(BATCH) {
                c.call_all_present(&format!(
                    "<m:MoveItem><m:ToFolderId>{}</m:ToFolderId><m:ItemIds>{}</m:ItemIds></m:MoveItem>",
                    folder_ref(Some(&dest)),
                    item_ids(chunk)
                ))?;
            }
            Ok(())
        }
        Op::Expunge { keys } => {
            let gone: Vec<String> = ids(keys).into_iter().map(|(id, _)| id).collect();
            for chunk in gone.chunks(BATCH) {
                c.call_all(&format!("<m:DeleteItem DeleteType=\"HardDelete\"><m:ItemIds>{}</m:ItemIds></m:DeleteItem>", item_ids(chunk)))?;
            }
            Ok(())
        }
        Op::CreateFolder { path } => {
            let (parent, name) = split_path(st, path)?;
            c.call_one(&format!(
                "<m:CreateFolder><m:ParentFolderId>{}</m:ParentFolderId><m:Folders><t:Folder><t:FolderClass>IPF.Note</t:FolderClass>\
                 <t:DisplayName>{}</t:DisplayName></t:Folder></m:Folders></m:CreateFolder>",
                folder_ref(parent.as_deref()),
                esc(&name)
            ))
            .map(drop)
        }
        Op::RenameFolder { from, to } => {
            let mut id = st.mailbox(from)?.to_string();
            let (old_parent, _) = split_path(st, from)?;
            let (parent, name) = split_path(st, to)?;
            if parent != old_parent {
                let m = c.call_one(&format!(
                    "<m:MoveFolder><m:ToFolderId>{}</m:ToFolderId><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:MoveFolder>",
                    folder_ref(parent.as_deref()),
                    esc(&id)
                ))?;
                if let Some(new) = m.find("FolderId").and_then(|f| f.attr("Id")) {
                    id = new.to_string();
                }
            }
            c.call_one(&format!(
                "<m:UpdateFolder><m:FolderChanges><t:FolderChange><t:FolderId Id=\"{}\"/><t:Updates><t:SetFolderField>\
                 <t:FieldURI FieldURI=\"folder:DisplayName\"/><t:Folder><t:DisplayName>{}</t:DisplayName></t:Folder></t:SetFolderField>\
                 </t:Updates></t:FolderChange></m:FolderChanges></m:UpdateFolder>",
                esc(&id),
                esc(&name)
            ))
            .map(drop)
        }
        Op::DeleteFolder { path } => {
            let id = st.mailbox(path)?.to_string();
            c.call_one(&format!(
                "<m:DeleteFolder DeleteType=\"MoveToDeletedItems\"><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:DeleteFolder>",
                esc(&id)
            ))
            .map(drop)
        }
        Op::EmptyFolder { path } => {
            let id = st.mailbox(path)?.to_string();
            c.call_one(&format!(
                "<m:EmptyFolder DeleteType=\"HardDelete\" DeleteSubFolders=\"false\"><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:EmptyFolder>",
                esc(&id)
            ))
            .map(drop)
        }
    }
}

/// The parent folder id (`None` at the top) and the leaf name of a folder path.
fn split_path(st: &State, path: &str) -> Result<(Option<String>, String)> {
    match path.rsplit_once('/') {
        Some((parent, leaf)) => Ok((Some(st.mailbox(parent)?.to_string()), utf7::decode(leaf))),
        None => Ok((None, utf7::decode(path))),
    }
}

/// Store a draft in a folder.
pub fn append(c: &Client, st: &State, folder: &str, raw: &[u8]) -> Result<String> {
    c.create(raw, st.mailbox(folder)?, true)
}

/// Send a message. Exchange reads the recipients from the MIME source, so anyone only on the
/// envelope (Bcc) goes into a `Bcc:` header, which Exchange removes before delivery.
pub fn send(c: &Client, raw: &[u8], recipients: &[String]) -> Result<()> {
    let shown: Vec<String> = mime::parse(raw).map(|p| p.to.iter().chain(&p.cc).map(|a| a.email.to_ascii_lowercase()).collect()).unwrap_or_default();
    let bcc: Vec<&str> = recipients.iter().map(String::as_str).filter(|r| !shown.contains(&r.to_ascii_lowercase())).collect();
    if bcc.is_empty() {
        return c.send_mime(raw);
    }
    let mut with = format!("Bcc: {}\r\n", bcc.join(", ")).into_bytes();
    with.extend_from_slice(raw);
    c.send_mime(&with)
}
