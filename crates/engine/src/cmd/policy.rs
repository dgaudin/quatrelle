// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Commands: Exchange's archive and retention policies of a message or a folder.

use quatrelle_model::PolicyKind;

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

/// Mail selected that a policy of this kind can be set on: not in the online archive, in an
/// account whose server offers tags of that kind.
fn can_set_on_mail(s: &Session, kind: PolicyKind) -> Option<&'static str> {
    has_selection(s).or_else(|| s.selection_read_only().then_some(crate::ARCHIVE_READ_ONLY)).or_else(|| {
        if s.selection.iter().all(|k| s.offers_policy_of(&k.account, &k.folder, kind)) { None } else { Some(crate::session::none_offered(kind)) }
    })
}

fn can_set_on_folder(s: &Session, kind: PolicyKind) -> Option<&'static str> {
    has_account(s).or_else(|| {
        let any = s.accounts.iter().any(|a| s.policy_tags(&a.config.id, kind).next().is_some());
        if any { None } else { Some(crate::session::none_offered(kind)) }
    })
}

/// `mail.<name>` and `folder.<name>` for one kind of policy.
fn pair(kind: PolicyKind) -> [CommandSpec; 2] {
    match kind {
        PolicyKind::Archive => [
            CommandSpec::new("mail.archivePolicy", "Archive policy", "Home › Move", |s, v| s.policy_selection(PolicyKind::Archive, tag_param(v)?))
                .params(r#"{"tag": string (one of the server's archive policy tags) | null (as its folder)}"#)
                .when(|s| can_set_on_mail(s, PolicyKind::Archive)),
            CommandSpec::new("folder.archivePolicy", "Archive policy", "Folder", |s, v| on_folder(s, v, PolicyKind::Archive))
                .params(r#"{"folder": string, "tag": string (one of the server's archive policy tags) | null (as its parent), "account"?: string}"#)
                .when(|s| can_set_on_folder(s, PolicyKind::Archive)),
        ],
        PolicyKind::Retention => [
            CommandSpec::new("mail.retentionPolicy", "Retention policy", "Home › Move", |s, v| {
                s.policy_selection(PolicyKind::Retention, tag_param(v)?)
            })
            .params(r#"{"tag": string (one of the server's retention policy tags) | null (as its folder)}"#)
            .when(|s| can_set_on_mail(s, PolicyKind::Retention)),
            CommandSpec::new("folder.retentionPolicy", "Retention policy", "Folder", |s, v| on_folder(s, v, PolicyKind::Retention))
                .params(r#"{"folder": string, "tag": string (one of the server's retention policy tags) | null (as its parent), "account"?: string}"#)
                .when(|s| can_set_on_folder(s, PolicyKind::Retention)),
        ],
    }
}

fn on_folder(s: &mut Session, v: &Value, kind: PolicyKind) -> CmdResult {
    let account = account_param(s, v)?;
    let tag = tag_param(v)?;
    s.policy_folder(kind, &account, p::req_str(v, "folder")?, tag)
}

pub(super) fn commands() -> Vec<CommandSpec> {
    PolicyKind::ALL.into_iter().flat_map(pair).collect()
}
