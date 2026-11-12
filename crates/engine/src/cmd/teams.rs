// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Commands: Teams (File › Options › Teams and the Conversations module). The engine only asks:
//! the Teams worker talks to Graph and keeps the copy, and what it reports is applied by
//! [`Session::teams_apply`]. Every param is checked before anything changes.

use std::collections::HashSet;

use quatrelle_model::Job;
use quatrelle_model::addr::trust::url_host;
use quatrelle_model::pim::teams::{self, MAX_ID, PRESENCE_IDS, Picture, Status};
use quatrelle_model::{valid_oauth_client_id, valid_oauth_tenant};

use super::*;
use crate::Module;

const OPTIONS: &str = "File › Options › Teams";
const MODULE: &str = "Conversations";
/// The longest search of the conversations, in characters.
const QUERY_CHARS: usize = 200;
/// The longest address of a picture from elsewhere.
const PICTURE_URL_MAX: usize = 2048;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("teams.settings", "Teams settings", OPTIONS, settings).params(
            r#"{"on"?: bool (use Teams in Quatrelle), "clientId"?: string (the application (client) ID; "" for none), "tenant"?: string (the directory (tenant) ID or domain; "" for none), "notify"?: bool, "notifyOnlyDirect"?: bool}"#,
        ),
        CommandSpec::new("teams.signIn", "Sign in to Teams", OPTIONS, sign_in).params("(opens the browser to sign in)").when(on),
        CommandSpec::new("teams.signOut", "Sign out of Teams", OPTIONS, sign_out)
            .params(r#"{"deleteCopy"?: bool (delete the local copy of the conversations too)}"#)
            .when(signed_in),
        CommandSpec::new("teams.deleteSetAside", "Delete the copy set aside", OPTIONS, delete_set_aside).params("()").when(has_set_aside),
        CommandSpec::new("teams.open", "Open a conversation", MODULE, open).params(r#"{"chat": string (a listed conversation's id) | null (none)}"#).when(ready),
        CommandSpec::new("teams.older", "Older messages", MODULE, older).params(r#"{"chat": string (the open conversation)}"#).when(ready),
        CommandSpec::new("teams.search", "Search conversations", MODULE, search)
            .params(r#"{"query": string (at most 200 characters; "" shows the list again)}"#)
            .when(ready),
        CommandSpec::new("teams.visible", "People shown", MODULE, visible).params(r#"{"users": [string] (at most 650 user ids)}"#).when(ready),
        CommandSpec::new("teams.picture", "Show a picture", MODULE, picture)
            .params(r#"{"kind": "hosted", "chat": string, "message": string, "hosted": string} | {"kind": "photo", "user": string} | {"kind": "outside", "url": string (https)}"#)
            .when(ready),
        CommandSpec::new("teams.next", "Next conversation", MODULE, |s, _| step(s, true)).key("Alt+Down").when(in_module),
        CommandSpec::new("teams.previous", "Previous conversation", MODULE, |s, _| step(s, false)).key("Alt+Up").when(in_module),
    ]
}

fn on(s: &Session) -> Option<&'static str> {
    if s.prefs.teams.on { None } else { Some("switch Teams on in File › Options › Teams first") }
}

/// Signed in, Teams on or off (the sign-in can always be forgotten).
fn signed_in(s: &Session) -> Option<&'static str> {
    if s.prefs.teams.signed_in { None } else { Some("not signed in to Teams") }
}

fn ready(s: &Session) -> Option<&'static str> {
    on(s).or_else(|| signed_in(s))
}

fn in_module(s: &Session) -> Option<&'static str> {
    ready(s).or(if s.module == Module::Conversations { None } else { Some("open Conversations first") })
}

fn has_set_aside(s: &Session) -> Option<&'static str> {
    if s.teams.set_aside.is_some() { None } else { Some("no copy was set aside") }
}

/// An id from the params: given, a string, 1 to [`MAX_ID`] bytes.
fn id_param<'a>(v: &'a Value, k: &str) -> Result<&'a str, CmdError> {
    p::str(v, k)
        .filter(|x| !x.is_empty() && x.len() <= MAX_ID)
        .ok_or_else(|| CmdError::Params(format!("`{k}` (an id, at most {MAX_ID} bytes) is required")))
}

