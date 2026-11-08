// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! *Move to* across accounts: IMAP mail can go to the Archives account on this computer (archiving
//! it, whichever folder is picked: it lands in the folder it came from), and archived mail to a
//! folder of an IMAP account (put back on the server).

use super::*;

/// The folders of other accounts offered below the account's own, for a selection from `account`:
/// the Archives account's for IMAP mail, the IMAP accounts' for archived mail. Their words
/// matching `needle` only. Returns the parameters of the `mail.move` picked, if one is.
pub(super) fn other_accounts(ui: &mut Ui, s: &Session, t: &Tokens, account: &str, needle: &str) -> Option<serde_json::Value> {
    let matches = |f: &Folder| quatrelle_engine::model::utf7::decode(&f.path).to_lowercase().contains(needle);
    let mut chosen = None;
    if s.is_imap(account) {
        let archives = s.archives_id().and_then(|id| s.account(&id));
        let name = archives.map_or("Archives", |a| a.config.name.as_str());
        let target = archives.map_or("archives", |a| a.config.id.as_str());
        let folders: Vec<&Folder> = archives.map(|a| a.folders.iter().filter(|f| f.selectable && matches(f)).collect()).unwrap_or_default();
        if needle.is_empty() || name.to_lowercase().contains(needle) || !folders.is_empty() {
            ui.add_space(6.0);
            if ui.selectable_label(false, RichText::new(name).strong()).on_hover_text(tl!("Archive on this computer")).clicked() {
                chosen = Some(json!({"account": target, "folder": "-"}));
            }
            for f in folders {
                let indent = "    ".repeat(f.depth());
                if ui.selectable_label(false, format!("{indent}{}", crate::i18n::folder(f))).clicked() {
                    chosen = Some(json!({"account": target, "folder": "-"}));
                }
            }
        }
    } else if s.archives_id().as_deref() == Some(account) {
        for other in s.accounts.iter().filter(|a| s.is_imap(&a.config.id)) {
            let folders: Vec<&Folder> = other.folders.iter().filter(|f| f.selectable && f.mailbox.is_none() && matches(f)).collect();
            if folders.is_empty() {
                continue;
            }
            ui.add_space(6.0);
            ui.label(RichText::new(&other.config.email).strong().color(t.dim));
            for f in folders {
                let indent = "    ".repeat(f.depth());
                if ui.selectable_label(false, format!("{indent}{}", crate::i18n::folder(f))).clicked() {
                    chosen = Some(json!({"account": other.config.id, "folder": f.path}));
                }
            }
        }
    }
    chosen
}
