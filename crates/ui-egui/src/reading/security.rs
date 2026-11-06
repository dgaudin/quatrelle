// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! The reading pane: how a message was protected (S/MIME or OpenPGP), said above it in
//! Quatrelle's own colours, outside anything the message itself draws, so a message can't pass
//! for signed. OpenPGP's lines (adapted from upstream f32b54b) also say how the signer's key came,
//! and, once, that it changed.

use quatrelle_engine::{Learnt, Lock, Reading, RevocationCheck, Seal, Security, smime};

use super::*;

/// An encrypted message not decrypted (yet, or at all): nothing of it is shown but the bar.
pub(super) fn still_locked(sec: &Security) -> bool {
    matches!(sec.lock, Some(Lock::Opening | Lock::Failed(_) | Lock::NoKey(_)))
}

fn when(s: &Session, at: i64) -> String {
    crate::i18n::date(&dates::day_label(at.saturating_add(i64::from(s.clock.offset)).div_euclid(86_400)))
}

/// Why a certificate (or, `pgp`, an OpenPGP key) isn't trusted for a message, each reason in
/// words.
fn problem(s: &Session, p: &smime::Problem, pgp: bool) -> String {
    if pgp {
        match p {
            smime::Problem::Expired(at) => return tf!("its key ran out on {date}", date = when(s, *at)),
            smime::Problem::NotYetValid(at) => return tf!("its key isn't valid until {date}", date = when(s, *at)),
            smime::Problem::OtherAddress(emails) => {
                let addresses = if emails.is_empty() { "-".to_string() } else { emails.join(", ") };
                return tf!("its key is for {addresses}, not for the sender", addresses = addresses);
            }
            smime::Problem::Revoked(at) => return tf!("its owner withdrew the key on {date}", date = when(s, *at)),
            smime::Problem::UnknownIssuer | smime::Problem::NotForEmail => {}
        }
    }
    match p {
        smime::Problem::UnknownIssuer => tl!("its certificate isn't from an authority this computer trusts").to_string(),
        smime::Problem::Expired(at) => tf!("its certificate ran out on {date}", date = when(s, *at)),
        smime::Problem::NotYetValid(at) => tf!("its certificate isn't valid until {date}", date = when(s, *at)),
        smime::Problem::OtherAddress(emails) => {
            tf!(
                "its certificate is for {addresses}, not for the sender",
                addresses = if emails.is_empty() { "-".to_string() } else { emails.join(", ") }
            )
        }
        smime::Problem::NotForEmail => tl!("its certificate isn't for signing email").to_string(),
        smime::Problem::Revoked(at) => tf!("its authority withdrew its certificate on {date}", date = when(s, *at)),
    }
}

fn line(ui: &mut Ui, t: &Tokens, icon: Icon, color: Color32, text: String, note: Option<String>, hover: Option<String>) {
    egui::Frame::new().fill(color.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 7)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let r = ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r, icon, color);
            ui.vertical(|ui| {
                ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
                if let Some(n) = note {
                    ui.add(egui::Label::new(RichText::new(n).color(t.dim).small()).wrap());
                }
            });
        });
        if let Some(h) = hover {
            r.response.on_hover_text(h);
        }
    });
    ui.add_space(6.0);
}

/// A fingerprint in groups of four, as people compare them.
fn spaced(fingerprint: &str) -> String {
    let chars: Vec<char> = fingerprint.chars().collect();
    chars.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
}

/// OpenPGP: the key kept for the sender changed with this message (said once).
fn key_changed(s: &Session, t: &Tokens, ui: &mut Ui, r: &Reading) {
    let from = r.parsed.as_ref().map(|p| p.from.email.clone()).unwrap_or_default();
    let hover = s.person_key(&r.key.account, &from).map(|k| {
        let before = k.previous.as_deref().map(spaced).unwrap_or_default();
        format!("{}\n{}", tf!("Before: {key}", key = before), tf!("Now: {key}", key = k.info.spaced()))
    });
    line(
        ui,
        t,
        Icon::Warning,
        t.warn,
        tl!("This person's OpenPGP key changed: this message came with a different key from their earlier mail.").to_string(),
        Some(tl!("If they didn't tell you they made a new key, check with them some other way: someone may be writing in their name.").to_string()),
        hover,
    );
}

