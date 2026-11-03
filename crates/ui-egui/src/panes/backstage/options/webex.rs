// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The File page: Options › Webex, the site and integration meetings are scheduled with.

use quatrelle_engine::model::{WEBEX_REDIRECT_URI, WebexError};

use super::*;

/// A text box saved once it's left: `field` is the `webex.settings` param it sets, `current` what
/// is kept now. What is typed stays in the box when the setting is refused (a notice says why).
/// A secret is cleared once it's sent, and an empty one is left alone rather than forgotten.
fn field(ui: &mut Ui, acts: &mut Vec<Action>, field: &'static str, current: &str, hint: &str, secret: bool) {
    let id = egui::Id::unique(("options-webex", field));
    let mut text: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| current.to_string());
    let r = ui.add(egui::TextEdit::singleline(&mut text).hint_text(hint).password(secret).desired_width(320.0));
    if r.lost_focus() && text.trim() != current && !(secret && text.trim().is_empty()) {
        acts.push(Action::Run("webex.settings", json!({ field: text.trim() })));
        if secret {
            text.clear();
        }
    }
    ui.data_mut(|d| d.insert_temp(id, text));
}

/// What went wrong signing in, in the interface's language.
fn said(e: &WebexError) -> String {
    match e {
        WebexError::SignIn => tl!("Sign in to Webex again").to_string(),
        WebexError::Forbidden => tl!("Your Webex account can't schedule meetings on this site").to_string(),
        other => other.to_string(),
    }
}

/// Webex's site, the integration's client id and secret, the address to register, then the
/// sign-in. Not drawn where there is no browser to sign in with (the web app).
pub(super) fn webex(s: &Session, st: &UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let t = &st.tokens;
    // Asked for by `Action::WebexOptions`, once.
    if st.webex_scroll.take() {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
    card(
        ui,
        t,
        tl!("Webex"),
        Some(tl!("Schedule Webex meetings from an appointment's window. Register an integration at developer.webex.com, then sign in with it here.")),
        |ui| {
            setting_row(ui, t, true, name(t, tl!("Site")), None, |ui| field(ui, acts, "site", &s.prefs.webex_site, "acme.webex.com", false));
            setting_row(ui, t, false, name(t, tl!("Client ID")), None, |ui| field(ui, acts, "clientId", &s.prefs.webex_client_id, "", false));
            setting_row(
                ui,
                t,
                false,
                name(t, tl!("Client secret")),
                Some(note(t, tl!("Kept in the system keychain, and never shown again"))),
                |ui| field(ui, acts, "secret", "", "", true),
            );
            setting_row(
                ui,
                t,
                false,
                name(t, tl!("Redirect URI")),
                Some(note(t, tl!("Paste it where the integration asks for its redirect URI"))),
                |ui| {
                    // Against the right edge, the button first.
                    if ui.button(tl!("Copy")).clicked() {
                        ui.ctx().copy_text(WEBEX_REDIRECT_URI.to_string());
                    }
                    ui.label(RichText::new(WEBEX_REDIRECT_URI).monospace().color(t.text));
                },
            );
            let user = s.prefs.webex_user.trim();
            let who = if user.is_empty() { tl!("Signed in").to_string() } else { tf!("Signed in as {user}", user = user) };
            if s.prefs.webex_signed_in {
                setting_row(ui, t, false, name(t, &who), None, |ui| {
                    if ui.button(tl!("Sign out")).clicked() {
                        acts.push(Action::Run("webex.signOut", json!({})));
                    }
                });
            } else {
                setting_row(ui, t, false, name(t, tl!("Account")), None, |ui| {
                    if s.webex.signing_in {
                        ui.label(RichText::new(tl!("Waiting for the browser…")).color(t.dim));
                        ui.spinner();
                    } else if ui.add_enabled(!s.prefs.webex_client_id.is_empty(), egui::Button::new(tl!("Sign in to Webex"))).clicked() {
                        acts.push(Action::Run("webex.signIn", json!({})));
                    }
                });
            }
            if let Some(e) = &s.webex.sign_in_error {
                ui.label(RichText::new(said(e)).color(t.danger).size(12.5));
                ui.add_space(4.0);
            }
        },
    );
}
