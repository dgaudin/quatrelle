//! Quatrelle data model.
//!
//! Pure data and algorithms shared by every layer: accounts and servers, folders (with their
//! RFC 6154 special-use roles), message keys and summaries, flags, the jobs the engine asks the
//! network layer to run and the events it gets back, conversation threading (JWZ), search queries
//! and Outlook-style date grouping. Nothing here does I/O, so it builds for wasm.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod addr;
pub mod cleanup;
pub mod contacts;
pub mod dates;
pub mod ical;
pub mod job;
pub mod lookalike;
pub mod mailto;
pub mod organize;
pub mod pim;
pub mod search;
pub mod thread;
pub mod utf7;
pub mod vcard;
pub mod zone;

pub use addr::providers::*;
pub use pim::scheduling;
pub use thread::voting;

use serde::{Deserialize, Serialize};

pub use addr::{Addressee, AddresseeKind, EmailAddress};
pub use job::{
    Event, FOLLOWED_PUBLIC_DOC, FolderSize, HeldKey, Job, MailboxQuota, Op, PUBLIC_FOLDERS, Private, Revocation, Secret, SyncStatus, is_public_folder,
};

/// How a connection to a server is secured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// TLS from the first byte (IMAPS 993, SMTPS 465). The default: RFC 8314 prefers it.
    #[default]
    Tls,
    /// Plain text upgraded with STARTTLS (IMAP 143, submission 587).
    StartTls,
    /// No encryption. Only for local test servers; the UI warns.
    None,
}

impl Security {
    pub fn code(self) -> &'static str {
        match self {
            Security::Tls => "tls",
            Security::StartTls => "starttls",
            Security::None => "none",
        }
    }

    pub fn from_code(s: &str) -> Option<Security> {
        match s.trim().to_ascii_lowercase().as_str() {
            "tls" | "ssl" | "ssl/tls" => Some(Security::Tls),
            "starttls" => Some(Security::StartTls),
            "none" | "plain" => Some(Security::None),
            _ => None,
        }
    }
}

/// One server endpoint.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub security: Security,
}

/// A picture in a signature: its id in the signature's HTML, its file name and type, and the
/// picture itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignaturePicture {
    pub cid: String,
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