/// One of the two ids from the params: absent, "" (none), or one `valid` takes.
fn setting_id(v: &Value, k: &str, valid: fn(&str) -> bool, what: &str) -> Result<Option<String>, CmdError> {
    match p::str(v, k).map(str::trim) {
        Some(x) if !x.is_empty() && !valid(x) => Err(CmdError::Params(format!("`{k}` isn't {what}"))),
        x => Ok(x.map(str::to_string)),
    }
}

fn settings(s: &mut Session, v: &Value) -> CmdResult {
    // Everything read before anything changes: a refused one changes nothing.
    let client_id = setting_id(v, "clientId", valid_oauth_client_id, "an application (client) ID")?;
    let tenant = setting_id(v, "tenant", valid_oauth_tenant, "a directory (tenant) ID or domain")?;
    let t = &mut s.prefs.teams;
    if let Some(x) = client_id {
        t.client_id = x;
    }
    if let Some(x) = tenant {
        t.tenant = x;
    }
    if let Some(x) = p::bool(v, "notify") {
        t.notify = x;
    }
    if let Some(x) = p::bool(v, "notifyOnlyDirect") {
        t.notify_only_direct = x;
    }
    match p::bool(v, "on") {
        Some(true) if !s.prefs.teams.on => {
            s.prefs.teams.on = true;
            s.teams_start();
        }
        Some(false) if s.prefs.teams.on => s.teams_off(),
        _ => s.teams_fallback(),
    }
    let t = &s.prefs.teams;
    Ok(
        json!({"on": t.on, "clientId": t.client_id, "tenant": t.tenant, "notify": t.notify, "notifyOnlyDirect": t.notify_only_direct, "signedIn": t.signed_in}),
    )
}

fn sign_in(s: &mut Session, _: &Value) -> CmdResult {
    let t = &s.prefs.teams;
    if t.client_id.is_empty() || t.tenant.is_empty() {
        return Err(CmdError::Disabled("set the application (client) ID and the directory (tenant) ID in File › Options › Teams first".into()));
    }
    if s.teams.status == Status::SigningIn {
        return Err(CmdError::Disabled("already signing in to Teams".into()));
    }
    let job = teams::Job::SignIn { client_id: t.client_id.clone(), tenant: t.tenant.clone() };
    s.teams.status = Status::SigningIn;
    s.push_job(Job::Teams { job });
    Ok(json!({}))
}

fn sign_out(s: &mut Session, v: &Value) -> CmdResult {
    let delete_copy = p::bool(v, "deleteCopy").unwrap_or(false);
    s.teams_signed_out();
    s.push_job(Job::Teams { job: teams::Job::SignOut { delete_copy } });
    Ok(json!({"deleteCopy": delete_copy}))
}

fn delete_set_aside(s: &mut Session, _: &Value) -> CmdResult {
    let path = s.teams.set_aside.take().ok_or_else(|| CmdError::Disabled("no copy was set aside".into()))?;
    s.push_job(Job::Teams { job: teams::Job::DeleteSetAside { path } });
    Ok(json!({}))
}

/// `chat` (or none) on screen: its window filled again by the worker.
fn show(s: &mut Session, chat: Option<String>) {
    s.teams.open_chat(chat.clone());
    s.push_job(Job::Teams { job: teams::Job::Open { chat } });
}

fn open(s: &mut Session, v: &Value) -> CmdResult {
    let chat = match v.get("chat") {
        Some(Value::Null) => None,
        Some(Value::String(c)) if s.teams.has_chat(c) => Some(c.clone()),
        Some(Value::String(_)) => return Err(CmdError::Params("that conversation isn't listed".into())),
        _ => return Err(CmdError::Params("`chat` (a conversation's id, or null) is required".into())),
    };
    show(s, chat.clone());
    Ok(json!({"chat": chat}))
}

