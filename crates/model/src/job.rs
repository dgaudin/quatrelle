//! The engine is sans-I/O: it never touches the network. It queues [`Job`]s, the network layer
//! runs them (one worker per account) and answers with [`Event`]s, which the engine applies.
//! The same split lets tests drive the engine with scripted events, and lets a web build plug in
//! a different transport.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{AccountConfig, Flag, Folder, MessageKey, organize::Rule};

/// A password in transit. Its `Debug` form is redacted so it never reaches a log.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secret(pub String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_empty() { "Secret(<empty>)" } else { "Secret(<redacted>)" })
    }
}

/// What a protected message says in clear (a message about to be signed or encrypted, a decrypted
/// one), in transit between the engine and the network layer. Its `Debug` form gives only its
/// size, so it never reaches a log.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Private(pub Vec<u8>);

impl fmt::Debug for Private {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Private(<{} bytes>)", self.0.len())
    }
}

/// A change to the mailbox the user makes, applied to the local cache at once and replayed on
/// the server when it can be reached (RFC 4549's "pending actions").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Op {
    SetFlag {
        keys: Vec<MessageKey>,
        flag: Flag,
        on: bool,
    },
    /// Add or remove a keyword (a category, `$Pinned`).
    SetKeyword {
        keys: Vec<MessageKey>,
        keyword: String,
        on: bool,
        /// A category's name, for the servers that keep categories by name (Exchange); none for
        /// other keywords.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Move to another folder of the same account.
    Move {
        keys: Vec<MessageKey>,
        to: String,
    },
    /// Delete permanently (`\Deleted` + `UID EXPUNGE`).
    Expunge {
        keys: Vec<MessageKey>,
    },
    /// Create a folder (`CREATE`, then `SUBSCRIBE`). `path` is the server's name for it.
    CreateFolder {
        path: String,
    },
    /// Rename or move a folder; folders inside it move with it (`RENAME`).
    RenameFolder {
        from: String,
        to: String,
    },
    /// Delete a folder and the mail in it (`DELETE`). The engine deletes subfolders first.
    DeleteFolder {
        path: String,
    },
    /// Delete every message in a folder for good (Deleted Items, Junk Email), including those
    /// not downloaded.
    EmptyFolder {
        path: String,
    },
}

/// Automatic replies (out of office): sent once per sender while on, between the dates when
/// set. Kept on the server, so they work while this computer is off.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vacation {
    pub enabled: bool,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub text: String,
    /// First and last day (`YYYY-MM-DD`), when limited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Exchange: the reply to people outside the organisation, when it isn't the same as `text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<String>,
    /// Exchange: who outside the organisation gets a reply: `all` (the default, empty), `known`
    /// (the user's contacts only) or `none`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub outside: String,
}

impl Vacation {
    /// The reply to people outside the organisation.
    pub fn external_text(&self) -> &str {
        self.external.as_deref().unwrap_or(&self.text)
    }
}

/// A folder's size, as Outlook's Folder Size lists it: its place (its names from the top down),
/// the bytes its own items take (its subfolders' not counted) and how many items it holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderSize {
    pub path: Vec<String>,
    pub size: u64,
    #[serde(default)]
    pub items: u32,
    /// The folder as the account lists it (its [`Folder::path`]), for one kept on this computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

/// A mailbox's limits on its server, in bytes, as Exchange sets them (0: none set): when its
/// owner is warned, when sending stops, and when mail stops coming in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxQuota {
    #[serde(default)]
    pub warning: u64,
    #[serde(default)]
    pub send: u64,
    #[serde(default)]
    pub receive: u64,
}

impl MailboxQuota {
    /// The limit the mailbox is measured against, as Outlook's bar shows it: where mail stops
    /// coming in, else where sending stops, else the warning (0: no limit at all).
    pub fn limit(&self) -> u64 {
        [self.receive, self.send, self.warning].into_iter().find(|n| *n > 0).unwrap_or(0)
    }
}

