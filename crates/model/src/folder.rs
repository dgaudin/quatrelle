//! Folders: what a folder is for (RFC 6154 special-use roles, or the usual names in many languages
//! when a server marks none), the mailbox a folder is on, paths below a parent, and the order the
//! pane lists them in. Mailbox names on the wire are IMAP's modified UTF-7 ([`utf7`]).

use serde::{Deserialize, Serialize};

pub mod utf7;

/// What a folder is for (RFC 6154 special-use attributes, or the usual names when a server has
/// none).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderRole {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    #[default]
    Other,
}

/// What Sent Items is called in other languages by mail servers and programs, lower case and
/// `|` between (issue #19: a Sent folder by one of these names wasn't known for one).
const SENT_NAMES: &str = "gesendet|gesendete elemente|gesendete objekte|gesendete nachrichten|envoyés|éléments envoyés|messages envoyés|\
     enviados|elementos enviados|mensajes enviados|itens enviados|mensagens enviadas|inviati|posta inviata|\
     elementi inviati|verzonden|verzonden items|verzonden berichten|skickat|skickade objekt|sendt|sendte elementer|\
     lähetetyt|wysłane|elementy wysłane|odeslané|odeslaná pošta|elküldött|elküldött elemek|gönderilmiş öğeler|\
     gönderilenler|отправленные|надіслані|送信済み|送信済みアイテム|已发送|已发送邮件|寄件備份|보낸 편지함|보낸편지함|المرسلة|פריטים שנשלחו";

impl FolderRole {
    /// The role an RFC 6154 attribute (`\Sent`) names.
    pub fn from_attribute(attr: &str) -> Option<FolderRole> {
        match attr.trim_start_matches('\\').to_ascii_lowercase().as_str() {
            "sent" => Some(FolderRole::Sent),
            "drafts" => Some(FolderRole::Drafts),
            "trash" => Some(FolderRole::Trash),
            "junk" | "spam" => Some(FolderRole::Junk),
            "archive" | "all" => Some(FolderRole::Archive),
            _ => None,
        }
    }

    /// The role a folder name suggests, for servers without special-use attributes: the English
    /// names, and the ones mail servers and programs give them in other languages (the name as
    /// the server writes it, modified UTF-7, or as read). Only at the top or right in the Inbox
    /// (`INBOX.Sent`, as cPanel has it): one further down is the user's own (`Clients/Sent`,
    /// issue #19: one such took Sent Items, and sent mail's copies went in it).
    pub fn from_name(path: &str, delimiter: Option<char>) -> FolderRole {
        if path.eq_ignore_ascii_case("INBOX") {
            return FolderRole::Inbox;
        }
        let (parent, leaf) = delimiter.and_then(|d| path.rsplit_once(d)).unwrap_or(("", path));
        if !parent.is_empty() && !parent.eq_ignore_ascii_case("INBOX") {
            return FolderRole::Other;
        }
        let leaf = utf7::decode(leaf).to_lowercase();
        if SENT_NAMES.split('|').any(|n| n == leaf) {
            return FolderRole::Sent;
        }
        match leaf.as_str() {
            "sent" | "sent items" | "sent mail" | "sent messages" => FolderRole::Sent,
            "drafts" | "draft" => FolderRole::Drafts,
            "trash" | "deleted" | "deleted items" | "deleted messages" | "bin" => FolderRole::Trash,
            "junk" | "spam" | "junk e-mail" | "junk email" | "bulk mail" => FolderRole::Junk,
            "archive" | "archives" | "all mail" => FolderRole::Archive,
            _ => FolderRole::Other,
        }
    }

    pub fn is_other(&self) -> bool {
        *self == FolderRole::Other
    }

    /// Display order in the folder pane: Inbox first, then the usual folders, then the rest.
    pub fn rank(self) -> u8 {
        match self {
            FolderRole::Inbox => 0,
            FolderRole::Drafts => 1,
            FolderRole::Sent => 2,
            FolderRole::Archive => 3,
            FolderRole::Junk => 4,
            FolderRole::Trash => 5,
            FolderRole::Other => 6,
        }
    }

