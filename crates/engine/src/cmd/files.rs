// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 the ArtCraft Team and the MailCraft contributors
// Copyright (c) 2026 Didier Gaudin
//! Commands: what the system hands Quatrelle as its mail app (`mailto:` links, and `.eml`,
//! `.ics` and `.vcf` files opened with it, and message files dropped on a folder).

use quatrelle_model::Job;

use crate::session::pim::FromFile;

use super::*;

/// Largest message file read (`.eml`).
const MAX_MESSAGE: u64 = 64 << 20;
/// Largest calendar or contacts file read (`.ics`, `.vcf`).
const MAX_PIM: u64 = 16 << 20;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("mail.mailto", "Write to a mailto: link", "Another app › mailto: link", |s, v| {
            let link = p::req_str(v, "url")?;
            let m = quatrelle_model::mailto::parse(link).ok_or_else(|| CmdError::Params("not a mailto: link".into()))?;
            s.mailto_compose(&m)
        })
        .params(r#"{"url": "mailto:…" (RFC 6068: to, cc, bcc, subject and body; nothing is attached)}"#),
        CommandSpec::new("file.open", "Open a mail file", "Another app › Open with Quatrelle", open_mail_file)
            .params(r#"{"path": string (.eml and .msg show the message; .ics and .vcf answer what the file holds, and add nothing)}"#),
        CommandSpec::new("file.add", "Add a calendar or contacts file", "Another app › Open with Quatrelle (once agreed to)", add_pim_file).params(
            r#"{"path": string (.ics adds its events to the calendar, .vcf its people to the contacts; nobody is invited, nothing is replaced)}"#,
        ),
        CommandSpec::new("file.addToFolder", "Add message files to a folder", "Folder pane (files dropped on a folder)", add_to_folder)
            .params(r#"{"paths": [string] (.eml and Outlook .msg files), "folder"?: string (the folder open), "account"?: string}"#),
    ]
}

/// Whether a file is a message file by its name: what a folder takes when it's dropped there.
pub fn is_message_file(path: &str) -> bool {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "eml" | "msg")
}

/// Message files put into a folder: the open one unless another is named. The files are read and
/// filed by the network layer ([`Job::FileMessages`]); the folder shows them once it's read
/// again, and [`Session::take_filed`] says how many went in.
fn add_to_folder(s: &mut Session, v: &Value) -> CmdResult {
    let paths: Vec<String> = v
        .get("paths")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(Value::as_str).map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).take(500).collect())
        .unwrap_or_default();
    if paths.is_empty() {
        return Err(CmdError::Params("`paths` names no files".into()));
    }
    let (account, folder) = match (p::str(v, "account"), p::str(v, "folder")) {
        (a, Some(f)) => (a.map(str::to_string).or_else(|| s.current_account().map(|x| x.config.id.clone())), f.to_string()),
        (None, None) => match &s.view {
            View::Folder { account, folder } => (Some(account.clone()), folder.clone()),
            _ => return Err(CmdError::Params("open a folder to add them to".into())),
        },
        (Some(_), None) => return Err(CmdError::Params("`folder` is required with `account`".into())),
    };
    let account = account.ok_or_else(|| CmdError::Disabled("add an account first".into()))?;
    let Some(a) = s.account(&account) else { return Err(CmdError::Params(format!("no account `{account}`"))) };
    if !a.folders.iter().any(|f| f.path == folder && f.selectable) {
        return Err(CmdError::Params(format!("no folder `{folder}` to add them to")));
    }
    let n = paths.len();
    s.push_job(Job::FileMessages { account: account.clone(), folder: folder.clone(), paths });
    Ok(json!({"account": account, "folder": folder, "files": n}))
}

/// How much of a file tells what it is ([`Kind::of`]).
const HEAD: u64 = 64;

/// An Outlook message file says so itself, whatever it was renamed to.
const OUTLOOK_FILE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// What a file opened with Quatrelle holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Calendar,
    Contacts,
    Message,
    /// An Outlook message file (`.msg`), read by the network layer.
    Outlook,
}

impl Kind {
    /// What `path` holds, from its first bytes (`head`): by its extension, else by its first line.
    fn of(path: &str, head: &[u8]) -> Kind {
        if head.starts_with(&OUTLOOK_FILE) {
            return Kind::Outlook;
        }
        let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "ics" | "ical" | "icalendar" | "ifb" => Kind::Calendar,
            "vcf" | "vcard" => Kind::Contacts,
            "eml" => Kind::Message,
            "msg" => Kind::Outlook,
            _ => Kind::sniff(head),
        }
    }

    /// A file with no telling extension: calendars and cards say what they are on their first
    /// line (after a byte-order mark and blank space, if any); anything else is taken for a
    /// message.
    fn sniff(head: &[u8]) -> Kind {
        let opening = String::from_utf8_lossy(head.get(..HEAD as usize).unwrap_or(head));
        let opening = opening.trim_start_matches('\u{feff}').trim_start();
        let opens_with = |tag: &str| opening.get(..tag.len()).is_some_and(|h| h.eq_ignore_ascii_case(tag));
        if opens_with("BEGIN:VCALENDAR") {
            Kind::Calendar
        } else if opens_with("BEGIN:VCARD") {
            Kind::Contacts
        } else {
            Kind::Message
        }
    }

    /// The most of such a file read (an Outlook one is read by the network layer, as a message).
    fn max(self) -> u64 {
        match self {
            Kind::Calendar | Kind::Contacts => MAX_PIM,
            Kind::Message | Kind::Outlook => MAX_MESSAGE,
        }
    }

    /// The kind as the answer says it.
    fn label(self) -> &'static str {
        match self {
            Kind::Calendar => "calendar",
            Kind::Contacts => "contacts",
            Kind::Message => "message",
            Kind::Outlook => "outlook",
        }
    }
}