/// A `YYYY-MM-DD` date that is a real day.
pub fn is_day(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return false;
    }
    let num = |a: usize, z: usize| s.get(a..z).filter(|x| x.bytes().all(|c| c.is_ascii_digit())).and_then(|x| x.parse::<u32>().ok());
    match (num(0, 4), num(5, 7), num(8, 10)) {
        (Some(y), Some(m), Some(d)) => {
            let days = match m {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
                2 => 28,
                _ => 0,
            };
            y >= 1970 && (1..=days).contains(&d)
        }
        _ => false,
    }
}

impl Op {
    /// The messages a change touches (none for folder changes).
    pub fn keys(&self) -> &[MessageKey] {
        match self {
            Op::SetFlag { keys, .. } | Op::SetKeyword { keys, .. } | Op::Move { keys, .. } | Op::Expunge { keys } => keys,
            Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. } | Op::EmptyFolder { .. } => &[],
        }
    }

    /// Whether this changes the folder list rather than messages.
    pub fn is_folder_op(&self) -> bool {
        matches!(self, Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. })
    }
}

/// A published Quatrelle release.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// As tagged (`v0.19.0-test.15`).
    pub tag: String,
    /// `0.19.0`, and its test build (15; 0 for none).
    pub version: String,
    pub build: u32,
    /// The commit it was built from (empty when not said).
    pub commit: String,
    /// Its page (release notes and every file).
    pub page: String,
    /// The installer for this system, when there is one to offer (else the page).
    pub download: String,
    /// That installer's SHA-256 (lowercase hex) as GitHub records it, when it does.
    #[serde(default)]
    pub sha256: String,
    /// That installer's size in bytes (0 when not said).
    #[serde(default)]
    pub size: u64,
    /// The release's `SHA256SUMS.txt`, for checking a download GitHub gives no digest for.
    #[serde(default)]
    pub sums: String,
}

