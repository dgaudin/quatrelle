// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! The File page: Options › Teams. Teams switched on, the application and directory ids the
//! organisation registered, the sign-in (Microsoft's page, while it's waited for, to open again or
//! copy; Sign out saying how many messages not sent it lets go), the permission to make Teams
//! meetings from the calendar, the announcements, where the copy stands, and a copy made for
//! another account that was set aside.
//!
//! What is hostile here: the name, address and permission the worker reports came from Graph
//! (each drawn on one line, cut short); the path of a copy set aside is never shown.

use quatrelle_engine::model::pim::teams::{Status, one_line};

use super::*;

/// The guide to registering the application, in the repository the About page opens.
fn guide() -> String {
    format!("{}/blob/main/docs/teams.md", crate::backstage::REPOSITORY)
}

/// A text box saved once it's left: `field` is the `teams.settings` param it sets, `current`
/// what is kept now. What is typed stays in the box when it's refused (a notice says why).
fn field(ui: &mut Ui, acts: &mut Vec<Action>, field: &'static str, current: &str) {
    let id = egui::Id::unique(("options-teams", field));
    let mut text: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| current.to_string());
    let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(320.0));
    if r.lost_focus() && text.trim() != current {
        acts.push(Action::Run("teams.settings", json!({ field: text.trim() })));
    }
    ui.data_mut(|d| d.insert_temp(id, text));
}

/// Where the copy stands, and whether it's something to fix (drawn in the danger colour).
fn status_line(status: &Status) -> Option<(String, bool)> {
    Some(match status {
        Status::SignedOut | Status::SigningIn | Status::NoKeychain => return None,
        Status::Copying { done, of } => (tf!("Copying your conversations… {done} of {of} conversations", done = done, of = of), false),
        Status::UpToDate => (tl!("Your conversations are up to date").to_string(), false),
        Status::Offline => (tl!("Offline").to_string(), false),
        Status::SignInAgain => (tl!("Teams: sign in again to go on").to_string(), true),
        Status::PermissionMissing { scope } => (
            tf!(
                "Quatrelle needs the permission {scope}: sign in again to accept it, or ask your administrator to grant admin consent in Microsoft Entra",
                scope = one_line(scope)
            ),
            true,
        ),
        Status::Failed { why } => (crate::conversations::why_stopped(why), true),
    })
}

/// The id of the question Sign out asks (whether the local copy goes too).
fn asking() -> egui::Id {
    egui::Id::unique("options-teams-sign-out")
}

/// Whether Sign out's question is asked (Escape answers it, not the File page).
pub(crate) fn sign_out_asked(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(asking())).unwrap_or(false)
}

/// The File page closed: its question isn't asked any more (it would come back with the page).
pub(crate) fn forget_sign_out(ctx: &egui::Context) {
    if sign_out_asked(ctx) {
        ctx.data_mut(|d| d.remove::<bool>(asking()));
    }
}

