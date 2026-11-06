// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! File › Info › Security: the OpenPGP card (adapted from upstream f32b54b), under the S/MIME one.
//! Nothing until the account has a key: then « Create an OpenPGP key » or « Import a key… ». With
//! one, its fingerprint and end, « Save a backup… » (and, until one is saved, a line that says
//! none is), « Extend » in the month before it runs out, whether the public key goes with the
//! account's mail; then the keys Quatrelle has for the people it writes to, each with how it came
//! and « Forget ». What needs a file picks it here ([`PgpAsk`]), then asks in a dialog.

use quatrelle_engine::pgp::KeyInfo;
use quatrelle_engine::{Learnt, OwnKey, PgpCard};

use super::*;
use crate::{FileAsk, MailApp};

/// The most correspondents' keys listed (the most recently used first); the others are counted.
const PEOPLE_SHOWN: usize = 20;

/// What the OpenPGP card asks of the window: a file to pick or a place to save one, then a dialog
/// or a command.
pub enum PgpAsk {
    /// A secret key file for the account, then its passphrase.
    ImportKey(String),
    /// Where to save a backup of the account's key `fingerprint` (for `email`), then its
    /// passphrase.
    Backup { account: String, fingerprint: String, email: String },
    /// Where to write the account's public key `fingerprint` (for `email`).
    ExportPublic { account: String, fingerprint: String, email: String },
    /// A file of correspondents' keys, kept.
    ImportPublic(String),
}

impl PgpAsk {
    fn again(&self) -> PgpAsk {
        match self {
            PgpAsk::ImportKey(a) => PgpAsk::ImportKey(a.clone()),
            PgpAsk::Backup { account, fingerprint, email } => {
                PgpAsk::Backup { account: account.clone(), fingerprint: fingerprint.clone(), email: email.clone() }
            }
            PgpAsk::ExportPublic { account, fingerprint, email } => {
                PgpAsk::ExportPublic { account: account.clone(), fingerprint: fingerprint.clone(), email: email.clone() }
            }
            PgpAsk::ImportPublic(a) => PgpAsk::ImportPublic(a.clone()),
        }
    }
}

/// A file name made of an address (nothing that would leave the folder).
fn file_of(email: &str, ending: &str) -> String {
    let name: String = email.chars().filter(|c| c.is_alphanumeric() || "@.-_+".contains(*c)).collect();
    format!("{}{ending}", if name.is_empty() { "openpgp" } else { &name })
}