/// A mail account. Never holds the password: that lives in the system keychain.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountConfig {
    /// Stable local id (`a1`, `a2`…), used in message keys and cache paths.
    pub id: String,
    /// The name shown on mail you send.
    pub name: String,
    pub email: String,
    /// Login name; usually the email address.
    pub username: String,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    /// The outgoing server's own user name, when it doesn't share the incoming server's sign-in
    /// (its password is kept apart in the keychain); otherwise SMTP signs in as IMAP or POP does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_username: Option<String>,
    #[serde(default)]
    pub signature: String,
    /// The signature's look, as HTML (empty when it's plain text): bold, colours, sizes and
    /// links, as pasted from Outlook or set in the signature editor.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature_html: String,
    /// The signature's look as its editor keeps it: the message editor's own styles (as JSON),
    /// so every size, typeface, underline and alignment comes back exactly as set. Empty for a
    /// signature made before this was kept, whose HTML above is read instead.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature_styles: String,
    /// The signature as HTML written by hand (cleaned): sent as it is, its layout kept (a logo
    /// beside the name); the text and look above are what the message shows while it's written.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature_raw_html: String,
    /// The signature's pictures (a company logo): kept here, and put into each message with it,
    /// where its HTML names them (`cid:`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signature_pictures: Vec<SignaturePicture>,
    /// The signature isn't added to new messages (Outlook's New messages: none).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub signature_not_new: bool,
    /// The signature isn't added to replies and forwards (Outlook's Replies/forwards: none).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub signature_not_replies: bool,
    /// The colour index the UI uses for this account's marker.
    #[serde(default)]
    pub color: u8,
    /// Signs in with OAuth through this provider (`google`, `microsoft`) instead of a password;
    /// the keychain then holds the refresh token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<String>,
    /// Syncs and sends over JMAP (RFC 8620/8621) through this session URL instead of IMAP and
    /// SMTP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jmap: Option<String>,
    /// Syncs and sends through Microsoft Graph (Exchange Online) at this API address instead of
    /// IMAP and SMTP; signs in with the account's OAuth token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<String>,
    /// Syncs and sends through Exchange Web Services at this address
    /// (`https://mail.example.com/EWS/Exchange.asmx`, on-premises Exchange) instead of IMAP and
    /// SMTP; signs in with the user name (`DOMAIN\user` or an address) and password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ews: Option<String>,
    /// Downloads mail from this POP3 server (RFC 1939) into folders kept on this computer, instead
    /// of syncing over IMAP; sends over `smtp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pop: Option<ServerConfig>,
    /// POP: remove mail from the server once it's downloaded (otherwise a copy stays there).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pop_delete: bool,
    /// POP, keeping a copy on the server: remove it this many days after it's downloaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pop_keep_days: Option<u32>,
    /// POP, keeping a copy on the server: remove it once it's deleted here for good (from
    /// Deleted Items).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pop_delete_with_trash: bool,
    /// The address the OAuth provider says the browser signed in as (from its ID token), when it
    /// differs from `email`: an alias, or another account picked on the sign-in page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_user: Option<String>,
    /// Other mailboxes this account opens, added by hand (Exchange: Outlook's "Open these
    /// additional mailboxes"); those the server maps to it open by themselves. Each shows as a
    /// mailbox of its own, its folders under it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_mailboxes: Vec<String>,
    /// Other addresses mail was sent from through this account (Outlook's From list keeps those
    /// typed in Other E-mail Address), the last used first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub send_from: Vec<String>,
    /// The User-Agent its Exchange (EWS) requests name instead of Quatrelle's, for a server that
    /// lets in only the clients it knows. Checked by [`user_agent`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// Online only (issue #4): none of the account's mail is kept on this computer. Its folders,
    /// messages, changes waiting for the server and Outbox are held in memory while Quatrelle
    /// runs, and a message is downloaded when it's opened. Not for POP, whose mail has no other
    /// home, nor an account with no server ([`AccountConfig::keeps_online`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub online_only: bool,
    /// Exchange: the shared mailboxes it opens are read on the server when one of their folders
    /// is opened, not kept on this computer (Outlook's Cached Exchange Mode keeps them unless
    /// its "Download shared folders" is turned off).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shared_online: bool,
    /// Exchange: the public folders among the Favorites are kept on this computer (Outlook's
    /// "Download Public Folder Favorites", off unless chosen); otherwise every public folder is
    /// read on the server when it's opened.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub public_favorites_kept: bool,
    /// Microsoft sign-in through an organisation's own app registration: its Application
    /// (client) ID, in place of the one built into this copy of Quatrelle (a single-tenant
    /// registration, say, whose administrator approved it for the organisation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    /// Microsoft sign-in: the tenant the registration is for — a directory's id or domain
    /// (`contoso.onmicrosoft.com`), `organizations` or `consumers` — in place of `common` (any
    /// account).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_tenant: Option<String>,
}

/// Longest User-Agent an account can name.
pub const MAX_USER_AGENT: usize = 256;

/// A User-Agent as an account may name it (RFC 9110 10.1.5: visible ASCII and spaces), trimmed;
/// `None` when it's empty, too long or has anything else (a line break would end the header).
pub fn user_agent(s: &str) -> Option<String> {
    let s = s.trim();
    let ok = !s.is_empty() && s.len() <= MAX_USER_AGENT && s.bytes().all(|b| b == b' ' || b.is_ascii_graphic());
    ok.then(|| s.to_string())
}

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
    /// the server writes it, modified UTF-7, or as read).
    pub fn from_name(path: &str, delimiter: Option<char>) -> FolderRole {
        if path.eq_ignore_ascii_case("INBOX") {
            return FolderRole::Inbox;
        }
        let leaf = delimiter.and_then(|d| path.rsplit(d).next()).unwrap_or(path);
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

/// Identifies one message: the account, the folder, and the folder's UIDVALIDITY and the message's
/// UID (RFC 3501 §2.3.1.1: the pair is unique and never reused).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MessageKey {
    pub account: String,
    pub folder: String,
    pub validity: u32,
    pub uid: u32,
}

