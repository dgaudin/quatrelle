// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Dialogs: OpenPGP's (adapted from upstream f32b54b), drawn as the `.p12` one is: a secret key
//! file's passphrase, the passphrase a backup is saved behind (typed twice), and a question
//! before one of the account's keys is forgotten.

use super::*;

/// A file's name, from its path.
fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// A passphrase field (`focus`: it takes the keyboard when nothing has it); whether Enter ended
/// it.
fn passphrase_field(ui: &mut Ui, text: &mut String, focus: bool) -> bool {
    let r = ui.add(egui::TextEdit::singleline(text).password(true).desired_width(f32::INFINITY));
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if focus && ui.memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
    enter
}

fn faint(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(t.faint).small()).wrap());
}

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::PgpKey { account, path, passphrase } => {
            ui.set_width(420.0);
            title(ui, t, "Import an OpenPGP key");
            let ask = tf!("The passphrase {file} is locked with (none, if it has none):", file = file_name(path));
            ui.add(egui::Label::new(RichText::new(ask).color(t.dim)).wrap());
            ui.add_space(8.0);
            let enter = passphrase_field(ui, &mut passphrase.0, true);
            ui.add_space(4.0);
            faint(ui, t, tl!("Its secret part is kept in the system's keychain, not in Quatrelle's files; the passphrase isn't kept."));
            let (ok, cancel) = buttons(ui, t, "Import", true);
            if ok || enter {
                acts.push(Action::Run("pgp.import", json!({"account": account, "path": path, "passphrase": passphrase.0})));
                return false;
            }
            !cancel
        }
        Dialog::PgpBackup { account, fingerprint, path, passphrase, again } => {
            ui.set_width(440.0);
            title(ui, t, "Save a backup of your OpenPGP key");
            let ask = tf!("A passphrase to lock {file} with, twice:", file = file_name(path));
            ui.add(egui::Label::new(RichText::new(ask).color(t.dim)).wrap());
            ui.add_space(8.0);
            let first = passphrase_field(ui, &mut passphrase.0, true);
            ui.add_space(4.0);
            let second = passphrase_field(ui, &mut again.0, false);
            let same = !passphrase.0.is_empty() && passphrase.0 == again.0;
            ui.add_space(4.0);
            if !again.0.is_empty() && !same {
                ui.label(RichText::new(tl!("The two passphrases aren't the same.")).color(t.danger).small());
            }
            faint(
                ui,
                t,
                tl!(
                    "Without this passphrase the backup can't be opened, by you or anyone: keep both somewhere safe. GnuPG and other OpenPGP programs read it too."
                ),
            );
            let (ok, cancel) = buttons(ui, t, "Save", same);
            if same && (ok || first || second) {
                acts.push(Action::Run(
                    "pgp.backup",
                    json!({"account": account, "fingerprint": fingerprint, "path": path, "passphrase": passphrase.0}),
                ));
                return false;
            }
            !cancel
        }
        Dialog::PgpForget { account, fingerprint, who, backed_up } => {
            ui.set_width(420.0);
            title(ui, t, "Forget this OpenPGP key?");
            let what = tf!("Your key for {who} goes from this computer, its secret part from the system's keychain too.", who = who);
            ui.add(egui::Label::new(RichText::new(what).color(t.dim)).wrap());
            ui.add_space(4.0);
            let then = if *backed_up {
                tl!("Mail encrypted to it can't be read here any more, until its backup is imported again.")
            } else {
                tl!("No backup of it was saved: mail encrypted to it can't be read any more, here or anywhere.")
            };
            ui.add(egui::Label::new(RichText::new(then).color(if *backed_up { t.dim } else { t.danger })).wrap());
            let (ok, cancel) = buttons(ui, t, "Forget", true);
            if ok {
                acts.push(Action::Run("pgp.forget", json!({"account": account, "fingerprint": fingerprint})));
                return false;
            }
            !cancel
        }
        _ => false,
    }
}
