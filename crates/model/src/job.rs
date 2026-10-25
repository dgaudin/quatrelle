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
    /// Fetch the picture kept for a person: the one their contact item carries (its id in
    /// `contact`, as the address book keeps it), or else the one the directory (Exchange,
    /// Microsoft 365) keeps for their address.
    FetchPhoto { account: String, email: String, contact: Option<String> },
    /// Sync the account's calendars and address books with its CalDAV / CardDAV server (changes
    /// made here first).
    SyncDav { account: String },
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
    /// Forget the account's certificate and key (the keychain's copy goes).
    ForgetIdentity { account: AccountConfig },
    /// Sign (`sign`) and/or encrypt (to `encrypt_to`, certificates as DER, the account's own
    /// among them; empty: not encrypted) a message about to be sent, with the account's key;
    /// `gcm`: AES-GCM, every recipient reads it. Its header fields go inside too (RFC 9788);
    /// encrypted with `hide_subject`, the Subject outside is `[...]`. Answered by
    /// [`Event::Protected`].
    Protect { account: AccountConfig, id: u64, raw: Private, sign: bool, encrypt_to: Vec<Vec<u8>>, gcm: bool, hide_subject: bool },
    /// Decrypt an encrypted message (its `smime.p7m`, `cms`) with the account's key. Answered by
    /// [`Event::Decrypted`].
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
            | Job::FetchPhoto { account, .. }
            | Job::Search { account, .. }
            | Job::SyncDav { account }
            | Job::ImportPst { account, .. }
            | Job::CompactDataFile { account }
            | Job::LocalFolderSizes { account }
            | Job::ServerFolderSizes { account }
            | Job::ExportPst { account, .. }
            | Job::GetRooms { account }
            | Job::GetAvailability { account, .. } => account,
            Job::FetchUrl { .. }
            | Job::Discover { .. }
            | Job::WorkOffline { .. }
            | Job::SetSync { .. }
            | Job::CheckUpdates
            | Job::StopSearch
            | Job::CheckRevocation { .. }
            | Job::FindHeldKeys { .. } => "",
            Job::SignIn { account }
            | Job::ImportIdentity { account, .. }
            | Job::ForgetIdentity { account }
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

