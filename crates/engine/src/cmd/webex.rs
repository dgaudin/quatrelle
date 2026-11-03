// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Commands: Webex scheduled meetings (File › Options › Calendar and the appointment window).
//! The engine only asks: the worker's thread calls Webex, and its answers wait in
//! [`Session::webex_take_answer`] for the window that asked.

use quatrelle_model::addr::trust::url_host;
use quatrelle_model::ical::{CalEvent, valid_webex_id};
use quatrelle_model::job::Secret;
use quatrelle_model::{Job, WebexFields};

use super::pim::event_from;
use super::*;
use crate::WebexAsk;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("webex.settings", "Webex settings", "File › Options › Calendar", settings)
            .params(r#"{"site"?: string (the Webex site, `acme.webex.com` or its address; "" for none), "clientId"?: string (the integration's client ID; "" for none), "secret"?: string (the integration's client secret, kept in the keychain; "" forgets it)}"#),
        CommandSpec::new("webex.signIn", "Sign in to Webex", "File › Options › Calendar", sign_in).params("(opens the browser to sign in)"),
        CommandSpec::new("webex.signOut", "Sign out of Webex", "File › Options › Calendar", |s, _| {
            s.prefs.webex_signed_in = false;
            s.prefs.webex_user.clear();
            s.webex.signing_in = false;
            s.webex.sign_in_error = None;
            s.webex.unacknowledged.clear();
            s.push_job(Job::WebexSignOut);
            Ok(json!({}))
        })
        .params("()"),
        CommandSpec::new("webex.createMeeting", "Webex meeting", "Appointment › Meeting", create)
            .params(r#"{"summary": string, "start": "YYYY-MM-DD[THH:MM]", "end"?: …, "allDay"?: bool, "repeat"?: "daily"|"weekly"|"monthly"|"yearly"|RRULE, "quiet"?: bool (Webex mails no one)} → {"ask": number}, answered later: see webex_take_answer"#),
        CommandSpec::new("webex.updateMeeting", "Update Webex meeting", "Appointment › Meeting", update)
            .params(r#"{"id": string, … as webex.createMeeting}"#),
        CommandSpec::new("webex.deleteMeeting", "Delete Webex meeting", "Appointment › Meeting", delete)
            .params(r#"{"id": string, "quiet"?: bool} | {"ask": number (its window closed before the answer was taken)}"#),
    ]
}

const SETUP: &str = "sign in to Webex in File › Options › Calendar first";

fn settings(s: &mut Session, v: &Value) -> CmdResult {
    // Read everything before changing anything: a refused one changes nothing.
    let site = match p::str(v, "site").map(str::trim) {
        Some("") => Some(String::new()),
        Some(x) => {
            Some(url_host(x).filter(|h| h.contains('.')).ok_or_else(|| CmdError::Params(format!("`{x}` is not a Webex site (acme.webex.com)")))?)
        }
        None => None,
    };
    let client_id = match p::str(v, "clientId").map(str::trim) {
        Some(x) if x.len() > 200 || !x.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) => {
            return Err(CmdError::Params("`clientId` is 1 to 200 letters, digits, `-`, `_` or `.`".into()));
        }
        x => x.map(str::to_string),
    };
    if let Some(x) = site {
        s.prefs.webex_site = x;
    }
    if let Some(x) = client_id {
        s.prefs.webex_client_id = x;
    }
    if let Some(x) = p::str(v, "secret") {
        s.push_job(Job::SetWebexSecret { secret: Secret(x.to_string()) });
    }
    Ok(json!({"site": s.prefs.webex_site, "clientId": s.prefs.webex_client_id}))
}

fn sign_in(s: &mut Session, _: &Value) -> CmdResult {
    if s.prefs.webex_client_id.is_empty() {
        return Err(CmdError::Disabled("set the Webex client ID in File › Options › Calendar first".into()));
    }
    if s.webex.signing_in {
        return Err(CmdError::Disabled("already signing in to Webex".into()));
    }
    s.webex.signing_in = true;
    s.webex.sign_in_error = None;
    s.push_job(Job::WebexSignIn { client_id: s.prefs.webex_client_id.clone() });
    Ok(json!({}))
}

/// The meeting's fields from the params, and whether Webex mails no one.
fn fields_of(s: &Session, v: &Value) -> Result<(WebexFields, bool), CmdError> {
    if p::str(v, "summary").is_none_or(|t| t.trim().is_empty()) || v.get("start").is_none() {
        return Err(CmdError::Params("a Webex meeting needs a title and a start".into()));
    }
    let e = event_from(s, v, CalEvent::default())?;
    Ok((WebexFields::of(&e), p::bool(v, "quiet").unwrap_or(false)))
}

fn create(s: &mut Session, v: &Value) -> CmdResult {
    let (fields, quiet) = fields_of(s, v)?;
    if !s.prefs.webex_signed_in || s.prefs.webex_site.is_empty() {
        return Err(CmdError::Disabled(SETUP.into()));
    }
    let ask = s.fresh_id();
    s.webex.asks.insert(ask, WebexAsk { fields: fields.clone(), quiet, abandoned: false });
    s.push_job(Job::WebexCreate { ask, client_id: s.prefs.webex_client_id.clone(), site: s.prefs.webex_site.clone(), fields, quiet });
    Ok(json!({"ask": ask}))
}

fn meeting_id(v: &Value) -> Result<String, CmdError> {
    let id = p::req_str(v, "id")?;
    if valid_webex_id(id) { Ok(id.to_string()) } else { Err(CmdError::Params("`id` is not a Webex meeting id".into())) }
}

fn update(s: &mut Session, v: &Value) -> CmdResult {
    let id = meeting_id(v)?;
    let (after, quiet) = fields_of(s, v)?;
    if !s.prefs.webex_signed_in {
        return Err(CmdError::Disabled(SETUP.into()));
    }
    let before = s.webex.sent.insert(id.clone(), after.clone()).unwrap_or_default();
    s.webex.unacknowledged.entry(id.clone()).or_default().push_back(before.clone());
    s.push_job(Job::WebexUpdate { client_id: s.prefs.webex_client_id.clone(), id: id.clone(), before, after, quiet });
    Ok(json!({"id": id}))
}

fn delete(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(ask) = p::u64(v, "ask") {
        // The window closed: a creation still on its way is deleted when it answers, and an
        // answer nobody took has a meeting that nothing will ever hold.
        if let Some(a) = s.webex.asks.get_mut(&ask) {
            a.abandoned = true;
        } else if let Some(Ok(m)) = s.webex_take_answer(ask) {
            s.webex_delete(m.id, true);
        }
        return Ok(json!({}));
    }
    let id = meeting_id(v)?;
    if !s.prefs.webex_signed_in {
        return Err(CmdError::Disabled(SETUP.into()));
    }
    s.webex.sent.remove(&id);
    s.webex.unacknowledged.remove(&id);
    s.webex_delete(id, p::bool(v, "quiet").unwrap_or(false));
    Ok(json!({}))
}