/// The file named by a command's `path`: the path as given, and the file's own name.
fn named(v: &Value) -> Result<(&str, String), CmdError> {
    let path = p::req_str(v, "path")?.trim();
    if path.is_empty() {
        return Err(CmdError::Params("`path` is required".into()));
    }
    Ok((path, path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()))
}

/// The first few titles of what a file would add, each cut short: enough to tell what it is.
fn titles<'a>(all: impl Iterator<Item = &'a str>) -> Vec<String> {
    all.take(5).map(|t| t.trim().chars().take(80).collect()).collect()
}

/// What a calendar or contacts file would add, said before anything is added.
fn held<T>(kind: &str, name: &str, path: &str, held: &FromFile<T>, titles: Vec<String>, s: &Session) -> Value {
    let account = s.account(&held.account).map(|a| a.config.email.clone()).unwrap_or_default();
    json!({
        "kind": kind, "name": name, "path": path, "count": held.items.len(), "already": held.already,
        "unreadable": held.unreadable, "more": held.more, "titles": titles, "into": held.place_name, "account": account,
    })
}

/// Open `path`: a message is answered with what to show (as `attachment.preview` answers). A
/// calendar or a contacts file is only read: the answer says what it holds and where it would
/// go, and `file.add` adds it once the user has said so.
fn open_mail_file(s: &mut Session, v: &Value) -> CmdResult {
    use std::io::Read;
    let (path, name) = named(v)?;
    // Opened once: its first bytes say what it is, and the rest is read up to that kind's cap.
    let size = plain_file_size(path)?;
    let mut file = open_file(path)?;
    let mut head = Vec::new();
    (&mut file).take(HEAD).read_to_end(&mut head).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    let kind = Kind::of(path, &head);
    let too_big = || CmdError::Params(format!("{path} is too big"));
    // An Outlook message file is made a standard message by the network layer, and shown when
    // it comes ([`Session::message_file_read`]).
    if kind == Kind::Outlook {
        s.push_job(Job::ReadMessageFile { path: path.to_string() });
        return Ok(json!({"kind": "reading", "name": name}));
    }
    if size > kind.max() {
        return Err(too_big());
    }
    let bytes = read_more(path, file, head, kind.max(), too_big)?;
    match kind {
        Kind::Calendar => {
            let events = s.file_events(&String::from_utf8_lossy(&bytes))?;
            let titles = titles(events.items.iter().map(|e| e.summary.as_str()));
            Ok(held(kind.label(), &name, path, &events, titles, s))
        }
        Kind::Contacts => {
            let cards = s.file_cards(&String::from_utf8_lossy(&bytes))?;
            let names: Vec<String> = cards.items.iter().map(quatrelle_model::vcard::Card::display).collect();
            let titles = titles(names.iter().map(String::as_str));
            Ok(held(kind.label(), &name, path, &cards, titles, s))
        }
        // (An Outlook one went to the network layer above.)
        _ => message_preview(s, &name, &bytes),
    }
}

/// Add a calendar file's events or a contacts file's people to the account on screen: what
/// `file.open` said the file holds, read again. Nobody is invited and nothing there is replaced.
fn add_pim_file(s: &mut Session, v: &Value) -> CmdResult {
    let (path, name) = named(v)?;
    let bytes = read_capped(path, MAX_PIM)?;
    let text = String::from_utf8_lossy(&bytes);
    match Kind::of(path, &bytes) {
        kind @ Kind::Calendar => {
            let events = s.file_events(&text)?;
            let (skipped, already, more) = (events.unreadable, events.already, events.more);
            let (added, first) = s.add_file_events(events);
            // The calendar goes to the first one's day, to show it.
            if let Some(start) = first {
                s.cal_day = Some(start.saturating_add(i64::from(s.clock.offset)).div_euclid(86_400));
            }
            Ok(json!({"kind": kind.label(), "name": name, "added": added, "skipped": skipped, "already": already, "more": more, "start": first}))
        }
        kind @ Kind::Contacts => {
            let cards = s.file_cards(&text)?;
            let (skipped, already, more) = (cards.unreadable, cards.already, cards.more);
            let added = s.add_file_cards(cards);
            Ok(json!({"kind": kind.label(), "name": name, "added": added, "skipped": skipped, "already": already, "more": more}))
        }
        _ => Err(CmdError::Params(format!("{name} is neither a calendar nor a contacts file"))),
    }
}

/// What to show of a message file.
pub(crate) fn message_preview(s: &mut Session, name: &str, bytes: &[u8]) -> CmdResult {
    let m = quatrelle_mime::parse(bytes).ok_or_else(|| CmdError::Failed(format!("{name} can't be read as a message")))?;
    let attachments: Vec<&str> = m.attachments.iter().map(|a| a.name.as_str()).collect();
    let warning = s.sender_warning(&m.from);
    Ok(json!({
        "kind": "message", "name": name, "subject": m.subject, "from": m.from.to_string(),
        "to": quatrelle_model::addr::format_list(&m.to), "cc": quatrelle_model::addr::format_list(&m.cc),
        "date": m.date, "text": m.text, "attachments": attachments, "warning": warning,
    }))
}
