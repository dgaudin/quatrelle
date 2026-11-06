// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Commands: OpenPGP (File › Info › Security): the account's own keys, made, extended, backed
//! up, exported, imported, used and forgotten; its correspondents' keys, imported (from a file or
//! the open message), listed and forgotten. They never open a dialog: the window asks for the
//! file or the passphrase, then calls them.

use super::*;

/// What a key is, as the commands answer.
fn show(i: &quatrelle_pgp::KeyInfo) -> Value {
    json!({
        "name": i.name, "emails": i.emails, "fingerprint": i.fingerprint, "keyId": i.key_id, "created": i.created,
        "expires": i.expires, "revoked": i.revoked, "encrypts": i.encrypts, "signs": i.signs, "algorithm": i.algorithm,
    })
}

/// A file's bytes given as `data` (the browser's picker), else read from `path`.
fn file(v: &Value, max: u64) -> Result<Vec<u8>, CmdError> {
    if let Some(data) = v.get("data").filter(|d| d.is_array()) {
        return serde_json::from_value(data.clone()).map_err(|_| CmdError::Params("`data`: the file's bytes".into()));
    }
    let path = p::req_str(v, "path")?.trim().to_string();
    read_capped(&path, max)
}

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("pgp.create", "Create an OpenPGP key", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            s.create_pgp_key(&account)
        })
        .params(r#"{"account"?: string}"#),
        CommandSpec::new("pgp.extend", "Extend", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            s.extend_pgp_key(&account, p::req_str(v, "fingerprint")?)
        })
        .params(r#"{"fingerprint": string (of one of the account's keys, from pgp.keys), "account"?: string}"#),
        CommandSpec::new("pgp.backup", "Save a backup", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            let fingerprint = p::req_str(v, "fingerprint")?.to_string();
            let passphrase = p::req_str(v, "passphrase")?.to_string();
            s.backup_pgp_key(&account, &fingerprint, &passphrase, p::req_str(v, "path")?)
        })
        .params(r#"{"fingerprint": string, "passphrase": string (the backup opens with it only), "path": string (an .asc file), "account"?: string}"#),
        CommandSpec::new("pgp.exportPublic", "Save my public key", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            let fingerprint = p::str(v, "fingerprint").map(str::to_string);
            s.export_public_key(&account, fingerprint.as_deref(), p::req_str(v, "path")?)
        })
        .params(r#"{"path": string (an .asc file), "fingerprint"?: string (else the key in use), "account"?: string}"#),
        CommandSpec::new("pgp.import", "Import OpenPGP key", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            let passphrase = p::str(v, "passphrase").unwrap_or_default().to_string();
            let data = file(v, 1 << 20)?;
            s.import_pgp_key_data(&account, data, &passphrase)
        })
        .params(r#"{"path": string (a secret key file: .asc, .gpg, .key), "data"?: [bytes] (the file itself, in place of path), "passphrase"?: string, "account"?: string}"#),
        CommandSpec::new("pgp.keys", "OpenPGP keys", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            let card = s.pgp_card(&account);
            let older: Vec<Value> = card.keys.iter().filter(|k| !k.in_use).map(|k| show(&k.info)).collect();
            Ok(json!({
                "inUse": card.keys.iter().find(|k| k.in_use).map(|k| show(&k.info)),
                "older": older,
                "needsBackup": card.needs_backup,
                "expiresSoon": card.expires_soon,
                "autocrypt": card.autocrypt,
            }))
        })
        .params(r#"{"account"?: string}"#),
        CommandSpec::new("pgp.forget", "Remove OpenPGP key", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            s.forget_pgp_key(&account, p::req_str(v, "fingerprint")?)
        })
        .params(r#"{"fingerprint": string (of one of the account's keys, from pgp.keys; its key ID will do), "account"?: string}"#),
        CommandSpec::new("pgp.use", "Use for sending", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            s.use_pgp_key(&account, p::req_str(v, "fingerprint")?)
        })
        .params(r#"{"fingerprint": string (an older key's, from pgp.keys), "account"?: string}"#),
        CommandSpec::new("pgp.options", "OpenPGP options", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            s.set_pgp_options(&account, p::bool(v, "autocrypt"))
        })
        .params(r#"{"autocrypt"?: bool (send the account's public key with its mail, so answers can be encrypted), "account"?: string}"#),
        CommandSpec::new("pgp.importPublic", "Import someone's key", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            let data = file(v, 8 << 20)?;
            s.import_public_keys_data(&account, &data)
        })
        .params(r#"{"path": string (a public key file, armored or binary, one key or many), "data"?: [bytes], "account"?: string}"#),
        CommandSpec::new("pgp.importAttached", "Import key", "Reading pane › Attachments", |s, v| {
            let index = p::u64(v, "index").ok_or_else(|| CmdError::Params("`index`: which attachment".into()))?;
            let data = s.attachment_bytes(usize::try_from(index).unwrap_or(usize::MAX))?;
            let account = s.reading.as_ref().map(|r| r.key.account.clone()).ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
            s.import_public_keys_data(&account, &data)
        })
        .params(r#"{"index": number (an attached key file of the open message)}"#),
        CommandSpec::new("pgp.known", "People's OpenPGP keys", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            let now = s.clock.now;
            let known: Vec<Value> = s
                .pgp_people(&account)
                .iter()
                .map(|k| {
                    json!({
                        "email": k.email, "learnt": k.learnt, "fromDate": k.from_date, "lastUsed": k.last_used,
                        "canEncrypt": k.info.encryptable(now).is_ok(), "previous": k.previous, "key": show(&k.info),
                    })
                })
                .collect();
            Ok(json!({ "known": known }))
        })
        .params(r#"{"account"?: string}"#),
        CommandSpec::new("pgp.forgetPublic", "Forget someone's key", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            s.forget_public_key(&account, p::req_str(v, "email")?)
        })
        .params(r#"{"email": string, "account"?: string}"#),
    ]
}