pub(super) fn teams(s: &Session, st: &UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    // Signing in needs a browser on this computer: not in the web app.
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let t = &st.tokens;
    // Asked for by `Action::TeamsOptions`, once.
    if st.teams_scroll.take() {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
    let p = &s.prefs.teams;
    card(
        ui,
        t,
        tl!("Teams"),
        Some(tl!(
            "Read your Teams conversations in Quatrelle, from a copy kept on this computer. Your organization registers an application for it first."
        )),
        |ui| {
            setting_row(ui, t, true, name(t, tl!("Use Teams in Quatrelle")), None, |ui| {
                let mut on = p.on;
                if widgets::switch(ui, t, &mut on, tl!("Use Teams in Quatrelle")).changed() {
                    acts.push(Action::Run("teams.settings", json!({"on": on})));
                }
            });
            setting_row(ui, t, false, name(t, tl!("Application (client) ID")), None, |ui| field(ui, acts, "clientId", &p.client_id));
            setting_row(ui, t, false, name(t, tl!("Directory (tenant) ID")), Some(note(t, tl!("Your organization's ID, or its domain"))), |ui| {
                field(ui, acts, "tenant", &p.tenant)
            });
            setting_row(ui, t, false, name(t, tl!("Registering the application")), None, |ui| {
                if ui.link(tl!("How to register the application")).clicked() {
                    acts.push(Action::OpenUrl(guide()));
                }
            });
            account(s, t, ui, acts);
            meetings(s, t, ui, acts);
            setting_row(ui, t, false, name(t, tl!("Announce new messages")), None, |ui| {
                let mut on = p.notify;
                if widgets::switch(ui, t, &mut on, tl!("Announce new messages")).changed() {
                    acts.push(Action::Run("teams.settings", json!({"notify": on})));
                }
            });
            setting_row(ui, t, false, name(t, tl!("Only messages sent to me alone or naming me")), None, |ui| {
                let mut on = p.notify_only_direct;
                if widgets::switch(ui, t, &mut on, tl!("Only messages sent to me alone or naming me")).changed() {
                    acts.push(Action::Run("teams.settings", json!({"notifyOnlyDirect": on})));
                }
            });
            let why = note(t, tl!("Teams refuses their messages: mostly old conversations you're no longer in"));
            setting_row(ui, t, false, name(t, tl!("Show conversations that can't be read")), Some(why), |ui| {
                let mut on = p.show_unreadable;
                if widgets::switch(ui, t, &mut on, tl!("Show conversations that can't be read")).changed() {
                    acts.push(Action::Run("teams.settings", json!({"showUnreadable": on})));
                }
            });
            if s.teams.set_aside.is_some() {
                setting_row(ui, t, false, name(t, tl!("A copy made for another account was set aside")), None, |ui| {
                    if ui.button(tl!("Delete that copy")).clicked() {
                        acts.push(Action::Run("teams.deleteSetAside", json!({})));
                    }
                });
            }
        },
    );
    sign_out_question(ui.ctx(), t, s.teams.unsent, acts);
}

/// Waiting for the browser: a spinner, with the words until Microsoft's page is known (then the
/// panel under the row says more, [`crate::conversations::sign_in_waiting`]).
fn waiting(s: &Session, t: &Tokens, ui: &mut Ui) {
    if s.teams.sign_in_page.is_none() {
        ui.label(RichText::new(tl!("Waiting for the browser…")).color(t.dim));
    }
    ui.spinner();
}

/// Who is signed in and Sign out, or Sign in; signing in, Microsoft's page to open again or copy;
/// where the copy stands, and a sign-in that won't be remembered.
fn account(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    let p = &s.prefs.teams;
    let status = &s.teams.status;
    if p.signed_in {
        let (name_, address) = (one_line(&p.name), one_line(&p.address));
        let who = match (name_.is_empty(), address.is_empty()) {
            (false, false) => tf!("Signed in as {name} ({address})", name = name_, address = address),
            (true, false) => tf!("Signed in as {user}", user = address),
            (false, true) => tf!("Signed in as {user}", user = name_),
            (true, true) => tl!("Signed in").to_string(),
        };
        setting_row(ui, t, false, name(t, tl!("Account")), Some(note(t, &who)), |ui| {
            if ui.button(tl!("Sign out")).clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(asking(), true));
            }
            // The sign-in no longer holds, lacks a permission, or failed: it's tried again, the
            // copy kept.
            if p.on && *status == Status::SigningIn {
                waiting(s, t, ui);
            } else if p.on && crate::conversations::sign_in_again(status) && ui.button(tl!("Sign in")).clicked() {
                acts.push(Action::Run("teams.signIn", json!({})));
            }
        });
    } else {
        let ready = p.on && !p.client_id.is_empty() && !p.tenant.is_empty();
        let about = (!ready).then(|| note(t, tl!("Switch Teams on and fill in both IDs first")));
        setting_row(ui, t, false, name(t, tl!("Account")), about, |ui| {
            if *status == Status::SigningIn {
                waiting(s, t, ui);
            } else if ui.add_enabled(ready, egui::Button::new(tl!("Sign in"))).clicked() {
                acts.push(Action::Run("teams.signIn", json!({})));
            }
        });
    }
    crate::conversations::sign_in_waiting(s, t, ui, acts);
    if let Some((line, wrong)) = status_line(status) {
        ui.label(RichText::new(line).color(if wrong { t.danger } else { t.dim }).size(12.5));
        ui.add_space(4.0);
    }
    if s.teams.not_remembered {
        ui.label(RichText::new(tl!("Your sign-in won't be remembered: no keychain is available")).color(t.dim).size(12.5));
        ui.add_space(4.0);
    }
}

/// Once signed in, whether Teams meetings can be made from the calendar: Authorize Teams meetings
/// opens Microsoft's sign-in for that permission (the conversations don't need it), until it's
/// given.
fn meetings(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    let p = &s.prefs.teams;
    if !(p.on && p.signed_in) {
        return;
    }
    setting_row(ui, t, false, name(t, tl!("Teams meetings")), None, |ui| {
        if p.meetings {
            ui.label(RichText::new(tl!("Teams meetings are allowed")).color(t.dim));
        } else if ui.add_enabled(s.teams.status != Status::SigningIn, egui::Button::new(tl!("Authorize Teams meetings"))).clicked() {
            acts.push(Action::Run("teams.signIn", json!({"meetings": true})));
        }
    });
}

/// Sign out's question: keep the copy of the conversations on this computer, or delete it too;
/// the messages not sent yet (`unsent`), which signing out lets go whatever the answer, counted.
fn sign_out_question(ctx: &egui::Context, t: &Tokens, unsent: u32, acts: &mut Vec<Action>) {
    if !sign_out_asked(ctx) {
        return;
    }
    let mut answer: Option<Option<bool>> = None;
    egui::Modal::new(asking().with("modal")).frame(crate::dialogs::dialog_frame(t)).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(RichText::new(tl!("Sign out of Teams")).font(theme::bold(18.0)).color(t.text));
        ui.add_space(4.0);
        ui.label(RichText::new(tl!("Delete the copy of your conversations kept on this computer too?")).color(t.text));
        if unsent > 0 {
            ui.add_space(4.0);
            ui.label(RichText::new(tf!("{n} messages not sent yet will be lost", n = unsent)).color(t.danger));
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button(tl!("Keep the copy")).clicked() {
                answer = Some(Some(false));
            }
            if ui.button(tl!("Delete the copy")).clicked() {
                answer = Some(Some(true));
            }
            if ui.button(tl!("Cancel")).clicked() {
                answer = Some(None);
            }
        });
    });
    if answer.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        answer = Some(None);
    }
    let Some(answer) = answer else { return };
    ctx.data_mut(|d| d.remove::<bool>(asking()));
    if let Some(delete_copy) = answer {
        acts.push(Action::Run("teams.signOut", json!({"deleteCopy": delete_copy})));
    }
}