/// Work for the network layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "job", rename_all = "camelCase")]
pub enum Job {
    /// Start (or restart) the worker for an account. `password` is set when the user just typed
    /// it (the app stores it in the system keychain); otherwise the keychain is asked.
    Start { account: AccountConfig, password: Option<Secret> },
    /// Search the server itself (`folder`: that one only, else the account's own folders), for
    /// what the local index can't answer for: bodies that were never downloaded.
    Search { account: String, query: String, folder: Option<String> },
    /// Keep the outgoing server's own password (an account whose SMTP sign-in isn't the incoming
    /// server's: `smtp_username`), in the system keychain apart from the account's password.
    SmtpPassword { account: AccountConfig, password: Secret },
    /// Stop the worker and forget the stored password.
    Remove { account: String },
    /// Refresh the folder list and every folder.
    SyncAll { account: String },
    /// Refresh one folder (new mail, flag changes, deletions).
    SyncFolder { account: String, folder: String },
    /// Keep `more` of a folder's older messages than it keeps now, fetched from the server
    /// (mail past what's kept on this computer, shown when asked for).
    MoreMail { account: String, folder: String, more: u32 },
    /// A folder's mail older than what's kept on this computer, read on the server and held in
    /// memory only (Cached Exchange Mode's "more on the server"): the next page from `offset`,
    /// answered by [`Event::Older`].
    Older { account: String, folder: String, offset: u32 },
    /// A message's source read on the server for showing, not kept (mail older than what's
    /// kept): answered by [`Event::Body`] as any.
    ReadOnline { key: MessageKey },
    /// Search these folders on the server for `query` (the search box's words), for mail past
    /// what's kept here; the messages found come into their folders' lists, answered by
    /// [`Event::ServerSearched`], as they're found. `search` names the search: every account's
    /// share of one carries the same, and a newer one (or [`Job::StopSearch`]) stops it.
    SearchServer { account: String, folders: Vec<String>, query: String, search: u64 },
    /// Stop the servers' searches (the search box cleared, issue #4): what's found so far stays.
    StopSearch,
    /// Download a message's full source.
    FetchBody { key: MessageKey },
    /// Apply a mailbox change on the server.
    Apply { account: String, op: Op },
    /// Submit a message over SMTP, then file a copy in the Sent folder (when the server doesn't
    /// already, which the worker can't know: so always when a Sent folder exists).
    Send { account: String, id: u64, raw: Vec<u8>, from: String, recipients: Vec<String>, sent_folder: Option<String> },
    /// Store a draft (`APPEND` with `\Draft`), replacing an earlier copy.
    SaveDraft { account: String, raw: Vec<u8>, folder: String, replaces: Option<MessageKey> },
    /// A draft kept on this computer at once, filed in Drafts on the server at the next
    /// connection: Quatrelle is closing with the message being written.
    KeepDraft { account: String, raw: Vec<u8>, folder: String, replaces: Option<MessageKey> },
    /// Try to send what waits in the Outbox now.
    FlushOutbox { account: String },
    /// Download a picture a message links to (only after the user asked to see pictures).
    FetchUrl { url: String },
    /// Read a calendar subscribed to by its link (issue #7), unless it's unchanged since the
    /// version named by `etag` / `modified`; answered by [`Event::CalendarFetched`].
    FetchCalendar { account: String, calendar: String, url: String, etag: String, modified: String },
    /// Look up the server settings for an address (autoconfiguration).
    Discover { email: String },
    /// Sign an OAuth account in through the browser; it starts once signed in.
    SignIn { account: AccountConfig },
    /// Work offline (every account's connection closed, changes and mail kept to send later), or
    /// connect again.
    WorkOffline { on: bool },
    /// How much mail is kept on this computer, as Outlook's Cached Exchange Mode: what came in
    /// the last `days` (0: all of it), at most the newest `messages` of each folder (0: as many
    /// as the server lists); `full`: every message kept is downloaded whole in the end.
    SetSync { days: u32, messages: u32, full: bool },
    /// Read the account's automatic replies from the server.
    GetVacation { account: String },
    /// Change them.
    SetVacation { account: String, vacation: Vacation },
    /// Put the rules the account keeps on its server (Exchange's inbox rules, an IMAP account's
    /// Sieve script), so mail is filed while Quatrelle is closed: the ones marked `server` (with
    /// their `remote_id` when the server already has them), the ids of the rules to remove there,
    /// and the account's categories by name with the keyword each is kept as.
    SetServerRules {
        account: String,
        rules: Vec<Rule>,
        delete: Vec<String>,
        #[allow(clippy::type_complexity)]
        categories: Vec<(String, String)>,
    },
    /// Read the rules the account's server keeps, with the ids Quatrelle's own rules have there
    /// (`ours`): those made elsewhere come back with them, to be listed.
    GetServerRules { account: String, ours: Vec<String> },
    /// Turn one of the server's own rules on or off (one Quatrelle didn't make: its id is
    /// enough, everything else stays as the server keeps it).
    SetServerRuleEnabled { account: String, id: String, enabled: bool, ours: Vec<String> },
    /// Change one of the server's own rules (its id at `rule.remote_id`): only that rule goes,
    /// with the place it has on the server, so the others keep theirs.
    SetServerRule { account: String, id: String, rule: Rule, ours: Vec<String> },
    /// Remove one of the server's own rules.
    DeleteServerRule { account: String, id: String, ours: Vec<String> },
    /// The user said so: the next change of the account's rules on Exchange removes the copy of
    /// them Outlook keeps in the mailbox, which makes Exchange refuse every change (the rules
    /// Outlook runs only on its computer go with it).
    ReplaceOutlookRules { account: String },
    /// Fetch the picture kept for a person: the one their contact item carries (its id in
    /// `contact`, as the address book keeps it), or else the one the directory (Exchange,
    /// Microsoft 365) keeps for their address.
    FetchPhoto { account: String, email: String, contact: Option<String> },
    /// Sync the account's calendars and address books with its CalDAV / CardDAV server (changes
    /// made here first).
    SyncDav { account: String },
    /// Read these calendars of other people alone (just ticked), not everything again: the sync
    /// says it looked at no more ([`DavSync::partial`](crate::pim::DavSync)). None named: every
    /// calendar ticked (the calendar just opened), without the address books, notes and tasks.
    SyncCalendars { account: String, calendars: Vec<String> },
    /// Ask where Quatrelle is published for its recent releases (File › About › Check for
    /// updates); answered by [`Event::Releases`].
    CheckUpdates,
    /// Read an Outlook data file (`.pst`) into an account with no server.
    ImportPst { account: String, path: String },
    /// Compact the account's data file (Outlook's Compact Now); answered by [`Event::Compacted`].
    CompactDataFile { account: String },
    /// How much space each of the account's folders takes on this computer (Outlook's Folder
    /// Size, Local Data): the bytes its data file keeps for each. Answered by
    /// [`Event::FolderSizes`].
    LocalFolderSizes { account: String },
    /// The same on the server (Server Data), with the mailbox's limits where the server tells
    /// them. Exchange only; answered by [`Event::FolderSizes`].
    ServerFolderSizes { account: String },
    /// Write the account's mail to an Outlook data file, which opens as `name`.
    ExportPst { account: String, path: String, name: String },
    /// Answer a message's voting buttons with `choice`: a reply to its sender that names it, as
    /// the sender's mail client counts votes.
    Vote { key: MessageKey, choice: String },
    /// The rooms the account's server offers for meetings (Outlook's Room Finder): its room
    /// lists and their rooms; answered by [`Event::Rooms`]. Exchange and Microsoft 365 only.
    GetRooms { account: String },
    /// When these people and rooms are busy between `from` and `to` (Unix seconds; a day or
    /// more, two months at most), as the server's free/busy has it; answered by
    /// [`Event::Availability`]. Exchange and Microsoft 365 only.
    GetAvailability { account: String, emails: Vec<String>, from: i64, to: i64 },
    /// Keep the certificate and private key in a `.p12` / `.pfx` file (opened with `password`)
    /// for signing and decrypting the account's mail (S/MIME): the key goes to the system's
    /// keychain only. Answered by [`Event::Identity`].
    ImportIdentity { account: AccountConfig, data: Vec<u8>, password: Secret },
    /// Look these people's certificates up in the organisation's address book (Exchange's), to
    /// encrypt to them as Outlook does; answered by [`Event::DirectoryCertificates`].
    FindCertificates { account: String, emails: Vec<String> },
    /// Look `query` up in the organisation's directory (Exchange's Global Address List), by name
    /// or address as Outlook's Check Names does; answered by [`Event::DirectoryPeople`].
    SearchDirectory { account: String, query: String },
    /// Ask the language model set in Options: an OpenAI-compatible `chat/completions` under
    /// `url`, with `model` (its key, when it needs one, is in the keychain); `messages` are
    /// (role, text). Answered by [`Event::LlmAnswered`] with the same `id`.
    AskLlm { id: u64, url: String, model: String, messages: Vec<(String, String)> },
    /// List the models the language model's server at `url` offers (with the keychain's key,
    /// when there's one); answered by [`Event::LlmModels`].
    ListLlmModels { url: String },
    /// Keep the language model's key in the keychain (empty: forget it).
    SetLlmKey { key: Secret },
    /// Forget the account's certificate and key (the keychain's copy goes). Older ones kept for
    /// reading stay.
    ForgetIdentity { account: AccountConfig },
    /// Forget an older certificate kept to read mail encrypted to it (`fingerprint`, its
    /// SHA-256 as `AB:CD:…` hex pairs), and its key.
    ForgetOlderIdentity { account: AccountConfig, fingerprint: String },
    /// Sign and encrypt with an older certificate again (`fingerprint`): the one used until now is
    /// kept for reading. Answered by [`Event::Identity`].
    UseOlderIdentity { account: AccountConfig, fingerprint: String },
    /// Sign (`sign`) and/or encrypt (to `encrypt_to`, certificates as DER, the account's own
    /// among them; empty: not encrypted) a message about to be sent, with the account's key;
    /// `gcm`: AES-GCM, every recipient reads it. Its header fields go inside too (RFC 9788);
    /// encrypted with `hide_subject`, the Subject outside is `[...]`. Answered by
    /// [`Event::Protected`].
    Protect { account: AccountConfig, id: u64, raw: Private, sign: bool, encrypt_to: Vec<Vec<u8>>, gcm: bool, hide_subject: bool },
    /// Decrypt an encrypted message (its `smime.p7m`, `cms`) with the account's key, or the older
    /// one it was encrypted to. Answered by [`Event::Decrypted`].
    Decrypt { account: AccountConfig, key: MessageKey, cms: Vec<u8> },
    /// A draft of a message to be encrypted: encrypted to the account's own certificate first, so
    /// the copy in Drafts is no more readable than what will be sent, then saved as
    /// [`Job::SaveDraft`] (or kept as [`Job::KeepDraft`], `keep`).
    SealDraft { account: AccountConfig, raw: Private, folder: String, replaces: Option<MessageKey>, keep: bool },
    /// Ask a signer's certificate's authority whether it withdrew it (OCSP, else its CRL, where
    /// the certificate says): `chain` is the certificate first, then each issuer. Only with File ›
    /// Options › Safety's switch on: the authority learns whose mail is read. Answered by
    /// [`Event::Revocation`].
    CheckRevocation { chain: Vec<Vec<u8>> },
    /// The certificates whose private keys are held where Quatrelle can't read them: on the smart
    /// cards the PKCS #11 module at `module` sees (logging in with `pin`), or with no module in
    /// the system's keychain (macOS). Answered by [`Event::HeldKeys`].
    FindHeldKeys { module: Option<String>, pin: Option<Secret> },
    /// Sign and decrypt the account's mail with a key [`Job::FindHeldKeys`] found (`at`, its
    /// [`HeldKey::at`]; `pin` kept in the system's keychain for it). Answered by
    /// [`Event::Identity`].
    UseHeldKey { account: AccountConfig, at: String, cert: Vec<u8>, pin: Option<Secret> },
}

