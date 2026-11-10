// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Commands: Exchange's archive policy of a message or a folder.

use super::*;

/// The tag the params name: an id, or `null` for "as its folder" (the tag taken off). The key
/// must be there, so that a typo doesn't take a tag off.
fn tag_param(v: &Value) -> Result<Option<&str>, CmdError> {
    match v.get("tag") {
        Some(Value::Null) => Ok(None),
        Some(Value::String(t)) => Ok(Some(t.as_str())),
        _ => Err(CmdError::Params("`tag` (an id of the server's, or null) is required".into())),
    }
}

/// Mail selected that an archive policy can be set on: not in the online archive, in an account
/// whose server offers tags.
fn can_set_on_mail(s: &Session) -> Option<&'static str> {
    has_selection(s).or_else(|| s.selection_read_only().then_some(crate::ARCHIVE_READ_ONLY)).or_else(|| {
        if s.selection.iter().all(|k| s.offers_archive_policy(&k.account, &k.folder)) { None } else { Some("this server offers no archive policy") }
    })
}

fn can_set_on_folder(s: &Session) -> Option<&'static str> {
    has_account(s).or_else(|| {
        let any = s.accounts.iter().any(|a| !s.archive_tags(&a.config.id).is_empty());
        if any { None } else { Some("no server here offers an archive policy") }
    })
}

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("mail.archivePolicy", "Archive policy", "Home › Move", |s, v| s.archive_policy_selection(tag_param(v)?))
            .params(r#"{"tag": string (one of the server's archive policy tags) | null (as its folder)}"#)
            .when(can_set_on_mail),
        CommandSpec::new("folder.archivePolicy", "Archive policy", "Folder", |s, v| {
            let account = account_param(s, v)?;
            let tag = tag_param(v)?;
            s.archive_policy_folder(&account, p::req_str(v, "folder")?, tag)
        })
        .params(r#"{"folder": string, "tag": string (one of the server's archive policy tags) | null (as its parent), "account"?: string}"#)
        .when(can_set_on_folder),
    ]
}
