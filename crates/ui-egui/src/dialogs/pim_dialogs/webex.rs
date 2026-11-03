// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! An appointment's Webex meeting, in its form: what's asked of Webex, what its answer writes,
//! and what's left to delete when the window goes without saving.

use quatrelle_engine::fill;
use quatrelle_engine::model::{WebexError, WebexMeeting};

use super::*;

/// New Webex meeting, the button's click or its menu's: a title and a start first, then Options
/// until Webex is set up (the engine asks for a site too), and otherwise the window asks.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn new_meeting(f: &mut EventForm, prefs: &quatrelle_engine::Prefs, acts: &mut Vec<Action>) {
    // Webex asking for a new sign-in stays said while Options opens for it.
    if f.error.as_deref() != Some(error_text(&WebexError::SignIn).as_str()) {
        f.error = None;
    }
    if f.summary.trim().is_empty() || f.date.trim().is_empty() {
        f.error = Some(tl!("A meeting needs a title and a start").to_string());
    } else if !prefs.webex_signed_in || prefs.webex_site.is_empty() {
        acts.push(Action::WebexOptions);
    } else {
        f.error = None;
        acts.push(Action::ForWebex(f.window, webex_params(f)));
    }
}

/// `webex.createMeeting`'s params: what the appointment says of itself, and no mail from Webex
/// for a meeting whose guests get the invitation.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn webex_params(f: &EventForm) -> Value {
    let p = f.params();
    json!({"summary": p["summary"], "start": p["start"], "end": p["end"], "allDay": p["allDay"], "repeat": p["repeat"], "quiet": f.inviting})
}

/// Webex's answer in the appointment: the link as the place (one typed is kept), what the guests
/// need at the end of the notes, a line for each value it gave, and Join.
pub(crate) fn take_meeting(f: &mut EventForm, m: &WebexMeeting) {
    if f.location.trim().is_empty() {
        f.location = m.web_link.clone();
    }
    // Filled in as they are, not with `tf!`: its marks around a value would go out in the
    // invitation, and along with a password a guest copies.
    let mut lines = vec![tl!("Join the Webex meeting:").to_string(), m.web_link.clone()];
    if !m.meeting_number.is_empty() {
        lines.push(fill(tl!("Meeting number: {number}"), &[("number", &m.meeting_number)]));
    }
    if !m.password.0.is_empty() {
        lines.push(fill(tl!("Password: {password}"), &[("password", &m.password.0)]));
    }
    if !m.sip_address.is_empty() {
        lines.push(fill(tl!("Video: {address}"), &[("address", &m.sip_address)]));
    }
    let notes = f.description.trim_end();
    let gap = if notes.is_empty() { "" } else { "\n\n" };
    f.description = format!("{notes}{gap}{}\n", lines.join("\n"));
    f.join = Some(m.web_link.clone());
    f.webex_id = Some(m.id.clone());
    f.webex_fresh = true;
}

/// Why Webex didn't make the meeting, as the window says it.
pub(crate) fn error_text(e: &WebexError) -> String {
    match e {
        WebexError::SignIn => tl!("Sign in to Webex again").to_string(),
        WebexError::Forbidden => tl!("Your Webex account can't schedule meetings on this site").to_string(),
        WebexError::Refused { status, message } => tf!("Webex answered {status}: {message}", status = status, message = message),
        WebexError::Failed(why) => crate::i18n::error(why),
    }
}

/// `webex.deleteMeeting`'s params for what the window leaves unsaved: the meeting it made, or the
/// one on its way.
pub(crate) fn leaving(f: &EventForm) -> Option<Value> {
    match (&f.webex_id, f.webex_wait) {
        (Some(id), _) if f.webex_fresh => Some(json!({"id": id, "quiet": f.inviting})),
        (_, Some(ask)) => Some(json!({"ask": ask})),
        _ => None,
    }
}