impl MessageKey {
    /// `account/validity/uid/folder`: a compact string form for agents and the CLI.
    pub fn to_id(&self) -> String {
        format!("{}:{}:{}:{}", self.account, self.validity, self.uid, self.folder)
    }

    pub fn from_id(s: &str) -> Option<MessageKey> {
        let mut it = s.splitn(4, ':');
        let account = it.next()?.to_string();
        let validity = it.next()?.parse().ok()?;
        let uid = it.next()?.parse().ok()?;
        let folder = it.next()?.to_string();
        (!account.is_empty() && !folder.is_empty()).then_some(MessageKey { account, folder, validity, uid })
    }
}

/// System flags (RFC 3501 §2.3.2). Keywords other than these are kept as strings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    #[serde(default)]
    pub seen: bool,
    #[serde(default)]
    pub flagged: bool,
    #[serde(default)]
    pub answered: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub deleted: bool,
    /// `$Forwarded` (RFC 5550).
    #[serde(default)]
    pub forwarded: bool,
    /// Other keywords (RFC 3501 §2.3.2): categories (`$label1`, `work`…), `$Pinned`, `$Junk`…
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
}

/// The keyword that pins a message to the top of its folder.
pub const PINNED: &str = "$Pinned";

/// Whether `k` can be sent as an IMAP keyword: an atom (no spaces, brackets, wildcards, quotes or
/// controls), not a system flag, at most 64 characters.
pub fn is_keyword(k: &str) -> bool {
    !k.is_empty() && k.len() <= 64 && !k.starts_with('\\') && k.chars().all(|c| c.is_ascii_graphic() && !"(){%*\"\\]".contains(c))
}

/// A keyword for a category name (`To do` → `to_do`), for categories made in Quatrelle.
pub fn keyword_for(name: &str) -> String {
    if !name.is_ascii() {
        // Keywords are ASCII atoms: other scripts get a stable hash of the name.
        let mut h: u32 = 2_166_136_261;
        for b in name.trim().to_lowercase().bytes() {
            h = (h ^ u32::from(b)).wrapping_mul(16_777_619);
        }
        return format!("cat_{h:08x}");
    }
    let k: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .filter(|c| c.is_ascii_graphic() && !"(){%*\"\\]".contains(*c))
        .take(64)
        .collect();
    if k.is_empty() || k.starts_with('$') { format!("cat_{k}") } else { k }
}

impl Flags {
    pub fn from_imap<'a>(flags: impl IntoIterator<Item = &'a str>) -> Flags {
        let mut f = Flags::default();
        for fl in flags {
            match fl.to_ascii_lowercase().as_str() {
                "\\seen" => f.seen = true,
                "\\flagged" => f.flagged = true,
                "\\answered" => f.answered = true,
                "\\draft" => f.draft = true,
                "\\deleted" => f.deleted = true,
                "$forwarded" => f.forwarded = true,
                other
                    if !other.starts_with('\\')
                        && is_keyword(fl)
                        && f.keywords.len() < 32
                        && !f.keywords.iter().any(|k| k.eq_ignore_ascii_case(fl)) =>
                {
                    f.keywords.push(fl.to_string());
                }
                _ => {}
            }
        }
        f.keywords.sort_by_key(|k| k.to_ascii_lowercase());
        f
    }

    pub fn has(&self, keyword: &str) -> bool {
        self.keywords.iter().any(|k| k.eq_ignore_ascii_case(keyword))
    }

    /// Add or remove a keyword.
    pub fn set_keyword(&mut self, keyword: &str, on: bool) {
        self.keywords.retain(|k| !k.eq_ignore_ascii_case(keyword));
        if on {
            self.keywords.push(keyword.to_string());
            self.keywords.sort_by_key(|k| k.to_ascii_lowercase());
        }
    }

    /// A category set or taken off; taken off, under its name's keyword too: Exchange keeps
    /// categories by name, and they're read back as [`keyword_for`] the name.
    pub fn set_category(&mut self, keyword: &str, name: Option<&str>, on: bool) {
        self.set_keyword(keyword, on);
        if !on && let Some(alias) = name.map(keyword_for).filter(|a| !a.eq_ignore_ascii_case(keyword)) {
            self.set_keyword(&alias, false);
        }
    }

    pub fn pinned(&self) -> bool {
        self.has(PINNED)
    }

    /// The flags as an IMAP flag list (without parentheses).
    pub fn to_imap(&self) -> String {
        let mut v = Vec::new();
        for (on, name) in [
            (self.seen, "\\Seen"),
            (self.flagged, "\\Flagged"),
            (self.answered, "\\Answered"),
            (self.draft, "\\Draft"),
            (self.deleted, "\\Deleted"),
            (self.forwarded, "$Forwarded"),
        ] {
            if on {
                v.push(name);
            }
        }
        v.extend(self.keywords.iter().map(String::as_str));
        v.join(" ")
    }
}