    /// The display name for a special folder (servers name them in many ways).
    pub fn label(self) -> Option<&'static str> {
        match self {
            FolderRole::Inbox => Some("Inbox"),
            FolderRole::Drafts => Some("Drafts"),
            FolderRole::Sent => Some("Sent Items"),
            FolderRole::Archive => Some("Archive"),
            FolderRole::Junk => Some("Junk Email"),
            FolderRole::Trash => Some("Deleted Items"),
            FolderRole::Other => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            FolderRole::Inbox => "inbox",
            FolderRole::Drafts => "drafts",
            FolderRole::Sent => "sent",
            FolderRole::Archive => "archive",
            FolderRole::Junk => "junk",
            FolderRole::Trash => "trash",
            FolderRole::Other => "other",
        }
    }

    pub fn from_code(s: &str) -> Option<FolderRole> {
        [FolderRole::Inbox, FolderRole::Drafts, FolderRole::Sent, FolderRole::Archive, FolderRole::Junk, FolderRole::Trash, FolderRole::Other]
            .into_iter()
            .find(|r| r.code().eq_ignore_ascii_case(s.trim()))
    }
}

/// A mailbox on the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub account: String,
    /// The server's name for it (modified UTF-7, as sent on the wire).
    pub path: String,
    /// Hierarchy delimiter, if the server has a hierarchy.
    pub delimiter: Option<char>,
    #[serde(default)]
    pub role: FolderRole,
    /// `false` for `\Noselect` containers.
    #[serde(default = "yes")]
    pub selectable: bool,
    #[serde(default)]
    pub unread: u32,
    #[serde(default)]
    pub total: u32,
    /// The server's own name for a special folder, shown instead of the role's label: Exchange
    /// names them in the language the mailbox was made in ("Postvak IN"), as Outlook shows them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Another mailbox's folder (an Exchange shared mailbox the account opens): that mailbox's
    /// address. Its top folder is shown as a mailbox of its own, beside the account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailbox: Option<String>,
    /// In another mailbox, the role the folder has there: its icon and place follow it, while
    /// `role` stays `Other`, so that it isn't taken for the account's own Inbox or Sent Items.
    #[serde(default, skip_serializing_if = "FolderRole::is_other")]
    pub mailbox_role: FolderRole,
}

fn yes() -> bool {
    true
}

impl Folder {
    /// The decoded leaf name (`Projects/Café` → `Café`), or the role's label for special folders
    /// (the server's own name for one, when it gives it).
    pub fn display_name(&self) -> String {
        if let Some(n) = self.server_name() {
            return n.to_string();
        }
        if let Some(l) = self.role.label() {
            return l.to_string();
        }
        let decoded = utf7::decode(&self.path);
        match self.delimiter {
            Some(d) => decoded.rsplit(d).next().unwrap_or(&decoded).to_string(),
            None => decoded,
        }
    }

    /// The name the server gives a special folder, when it's shown as it is (not translated).
    pub fn server_name(&self) -> Option<&str> {
        self.name.as_deref().map(str::trim).filter(|n| !n.is_empty() && self.role != FolderRole::Other)
    }

    /// A folder that can't be renamed, moved or deleted: the Inbox and the other special
    /// folders, and in a shared mailbox its top folder and its special folders too.
    pub fn is_fixed(&self) -> bool {
        self.role != FolderRole::Other
            || self.path.eq_ignore_ascii_case("INBOX")
            || (self.mailbox.is_some() && (self.mailbox_role != FolderRole::Other || self.depth() == 0))
    }

    /// The role the folder has where it is: its own mailbox's, or the shared mailbox's it's in.
    pub fn look(&self) -> FolderRole {
        if self.mailbox.is_some() { self.mailbox_role } else { self.role }
    }

    /// Nesting depth (0 for top-level folders).
    pub fn depth(&self) -> usize {
        match self.delimiter {
            Some(d) if !self.path.eq_ignore_ascii_case("INBOX") => self.path.matches(d).count(),
            _ => 0,
        }
    }
}