/// What the network layer reports back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Event {
    Status {
        account: String,
        status: SyncStatus,
    },
    /// The folder list (with counts) changed.
    Folders {
        account: String,
        folders: Vec<Folder>,
    },
    /// A folder's cached contents changed on disk: reload it.
    FolderChanged {
        account: String,
        folder: String,
    },
    /// A page of a folder's older mail read on the server ([`Job::Older`]), not kept; `more`
    /// when the server has more past it; `error` when it couldn't be asked.
    Older {
        account: String,
        folder: String,
        items: Vec<crate::Summary>,
        more: bool,
        error: Option<String>,
    },
    /// What the server found for a search (whole messages, ready to show), merged with what the
    /// local index found; `error` when it couldn't be asked (offline, a transport without one).
    Found {
        account: String,
        query: String,
        items: Vec<crate::Summary>,
        error: Option<String>,
    },
    /// A message's source arrived (it is cached on disk too).
    Body {
        key: MessageKey,
        raw: Vec<u8>,
    },
    BodyFailed {
        key: MessageKey,
        error: String,
    },
    Sent {
        account: String,
        id: u64,
    },
    SendFailed {
        account: String,
        id: u64,
        error: String,
    },
    /// The message couldn't be sent yet (offline, or the server asked to try later): it waits in
    /// the Outbox and goes out when the account is back online.
    Queued {
        account: String,
        id: u64,
        error: String,
    },
    /// A mailbox change could not be applied on the server (it was rolled back locally).
    OpFailed {
        account: String,
        op: Op,
        error: String,
    },
    /// Quatrelle's recent releases (newest first), or why they couldn't be had.
    Releases {
        releases: Vec<Release>,
        error: Option<String>,
    },
    /// A downloaded picture (`data`), or why it couldn't be fetched.
    Fetched {
        url: String,
        data: Option<Vec<u8>>,
        error: Option<String>,
    },
    /// Server settings found for an address (`None`: nothing published; guess instead).
    Discovered {
        email: String,
        config: Option<Box<AccountConfig>>,
        source: String,
    },
    /// Show this page in the browser (an OAuth sign-in page).
    OpenUrl {
        url: String,
    },
    /// An OAuth account is signed in (its token is in the keychain): start it. `user` is who
    /// the provider says signed in, when it said.
    SignedIn {
        account: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<String>,
    },
    /// How much of the account's space on the server is used and its limit, in bytes (`limit`
    /// 0: no limit published).
    Quota {
        account: String,
        used: u64,
        limit: u64,
    },
    /// The account's automatic replies as the server has them; `None` with an error when it
    /// offers none (or they couldn't be read or changed).
    Vacation {
        account: String,
        vacation: Option<Vacation>,
        error: Option<String>,
    },
    /// The server's rules were read, or brought up to date: the id each of Quatrelle's rules has
    /// there (by name), the rules it didn't make (Outlook's own, the server's web page, a Sieve
    /// script) as the server keeps them, the ids of the rules removed, or why it couldn't be
    /// done.
    ServerRules {
        account: String,
        ids: Vec<(String, String)>,
        #[serde(default)]
        others: Vec<crate::organize::ForeignRule>,
        cleared: Vec<String>,
        error: Option<String>,
    },
    /// A directory photo: its bytes and type (`image/jpeg`), or why there is none.
    Photo {
        email: String,
        media: String,
        data: Vec<u8>,
        error: Option<String>,
    },
    /// A CalDAV / CardDAV sync's result, to merge.
    DavSynced {
        account: String,
        sync: Box<crate::pim::DavSync>,
    },
    /// How a data file import is going (`done`: finished, or failed with `error`). The folders
    /// come as [`Event::Folders`] first.
    Imported {
        account: String,
        messages: usize,
        skipped: usize,
        done: bool,
        error: Option<String>,
    },
    /// A data file compacted: its size before and after, or why it couldn't be.
    Compacted {
        account: String,
        before: u64,
        after: u64,
        error: Option<String>,
    },
    /// The size of each of an account's folders ([`Job::LocalFolderSizes`], or with `server`
    /// [`Job::ServerFolderSizes`]), from the top down, and the mailbox's limits where the
    /// server tells them; or why they couldn't be read.
    FolderSizes {
        account: String,
        server: bool,
        folders: Vec<FolderSize>,
        quota: Option<MailboxQuota>,
        error: Option<String>,
    },
    /// How an export to a data file is going (`done`: finished, or failed with `error`).
    Exported {
        account: String,
        messages: usize,
        skipped: usize,
        done: bool,
        error: Option<String>,
    },
    /// A vote went (`error` `None`), or why it didn't.
    Voted {
        key: MessageKey,
        choice: String,
        error: Option<String>,
    },
    /// Something the user should see in the status bar.
    Notice {
        account: String,
        message: String,
    },
    /// The categories the account's server keeps, as Outlook made them (Exchange's master
    /// category list): each name, and its colour in Quatrelle's palette when it has one.
    Categories {
        account: String,
        categories: Vec<(String, Option<u8>)>,
    },
    /// What the server found for a [`Job::SearchServer`]: the messages, now in their folders'
    /// lists, or why it couldn't search. Sent as they're found (more to come until `done`).
    ServerSearched {
        account: String,
        query: String,
        keys: Vec<MessageKey>,
        error: Option<String>,
        done: bool,
    },
    /// The rooms the server offers ([`Job::GetRooms`]): its room lists and the rooms in them
    /// (none when the organisation set no lists up, which many on-premises servers haven't),
    /// or why they couldn't be read.
    Rooms {
        account: String,
        lists: Vec<crate::scheduling::RoomList>,
        rooms: Vec<crate::scheduling::Room>,
        error: Option<String>,
    },
    /// When people and rooms are busy over a window ([`Job::GetAvailability`]): one entry per
    /// address asked about, in that order, each with its own refusal when the server gave
    /// one; `error` when the whole request failed.
    Availability {
        account: String,
        from: i64,
        to: i64,
        people: Vec<crate::scheduling::Availability>,
        error: Option<String>,
    },
    /// What [`Job::ImportIdentity`] kept: the certificate and its issuers (DER), or why not.
    Identity {
        account: String,
        chain: Vec<Vec<u8>>,
        error: Option<String>,
    },
    /// A message [`Job::Protect`] signed and/or encrypted, ready to send, or why it couldn't be.
    Protected {
        account: String,
        id: u64,
        raw: Option<Private>,
        error: Option<String>,
    },
    /// What [`Job::Decrypt`] found inside (a MIME entity); `authenticated`: it was encrypted with
    /// AES-GCM, so it can't have been changed on the way.
    Decrypted {
        key: MessageKey,
        content: Option<Private>,
        authenticated: bool,
        error: Option<String>,
    },
    /// What [`Job::FindHeldKeys`] found, or why it couldn't look.
    HeldKeys {
        keys: Vec<HeldKey>,
        error: Option<String>,
    },
    /// What [`Job::CheckRevocation`] heard about `cert` (DER, the chain's first).
    Revocation {
        cert: Vec<u8>,
        status: Revocation,
    },
}

/// A certificate with its private key held where Quatrelle can't read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldKey {
    /// What it's called there (the card's label, the keychain's name).
    pub label: String,
    /// The certificate (DER).
    pub cert: Vec<u8>,
    /// Where the key is, for the network layer to find it again (opaque here).
    pub at: String,
}

/// Whether a certificate's authority withdrew it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Revocation {
    /// Not withdrawn; the answer holds until `until` (Unix seconds), when it says.
    Good { until: Option<i64> },
    /// Withdrawn at `at` (Unix seconds).
    Revoked { at: i64 },
    /// No answer to go by (none given, or none to trust), and why.
    Unknown { why: String },
}

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