/// A flag a command can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Flag {
    Seen,
    Flagged,
    Answered,
    Forwarded,
    Deleted,
}

impl Flag {
    pub fn imap(self) -> &'static str {
        match self {
            Flag::Seen => "\\Seen",
            Flag::Flagged => "\\Flagged",
            Flag::Answered => "\\Answered",
            Flag::Forwarded => "$Forwarded",
            Flag::Deleted => "\\Deleted",
        }
    }

    pub fn set(self, flags: &mut Flags, on: bool) {
        match self {
            Flag::Seen => flags.seen = on,
            Flag::Flagged => flags.flagged = on,
            Flag::Answered => flags.answered = on,
            Flag::Forwarded => flags.forwarded = on,
            Flag::Deleted => flags.deleted = on,
        }
    }

    pub fn get(self, flags: &Flags) -> bool {
        match self {
            Flag::Seen => flags.seen,
            Flag::Flagged => flags.flagged,
            Flag::Answered => flags.answered,
            Flag::Forwarded => flags.forwarded,
            Flag::Deleted => flags.deleted,
        }
    }
}

/// What the message list shows for a message: everything from its header, its flags and size,
/// and a short preview of the text once the body has been fetched.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub key: MessageKey,
    /// `Message-ID` without angle brackets.
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub in_reply_to: String,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub from: EmailAddress,
    #[serde(default)]
    pub to: Vec<EmailAddress>,
    #[serde(default)]
    pub cc: Vec<EmailAddress>,
    /// Unix seconds (UTC) from the `Date` header, or the server's INTERNALDATE.
    #[serde(default)]
    pub date: i64,
    /// When the server received it (Unix seconds; 0: not known), what mail kept for so long is
    /// measured by, as the server measures it (a message's own date can be far older).
    #[serde(default)]
    pub received: i64,
    #[serde(default)]
    pub size: u32,
    #[serde(default)]
    pub flags: Flags,
    #[serde(default)]
    pub has_attachments: bool,
    #[serde(default)]
    pub preview: String,
    /// CONDSTORE modification sequence of the last flag change seen (RFC 7162).
    #[serde(default)]
    pub modseq: u64,
    /// Sent in bulk: a mailing list, newsletter or automatic message (`List-Id`,
    /// `List-Unsubscribe`, `Precedence: bulk`, `Auto-Submitted`). Such mail goes to Other.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bulk: bool,
    /// How important the sender marked it: 1 high, -1 low, 0 normal (`Importance`, `X-Priority`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub importance: i8,
    /// The voting buttons the sender put on it (Exchange's verb stream), in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub voting: Vec<String>,
    /// The choice this user sent from here, once the vote has gone.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub voted: String,
    /// Signed or encrypted with S/MIME, as its header says (a claim until the message is opened
    /// and checked; an encrypted one may be signed inside too).
    #[serde(default, skip_serializing_if = "Smime::is_none")]
    pub smime: Smime,
}

/// What a message's header says protects it (S/MIME, RFC 8551 §3.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Smime {
    #[default]
    None,
    Signed,
    Encrypted,
}

impl Smime {
    pub fn is_none(&self) -> bool {
        *self == Smime::None
    }