impl MailApp {
    /// The card's file asked for (the system's dialog answers in a later frame), then what it's
    /// for.
    pub(crate) fn pgp_ask(&mut self, ask: PgpAsk) {
        let file = match &ask {
            PgpAsk::ImportKey(_) | PgpAsk::ImportPublic(_) => FileAsk::PgpKey,
            PgpAsk::Backup { email, .. } => FileAsk::Save(file_of(email, "-openpgp-backup.asc")),
            PgpAsk::ExportPublic { email, .. } => FileAsk::Save(file_of(email, ".asc")),
        };
        let Some(path) = self.file_answer_one(file, Action::Pgp(ask.again())) else { return };
        let path = path.to_string_lossy().into_owned();
        match ask {
            PgpAsk::ImportKey(account) => self.ui.dialog = Some(Dialog::PgpKey { account, path, passphrase: Default::default() }),
            PgpAsk::Backup { account, fingerprint, .. } => {
                self.ui.dialog = Some(Dialog::PgpBackup { account, fingerprint, path, passphrase: Default::default(), again: Default::default() });
            }
            PgpAsk::ExportPublic { account, fingerprint, .. } => {
                match self.session.run("pgp.exportPublic", &json!({"account": account, "fingerprint": fingerprint, "path": path})) {
                    Ok(_) => {
                        let file = std::path::Path::new(&path).file_name().unwrap_or_default().to_string_lossy().to_string();
                        self.session
                            .notice(tf!("Your OpenPGP public key is saved in {file}: give it to those who'll write to you", file = file), false);
                    }
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            // The session says what it kept.
            PgpAsk::ImportPublic(account) => {
                if let Err(e) = self.session.run("pgp.importPublic", &json!({"account": account, "path": path})) {
                    self.session.notice(e.to_string(), true);
                }
            }
        }
    }
}

/// A day, as the interface writes dates.
fn day(s: &Session, at: i64) -> String {
    crate::i18n::date(&quatrelle_engine::model::dates::day_label(at.saturating_add(i64::from(s.clock.offset)).div_euclid(86_400)))
}

/// How long a key is valid, in words, and whether it can't be used any more.
fn validity(s: &Session, k: &KeyInfo) -> (String, bool) {
    match (k.revoked, k.expires) {
        (Some(at), _) => (tf!("Withdrawn on {date}", date = day(s, at)), true),
        (None, Some(at)) if at <= s.clock.now => (tf!("It ran out on {date}", date = day(s, at)), true),
        (None, Some(at)) => (tf!("Valid until {date}", date = day(s, at)), false),
        (None, None) => (tl!("No end date").to_string(), false),
    }
}

/// Whom a key is for.
fn who(k: &KeyInfo) -> String {
    k.emails.first().cloned().unwrap_or_else(|| k.name.clone())
}

pub(super) fn pgp(s: &Session, t: &Tokens, ui: &mut Ui, a: &AccountState, acts: &mut Vec<Action>) {
    let id = a.config.id.clone();
    let about =
        tl!("Sign and encrypt with OpenPGP, as many people and mail programs do: with a key of your own, and the keys of the people you write to.");
    card(ui, t, tl!("OpenPGP"), Some(about), |ui| {
        // The browser keeps no secret keys yet.
        if cfg!(target_arch = "wasm32") {
            setting_row(ui, t, true, name(t, tl!("OpenPGP keys work in the desktop app")), None, |_| {});
            return;
        }
        let c = s.pgp_card(&id);
        match c.keys.iter().find(|k| k.in_use) {
            None => setting_row(
                ui,
                t,
                true,
                name(t, tl!("No OpenPGP key")),
                Some(note(
                    t,
                    tl!("Nothing is signed or encrypted with OpenPGP until the account has a key; mail signed or encrypted with it is read anyway."),
                )),
                |ui| {
                    if widgets::primary_button(ui, tl!("Create an OpenPGP key"), t, true).clicked() {
                        acts.push(Action::Run("pgp.create", json!({"account": id})));
                    }
                    if ui.button(tl!("Import a key…")).on_hover_text(tl!("A secret key file (.asc or .gpg), with its passphrase")).clicked() {
                        acts.push(Action::Pgp(PgpAsk::ImportKey(id.clone())));
                    }
                },
            ),
            Some(k) => own_key(s, t, ui, &id, k, &c, acts),
        }
        for k in c.keys.iter().filter(|k| !k.in_use) {
            let about = tf!("Older key {key}: kept to decrypt mail sent to it", key = k.info.key_id);
            setting_row(ui, t, false, name(t, &who(&k.info)), Some(note(t, &about)), |ui| {
                if ui.button(RichText::new(tl!("Forget")).color(t.danger)).on_hover_text(tl!("Mail encrypted to it can't be read any more")).clicked()
                {
                    acts.push(forget(&id, k));
                }
                if ui
                    .button(tl!("Use for sending"))
                    .on_hover_text(tl!("Sign and decrypt with this key again; the one in use is kept to decrypt"))
                    .clicked()
                {
                    acts.push(Action::Run("pgp.use", json!({"account": id, "fingerprint": k.info.fingerprint})));
                }
            });
        }
        if c.keys.iter().any(|k| k.in_use) {
            setting_row(
                ui,
                t,
                false,
                name(t, tl!("Send my public key with my messages")),
                Some(note(t, tl!("Their mail programs can then check your signature and encrypt their answers to you (Autocrypt)."))),
                |ui| {
                    let mut on = c.autocrypt;
                    if widgets::switch(ui, t, &mut on, tl!("Send my public key with my messages")).changed() {
                        acts.push(Action::Run("pgp.options", json!({"account": id, "autocrypt": on})));
                    }
                },
            );
        }
        people(s, t, ui, &id, &c, acts);
    });
}

/// « Forget » on one of the account's keys: asked first.
fn forget(account: &str, k: &OwnKey) -> Action {
    Action::Dialog(Dialog::PgpForget {
        account: account.to_string(),
        fingerprint: k.info.fingerprint.clone(),
        who: who(&k.info),
        backed_up: k.backed_up,
    })
}

/// The key in use: its fingerprint and end, its backup, its public part; the reminder that no
/// backup is saved; « Extend » in the month before it runs out.
fn own_key(s: &Session, t: &Tokens, ui: &mut Ui, id: &str, k: &OwnKey, c: &PgpCard, acts: &mut Vec<Action>) {
    let (valid, bad) = validity(s, &k.info);
    let about = RichText::new(format!("{} · {valid}", k.info.spaced())).size(12.0).color(if bad { t.danger } else { t.faint });
    let email = who(&k.info);
    setting_row(ui, t, true, name(t, &email), Some(about), |ui| {
        if ui.button(RichText::new(tl!("Forget")).color(t.danger)).on_hover_text(tl!("Mail encrypted to it can't be read any more")).clicked() {
            acts.push(forget(id, k));
        }
        if ui.button(tl!("Export public key…")).on_hover_text(tl!("A file to give those who'll write to you")).clicked() {
            acts.push(Action::Pgp(PgpAsk::ExportPublic { account: id.to_string(), fingerprint: k.info.fingerprint.clone(), email: email.clone() }));
        }
        if ui.button(tl!("Save a backup…")).on_hover_text(tl!("The secret key in a file, locked with a passphrase")).clicked() {
            acts.push(Action::Pgp(PgpAsk::Backup { account: id.to_string(), fingerprint: k.info.fingerprint.clone(), email: email.clone() }));
        }
    });
    if !k.backed_up {
        setting_row(
            ui,
            t,
            false,
            RichText::new(tl!("No backup of this key is saved")).color(t.warn),
            Some(note(
                t,
                tl!(
                    "If this computer is lost or its keychain emptied, mail encrypted to this key can't be read any more. Save a backup, and keep it with its passphrase somewhere safe."
                ),
            )),
            |_| {},
        );
    }
    if let Some(end) = c.expires_soon {
        let when = if end <= s.clock.now {
            tf!("This key ran out on {date}", date = day(s, end))
        } else {
            tf!("This key runs out on {date}", date = day(s, end))
        };
        let about = note(t, tl!("Extending it signs it again to run three years from today; then send your public key again to those who have it."));
        setting_row(ui, t, false, RichText::new(when).color(t.warn), Some(about), |ui| {
            if widgets::primary_button(ui, tl!("Extend"), t, true).clicked() {
                acts.push(Action::Run("pgp.extend", json!({"account": id, "fingerprint": k.info.fingerprint})));
            }
        });
    }
}

/// The keys of the people the account writes to: the most recently used first, each with its
/// fingerprint, how it came and « Forget ».
fn people(s: &Session, t: &Tokens, ui: &mut Ui, id: &str, c: &PgpCard, acts: &mut Vec<Action>) {
    let label = tf!("Keys of the people you write to: {n}", n = c.people.len());
    setting_row(ui, t, false, name(t, &label), Some(note(t, tl!("They come with their mail, or from a key file they gave you."))), |ui| {
        if ui.button(tl!("Import someone's key…")).clicked() {
            acts.push(Action::Pgp(PgpAsk::ImportPublic(id.to_string())));
        }
    });
    let mut people: Vec<_> = c.people.iter().collect();
    people.sort_by_key(|p| std::cmp::Reverse(p.last_used));
    for p in people.iter().take(PEOPLE_SHOWN) {
        let came = match p.learnt {
            Learnt::FromMail => tl!("from their mail"),
            Learnt::ByHand => tl!("imported by you"),
        };
        let (_, bad) = validity(s, &p.info);
        let about = RichText::new(format!("{} · {came}", p.info.spaced())).size(12.0).color(if bad { t.danger } else { t.faint });
        setting_row(ui, t, false, name(t, &p.email), Some(about), |ui| {
            if ui.button(tl!("Forget")).on_hover_text(tl!("Their signatures can't be checked any more, until their key comes again")).clicked() {
                acts.push(Action::Run("pgp.forgetPublic", json!({"account": id, "email": p.email})));
            }
        });
    }
    if people.len() > PEOPLE_SHOWN {
        ui.label(note(t, &tf!("And {n} more", n = people.len() - PEOPLE_SHOWN)));
        ui.add_space(6.0);
    }
}