fn older(s: &mut Session, v: &Value) -> CmdResult {
    let chat = id_param(v, "chat")?;
    if s.teams.open.as_deref() != Some(chat) {
        return Err(CmdError::Params("`chat` isn't the open conversation".into()));
    }
    let chat = chat.to_string();
    let Some(before) = s.teams.older_page() else { return Ok(json!({"asked": false})) };
    s.push_job(Job::Teams { job: teams::Job::Page { chat, before } });
    Ok(json!({"asked": true}))
}

fn search(s: &mut Session, v: &Value) -> CmdResult {
    let query = p::req_str(v, "query")?.trim();
    if query.chars().count() > QUERY_CHARS {
        return Err(CmdError::Params(format!("a search is at most {QUERY_CHARS} characters")));
    }
    if query.is_empty() {
        s.teams.search(None);
        return Ok(json!({"asked": false}));
    }
    let query = query.to_string();
    s.teams.search(Some(query.clone()));
    s.push_job(Job::Teams { job: teams::Job::Search { query } });
    Ok(json!({"asked": true}))
}

fn visible(s: &mut Session, v: &Value) -> CmdResult {
    let Some(given) = v.get("users").and_then(Value::as_array) else {
        return Err(CmdError::Params("`users` (a list of user ids) is required".into()));
    };
    if given.len() > PRESENCE_IDS {
        return Err(CmdError::Params(format!("at most {PRESENCE_IDS} users at once")));
    }
    let mut users = Vec::new();
    let mut set = HashSet::new();
    for u in given {
        let u = u.as_str().filter(|u| !u.is_empty() && u.len() <= MAX_ID).ok_or_else(|| CmdError::Params("`users` are user ids".into()))?;
        if set.insert(u.to_string()) {
            users.push(u.to_string());
        }
    }
    if !s.teams.now_visible(set) {
        return Ok(json!({"asked": false}));
    }
    s.push_job(Job::Teams { job: teams::Job::Visible { users } });
    Ok(json!({"asked": true}))
}

/// Whether a picture from elsewhere may be asked for: `https`, a host, nothing two readers of an
/// address could take differently (no space, control or backslash), not too long.
fn outside_url(url: &str) -> bool {
    url.len() <= PICTURE_URL_MAX
        && url.get(..8).is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
        && !url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\')
        && url_host(url).is_some_and(|h| !h.is_empty())
}

fn picture(s: &mut Session, v: &Value) -> CmdResult {
    let picture = match p::req_str(v, "kind")? {
        "hosted" => Picture::Hosted {
            chat: id_param(v, "chat")?.to_string(),
            message: id_param(v, "message")?.to_string(),
            hosted: id_param(v, "hosted")?.to_string(),
        },
        "photo" => Picture::Photo { user: id_param(v, "user")?.to_string() },
        "outside" => match p::str(v, "url") {
            Some(url) if outside_url(url) => Picture::Outside { url: url.to_string() },
            _ => return Err(CmdError::Params("`url` (an https address) is required".into())),
        },
        other => return Err(CmdError::Params(format!("no picture of kind `{other}`"))),
    };
    if !s.teams.ask_picture(&picture) {
        return Ok(json!({"asked": false}));
    }
    s.push_job(Job::Teams { job: teams::Job::Picture { picture } });
    Ok(json!({"asked": true}))
}

/// The next (or previous) listed conversation opened: the first (or last) when none is open.
fn step(s: &mut Session, forward: bool) -> CmdResult {
    let listed: Vec<&str> = s.teams.listed().map(|r| r.id.as_str()).collect();
    let at = s.teams.open.as_deref().and_then(|o| listed.iter().position(|id| *id == o));
    let to = match (at, forward) {
        (None, true) => listed.first(),
        (None, false) => listed.last(),
        (Some(i), true) => listed.get(i.saturating_add(1)),
        (Some(i), false) => i.checked_sub(1).and_then(|i| listed.get(i)),
    };
    let Some(chat) = to.map(|c| c.to_string()) else { return Ok(json!({"chat": s.teams.open})) };
    show(s, Some(chat.clone()));
    Ok(json!({"chat": chat}))
}