    /// From a message's top-level `Content-Type`: its type and subtype, and its `protocol`,
    /// `smime-type` and `name` parameters.
    pub fn from_content_type(kind: &str, subtype: &str, protocol: Option<&str>, smime_type: Option<&str>, name: Option<&str>) -> Smime {
        let (kind, subtype) = (kind.to_ascii_lowercase(), subtype.to_ascii_lowercase());
        let lower = |v: Option<&str>| v.map(|s| s.trim().to_ascii_lowercase()).unwrap_or_default();
        if kind == "multipart" && subtype == "signed" {
            return match lower(protocol).as_str() {
                "application/pkcs7-signature" | "application/x-pkcs7-signature" => Smime::Signed,
                _ => Smime::None,
            };
        }
        let p7m = lower(name).ends_with(".p7m");
        if kind == "application" && (subtype == "pkcs7-mime" || subtype == "x-pkcs7-mime" || (subtype == "octet-stream" && p7m)) {
            return if lower(smime_type) == "signed-data" { Smime::Signed } else { Smime::Encrypted };
        }
        Smime::None
    }
}

fn is_zero(n: &i8) -> bool {
    *n == 0
}

impl Summary {
    /// When it came in: the server's time it was received, else its own date.
    pub fn received_at(&self) -> i64 {
        if self.received != 0 { self.received } else { self.date }
    }

    /// The subject with reply and forward prefixes removed, for conversation grouping.
    pub fn base_subject(&self) -> String {
        thread::base_subject(&self.subject)
    }
}

