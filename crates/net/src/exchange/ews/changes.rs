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

/// Apply one change on the server.
/// Vote on a message (see [`Client::vote`]).
pub fn vote(c: &Client, st: &State, key: &MessageKey, choice: &str, subject: &str) -> Result<()> {
    let id = st.id(key.uid).filter(|_| key.validity == VALIDITY).ok_or_else(|| NetError::No("the message is no longer on the server".into()))?;
    c.vote(&id, choice, subject)
}

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
        Op::SetFlag { keys, flag: Flag::Flagged, on: true } => update(
            keys,
            "<t:SetItemField><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Message><t:ExtendedProperty>\
             <t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty></t:Message></t:SetItemField>",
        ),
        Op::SetFlag { keys, flag: Flag::Flagged, on: false } => {
            update(keys, "<t:DeleteItemField><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/></t:DeleteItemField>")
        }
        Op::SetKeyword { keys, on, name: Some(name), .. } => set_category(c, &ids(keys), name, *on),
        // Answered and forwarded are Exchange's to set; other keywords (`$Pinned`) are this
        // computer's.
        Op::SetFlag { .. } | Op::SetKeyword { .. } => Ok(()),
        Op::Move { keys, to } => {
            let dest = st.mailbox(to)?.to_string();
            let moving: Vec<String> = ids(keys).into_iter().filter(|(_, from)| from != to).map(|(id, _)| id).collect();
            for chunk in moving.chunks(BATCH) {
                c.call_all(&format!(
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
