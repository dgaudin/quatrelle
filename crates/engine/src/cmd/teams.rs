// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Didier Gaudin
//! Commands: Teams (File › Options › Teams and the Conversations module). The engine only asks:
//! the Teams worker talks to Graph and keeps the copy, and what it reports is applied by
//! [`Session::teams_apply`]. Every param is checked before anything changes.

use std::collections::HashSet;
use std::ops::Range;

use quatrelle_model::Job;
use quatrelle_model::addr::trust::url_host;
use quatrelle_model::pim::teams::{self, LOCAL_MAX, MAX_ID, MAX_MENTIONS, PRESENCE_IDS, Picture, Status};
use quatrelle_model::{valid_oauth_client_id, valid_oauth_tenant};

use super::*;
use crate::mime::Typed;
use crate::{Module, TEAMS_RANGES_MAX, TeamsPage};

const OPTIONS: &str = "File › Options › Teams";
const MODULE: &str = "Conversations";
/// The longest search of the conversations, in characters.
const QUERY_CHARS: usize = 200;
/// The longest address of a picture from elsewhere.
const PICTURE_URL_MAX: usize = 2048;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("teams.settings", "Teams settings", OPTIONS, settings).params(
            r#"{"on"?: bool (use Teams in Quatrelle), "clientId"?: string (the application (client) ID; "" for none), "tenant"?: string (the directory (tenant) ID or domain; "" for none), "notify"?: bool, "notifyOnlyDirect"?: bool, "showUnreadable"?: bool (list the conversations whose messages Teams refuses)}"#,
        ),
        CommandSpec::new("teams.signIn", "Sign in to Teams", OPTIONS, sign_in).params("(opens the browser to sign in)").when(on),
        CommandSpec::new("teams.reopenSignIn", "Reopen the sign-in page", OPTIONS, reopen_sign_in)
            .params("() (while signing in: Microsoft's page opened again)")
            .when(has_sign_in_page),
        CommandSpec::new("teams.signOut", "Sign out of Teams", OPTIONS, sign_out)
            .params(r#"{"deleteCopy"?: bool (delete the local copy of the conversations too)}"#)
            .when(signed_in),
        CommandSpec::new("teams.deleteSetAside", "Delete the copy set aside", OPTIONS, delete_set_aside).params("()").when(has_set_aside),
        CommandSpec::new("teams.open", "Open a conversation", MODULE, open)
            .params(r#"{"chat": string (a listed conversation's id) | null (none), "message"?: string (one of its messages to open it at: a search's hit)}"#)
            .when(ready),
        CommandSpec::new("teams.older", "Older messages", MODULE, older).params(r#"{"chat": string (the open conversation)}"#).when(ready),
        CommandSpec::new("teams.search", "Search conversations", MODULE, search)
            .params(r#"{"query": string (at most 200 characters; "" shows the list again)}"#)
            .when(ready),
        CommandSpec::new("teams.visible", "People shown", MODULE, visible).params(r#"{"users": [string] (at most 650 user ids)}"#).when(ready),
        CommandSpec::new("teams.picture", "Show a picture", MODULE, picture)
            .params(r#"{"kind": "hosted", "chat": string, "message": string, "hosted": string} | {"kind": "photo", "user": string} | {"kind": "outside", "url": string (https)}"#)
            .when(ready),
        CommandSpec::new("teams.send", "Send a message", MODULE, send)
            .params(r#"{"chat": string (a listed conversation's id), "text": string, "bold"?: [[from, to]] (byte ranges of the text, at most 1,000), "italic"?: [[from, to]] (the same), "mentions"?: [[from, to, "user id"]] (at most 50 byte ranges of the text, each a name on one line, apart: the open conversation's members of the organisation)}"#)
            .when(ready),
        CommandSpec::new("teams.edit", "Edit a message", MODULE, edit)
            .params(r#"{"chat": string (the open conversation), "message": string (one's own message in it), "text": string, "bold"?: [[from, to]], "italic"?: [[from, to]] (as teams.send's), "mentions"?: [[from, to, "user id"]] (as teams.send's, or people the message mentions already)}"#)
            .when(ready),
        CommandSpec::new("teams.delete", "Delete a message", MODULE, delete)
            .params(r#"{"chat": string (the open conversation), "message": string (one's own message in it)}"#)
            .when(ready),
        CommandSpec::new("teams.react", "React to a message", MODULE, react)
            .params(r#"{"chat": string (the open conversation), "message": string (one of its messages), "emoji": string (a quick reaction, or one the message shows: set, or taken back when it's the user's)}"#)
            .when(ready),
        CommandSpec::new("teams.retry", "Send again", MODULE, retry)
            .params(r#"{"seq": number (a message of the open conversation that wasn't sent), "local"?: string (its local id, for one numbered 0)}"#)
            .when(ready),
        CommandSpec::new("teams.discard", "Delete a message going out", MODULE, discard)
            .params(r#"{"seq": number (a message going out in the open conversation), "local"?: string (its local id, for one numbered 0)}"#)
            .when(ready),
        CommandSpec::new("teams.openInTeams", "Open in Teams", MODULE, open_in_teams)
            .params("() (the open conversation in Teams, when its link is a Teams chat link)")
            .when(has_teams_link),
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

/// Signing in, Microsoft's page known.
fn has_sign_in_page(s: &Session) -> Option<&'static str> {
    on(s).or(if s.teams.status == Status::SigningIn && s.teams.sign_in_page.is_some() { None } else { Some("no sign-in page is waiting") })
}

/// A conversation open whose link is a Teams chat link.
fn has_teams_link(s: &Session) -> Option<&'static str> {
    ready(s).or(if s.teams.teams_link().is_some() { None } else { Some("this conversation has no link to Teams") })
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
    if let Some(x) = p::bool(v, "showUnreadable") {
        t.show_unreadable = x;
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
        json!({"on": t.on, "clientId": t.client_id, "tenant": t.tenant, "notify": t.notify, "notifyOnlyDirect": t.notify_only_direct, "showUnreadable": t.show_unreadable, "signedIn": t.signed_in}),
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

/// Microsoft's page to sign in on, opened again (the browser window closed, or behind another).
fn reopen_sign_in(s: &mut Session, _: &Value) -> CmdResult {
    let page = s.teams.sign_in_page.clone().filter(|_| s.teams.status == Status::SigningIn);
    let page = page.ok_or_else(|| CmdError::Disabled("no sign-in page is waiting".into()))?;
    s.teams.open_page(TeamsPage::SignIn(page));
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

/// A conversation opened (none: the list only); at one of its messages, a search's hit, when
/// `message` is given (an id, as the chat's).
fn open(s: &mut Session, v: &Value) -> CmdResult {
    let chat = match v.get("chat") {
        Some(Value::Null) => None,
        Some(Value::String(c)) if s.teams.has_chat(c) => Some(c.clone()),
        Some(Value::String(_)) => return Err(CmdError::Params("that conversation isn't listed".into())),
        _ => return Err(CmdError::Params("`chat` (a conversation's id, or null) is required".into())),
    };
    let message = match v.get("message") {
        None | Some(Value::Null) => None,
        Some(_) if chat.is_none() => return Err(CmdError::Params("`message` is a message of the conversation opened".into())),
        Some(_) => Some(id_param(v, "message")?.to_string()),
    };
    s.teams_show_at(chat.clone(), message.clone());
    Ok(json!({"chat": chat, "message": message}))
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

/// A style's byte ranges of `text` from the params (`[[from, to]]`, absent or null for none): each
/// on the text's characters, at most [`TEAMS_RANGES_MAX`].
fn ranges(v: &Value, k: &str, text: &str) -> Result<Vec<Range<usize>>, CmdError> {
    let bad = || CmdError::Params(format!("`{k}` is at most {TEAMS_RANGES_MAX} byte ranges [from, to] of the text, on its characters"));
    let given = match v.get(k) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(given)) if given.len() <= TEAMS_RANGES_MAX => given,
        Some(_) => return Err(bad()),
    };
    given.iter().map(|r| range(r, text).ok_or_else(bad)).collect()
}

/// `[from, to]`: a range of `text`'s bytes, from one character's start to another's.
fn range(r: &Value, text: &str) -> Option<Range<usize>> {
    let [from, to] = r.as_array()?.as_slice() else { return None };
    bounds(from, to, text)
}

fn bounds(from: &Value, to: &Value, text: &str) -> Option<Range<usize>> {
    let at = |x: &Value| x.as_u64().and_then(|n| usize::try_from(n).ok());
    let (from, to) = (at(from)?, at(to)?);
    (from <= to && text.is_char_boundary(from) && text.is_char_boundary(to)).then_some(from..to)
}

/// The people @mentioned in `text` from the params (`[[from, to, "user id"]]`, absent or null for
/// none): each a byte range on the text's characters, as a style's, and a user id; at most
/// [`MAX_MENTIONS`]. Whether each is a name on one line, apart from the others, and someone who
/// may be mentioned is the session's to say.
fn mentions(v: &Value, text: &str) -> Result<Vec<(Range<usize>, String)>, CmdError> {
    let bad =
        || CmdError::Params(format!("`mentions` is at most {MAX_MENTIONS} [from, to, \"user id\"]: byte ranges of the text, on its characters"));
    let given = match v.get("mentions") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(given)) if given.len() <= MAX_MENTIONS => given,
        Some(_) => return Err(bad()),
    };
    given.iter().map(|m| mention(m, text).ok_or_else(bad)).collect()
}

fn mention(m: &Value, text: &str) -> Option<(Range<usize>, String)> {
    let [from, to, user] = m.as_array()?.as_slice() else { return None };
    let user = user.as_str().filter(|u| !u.is_empty() && u.len() <= MAX_ID)?;
    Some((bounds(from, to, text)?, user.to_string()))
}

/// What the box holds, from the params: `text`, its `bold` and `italic` ranges and its `mentions`.
fn typed(v: &Value) -> Result<Typed, CmdError> {
    let text = p::req_str(v, "text")?;
    let (bold, italic, mentions) = (ranges(v, "bold", text)?, ranges(v, "italic", text)?, mentions(v, text)?);
    Ok(Typed { text: text.to_string(), bold, italic, mentions })
}

fn send(s: &mut Session, v: &Value) -> CmdResult {
    let chat = id_param(v, "chat")?;
    let local = s.teams_send(chat, &typed(v)?)?;
    Ok(json!({"local": local}))
}

fn edit(s: &mut Session, v: &Value) -> CmdResult {
    let (chat, message) = (id_param(v, "chat")?, id_param(v, "message")?);
    let local = s.teams_edit(chat, message, &typed(v)?)?;
    Ok(json!({"local": local}))
}

fn delete(s: &mut Session, v: &Value) -> CmdResult {
    let (chat, message) = (id_param(v, "chat")?, id_param(v, "message")?);
    let local = s.teams_delete(chat, message)?;
    Ok(json!({"local": local}))
}

fn react(s: &mut Session, v: &Value) -> CmdResult {
    let (chat, message, emoji) = (id_param(v, "chat")?, id_param(v, "message")?, p::req_str(v, "emoji")?);
    let (local, on) = s.teams_react(chat, message, emoji)?;
    Ok(json!({"local": local, "on": on}))
}

/// A message going out from the params: its number in the outbox, and its local id, which names
/// one the worker doesn't number (0).
fn outgoing_param(v: &Value) -> Result<(u64, Option<&str>), CmdError> {
    let seq = v
        .get("seq")
        .and_then(Value::as_u64)
        .ok_or_else(|| CmdError::Params("`seq` (a message's number; 0 with its `local` id) is required".into()))?;
    let local = match v.get("local") {
        None | Some(Value::Null) => None,
        Some(Value::String(l)) if !l.is_empty() && l.len() <= LOCAL_MAX => Some(l.as_str()),
        Some(_) => return Err(CmdError::Params(format!("`local` is a message's local id, at most {LOCAL_MAX} bytes"))),
    };
    Ok((seq, local))
}

fn retry(s: &mut Session, v: &Value) -> CmdResult {
    let (seq, local) = outgoing_param(v)?;
    let local = s.teams_retry(seq, local)?;
    Ok(json!({"local": local}))
}

fn discard(s: &mut Session, v: &Value) -> CmdResult {
    let (seq, local) = outgoing_param(v)?;
    s.teams_discard(seq, local)?;
    Ok(json!({}))
}

/// The open conversation in Teams: its link, only when it's a Teams chat link, for the window to
/// open in the meetings browser's window.
fn open_in_teams(s: &mut Session, _: &Value) -> CmdResult {
    let url = s.teams.teams_link().map(str::to_string).ok_or_else(|| CmdError::Disabled("this conversation has no link to Teams".into()))?;
    s.teams.open_page(TeamsPage::Chat(url.clone()));
    Ok(json!({"url": url}))
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
    let listed: Vec<&str> = s.teams.listed(s.prefs.teams.show_unreadable).map(|r| r.id.as_str()).collect();
    let at = s.teams.open.as_deref().and_then(|o| listed.iter().position(|id| *id == o));
    let to = match (at, forward) {
        (None, true) => listed.first(),
        (None, false) => listed.last(),
        (Some(i), true) => listed.get(i.saturating_add(1)),
        (Some(i), false) => i.checked_sub(1).and_then(|i| listed.get(i)),
    };
    let Some(chat) = to.map(|c| c.to_string()) else { return Ok(json!({"chat": s.teams.open})) };
    s.teams_show(Some(chat.clone()));
    Ok(json!({"chat": chat}))
}