/// Sort newest first, with the key as a tie-breaker so the order is stable.
pub fn sort_newest_first<M: std::borrow::Borrow<Summary>>(list: &mut [M]) {
    list.sort_by(|a, b| {
        let (a, b) = (a.borrow(), b.borrow());
        b.date.cmp(&a.date).then_with(|| b.key.cmp(&a.key))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_mailboxs_folders_stay_under_it() {
        let f = |path: &str, role: FolderRole| Folder {
            account: "a".into(),
            path: path.into(),
            delimiter: Some('/'),
            role,
            selectable: true,
            unread: 0,
            total: 0,
            name: None,
            mailbox: None,
            mailbox_role: FolderRole::Other,
        };
        let mut list = vec![
            f("support@contoso.example/Projects", FolderRole::Other),
            f("Archive", FolderRole::Archive),
            f("support@contoso.example/Sent Items", FolderRole::Sent),
            f("support@contoso.example/Inbox/Escalated", FolderRole::Other),
            f("support@contoso.example", FolderRole::Other),
            f("support@contoso.example/Inbox", FolderRole::Inbox),
            f("INBOX", FolderRole::Inbox),
            f("Projects", FolderRole::Other),
        ];
        sort_folders(&mut list);
        let paths: Vec<&str> = list.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "INBOX",
                "Archive",
                "Projects",
                "support@contoso.example",
                "support@contoso.example/Inbox",
                "support@contoso.example/Inbox/Escalated",
                "support@contoso.example/Sent Items",
                "support@contoso.example/Projects",
            ]
        );
        let config = AccountConfig { shared_mailboxes: vec!["support@contoso.example".into()], ..Default::default() };
        assert_eq!(config.folder_owner("support@contoso.example/Inbox"), Some("support@contoso.example"));
        assert_eq!(config.folder_owner("SUPPORT@contoso.example"), Some("support@contoso.example"));
        assert_eq!(config.folder_owner("support@contoso.examples/Inbox"), None);
        assert_eq!(config.folder_owner("INBOX"), None);
    }

    #[test]
    fn a_folder_made_in_another_follows_it() {
        let f = |path: &str, role: FolderRole| Folder {
            account: "a".into(),
            path: path.into(),
            delimiter: Some('/'),
            role,
            selectable: true,
            ..Default::default()
        };
        let mut list = vec![
            f("Projects", FolderRole::Other),
            f("Deleted Items", FolderRole::Trash),
            f("INBOX/gzamer", FolderRole::Other),
            f("Sent Items", FolderRole::Sent),
            f("INBOX", FolderRole::Inbox),
            f("Projects/Short film", FolderRole::Other),
            f("[Gmail]/Sent Mail", FolderRole::Sent),
            f("[Gmail]", FolderRole::Other),
        ];
        sort_folders(&mut list);
        let order: Vec<&str> = list.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            order,
            ["INBOX", "INBOX/gzamer", "[Gmail]/Sent Mail", "Sent Items", "Deleted Items", "[Gmail]", "Projects", "Projects/Short film"],
        );
    }

    #[test]
    fn guesses_servers_from_the_domain() {
        // An organisation's own Microsoft app registration goes with the account (read as
        // absent from a file kept before it could), and is checked for what it could be.
        let kept: AccountConfig = serde_json::from_str(r#"{"id":"a1","name":"Zoe","email":"zoe@contoso.example","username":"zoe@contoso.example","imap":{"host":"","port":0},"smtp":{"host":"","port":0}}"#).unwrap();
        assert_eq!((kept.oauth_client_id.as_deref(), kept.oauth_tenant.as_deref()), (None, None));
        assert!(!kept.own_oauth_app() && !microsoft_sign_in_possible(&kept, &[]) && microsoft_sign_in_possible(&kept, &["microsoft".to_string()]));
        let mut own = kept.clone();
        own.oauth_client_id = Some("11111111-2222-3333-4444-555555555555".into());
        own.oauth_tenant = Some("contoso.onmicrosoft.com".into());
        let back: AccountConfig = serde_json::from_str(&serde_json::to_string(&own).unwrap()).unwrap();
        assert_eq!(
            (back.oauth_client_id.as_deref(), back.oauth_tenant.as_deref()),
            (Some("11111111-2222-3333-4444-555555555555"), Some("contoso.onmicrosoft.com"))
        );
        assert!(back.own_oauth_app() && microsoft_sign_in_possible(&back, &[]));
        assert!(!serde_json::to_string(&kept).unwrap().contains("oauth_client_id"), "left out when there's none");
        for good in ["11111111-2222-3333-4444-555555555555", "quatrelle-test"] {
            assert!(valid_oauth_client_id(good), "{good}");
        }
        for bad in ["", "not a client id!", &"x".repeat(101)] {
            assert!(!valid_oauth_client_id(bad), "{bad}");
        }
        for good in ["common", "organizations", "consumers", "contoso.onmicrosoft.com", "11111111-2222-3333-4444-555555555555"] {
            assert!(valid_oauth_tenant(good), "{good}");
        }
        for bad in ["", "bad tenant", ".contoso", "contoso.", "a/b", "-x"] {
            assert!(!valid_oauth_tenant(bad), "{bad}");
        }
        let a = AccountConfig::guess("zoe@example.org");
        assert_eq!(a.imap.host, "mail.example.org");
        assert_eq!(a.imap.port, 993);
        assert_eq!(a.smtp.port, 465);
        assert_eq!(a.username, "zoe@example.org");
        let g = AccountConfig::guess("x@gmail.com");
        assert_eq!(g.imap.host, "imap.gmail.com");
        assert_eq!(AccountConfig::guess("nonsense").imap.host, "");
    }

    #[test]
    fn folder_roles_from_names_and_attributes() {
        assert_eq!(FolderRole::from_name("INBOX", Some('/')), FolderRole::Inbox);
        assert_eq!(FolderRole::from_name("INBOX.Sent", Some('.')), FolderRole::Sent);
        // In other languages (issue #19), as the server writes them (modified UTF-7) or as read.
        assert_eq!(FolderRole::from_name("INBOX.Gesendet", Some('.')), FolderRole::Sent);
        assert_eq!(FolderRole::from_name("Gesendete Elemente", None), FolderRole::Sent);
        assert_eq!(FolderRole::from_name("Envoy&AOk-s", Some('/')), FolderRole::Sent);
        assert_eq!(FolderRole::from_name("&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-", Some('/')), FolderRole::Sent);
        assert_eq!(FolderRole::from_name("Elementos enviados", None), FolderRole::Sent);
        assert_eq!(FolderRole::from_name("Gesendet/Alt", Some('/')), FolderRole::Other, "a folder inside it is just a folder");
        assert_eq!(FolderRole::from_name("Deleted Items", None), FolderRole::Trash);
        assert_eq!(FolderRole::from_name("Projects", Some('/')), FolderRole::Other);
        assert_eq!(FolderRole::from_attribute("\\Junk"), Some(FolderRole::Junk));
        assert_eq!(FolderRole::from_attribute("\\HasChildren"), None);
    }

    #[test]
    fn folder_names_and_depth() {
        let f = Folder { path: "Projects/Caf&AOk-".into(), delimiter: Some('/'), ..Default::default() };
        assert_eq!(f.display_name(), "Café");
        assert_eq!(f.depth(), 1);
        let inbox = Folder { path: "INBOX".into(), delimiter: Some('.'), role: FolderRole::Inbox, ..Default::default() };
        assert_eq!(inbox.display_name(), "Inbox");
        assert_eq!(inbox.depth(), 0);
    }

    #[test]
    fn message_key_round_trips_through_its_id() {
        let k = MessageKey { account: "a1".into(), folder: "INBOX/Sub:folder".into(), validity: 7, uid: 42 };
        assert_eq!(MessageKey::from_id(&k.to_id()), Some(k));
        assert_eq!(MessageKey::from_id("junk"), None);
        assert_eq!(MessageKey::from_id("a:x:1:F"), None);
    }

    #[test]
    fn flags_round_trip() {
        let f = Flags::from_imap(["\\Seen", "\\FLAGGED", "$Forwarded", "$Junk", "\\Recent", "work", "WORK"]);
        assert!(f.seen && f.flagged && f.forwarded && !f.answered);
        assert_eq!(f.keywords, vec!["$Junk".to_string(), "work".to_string()]);
        assert_eq!(f.to_imap(), "\\Seen \\Flagged $Forwarded $Junk work");
        let mut f = Flags::default();
        f.set_keyword(PINNED, true);
        assert!(f.pinned() && f.has("$pinned"));
        f.set_keyword("$PINNED", false);
        assert!(!f.pinned());
    }

    #[test]
    fn keywords() {
        assert!(is_keyword("$label1") && is_keyword("to_do"));
        assert!(!is_keyword("two words") && !is_keyword("\\Seen") && !is_keyword("a(b") && !is_keyword(""));
        assert_eq!(keyword_for("To do"), "to_do");
        assert!(keyword_for("Café").starts_with("cat_") && is_keyword(&keyword_for("Café")));
        assert_eq!(keyword_for("Café"), keyword_for(" café "));
        assert_eq!(keyword_for("$x"), "cat_$x");
        assert!(is_keyword(&keyword_for("Projects: Film!")));
    }

    #[test]
    fn folder_paths() {
        assert!(is_within("A/B", "A", Some('/')));
        assert!(is_within("A", "A", Some('/')));
        assert!(!is_within("AB", "A", Some('/')));
        assert!(!is_within("A/B", "A", None));
        assert_eq!(rebase_path("A/B/C", "A/B", "X", Some('/')).as_deref(), Some("X/C"));
        assert_eq!(rebase_path("A/BC", "A/B", "X", Some('/')), None);
        assert_eq!(child_path(Some("Projects"), Some('/'), "Café").as_deref(), Ok("Projects/Caf&AOk-"));
        assert_eq!(child_path(None, Some('.'), " Plans ").as_deref(), Ok("Plans"));
        assert!(child_path(None, Some('/'), "a/b").is_err());
        assert!(child_path(Some("X"), None, "a").is_err());
        assert!(child_path(None, None, "  ").is_err());
        assert!(child_path(None, None, "*").is_err());
    }

    #[test]
    fn folders_sort_special_first() {
        let mk = |p: &str, r| Folder { path: p.into(), role: r, delimiter: Some('/'), ..Default::default() };
        let mut v =
            vec![mk("Zeta", FolderRole::Other), mk("Trash", FolderRole::Trash), mk("INBOX", FolderRole::Inbox), mk("Alpha", FolderRole::Other)];
        sort_folders(&mut v);
        let names: Vec<_> = v.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, ["INBOX", "Trash", "Alpha", "Zeta"]);
    }
}