impl Job {
    pub fn account(&self) -> &str {
        match self {
            Job::Start { account, .. } | Job::SmtpPassword { account, .. } => &account.id,
            Job::FetchBody { key } | Job::ReadOnline { key } | Job::Vote { key, .. } => &key.account,
            Job::Remove { account }
            | Job::SyncAll { account }
            | Job::SyncFolder { account, .. }
            | Job::MoreMail { account, .. }
            | Job::Older { account, .. }
            | Job::SearchServer { account, .. }
            | Job::Apply { account, .. }
            | Job::Send { account, .. }
            | Job::SaveDraft { account, .. }
            | Job::KeepDraft { account, .. }
            | Job::FlushOutbox { account }
            | Job::GetVacation { account }
            | Job::SetVacation { account, .. }
            | Job::SetServerRules { account, .. }
            | Job::GetServerRules { account, .. }
            | Job::SetServerRuleEnabled { account, .. }
            | Job::SetServerRule { account, .. }
            | Job::DeleteServerRule { account, .. }
            | Job::ReplaceOutlookRules { account }
            | Job::FetchPhoto { account, .. }
            | Job::Search { account, .. }
            | Job::SyncDav { account }
            | Job::SyncCalendars { account, .. }
            | Job::ImportPst { account, .. }
            | Job::CompactDataFile { account }
            | Job::LocalFolderSizes { account }
            | Job::ServerFolderSizes { account }
            | Job::ExportPst { account, .. }
            | Job::GetRooms { account }
            | Job::FindCertificates { account, .. }
            | Job::SearchDirectory { account, .. }
            | Job::FetchCalendar { account, .. }
            | Job::GetAvailability { account, .. } => account,
            Job::FetchUrl { .. }
            | Job::Discover { .. }
            | Job::WorkOffline { .. }
            | Job::SetSync { .. }
            | Job::CheckUpdates
            | Job::AskLlm { .. }
            | Job::ListLlmModels { .. }
            | Job::SetLlmKey { .. }
            | Job::StopSearch
            | Job::CheckRevocation { .. }
            | Job::FindHeldKeys { .. } => "",
            Job::SignIn { account }
            | Job::ImportIdentity { account, .. }
            | Job::ForgetIdentity { account }
            | Job::ForgetOlderIdentity { account, .. }
            | Job::UseOlderIdentity { account, .. }
            | Job::Protect { account, .. }
            | Job::SealDraft { account, .. }
            | Job::UseHeldKey { account, .. }
            | Job::Decrypt { account, .. } => &account.id,
        }
    }
}

