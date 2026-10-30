//! The command registry: every user-visible action, by id.
//!
//! Shortcuts follow the ones mail users already know: Ctrl+N new, Ctrl+R reply, Ctrl+Shift+R
//! reply all, Ctrl+F forward, Ctrl+Q / Ctrl+U read / unread, Insert flag, Delete, Backspace
//! archive, Ctrl+Shift+V move, F9 send/receive, Ctrl+Enter send.

use quatrelle_mime::{OutAttachment, ReplyMode, mime_from_name, sanitize_filename};
use quatrelle_model::{AccountConfig, Flag, FolderRole, Security, ServerConfig};
use serde_json::{Value, json};

use crate::JunkList;
use crate::session::{ListFilter, MoveTarget, ReadingPane, RulesScope, SearchScope, View, parse_key};
use crate::{CmdError, CmdResult, CommandSpec, Registry, Session, p, parse_local};

fn has_selection(s: &Session) -> Option<&'static str> {
    if s.selection.is_empty() { Some("no message selected") } else { None }
}

fn has_reading(s: &Session) -> Option<&'static str> {
    match &s.reading {
        Some(r) if r.parsed.is_some() => None,
        Some(_) => Some("the message is still downloading"),
        None => Some("open a message first"),
    }
}

/// The message being read has voting buttons.
fn has_voting(s: &Session) -> Option<&'static str> {
    match &s.reading {
        Some(r) if s.summary(&r.key).is_some_and(|m| !m.voting.is_empty()) => None,
        Some(_) => Some("this message has no voting buttons"),
        None => Some("open a message first"),
    }
}

fn vote(s: &mut Session, v: &Value) -> CmdResult {
    let key = match v.get("key") {
        Some(_) => parse_key(v).ok_or_else(|| CmdError::Params("`key` isn't a message".into()))?,
        None => s.reading.as_ref().map(|r| r.key.clone()).ok_or_else(|| CmdError::Disabled("open a message first".into()))?,
    };
    s.vote(&key, p::req_str(v, "choice")?)
}

fn has_compose(s: &Session) -> Option<&'static str> {
    if s.compose.is_some() { None } else { Some("no message is being written") }
}

fn has_account(s: &Session) -> Option<&'static str> {
    if s.accounts.is_empty() { Some("add an account first") } else { None }
}

/// Connecting needs an account, and not working offline.
fn can_connect(s: &Session) -> Option<&'static str> {
    has_account(s).or(if s.prefs.work_offline { Some("working offline") } else { None })
}

mod accounts;
mod compose;
mod files;
mod home;
mod navigation;
mod people;
mod pim;
mod scheduling;
mod security;
mod tasks;
mod view;

/// Every command, in the order the CLI lists them: the Home tab's first, accounts' last.
/// The UI puts a pasted picture where the caret is.
pub use compose::insert_picture_at;

pub fn registry() -> Registry {
    let mut all = Vec::new();
    all.extend(home::commands());
    all.extend(navigation::commands());
    all.extend(compose::commands());
    all.extend(people::commands());
    all.extend(view::commands());
    all.extend(pim::commands());
    all.extend(scheduling::commands());
    all.extend(files::commands());
    all.extend(security::commands());
    all.extend(tasks::commands());
    all.extend(accounts::commands());
    Registry::new(all)
}

/// `account` from the params, else the account of the folder on screen.
fn account_param(s: &Session, v: &Value) -> Result<String, CmdError> {
    match p::str(v, "account") {
        Some(a) if s.account(a).is_some() => Ok(a.to_string()),
        Some(a) => Err(CmdError::Params(format!("no account `{a}`"))),
        None => s.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Disabled("add an account first".into())),
    }
}

fn key_param(s: &Session, v: &Value) -> Result<quatrelle_model::MessageKey, CmdError> {
    let k = parse_key(v).ok_or_else(|| CmdError::Params("`key` is required (account:validity:uid:folder)".into()))?;
    if s.summary(&k).is_none() {
        return Err(CmdError::Params(format!("`{}` is not in the list", k.to_id())));
    }
    Ok(k)
}

/// A file's bytes, refused when it's bigger than `max` (read no further than that).
pub(crate) fn read_capped(path: &str, max: u64) -> Result<Vec<u8>, CmdError> {
    use std::io::Read;
    // Only a plain file: a device (`/dev/zero`) reads forever and a pipe blocks on opening,
    // whatever size they report.
    match std::fs::metadata(path) {
        Err(e) => return Err(CmdError::Failed(format!("{path}: {e}"))),
        Ok(m) if !m.is_file() => return Err(CmdError::Params(format!("{path} isn't a file"))),
        Ok(m) if m.len() > max => return Err(CmdError::Params(format!("{path} is too big"))),
        Ok(_) => {}
    }
    let f = std::fs::File::open(path).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    let mut bytes = Vec::new();
    f.take(max.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    if bytes.len() as u64 > max {
        return Err(CmdError::Params(format!("{path} is too big")));
    }
    Ok(bytes)
}

fn compose_replace_text(s: &mut Session, v: &Value) -> CmdResult {
    let from = p::i64(v, "from").ok_or_else(|| CmdError::Params("`from` is required".into()))?;
    let to = p::i64(v, "to").ok_or_else(|| CmdError::Params("`to` is required".into()))?;
    let (from, to) = (usize::try_from(from), usize::try_from(to));
    let (Ok(from), Ok(to)) = (from, to) else {
        return Err(CmdError::Params("`from` and `to` are byte offsets".into()));
    };
    let text = p::req_str(v, "text")?;
    let field = p::str(v, "field").unwrap_or("body");
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    match field {
        "body" => {
            c.replace_text(from, to, text)?;
            Ok(json!({"body": c.body}))
        }
        "subject" => {
            c.replace_subject(from, to, text)?;
            Ok(json!({"subject": c.subject}))
        }
        other => Err(CmdError::Params(format!("there's no field `{other}`"))),
    }
}

/// Pictures a signature takes, and how large each.
const SIGNATURE_PICTURES: usize = 4;

const SIGNATURE_PICTURE_MAX: usize = 500 * 1024;