/// The bars for an encrypted and/or signed message (`r`, shown).
pub(super) fn security_bar(s: &Session, t: &Tokens, ui: &mut Ui, r: &Reading, sec: &Security) {
    if sec.key_changed {
        key_changed(s, t, ui, r);
    }
    match &sec.lock {
        Some(Lock::Opening) => line(ui, t, Icon::Lock, t.dim, tl!("Encrypted. Decrypting it…").to_string(), None, None),
        Some(Lock::Failed(why)) => {
            line(ui, t, Icon::Lock, t.danger, tf!("This message is encrypted, and it couldn't be decrypted: {error}", error = tl!(why)), None, None)
        }
        Some(Lock::NoKey(to)) => {
            let note = if s.pgp_key(&r.key.account).is_none() {
                tl!("To read mail encrypted to you, make or import your OpenPGP key in File › Info › Security.").to_string()
            } else {
                tl!("It was encrypted to someone else's key, or to one of yours Quatrelle wasn't given: import that one in File › Info › Security.")
                    .to_string()
            };
            let hover = (!to.is_empty()).then(|| tf!("Encrypted to the key {keys}", keys = to.join(", ")));
            line(ui, t, Icon::Lock, t.warn, tl!("This message is encrypted to a key Quatrelle doesn't have.").to_string(), Some(note), hover);
        }
        Some(Lock::Open { .. }) => {
            let note = sec
                .unauthenticated()
                .then(|| tl!("It isn't signed, so it could have been changed on the way: be careful with what it asks.").to_string());
            line(
                ui,
                t,
                Icon::Lock,
                if note.is_some() { t.warn } else { t.ok },
                tl!("Encrypted: only you and its other recipients can read it.").to_string(),
                note,
                None,
            );
        }
        None => {}
    }
    if sec.headers {
        ui.label(RichText::new(tl!("Its subject and addresses were protected inside it too.")).color(t.faint).small());
        ui.add_space(4.0);
    }
    let Some(signed) = &sec.signed else { return };
    let named = |name: &str, emails: &[String]| match emails.first() {
        Some(e) if !name.is_empty() && !name.eq_ignore_ascii_case(e) => format!("{name} <{e}>"),
        Some(e) => e.clone(),
        None => name.to_string(),
    };
    let who = signed.cert.as_ref().map(|c| named(&c.name, &c.emails)).or_else(|| signed.key.as_ref().map(|k| named(&k.name, &k.emails)));
    let signed_at = signed.at.map(|at| format!("\n{}", tf!("Signed {date}", date = crate::i18n::date(&dates::full_time(at, &s.clock)))));
    let hover = signed
        .cert
        .as_ref()
        .map(|c| {
            let h = tf!("Issued by {issuer}, valid until {date}", issuer = c.issuer, date = when(s, c.not_after));
            format!("{h}\n{}\n{}", c.key, c.fingerprint)
        })
        .or_else(|| signed.key.as_ref().map(|k| format!("{}\n{}", tf!("OpenPGP key {fingerprint}", fingerprint = k.spaced()), k.algorithm)))
        .map(|h| h + signed_at.as_deref().unwrap_or_default());
    // How an OpenPGP signer's key came.
    let came = signed.key.as_ref().map(|_| match signed.learnt {
        Some(Learnt::FromMail) => tl!("Their key came with their own mail."),
        Some(Learnt::ByHand) => tl!("You imported their key."),
        None => tl!("It's one of your own keys."),
    });
    let checked = match &signed.revocation {
        Some(RevocationCheck::Asking) => Some(tl!("Asking its authority whether the certificate was withdrawn…").to_string()),
        Some(RevocationCheck::Good) => Some(tl!("Its authority says the certificate wasn't withdrawn.").to_string()),
        Some(RevocationCheck::Unknown(why)) => Some(tf!("Whether the certificate was withdrawn couldn't be checked: {why}.", why = tl!(why))),
        None => None,
    };
    let weak = signed.weak.then(|| tl!("Signed with SHA-1, which no longer proves much.").to_string());
    let notes: Vec<String> = [weak, checked, came.map(str::to_string)].into_iter().flatten().collect();
    let weak = (!notes.is_empty()).then(|| notes.join("\n"));
    match &signed.seal {
        Seal::Trusted => {
            let text = match &signed.key {
                Some(k) => tf!("Signed by {signer}: the signature holds (OpenPGP key {key}).", signer = who.unwrap_or_default(), key = k.key_id),
                None => {
                    let issuer = signed.cert.as_ref().map(|c| c.issuer.clone()).unwrap_or_default();
                    tf!("Signed by {signer}: the signature holds (certificate from {issuer}).", signer = who.unwrap_or_default(), issuer = issuer)
                }
            };
            line(ui, t, Icon::Seal, t.ok, text, weak, hover);
        }
        Seal::UnknownKey(key) => line(
            ui,
            t,
            Icon::Seal,
            t.warn,
            tf!("Signed with the OpenPGP key {key}, which Quatrelle doesn't have, so the signature can't be checked.", key = key),
            Some(tl!("Their key comes with their mail, or can be imported from a file in File › Info › Security.").to_string()),
            None,
        ),
        Seal::Untrusted(problems) => {
            let why: Vec<String> = problems.iter().map(|p| problem(s, p, signed.key.is_some())).collect();
            let revoked = problems.iter().any(|p| matches!(p, smime::Problem::Revoked(_)));
            line(
                ui,
                t,
                Icon::Seal,
                if revoked { t.danger } else { t.warn },
                tf!("Signed by {signer}, but {why}.", signer = who.unwrap_or_default(), why = why.join("; ")),
                weak,
                hover,
            );
        }
        Seal::Broken(why) => line(ui, t, Icon::Seal, t.danger, tf!("The signature doesn't hold: {why}.", why = tl!(why)), None, hover),
    }
}