/// The folder Exchange's public folders are listed under. They are the organisation's, often
/// huge (years of automatic reports): as in Outlook, only the ones made Favorites are kept in
/// sync in the background; another is read from the server when it is opened.
pub const PUBLIC_FOLDERS: &str = "Public Folders";

/// An account's public folders kept in sync ([`PUBLIC_FOLDERS`]): the Favorites among them, a
/// list of paths the session writes with the account for its sync to read.
pub const FOLLOWED_PUBLIC_DOC: &str = "public-followed.json";

/// Whether a folder path is one of Exchange's public folders (or the folder they're under).
pub fn is_public_folder(path: &str) -> bool {
    path.strip_prefix(PUBLIC_FOLDERS).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// How an account's connection is doing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SyncStatus {
    #[default]
    Offline,
    Connecting,
    Syncing {
        folder: String,
    },
    /// Connected and up to date as of `at` (Unix seconds).
    Online {
        at: i64,
    },
    /// The server refused the login: the user must enter the password again.
    NeedsPassword {
        error: String,
    },
    /// The last attempt failed; the worker retries with back-off.
    Error {
        error: String,
    },
}

impl SyncStatus {
    pub fn is_online(&self) -> bool {
        matches!(self, SyncStatus::Online { .. } | SyncStatus::Syncing { .. })
    }
}

mod events;
pub use events::{Event, HeldKey, Revocation};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_never_print() {
        let j = Job::Start { account: AccountConfig::default(), password: Some(Secret("hunter2".into())) };
        assert!(!format!("{j:?}").contains("hunter2"));
        let e = Event::Decrypted { key: MessageKey::default(), content: Some(Private(b"the plan".to_vec())), authenticated: true, error: None };
        assert!(!format!("{e:?}").contains("the plan"), "what decrypted mail says never prints");
    }

    #[test]
    fn days() {
        for ok in ["2026-10-15", "2024-02-29", "2000-02-29"] {
            assert!(is_day(ok), "{ok}");
        }
        for bad in ["2026-02-29", "1900-02-29", "2026-13-01", "2026-1-01", "26-10-15", "2026-10-15T00", "", "2026-10-00", "abcd-ef-gh"] {
            assert!(!is_day(bad), "{bad}");
        }
    }

    #[test]
    fn jobs_serialize_tagged() {
        let j = Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() };
        let v = serde_json::to_value(&j).unwrap();
        assert_eq!(v["job"], "syncFolder");
        assert_eq!(serde_json::from_value::<Job>(v).unwrap(), j);
        assert_eq!(j.account(), "a1");
    }
}