/// Whether `path` is `parent` itself or a folder inside it.
pub fn is_within(path: &str, parent: &str, delimiter: Option<char>) -> bool {
    path == parent || delimiter.is_some_and(|d| path.strip_prefix(parent).is_some_and(|rest| rest.starts_with(d)))
}

/// `path` with its `from` prefix replaced by `to` (for folders inside a renamed folder).
pub fn rebase_path(path: &str, from: &str, to: &str, delimiter: Option<char>) -> Option<String> {
    if path == from {
        return Some(to.to_string());
    }
    let d = delimiter?;
    let rest = path.strip_prefix(from)?.strip_prefix(d)?;
    Some(format!("{to}{d}{rest}"))
}

/// The server path for a new folder called `name` (a display name, encoded to modified UTF-7)
/// inside `parent` (`None`: top level). Errors are user-facing.
pub fn child_path(parent: Option<&str>, delimiter: Option<char>, name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("the folder name is empty".into());
    }
    if name.chars().any(|c| c.is_control()) || name.chars().count() > 200 {
        return Err("the folder name has characters that can't be used".into());
    }
    if let Some(d) = delimiter
        && name.contains(d)
    {
        return Err(format!("folder names can't contain `{d}` on this server"));
    }
    if matches!(name, "." | "..") || name.contains('*') || name.contains('%') {
        return Err(format!("`{name}` can't be used as a folder name"));
    }
    let leaf = utf7::encode(name);
    match parent.filter(|p| !p.is_empty()) {
        None => Ok(leaf),
        Some(p) => match delimiter {
            Some(d) => Ok(format!("{p}{d}{leaf}")),
            None => Err("this server has no subfolders".into()),
        },
    }
}

/// Sort folders for the folder pane: special folders in their usual order, then the others by
/// path so children follow their parents. A folder of the user's goes with the folder at the top
/// of its path, so one made in the Inbox shows under the Inbox, as Outlook shows it.
pub fn sort_folders(folders: &mut [Folder]) {
    let roles: std::collections::HashMap<String, FolderRole> = folders.iter().map(|f| (f.path.clone(), f.look())).collect();
    folders.sort_by_cached_key(|f| {
        let mut parts: Vec<String> = match f.delimiter {
            Some(d) => f.path.split(d).map(|p| utf7::decode(p).to_lowercase()).collect(),
            None => vec![utf7::decode(&f.path).to_lowercase()],
        };
        let rank = |role: FolderRole| if role == FolderRole::Other { 6 } else { role.rank() };
        let top_role = |n: usize| {
            let top = f.delimiter.map(|d| f.path.split(d).take(n).collect::<Vec<_>>().join(&d.to_string())).unwrap_or_else(|| f.path.clone());
            roles.get(&top).copied().unwrap_or(FolderRole::Other)
        };
        // Another mailbox's folders come after the account's own, each mailbox together, and
        // inside it as the account's are: its top folder, then its special folders in their places.
        if f.mailbox.is_some() {
            let mailbox = if parts.is_empty() { String::new() } else { parts.remove(0) };
            let role = if f.mailbox_role == FolderRole::Other { top_role(2) } else { f.mailbox_role };
            return (7, mailbox, if parts.is_empty() { 0 } else { 1 + rank(role) }, parts);
        }
        // Another mailbox opened here whose folders aren't marked as such (Graph's `/users`):
        // its top folder is named after the address, and its special folders come first, in
        // their usual order, with what's in each under it.
        let top = f.delimiter.and_then(|d| f.path.split(d).next()).unwrap_or(&f.path);
        let in_other = top.len() < f.path.len() && top_role(1) == FolderRole::Other && top.contains('@');
        let role = if f.role == FolderRole::Other || in_other { top_role(1) } else { f.role };
        let inner = match (in_other, f.delimiter) {
            (true, Some(d)) => {
                let second: String = f.path.split(d).take(2).collect::<Vec<_>>().join(&d.to_string());
                match roles.get(&second).copied().unwrap_or(FolderRole::Other) {
                    FolderRole::Other => 6,
                    r => r.rank(),
                }
            }
            _ => 0,
        };
        let first = parts.first().cloned().unwrap_or_default();
        (rank(role), first, inner, parts)
    });
}
